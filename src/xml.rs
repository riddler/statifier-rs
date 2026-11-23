//! XML source in, a generic element tree out.
//!
//! The first step of reading a chart: this layer knows XML and nothing else,
//! and a later one gives the names meaning. It is a small hand-written reader
//! of the XML that SCXML documents use and needs no XML library.
//!
//! What it does, beyond splitting the markup:
//!
//! - Attribute values are normalized per XML 1.0 section 3.3.3: a literal TAB,
//!   LF or CR in the raw value becomes a space, a literal CR LF pair becomes
//!   one space, a character reference keeps its decoded character, and nothing
//!   is trimmed or collapsed.
//! - Character data folds line breaks per XML 1.0 section 2.11: a literal CR
//!   LF pair and a literal lone CR each become one LF, inside a CDATA section
//!   too, while a CR decoded from `&#xD;` stays a CR. Both read the raw
//!   source, so a CR followed by a comment is a lone CR.
//! - The five predefined entities and decimal and hex character references
//!   expand in text and in attribute values. Any other entity reference is
//!   refused in a document with no document type declaration, where it can
//!   name no declared entity; in a document with one, a reference to any
//!   other name is kept as its literal text (`&name;`), since the declarations
//!   are not read and nothing expands it.
//! - Every element carries the namespace its name resolves to under the
//!   `xmlns` and `xmlns:prefix` declarations in scope, or `None` when none is.
//!   An empty declaration binds the empty string and undeclares nothing.
//!
//! What it does not do: validate names against any vocabulary, drop
//! duplicate attributes, drop whitespace-only text, or read a document type
//! definition. Comments, processing instructions, the XML declaration and a
//! document type declaration produce no nodes, and anything outside the root
//! element is discarded. A document type declaration ends where the
//! reference ends one: at the `>` that balances the angle brackets,
//! `<!DOCTYPE` included; the reference also finds an end at the first `>`
//! outside square brackets, and a document whose two ends differ with markup
//! between them is refused.
//!
//! The document is read as UTF-8. A byte sequence that is not UTF-8 and a
//! leading byte order mark are refused. Malformed input is an `Err`, never a
//! panic: every refusal carries a reason, a message for a person that ends
//! with the line and column, and the zero-width span where reading stopped.
//! The tree is built with an explicit stack and dropped without recursion, so
//! nesting depth never touches the call stack.
//!
//! Offsets are byte offsets into the document; lines and columns are 1-based,
//! columns count code points, and only a line feed starts a new line.

mod cursor;
mod markup;
mod reader;
mod tags;
mod text;

#[cfg(test)]
mod tests;

use cursor::Cursor;

/// A source span: 1-based lines and columns, 0-based byte offsets and an
/// exclusive end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Location {
    /// The line the span starts on.
    pub(crate) start_line: usize,
    /// The column, in code points, the span starts at.
    pub(crate) start_column: usize,
    /// The byte offset the span starts at.
    pub(crate) start_offset: usize,
    /// The line the span ends on.
    pub(crate) end_line: usize,
    /// The column the span ends at.
    pub(crate) end_column: usize,
    /// The byte offset just past the span.
    pub(crate) end_offset: usize,
}

impl Location {
    /// The raw text this span covers in `source`, or `None` when the span
    /// does not lie inside it.
    pub(crate) fn slice<'s>(&self, source: &'s str) -> Option<&'s str> {
        source.get(self.start_offset..self.end_offset)
    }
}

/// One attribute, in document order. `value` is normalized and its
/// references expanded; `location` covers `name="value"` with its quotes, and
/// `value_location` only the raw text inside the quotes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Attribute {
    /// The qualified name as written.
    pub(crate) name: String,
    /// The normalized, expanded value.
    pub(crate) value: String,
    /// The span of the whole attribute.
    pub(crate) location: Location,
    /// The span of the raw value inside its quotes.
    pub(crate) value_location: Location,
}

/// An element. `name` is the qualified name as written, prefix included;
/// `namespace` is the URI that name resolves to (the empty string when an
/// empty declaration is in scope for it), or `None` when no declaration is.
/// `attributes` keeps document order and every duplicate; `children`
/// interleaves elements and text runs. `location` runs from the `<` of the
/// start tag to just past the `>` of the end tag (the tag itself when it
/// closes itself).
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Element {
    /// The qualified name as written.
    pub(crate) name: String,
    /// The namespace the name resolves to, if any.
    pub(crate) namespace: Option<String>,
    /// The attributes, in document order.
    pub(crate) attributes: Vec<Attribute>,
    /// The child elements and text runs, in document order.
    pub(crate) children: Vec<Node>,
    /// The span of the element.
    pub(crate) location: Location,
}

