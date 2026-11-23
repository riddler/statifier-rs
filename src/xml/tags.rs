//! Start tags, end tags, attributes, names and the namespace scope.

use super::cursor::{Mark, refuse_at};
use super::reader::Reader;
use super::{Attribute, Element, Location, Node, ParseError, Reason};
use std::collections::BTreeMap;
use std::rc::Rc;

/// The namespace declarations in scope at an element: the default namespace
/// (`None` when none is declared) and the bound prefixes. A value binds as
/// written, the empty string included: an empty declaration binds the empty
/// URI and undeclares nothing.
#[derive(Debug, Default)]
pub(super) struct Scope {
    default: Option<String>,
    prefixes: BTreeMap<String, String>,
}

impl Scope {
    /// The scope for an element's subtree: `inherited` with the element's own
    /// `xmlns` and `xmlns:prefix` attributes folded over it.
    fn declare(inherited: Rc<Scope>, attributes: &[Attribute]) -> Rc<Scope> {
        let mut declared = attributes
            .iter()
            .filter(|attribute| attribute.name.starts_with("xmlns"))
            .peekable();
        if declared.peek().is_none() {
            return inherited;
        }
        let mut scope = Scope {
            default: inherited.default.clone(),
            prefixes: inherited.prefixes.clone(),
        };
        for attribute in declared {
            if attribute.name == "xmlns" {
                scope.default = Some(attribute.value.clone());
            } else if let Some(prefix) = attribute.name.strip_prefix("xmlns:") {
                if !prefix.is_empty() {
                    scope
                        .prefixes
                        .insert(prefix.to_owned(), attribute.value.clone());
                }
            }
        }
        Rc::new(scope)
    }

    /// The namespace a qualified element name resolves to, if any.
    fn resolve(&self, name: &str) -> Option<String> {
        match name.split_once(':') {
            None => self.default.clone(),
            Some((prefix, _)) => self.prefixes.get(prefix).cloned(),
        }
    }
}

/// An element whose end tag is still to come.
pub(super) struct Open {
    /// The qualified name as written.
    pub(super) name: String,
    namespace: Option<String>,
    attributes: Vec<Attribute>,
    /// The children read so far.
    pub(super) children: Vec<Node>,
    /// Where its start tag begins.
    pub(super) start: Mark,
    /// The namespace scope of its subtree.
    pub(super) scope: Rc<Scope>,
}

impl Open {
    /// The finished element, spanning `location`.
    pub(super) fn finish(self, location: Location) -> Element {
        Element {
            name: self.name,
            namespace: self.namespace,
            attributes: self.attributes,
            children: self.children,
            location,
        }
    }
}

