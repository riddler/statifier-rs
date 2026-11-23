//! The location writer: how `<assign location>` (and, later, every other
//! location a chart writes through) reaches the datamodel, ported from the
//! reference's `write_location` (`Statifier.Interpreter.Datamodel` in
//! statifier-ex) and the predicator library's context location it rests on.
//!
//! The predicator crate has no location API, so both halves are here. A
//! location is a root name followed by any number of property accesses
//! (`a.b`) and bracket accesses (`a[0]`, `a['b']`, `a[i]`); a bracket holds
//! a string literal, an integer literal (a leading minus allowed) or a bare
//! name whose value, a string or an integer, is the key. Anything else is
//! not a location.
//!
//! [`write_location`] takes the reference's five steps in its order:
//! resolve the location against the datamodel as it stands, refuse a
//! system variable's root, refuse a root the datamodel does not hold, read
//! the value at the full path, then write, creating any missing
//! intermediate container (a map before a key, a list before an index) and
//! never destroying a value to make room.

#[cfg(test)]
mod tests;

use super::{Failure, system_root};
use crate::effect::PathSegment;
use predicator_rs::{Key, Map, Value};

/// The largest list a write grows: an index at or past this length, beyond
/// a list's end, is refused as an invalid index rather than padding the
/// list with that many undefined elements.
pub(crate) const MAX_LIST_LENGTH: i64 = 1 << 20;

/// Why a location could not be resolved or written: the reference's
/// location error types, under their names.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum LocationReason {
    /// The text is not a location: not a root name followed by property and
    /// bracket accesses.
    NotAssignable,
    /// A bracket holds an expression that is neither a literal key nor a
    /// bare name.
    ComputedKey,
    /// A bracket's bare name is not in the datamodel, or holds null.
    UndefinedVariable(String),
    /// A bracket's bare name holds a value that is neither a string nor an
    /// integer.
    InvalidKey(Value),
    /// The path runs through a value that is not a container, or names a
    /// property of a list.
    NotAContainer,
    /// The path indexes a list with a negative integer, or past
    /// [`MAX_LIST_LENGTH`].
    InvalidIndex(i64),
}

impl LocationReason {
    /// The reason's spelling, the reference's location error type.
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            LocationReason::NotAssignable => "not_assignable",
            LocationReason::ComputedKey => "computed_key",
            LocationReason::UndefinedVariable(_) => "undefined_variable",
            LocationReason::InvalidKey(_) => "invalid_key",
            LocationReason::NotAContainer => "not_a_container",
            LocationReason::InvalidIndex(_) => "invalid_index",
        }
    }

    /// A message for a person.
    pub(crate) fn message(&self) -> String {
        match self {
            LocationReason::NotAssignable => String::from("the text is not an assignable location"),
            LocationReason::ComputedKey => {
                String::from("a computed expression cannot be a location's key")
            }
            LocationReason::UndefinedVariable(name) => {
                format!("the bracket key variable {name} is not defined")
            }
            LocationReason::InvalidKey(value) => {
                format!("a bracket key must be a string or an integer, not {value:?}")
            }
            LocationReason::NotAContainer => {
                String::from("the location runs through a value that is not a container")
            }
            LocationReason::InvalidIndex(index) => format!("{index} is not a writable list index"),
        }
    }
}

/// What a write did: the resolved path, the value that stood there before
/// ([`Value::Undefined`] when none did) and the value written.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Written {
    /// The datamodel after the write.
    pub(crate) data: Map,
    /// The resolved path, the root first.
    pub(crate) path: Vec<PathSegment>,
    /// The value at the full path before the write.
    pub(crate) prior_value: Value,
    /// The value written.
    pub(crate) new_value: Value,
}