impl Drop for Element {
    // Drops the subtree with a work list rather than recursion, so a deeply
    // nested document cannot overflow the stack when its tree is dropped.
    fn drop(&mut self) {
        let mut pending = std::mem::take(&mut self.children);
        while let Some(node) = pending.pop() {
            if let Node::Element(mut element) = node {
                pending.append(&mut element.children);
            }
        }
    }
}

/// A run of character data between two tags, coalesced across any comment,
/// processing instruction or CDATA section inside it. `value` is expanded and
/// line-break folded; `location` covers the raw run, from the end of the tag
/// before it to the `<` of the tag after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Text {
    /// The expanded, folded text.
    pub(crate) value: String,
    /// The span of the raw run.
    pub(crate) location: Location,
}

/// A child of an element.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Node {
    /// A child element.
    Element(Element),
    /// A run of character data.
    Text(Text),
}

/// Why a document was refused.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reason {
    /// The bytes are not UTF-8.
    InvalidEncoding,
    /// The document starts with a byte order mark.
    ByteOrderMark,
    /// The document holds no root element.
    MissingRoot,
    /// Something other than markup comes before the root element.
    ContentBeforeRoot,
    /// Something other than markup comes after the root element.
    ContentAfterRoot,
    /// The document ends inside a tag.
    UnexpectedEnd,
    /// A name was due and none is there.
    ExpectedName,
    /// An attribute name is not followed by `=`.
    ExpectedEquals,
    /// An attribute value is not quoted.
    ExpectedQuote,
    /// Two attributes, or a name and an attribute, have nothing between them.
    ExpectedWhitespace,
    /// An end tag holds more than its name.
    ExpectedTagEnd,
    /// An attribute value holds a `<`.
    LtInAttributeValue,
    /// An end tag names an element other than the open one.
    MismatchedEndTag,
    /// The document ends with an element still open.
    UnclosedElement,
    /// A comment is never closed.
    UnterminatedComment,
    /// A comment holds `--`.
    DoubleHyphenInComment,
    /// A CDATA section is never closed.
    UnterminatedCdata,
    /// A processing instruction is never closed.
    UnterminatedProcessingInstruction,
    /// An XML declaration stands anywhere but the start of the document.
    MisplacedXmlDeclaration,
    /// The XML declaration does not follow its grammar.
    MalformedXmlDeclaration,
    /// The XML declaration names an encoding other than UTF-8.
    UnsupportedEncoding,
    /// A processing instruction target spells `xml` in another case.
    ReservedTarget,
    /// A document type declaration is never closed.
    UnterminatedDoctype,
    /// A document type declaration has two different ends with markup
    /// between them.
    AmbiguousDoctype,
    /// An entity reference names no entity this reader expands.
    UnknownEntity,
    /// An `&` starts no reference.
    MalformedReference,
    /// A character reference names no character XML allows.
    InvalidCharacterReference,
    /// A character XML does not allow.
    InvalidCharacter,
    /// Character data holds `]]>`.
    CdataEndInText,
}

/// A refusal: the reason, a message for a person that ends with the line and
/// column, and the zero-width span where reading stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParseError {
    /// Why the document was refused.
    pub(crate) reason: Reason,
    /// What is wrong, for a person.
    pub(crate) message: String,
    /// Where reading stopped.
    pub(crate) location: Location,
}

/// Parses `source` into its root element, or answers why it cannot.
///
/// Never panics: every refusal is an `Err` carrying the place where reading
/// stopped.
pub(crate) fn parse(source: &[u8]) -> Result<Element, ParseError> {
    let text = match std::str::from_utf8(source) {
        Ok(text) => text,
        Err(error) => {
            let valid = source
                .get(..error.valid_up_to())
                .and_then(|bytes| std::str::from_utf8(bytes).ok())
                .unwrap_or("");
            let mut cursor = Cursor::new(valid);
            cursor.move_to_end();
            return Err(cursor.refuse(Reason::InvalidEncoding, "the document is not valid UTF-8"));
        }
    };
    if text.starts_with('\u{FEFF}') {
        return Err(
            Cursor::new(text).refuse(Reason::ByteOrderMark, "a byte order mark is not read")
        );
    }
    reader::Reader::new(text).document()
}
