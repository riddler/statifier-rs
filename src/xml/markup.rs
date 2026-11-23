//! The constructs that produce no node: comments, processing instructions,
//! the XML declaration and the document type declaration.

use super::cursor::{Mark, is_whitespace, refuse_at};
use super::reader::Reader;
use super::{ParseError, Reason};

impl Reader<'_> {
    /// Skips whitespace, comments and processing instructions outside the
    /// root element; before it (`prolog`) also one document type
    /// declaration. Stops at the first other character.
    pub(super) fn skip_misc(&mut self, prolog: bool) -> Result<(), ParseError> {
        let mut doctype = prolog;
        loop {
            self.cursor.skip_whitespace();
            if self.cursor.at("<!--") {
                self.skip_comment()?;
            } else if self.cursor.at("<?") {
                self.skip_processing_instruction()?;
            } else if doctype && self.cursor.at("<!DOCTYPE") {
                self.skip_doctype()?;
                doctype = false;
            } else {
                return Ok(());
            }
        }
    }

    /// Skips `<!-- ... -->`, refusing `--` inside it as XML 1.0 section 2.5
    /// does.
    pub(super) fn skip_comment(&mut self) -> Result<(), ParseError> {
        self.cursor.skip("<!--");
        let Some(hyphens) = self.cursor.find("--") else {
            self.cursor.move_to_end();
            return Err(self
                .cursor
                .refuse(Reason::UnterminatedComment, "a comment is never closed"));
        };
        self.cursor.move_to(hyphens + 2);
        if self.cursor.skip(">") {
            return Ok(());
        }
        self.cursor.move_to(hyphens);
        Err(self.cursor.refuse(
            Reason::DoubleHyphenInComment,
            "'--' is not allowed inside a comment",
        ))
    }

    /// Skips `<? ... ?>`. The target `xml` is the XML declaration, read only
    /// at the very start of the document; any other spelling of it is
    /// reserved.
    pub(super) fn skip_processing_instruction(&mut self) -> Result<(), ParseError> {
        let start = self.cursor.mark();
        self.cursor.skip("<?");
        let target = self.read_name()?;
        if target == "xml" && start.offset == 0 {
            return self.read_declaration(start);
        }
        if target == "xml" {
            return Err(refuse_at(
                start,
                Reason::MisplacedXmlDeclaration,
                "the XML declaration is allowed only at the start of the document",
            ));
        }
        if target.eq_ignore_ascii_case("xml") {
            let message = format!("the processing instruction target {target} is reserved");
            return Err(refuse_at(start, Reason::ReservedTarget, &message));
        }
        match self.cursor.find("?>") {
            Some(end) => {
                self.cursor.move_to(end + 2);
                Ok(())
            }
            None => {
                self.cursor.move_to_end();
                Err(self.cursor.refuse(
                    Reason::UnterminatedProcessingInstruction,
                    "a processing instruction is never closed",
                ))
            }
        }
    }

    // Reads what follows `<?xml` in the XML declaration (XML 1.0 production
    // 23), refusing a declared encoding other than UTF-8.
    fn read_declaration(&mut self, start: Mark) -> Result<(), ParseError> {
        let Some((length, encoding)) = declaration(self.cursor.rest()) else {
            return Err(refuse_at(
                start,
                Reason::MalformedXmlDeclaration,
                "the XML declaration is malformed",
            ));
        };
        if let Some(encoding) = encoding {
            if !encoding.eq_ignore_ascii_case("UTF-8") {
                let message =
                    format!("the document declares the encoding {encoding}; only UTF-8 is read");
                return Err(refuse_at(start, Reason::UnsupportedEncoding, &message));
            }
        }
        self.cursor.move_to(self.cursor.offset() + length);
        Ok(())
    }

    // Skips `<!DOCTYPE ... >` as the reference reads one: the declaration
    // ends at the `>` that brings the count of open angle brackets,
    // `<!DOCTYPE` included, to zero. The reference also finds the end a
    // second way, at the first `>` outside square brackets, and reads the
    // document only when the two ends agree or nothing between them is
    // markup. Neither reading treats a quote specially, and the declarations
    // inside are not read: only the predefined entities expand.
    fn skip_doctype(&mut self) -> Result<(), ParseError> {
        let start = self.cursor.mark();
        self.cursor.skip("<!DOCTYPE");
        let (Some(angle), Some(bracket)) = doctype_ends(self.cursor.rest(), self.cursor.offset())
        else {
            self.cursor.move_to_end();
            return Err(self.cursor.refuse(
                Reason::UnterminatedDoctype,
                "a document type declaration is never closed",
            ));
        };
        let (low, high) = (angle.min(bracket), angle.max(bracket));
        if self.cursor.slice(low, high).contains('<') {
            return Err(refuse_at(
                start,
                Reason::AmbiguousDoctype,
                "the document type declaration has two different ends with markup between them",
            ));
        }
        self.cursor.move_to(angle);
        self.doctype = true;
        Ok(())
    }
}

