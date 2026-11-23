//! One parse of one document: the prolog, the element tree and the epilogue.
//!
//! The tree is built with an explicit stack, so nesting depth never touches
//! the call stack.

use super::cursor::{Cursor, Mark};
use super::tags::{Open, Scope};
use super::{Element, Node, ParseError, Reason, Text};
use std::rc::Rc;

/// The state of one parse.
pub(super) struct Reader<'a> {
    /// The read position.
    pub(super) cursor: Cursor<'a>,
    /// The elements whose end tags are still to come, innermost last.
    stack: Vec<Open>,
    /// The text run open since the last tag: where it starts and its value.
    run: Option<(Mark, String)>,
    /// Where the last tag ended: the start of the next text run.
    tag_end: Mark,
    /// Whether a document type declaration was read.
    pub(super) doctype: bool,
}

impl<'a> Reader<'a> {
    /// A reader at the start of `text`.
    pub(super) fn new(text: &'a str) -> Self {
        Reader {
            cursor: Cursor::new(text),
            stack: Vec::new(),
            run: None,
            tag_end: Mark::START,
            doctype: false,
        }
    }

    /// Reads the whole document and answers its root element.
    pub(super) fn document(mut self) -> Result<Element, ParseError> {
        self.skip_misc(true)?;
        if self.cursor.eos() {
            return Err(self.cursor.refuse(Reason::MissingRoot, "no root element"));
        }
        if !self.at_start_tag() {
            return Err(self.cursor.refuse(
                Reason::ContentBeforeRoot,
                "character data before the root element",
            ));
        }
        let root = self.read_root()?;
        self.skip_misc(false)?;
        if !self.cursor.eos() {
            return Err(self
                .cursor
                .refuse(Reason::ContentAfterRoot, "content after the root element"));
        }
        Ok(root)
    }

    // Whether a start tag begins at the read position: a `<` and a character
    // that can start a name.
    fn at_start_tag(&self) -> bool {
        let mut chars = self.cursor.rest().chars();
        chars.next() == Some('<') && chars.next().is_some_and(super::tags::is_name_start)
    }

    fn read_root(&mut self) -> Result<Element, ParseError> {
        if let Some(root) = self.open_tag()? {
            return Ok(root);
        }
        loop {
            if let Some(root) = self.step()? {
                return Ok(root);
            }
        }
    }

    // Reads one construct inside the root element; answers the root once its
    // end tag is read.
    fn step(&mut self) -> Result<Option<Element>, ParseError> {
        if self.cursor.eos() {
            let name = self.stack.last().map_or("", |open| open.name.as_str());
            let message = format!("element <{name}> is never closed");
            return Err(self.cursor.refuse(Reason::UnclosedElement, &message));
        }
        if self.cursor.at("</") {
            return self.close_tag();
        }
        if self.cursor.at("<!--") {
            self.skip_comment()?;
        } else if self.cursor.at("<![CDATA[") {
            let (start, mut value) = self.take_run();
            self.read_cdata(&mut value)?;
            self.run = Some((start, value));
        } else if self.cursor.at("<?") {
            self.skip_processing_instruction()?;
        } else if self.cursor.at("<") {
            return self.open_tag();
        } else {
            let (start, mut value) = self.take_run();
            self.read_character_data(&mut value)?;
            self.run = Some((start, value));
        }
        Ok(None)
    }

    // Reads a start tag; answers the root when the root closes itself.
    fn open_tag(&mut self) -> Result<Option<Element>, ParseError> {
        self.flush();
        let inherited = self
            .stack
            .last()
            .map_or_else(|| Rc::new(Scope::default()), |open| Rc::clone(&open.scope));
        let (open, closed) = self.read_start_tag(inherited)?;
        self.tag_end = self.cursor.mark();
        if closed {
            let location = self.cursor.span(open.start);
            return Ok(self.adopt(open.finish(location)));
        }
        self.stack.push(open);
        Ok(None)
    }

    // Reads an end tag; answers the root when it closes the root.
    fn close_tag(&mut self) -> Result<Option<Element>, ParseError> {
        self.flush();
        let open = self.stack.pop();
        let element = self.read_end_tag(open)?;
        self.tag_end = self.cursor.mark();
        Ok(self.adopt(element))
    }

    // Appends a finished element to its parent, or answers it when it is the
    // root.
    fn adopt(&mut self, element: Element) -> Option<Element> {
        match self.stack.last_mut() {
            Some(parent) => {
                parent.children.push(Node::Element(element));
                None
            }
            None => Some(element),
        }
    }

    // The open text run, or a new one starting where the last tag ended, as
    // the reference's spans do: a comment or processing instruction before
    // the first character data is inside the span.
    fn take_run(&mut self) -> (Mark, String) {
        self.run
            .take()
            .unwrap_or_else(|| (self.tag_end, String::new()))
    }

    // Closes the open text run, if any, at the read position: the `<` of the
    // next tag.
    fn flush(&mut self) {
        if let Some((start, value)) = self.run.take() {
            let location = self.cursor.span(start);
            if let Some(parent) = self.stack.last_mut() {
                parent.children.push(Node::Text(Text { value, location }));
            }
        }
    }
}