/// Writes `value` at the location `source` names in `data`, as the
/// reference's `write_location` does; or answers why it cannot, writing
/// nothing.
pub(crate) fn write_location(data: &Map, source: &str, value: Value) -> Result<Written, Failure> {
    let failure = |reason| Failure::Location {
        source: source.to_owned(),
        reason,
    };
    let path = resolve(source, data).map_err(failure)?;
    let Some(PathSegment::Key(root)) = path.first() else {
        return Err(failure(LocationReason::NotAssignable));
    };
    if system_root(root) {
        return Err(Failure::SystemVariable(root.clone()));
    }
    if !data.contains_key(&Key::from(root.as_str())) {
        return Err(Failure::UnboundLocation(source.to_owned()));
    }
    let prior_value = read_path(data, &path);
    let mut written = data.clone();
    put(&mut written, &path, value.clone()).map_err(failure)?;
    Ok(Written {
        data: written,
        path,
        prior_value,
        new_value: value,
    })
}

/// One bracket's content, as written.
#[derive(Debug, Clone, PartialEq)]
enum Bracket {
    /// A string literal's value.
    Text(String),
    /// An integer literal's value.
    Integer(i64),
    /// A bare name.
    Name(String),
    /// Anything else.
    Computed,
}

/// One access after the root, as written.
#[derive(Debug, Clone, PartialEq)]
enum Access {
    /// `.name`.
    Property(String),
    /// `[...]`.
    Bracket(Bracket),
}

/// Resolves `source` against `data` into a path. The bracket keys are
/// resolved last to first and before the root is checked, the order the
/// reference's resolver takes, so of two failures the later bracket's is
/// the one reported.
pub(crate) fn resolve(source: &str, data: &Map) -> Result<Vec<PathSegment>, LocationReason> {
    let (root, accesses) = parse(source).ok_or(LocationReason::NotAssignable)?;
    let mut keys = Vec::with_capacity(accesses.len());
    for access in accesses.iter().rev() {
        keys.push(match access {
            Access::Property(name) => PathSegment::Key(name.clone()),
            Access::Bracket(bracket) => bracket_key(bracket, data)?,
        });
    }
    let root = root.ok_or(LocationReason::NotAssignable)?;
    let mut path = Vec::with_capacity(keys.len() + 1);
    path.push(PathSegment::Key(root));
    path.extend(keys.into_iter().rev());
    Ok(path)
}

/// A bracket's key: a literal as written, a bare name's value when it is a
/// string or an integer.
fn bracket_key(bracket: &Bracket, data: &Map) -> Result<PathSegment, LocationReason> {
    match bracket {
        Bracket::Text(text) => Ok(PathSegment::Key(text.clone())),
        Bracket::Integer(index) => Ok(PathSegment::Index(*index)),
        Bracket::Name(name) => match data.get(&Key::from(name.as_str())) {
            Some(Value::String(text)) => Ok(PathSegment::Key(text.clone())),
            Some(Value::Integer(index)) => Ok(PathSegment::Index(*index)),
            None | Some(Value::Null) => Err(LocationReason::UndefinedVariable(name.clone())),
            Some(other) => Err(LocationReason::InvalidKey(other.clone())),
        },
        Bracket::Computed => Err(LocationReason::ComputedKey),
    }
}

/// The words the predicator language reserves: none of them is a name.
const RESERVED: [&str; 22] = [
    "true",
    "false",
    "undefined",
    "null",
    "AND",
    "and",
    "OR",
    "or",
    "NOT",
    "not",
    "IN",
    "in",
    "CONTAINS",
    "contains",
    "if",
    "else",
    "while",
    "ago",
    "from",
    "now",
    "next",
    "last",
];

/// The whitespace the predicator language skips between tokens.
fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r')
}

/// Reads `source` as a root followed by accesses; `None` when it is not
/// that shape. The root is `None` when the text before the first access is
/// not a bare name, which the resolver refuses after the keys.
fn parse(source: &str) -> Option<(Option<String>, Vec<Access>)> {
    let mut cursor = Cursor {
        bytes: source.as_bytes(),
        at: 0,
    };
    cursor.skip_space();
    let root = match cursor.word() {
        Some(word) if !RESERVED.contains(&word) => Some(word.to_owned()),
        Some(_) => None,
        None => {
            cursor.root_literal()?;
            None
        }
    };
    let mut accesses = Vec::new();
    loop {
        cursor.skip_space();
        match cursor.peek() {
            None => return Some((root, accesses)),
            Some(b'.') => {
                cursor.at += 1;
                cursor.skip_space();
                let word = cursor.word()?;
                if RESERVED.contains(&word) {
                    return None;
                }
                accesses.push(Access::Property(word.to_owned()));
            }
            Some(b'[') => {
                cursor.at += 1;
                let inner = cursor.bracket_body()?;
                accesses.push(Access::Bracket(bracket(inner)));
            }
            Some(_) => return None,
        }
    }
}

