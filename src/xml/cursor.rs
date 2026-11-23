//! The read position in one document, and the refusals made at it.
//!
//! The position is a byte offset; its line and column are counted on demand,
//! forward from the last position asked for, so a document is counted once
//! however often a span is taken.

use super::{Location, ParseError, Reason};

/// A position: the byte offset and the line and column it sits at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Mark {
    /// The byte offset.
    pub(super) offset: usize,
    /// The 1-based line.
    pub(super) line: usize,
    /// The 1-based column, in code points.
    pub(super) column: usize,
}

impl Mark {
    /// The start of a document.
    pub(super) const START: Mark = Mark {
        offset: 0,
        line: 1,
        column: 1,
    };
}

/// A read position over one document.
pub(super) struct Cursor<'a> {
    text: &'a str,
    offset: usize,
    known: Mark,
}

impl<'a> Cursor<'a> {
    /// A cursor at the start of `text`.
    pub(super) fn new(text: &'a str) -> Self {
        Cursor {
            text,
            offset: 0,
            known: Mark::START,
        }
    }

    /// The byte offset of the read position.
    pub(super) fn offset(&self) -> usize {
        self.offset
    }

    /// Whether the whole document has been read.
    pub(super) fn eos(&self) -> bool {
        self.offset >= self.text.len()
    }

    /// The text from the read position on.
    pub(super) fn rest(&self) -> &'a str {
        self.text.get(self.offset..).unwrap_or("")
    }

    /// The text between two byte offsets, or the empty string when they do
    /// not bound a slice of it.
    pub(super) fn slice(&self, from: usize, to: usize) -> &'a str {
        self.text.get(from..to).unwrap_or("")
    }

    /// Whether `literal` is at the read position; nothing is consumed.
    pub(super) fn at(&self, literal: &str) -> bool {
        self.rest().starts_with(literal)
    }

    /// The next character, not consumed.
    pub(super) fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    /// Consumes `literal` when it is at the read position.
    pub(super) fn skip(&mut self, literal: &str) -> bool {
        let found = self.at(literal);
        if found {
            self.offset += literal.len();
        }
        found
    }

    /// Consumes the next character.
    pub(super) fn bump(&mut self) -> Option<char> {
        let next = self.peek()?;
        self.offset += next.len_utf8();
        Some(next)
    }

    /// Consumes characters while `keep` holds; answers whether any were.
    pub(super) fn skip_while(&mut self, keep: impl Fn(char) -> bool) -> bool {
        let rest = self.rest();
        let taken = rest.find(|c: char| !keep(c)).unwrap_or(rest.len());
        self.offset += taken;
        taken > 0
    }

    /// Consumes XML whitespace; answers whether there was any.
    pub(super) fn skip_whitespace(&mut self) -> bool {
        self.skip_while(is_whitespace)
    }

    /// The byte offset of the next `literal` at or after the read position.
    pub(super) fn find(&self, literal: &str) -> Option<usize> {
        self.rest().find(literal).map(|at| self.offset + at)
    }

    /// Moves the read position to a byte offset.
    pub(super) fn move_to(&mut self, offset: usize) {
        self.offset = offset;
    }

    /// Moves the read position to the end of the document.
    pub(super) fn move_to_end(&mut self) {
        self.offset = self.text.len();
    }

    /// The read position with its line and column.
    pub(super) fn mark(&mut self) -> Mark {
        if self.offset < self.known.offset {
            self.known = Mark::START;
        }
        let chunk = self
            .text
            .as_bytes()
            .get(self.known.offset..self.offset)
            .unwrap_or(&[]);
        let mut mark = self.known;
        for &byte in chunk {
            if byte == b'\n' {
                mark.line += 1;
                mark.column = 1;
            } else if byte & 0xC0 != 0x80 {
                mark.column += 1;
            }
        }
        mark.offset = self.offset;
        self.known = mark;
        mark
    }

    /// The span from `from` to the read position.
    pub(super) fn span(&mut self, from: Mark) -> Location {
        span(from, self.mark())
    }

    /// A refusal at the read position.
    pub(super) fn refuse(&mut self, reason: Reason, message: &str) -> ParseError {
        let at = self.mark();
        refuse_at(at, reason, message)
    }
}

/// The span between two marks.
pub(super) fn span(from: Mark, to: Mark) -> Location {
    Location {
        start_line: from.line,
        start_column: from.column,
        start_offset: from.offset,
        end_line: to.line,
        end_column: to.column,
        end_offset: to.offset,
    }
}

/// A refusal at `at`: its message ends with the line and column, and its
/// span is zero-width.
pub(super) fn refuse_at(at: Mark, reason: Reason, message: &str) -> ParseError {
    ParseError {
        reason,
        message: format!("{message} (line {}, column {})", at.line, at.column),
        location: span(at, at),
    }
}

/// XML's `S` production: space, tab, line feed, carriage return.
pub(super) fn is_whitespace(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r')
}