/// The two ends of a document type declaration, read from `rest`, the text
/// just after `<!DOCTYPE`, which starts at byte offset `base`: the offset
/// just past the `>` that balances the angle brackets, and just past the
/// first `>` outside square brackets (a `]` never taking the depth below
/// zero). Either is `None` when the text ends first.
fn doctype_ends(rest: &str, base: usize) -> (Option<usize>, Option<usize>) {
    let mut angles: usize = 1;
    let mut depth: usize = 0;
    let mut angle = None;
    let mut bracket = None;
    for (index, byte) in rest.bytes().enumerate() {
        match byte {
            b'<' => angles += 1,
            b'[' => depth += 1,
            b']' => depth = depth.saturating_sub(1),
            b'>' => {
                if depth == 0 && bracket.is_none() {
                    bracket = Some(base + index + 1);
                }
                if angles == 1 && angle.is_none() {
                    angle = Some(base + index + 1);
                }
                angles = angles.saturating_sub(1);
            }
            _ => {}
        }
        if angle.is_some() && bracket.is_some() {
            break;
        }
    }
    (angle, bracket)
}

/// Reads an XML declaration's body, from just after `<?xml` through `?>`:
/// `S version Eq "1.n"`, then an optional `S encoding Eq "name"` and an
/// optional `S standalone Eq "yes|no"`, then `S? ?>`. Answers the bytes it
/// covers and the declared encoding, or `None` when it is malformed.
fn declaration(text: &str) -> Option<(usize, Option<&str>)> {
    let bytes = text.as_bytes();
    let mut at = attribute(bytes, 0, "version", is_version)?;
    let mut encoding = None;
    if let Some(next) = attribute(bytes, at, "encoding", is_encoding_name) {
        encoding = text.get(at..next).and_then(quoted_value);
        at = next;
    }
    if let Some(next) = attribute(bytes, at, "standalone", |value| {
        value == b"yes" || value == b"no"
    }) {
        at = next;
    }
    at = skip_spaces(bytes, at);
    if bytes.get(at..at + 2) == Some(b"?>".as_slice()) {
        Some((at + 2, encoding))
    } else {
        None
    }
}

// Reads `S+ name S? = S? quoted` at `at`, its value admitted by `valid`;
// answers the offset just past the closing quote.
fn attribute(bytes: &[u8], at: usize, name: &str, valid: impl Fn(&[u8]) -> bool) -> Option<usize> {
    let after_space = skip_spaces(bytes, at);
    if after_space == at {
        return None;
    }
    let after_name = after_space + name.len();
    if bytes.get(after_space..after_name) != Some(name.as_bytes()) {
        return None;
    }
    let mut at = skip_spaces(bytes, after_name);
    if bytes.get(at) != Some(&b'=') {
        return None;
    }
    at = skip_spaces(bytes, at + 1);
    let quote = *bytes
        .get(at)
        .filter(|&&byte| byte == b'"' || byte == b'\'')?;
    let open = at + 1;
    let close = open + bytes.get(open..)?.iter().position(|&byte| byte == quote)?;
    valid(bytes.get(open..close)?).then_some(close + 1)
}

fn skip_spaces(bytes: &[u8], mut at: usize) -> usize {
    while bytes
        .get(at)
        .is_some_and(|&byte| is_whitespace(char::from(byte)))
    {
        at += 1;
    }
    at
}

// `1.` and one or more digits.
fn is_version(value: &[u8]) -> bool {
    value
        .strip_prefix(b"1.")
        .is_some_and(|digits| !digits.is_empty() && digits.iter().all(u8::is_ascii_digit))
}

// XML 1.0's `EncName`: a letter, then letters, digits, `.`, `_` and `-`.
fn is_encoding_name(value: &[u8]) -> bool {
    value.split_first().is_some_and(|(first, rest)| {
        first.is_ascii_alphabetic()
            && rest
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    })
}

// The text between the quotes of `S encoding = "value"`.
fn quoted_value(attribute: &str) -> Option<&str> {
    let open = attribute.find(['"', '\''])?;
    let inner = attribute.get(open + 1..)?;
    inner.get(..inner.len().checked_sub(1)?)
}