/// What a bracket's content is.
fn bracket(inner: &str) -> Bracket {
    let text = inner.trim_matches(|c: char| c.is_ascii() && is_space(c as u8));
    if let Some(value) = string_literal(text) {
        return Bracket::Text(value);
    }
    if let Some(index) = integer_literal(text) {
        return Bracket::Integer(index);
    }
    let mut cursor = Cursor {
        bytes: text.as_bytes(),
        at: 0,
    };
    match cursor.word() {
        Some(word) if cursor.at == text.len() && !RESERVED.contains(&word) => {
            Bracket::Name(word.to_owned())
        }
        _ => Bracket::Computed,
    }
}

/// A whole quoted string's value, its escapes read; `None` for anything
/// else.
fn string_literal(text: &str) -> Option<String> {
    let mut chars = text.chars();
    let quote = chars.next().filter(|c| *c == '\'' || *c == '"')?;
    let mut value = String::new();
    while let Some(c) = chars.next() {
        match c {
            '\\' => value.push(match chars.next()? {
                'n' => '\n',
                't' => '\t',
                'r' => '\r',
                other @ ('\\' | '\'' | '"') => other,
                _ => return None,
            }),
            c if c == quote => return chars.next().is_none().then_some(value),
            c => value.push(c),
        }
    }
    None
}

/// A whole integer literal's value, a leading minus (and space after it)
/// allowed; `None` for anything else, an integer outside 64 bits included.
fn integer_literal(text: &str) -> Option<i64> {
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest.trim_start_matches([' ', '\t', '\n', '\r'])),
        None => (false, text),
    };
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let magnitude: i64 = digits.parse().ok()?;
    Some(if negative { -magnitude } else { magnitude })
}

/// A byte cursor over a location's text.
struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn skip_space(&mut self) {
        while self.peek().is_some_and(is_space) {
            self.at += 1;
        }
    }

    /// A word, `[A-Za-z_][A-Za-z0-9_]*`, at the cursor.
    fn word(&mut self) -> Option<&'a str> {
        let start = self.at;
        if !self
            .peek()
            .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        {
            return None;
        }
        while self
            .peek()
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            self.at += 1;
        }
        std::str::from_utf8(self.bytes.get(start..self.at)?).ok()
    }

    /// A literal standing where the root would: a quoted string or a run of
    /// digits, read past so the accesses after it still parse.
    fn root_literal(&mut self) -> Option<()> {
        match self.peek()? {
            quote @ (b'\'' | b'"') => {
                self.at += 1;
                while let Some(byte) = self.peek() {
                    self.at += 1;
                    match byte {
                        b'\\' => self.at += 1,
                        byte if byte == quote => return Some(()),
                        _ => {}
                    }
                }
                None
            }
            byte if byte.is_ascii_digit() => {
                while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                    self.at += 1;
                }
                Some(())
            }
            _ => None,
        }
    }

    /// The text up to the `]` that closes the bracket just opened, nested
    /// brackets and quoted strings skipped; the cursor ends past it.
    fn bracket_body(&mut self) -> Option<&'a str> {
        let start = self.at;
        let mut depth = 0_usize;
        let mut quote = None;
        while let Some(byte) = self.peek() {
            self.at += 1;
            match (quote, byte) {
                (Some(_), b'\\') => self.at += 1,
                (Some(open), byte) if byte == open => quote = None,
                (Some(_), _) => {}
                (None, b'\'' | b'"') => quote = Some(byte),
                (None, b'[') => depth += 1,
                (None, b']') if depth == 0 => {
                    return std::str::from_utf8(self.bytes.get(start..self.at - 1)?).ok();
                }
                (None, b']') => depth -= 1,
                (None, _) => {}
            }
        }
        None
    }
}

