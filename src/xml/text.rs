//! Character data, CDATA sections, attribute values and the references
//! inside them.

use super::cursor::refuse_at;
use super::reader::Reader;
use super::tags::is_whole_name;
use super::{ParseError, Reason};

impl Reader<'_> {
    /// Reads character data up to the next `<` or the end into `value`,
    /// expanding references and folding line breaks.
    pub(super) fn read_character_data(&mut self, value: &mut String) -> Result<(), ParseError> {
        loop {
            match self.cursor.peek() {
                None | Some('<') => return Ok(()),
                Some('&') => self.read_reference(value)?,
                Some('\r') => self.line_break(value, '\n'),
                Some(']') => {
                    if self.cursor.at("]]>") {
                        return Err(self.cursor.refuse(
                            Reason::CdataEndInText,
                            "']]>' is not allowed in character data",
                        ));
                    }
                    self.take_char(value);
                }
                Some(c) if is_excluded(c) => return Err(self.refuse_character(c)),
                Some(_) => self.take_plain(value, |c| !matches!(c, '<' | '&' | '\r' | ']')),
            }
        }
    }

    /// Reads `<![CDATA[ ... ]]>` into `value`: the content verbatim except
    /// that line breaks fold.
    pub(super) fn read_cdata(&mut self, value: &mut String) -> Result<(), ParseError> {
        self.cursor.skip("<![CDATA[");
        loop {
            if self.cursor.skip("]]>") {
                return Ok(());
            }
            match self.cursor.peek() {
                None => {
                    return Err(self
                        .cursor
                        .refuse(Reason::UnterminatedCdata, "a CDATA section is never closed"));
                }
                Some('\r') => self.line_break(value, '\n'),
                Some(']') => self.take_char(value),
                Some(c) if is_excluded(c) => return Err(self.refuse_character(c)),
                Some(_) => self.take_plain(value, |c| !matches!(c, ']' | '\r')),
            }
        }
    }

    /// Reads an attribute value up to its closing `quote`, which it leaves
    /// unread, into `value`, normalized.
    pub(super) fn read_value(&mut self, quote: char, value: &mut String) -> Result<(), ParseError> {
        loop {
            match self.cursor.peek() {
                None => return Err(self.unexpected_end()),
                Some(c) if c == quote => return Ok(()),
                Some('<') => {
                    return Err(self.cursor.refuse(
                        Reason::LtInAttributeValue,
                        "'<' is not allowed in an attribute value",
                    ));
                }
                Some('&') => self.read_reference(value)?,
                Some('\r') => self.line_break(value, ' '),
                Some('\t' | '\n') => {
                    self.cursor.bump();
                    value.push(' ');
                }
                Some(c) if is_excluded(c) => return Err(self.refuse_character(c)),
                Some(_) => self.take_plain(value, |c| {
                    c != quote && !matches!(c, '<' | '&' | '\t' | '\n' | '\r')
                }),
            }
        }
    }

    // Consumes a CR and the LF after it, if any, and appends `replacement`.
    fn line_break(&mut self, value: &mut String, replacement: char) {
        self.cursor.skip("\r");
        self.cursor.skip("\n");
        value.push(replacement);
    }

    // Consumes one character and appends it.
    fn take_char(&mut self, value: &mut String) {
        if let Some(c) = self.cursor.bump() {
            value.push(c);
        }
    }

    // Consumes and appends the characters that `plain` admits and XML
    // allows.
    fn take_plain(&mut self, value: &mut String, plain: impl Fn(char) -> bool) {
        let start = self.cursor.offset();
        self.cursor.skip_while(|c| plain(c) && !is_excluded(c));
        value.push_str(self.cursor.slice(start, self.cursor.offset()));
    }

    // Reads `&name;`, `&#digits;` or `&#xhex;` and appends what it stands
    // for. A reference body runs to the first `;` and never spans whitespace,
    // markup or a quote; one that does means the `&` began no reference.
    fn read_reference(&mut self, value: &mut String) -> Result<(), ParseError> {
        let at = self.cursor.mark();
        let after = self.cursor.rest().get(1..).unwrap_or("");
        let end = after.find([';', ' ', '\t', '\r', '\n', '<', '&', '"', '\'']);
        let body = match end {
            Some(length)
                if length > 0
                    && after
                        .get(length..)
                        .is_some_and(|rest| rest.starts_with(';')) =>
            {
                after.get(..length).unwrap_or("")
            }
            _ => {
                return Err(refuse_at(
                    at,
                    Reason::MalformedReference,
                    "'&' does not start a reference",
                ));
            }
        };
        self.cursor.move_to(at.offset + body.len() + 2);
        if body.starts_with('#') {
            return match character_reference(body) {
                Some(c) => {
                    value.push(c);
                    Ok(())
                }
                None => {
                    let message = format!("&{body}; is not a valid character reference");
                    Err(refuse_at(at, Reason::InvalidCharacterReference, &message))
                }
            };
        }
        // Any other name is kept as its literal text, in every document: the
        // reference's XML library keeps an entity it does not expand, and no
        // document type definition is read.
        match predefined(body) {
            Some(c) => value.push(c),
            None if is_whole_name(body) => {
                value.push('&');
                value.push_str(body);
                value.push(';');
            }
            None => {
                let message = format!("&{body}; does not name an entity");
                return Err(refuse_at(at, Reason::MalformedReference, &message));
            }
        }
        Ok(())
    }

    // A refusal of the character `c` at the read position.
    fn refuse_character(&mut self, c: char) -> ParseError {
        let message = format!("the character U+{:04X} is not allowed in XML", u32::from(c));
        self.cursor.refuse(Reason::InvalidCharacter, &message)
    }
}

/// The character one of the five predefined entities stands for.
fn predefined(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => None,
    }
}

/// The character `#digits` or `#xhex` names, or `None` when it names nothing
/// XML allows as a character.
fn character_reference(body: &str) -> Option<char> {
    let (digits, radix) = match body.strip_prefix("#x") {
        Some(hex) => (hex, 16),
        None => (body.strip_prefix('#').unwrap_or(""), 10),
    };
    if digits.is_empty() {
        return None;
    }
    let mut code: u32 = 0;
    for c in digits.chars() {
        code = code * radix + c.to_digit(radix)?;
        if code > 0x10_FFFF {
            return None;
        }
    }
    if is_xml_char(code) {
        char::from_u32(code)
    } else {
        None
    }
}

/// XML 1.0's `Char` production.
fn is_xml_char(code: u32) -> bool {
    matches!(code, 0x9 | 0xA | 0xD | 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x1_0000..=0x10_FFFF)
}

/// The characters XML 1.0's `Char` production leaves out that a Rust string
/// can still hold (a surrogate cannot be in one).
pub(super) fn is_excluded(c: char) -> bool {
    matches!(c,
        '\u{0}'..='\u{8}' | '\u{B}' | '\u{C}' | '\u{E}'..='\u{1F}' | '\u{FFFE}' | '\u{FFFF}')
}