impl Reader<'_> {
    /// Reads a name at the read position.
    pub(super) fn read_name(&mut self) -> Result<String, ParseError> {
        let start = self.cursor.offset();
        match self.cursor.peek() {
            Some(c) if is_name_start(c) => {
                self.cursor.bump();
                self.cursor.skip_while(is_name_char);
                Ok(self.cursor.slice(start, self.cursor.offset()).to_owned())
            }
            Some(_) => Err(self.cursor.refuse(Reason::ExpectedName, "expected a name")),
            None => Err(self.unexpected_end()),
        }
    }

    /// Reads a start tag, `<name (S attribute)* S? (> | />)`, under the scope
    /// it inherits; answers the open element and whether the tag closed
    /// itself.
    pub(super) fn read_start_tag(
        &mut self,
        inherited: Rc<Scope>,
    ) -> Result<(Open, bool), ParseError> {
        let start = self.cursor.mark();
        self.cursor.skip("<");
        let name = self.read_name()?;
        let attributes = self.read_attributes()?;
        let closed = self.cursor.skip("/>");
        if !closed {
            self.cursor.skip(">");
        }
        let scope = Scope::declare(inherited, &attributes);
        let namespace = scope.resolve(&name);
        let open = Open {
            name,
            namespace,
            attributes,
            children: Vec::new(),
            start,
            scope,
        };
        Ok((open, closed))
    }

    fn read_attributes(&mut self) -> Result<Vec<Attribute>, ParseError> {
        let mut attributes = Vec::new();
        loop {
            let spaced = self.cursor.skip_whitespace();
            if self.cursor.at("/>") || self.cursor.at(">") {
                return Ok(attributes);
            }
            if self.cursor.eos() {
                return Err(self.unexpected_end());
            }
            if !spaced {
                return Err(self.cursor.refuse(
                    Reason::ExpectedWhitespace,
                    "expected whitespace before an attribute",
                ));
            }
            attributes.push(self.read_attribute()?);
        }
    }

    // Reads `name S? = S? quoted-value`.
    fn read_attribute(&mut self) -> Result<Attribute, ParseError> {
        let start = self.cursor.mark();
        let name = self.read_name()?;
        self.cursor.skip_whitespace();
        if !self.cursor.skip("=") {
            if self.cursor.eos() {
                return Err(self.unexpected_end());
            }
            let message = format!("expected '=' after attribute {name}");
            return Err(self.cursor.refuse(Reason::ExpectedEquals, &message));
        }
        self.cursor.skip_whitespace();
        let quote = match self.cursor.peek() {
            Some(quote @ ('"' | '\'')) => quote,
            Some(_) => {
                let message = format!("expected a quoted value for attribute {name}");
                return Err(self.cursor.refuse(Reason::ExpectedQuote, &message));
            }
            None => return Err(self.unexpected_end()),
        };
        self.cursor.bump();
        let value_start = self.cursor.mark();
        let mut value = String::new();
        self.read_value(quote, &mut value)?;
        let value_location = self.cursor.span(value_start);
        self.cursor.bump();
        Ok(Attribute {
            name,
            value,
            location: self.cursor.span(start),
            value_location,
        })
    }

    /// Reads `</name S? >` and closes `open` with it.
    pub(super) fn read_end_tag(&mut self, open: Option<Open>) -> Result<Element, ParseError> {
        let start = self.cursor.mark();
        self.cursor.skip("</");
        let name = self.read_name()?;
        self.cursor.skip_whitespace();
        if !self.cursor.skip(">") {
            if self.cursor.eos() {
                return Err(self.unexpected_end());
            }
            return Err(self
                .cursor
                .refuse(Reason::ExpectedTagEnd, "expected '>' to end the tag"));
        }
        match open {
            Some(open) if open.name == name => {
                let location = self.cursor.span(open.start);
                Ok(open.finish(location))
            }
            open => {
                let due = open.map_or_else(
                    || "no open element".to_owned(),
                    |open| format!("</{}>", open.name),
                );
                let message = format!("end tag </{name}> where {due} was due");
                Err(refuse_at(start, Reason::MismatchedEndTag, &message))
            }
        }
    }

    /// A refusal for a document that ends inside a tag.
    pub(super) fn unexpected_end(&mut self) -> ParseError {
        self.cursor
            .refuse(Reason::UnexpectedEnd, "the document ends inside a tag")
    }
}

/// XML 1.0's `NameStartChar` production.
pub(super) fn is_name_start(c: char) -> bool {
    matches!(c,
        ':' | 'A'..='Z' | '_' | 'a'..='z'
        | '\u{C0}'..='\u{D6}' | '\u{D8}'..='\u{F6}' | '\u{F8}'..='\u{2FF}'
        | '\u{370}'..='\u{37D}' | '\u{37F}'..='\u{1FFF}' | '\u{200C}'..='\u{200D}'
        | '\u{2070}'..='\u{218F}' | '\u{2C00}'..='\u{2FEF}' | '\u{3001}'..='\u{D7FF}'
        | '\u{F900}'..='\u{FDCF}' | '\u{FDF0}'..='\u{FFFD}' | '\u{10000}'..='\u{EFFFF}')
}

/// XML 1.0's `NameChar` production.
pub(super) fn is_name_char(c: char) -> bool {
    is_name_start(c)
        || matches!(c,
            '-' | '.' | '0'..='9' | '\u{B7}' | '\u{300}'..='\u{36F}' | '\u{203F}'..='\u{2040}')
}

/// Whether `text` is one whole XML name.
pub(super) fn is_whole_name(text: &str) -> bool {
    let mut chars = text.chars();
    chars.next().is_some_and(is_name_start) && chars.all(is_name_char)
}