/// The value at `path` in `data`, [`Value::Undefined`] on any miss: a key
/// read from a map, a non-negative index from a list, and anything else a
/// miss, as the reference reads the prior value.
pub(crate) fn read_path(data: &Map, path: &[PathSegment]) -> Value {
    let Some((PathSegment::Key(root), rest)) = path.split_first() else {
        return Value::Undefined;
    };
    let Some(mut value) = data.get(&Key::from(root.as_str())) else {
        return Value::Undefined;
    };
    for segment in rest {
        let next = match (value, segment) {
            (Value::Map(map), PathSegment::Key(key)) => map.get(&Key::from(key.as_str())),
            (Value::List(list), PathSegment::Index(index)) => usize::try_from(*index)
                .ok()
                .and_then(|index| list.get(index)),
            _ => None,
        };
        match next {
            Some(found) => value = found,
            None => return Value::Undefined,
        }
    }
    value.clone()
}

/// The datamodel key a segment names.
fn key(segment: &PathSegment) -> Key {
    match segment {
        PathSegment::Key(name) => Key::from(name.as_str()),
        PathSegment::Index(index) => Key::from(*index),
    }
}

/// Writes `value` at `path` under `data`, creating each missing (or null,
/// or undefined) intermediate value as a map, or a list when the segment
/// after it is an index; the predicator library's context-location put.
fn put(data: &mut Map, path: &[PathSegment], value: Value) -> Result<(), LocationReason> {
    let Some((first, rest)) = path.split_first() else {
        return Err(LocationReason::NotAssignable);
    };
    let slot = key(first);
    let Some(next) = rest.first() else {
        data.insert(slot, value);
        return Ok(());
    };
    let child = data.get(&slot).cloned().unwrap_or(Value::Undefined);
    let child = put_in(vivify(child, next)?, rest, value)?;
    data.insert(slot, child);
    Ok(())
}

/// `container` with `value` written at `path` under it.
fn put_in(
    mut container: Value,
    path: &[PathSegment],
    value: Value,
) -> Result<Value, LocationReason> {
    let Some((segment, rest)) = path.split_first() else {
        return Ok(value);
    };
    match &mut container {
        Value::Map(map) => {
            put(map, path, value)?;
            Ok(container)
        }
        Value::List(list) => {
            let PathSegment::Index(index) = segment else {
                return Err(LocationReason::NotAContainer);
            };
            let position = list_position(*index, list.len())?;
            let child = match rest.first() {
                None => value,
                Some(next) => {
                    let current = list.get(position).cloned().unwrap_or(Value::Undefined);
                    put_in(vivify(current, next)?, rest, value)?
                }
            };
            if position < list.len() {
                if let Some(slot) = list.get_mut(position) {
                    *slot = child;
                }
            } else {
                list.resize(position, Value::Undefined);
                list.push(child);
            }
            Ok(container)
        }
        _ => Err(LocationReason::NotAContainer),
    }
}

/// A list index's position: non-negative, and within the list or short of
/// [`MAX_LIST_LENGTH`].
fn list_position(index: i64, length: usize) -> Result<usize, LocationReason> {
    let position = usize::try_from(index).map_err(|_| LocationReason::InvalidIndex(index))?;
    if position >= length && index >= MAX_LIST_LENGTH {
        return Err(LocationReason::InvalidIndex(index));
    }
    Ok(position)
}

/// The container a path runs through: an absent value becomes a list
/// before an index and a map before a key; a map or a list stays; anything
/// else is not a container.
fn vivify(value: Value, next: &PathSegment) -> Result<Value, LocationReason> {
    match (value, next) {
        (Value::Null | Value::Undefined, PathSegment::Index(_)) => Ok(Value::List(Vec::new())),
        (Value::Null | Value::Undefined, PathSegment::Key(_)) => Ok(Value::Map(Map::new())),
        (value @ (Value::Map(_) | Value::List(_)), _) => Ok(value),
        _ => Err(LocationReason::NotAContainer),
    }
}
