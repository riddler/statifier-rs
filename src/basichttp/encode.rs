//! The request one Basic HTTP send becomes, built as the reference builds
//! it (`Statifier.Send.BasicHTTP` in statifier-ex): the form body or the
//! text body, the query parameter, the percent-encoding, the order of the
//! fields and the dedup key header.

use super::Request;
use crate::effect::{BlockOwner, Effect};
use predicator_rs::{Key, Value, tagged};
use std::cmp::Ordering;

/// The form parameter, and query parameter, that carries the event's name.
pub(super) const EVENT_NAME: &str = "_scxmleventname";

/// The content type of a form body.
pub(super) const FORM: &str = "application/x-www-form-urlencoded";

/// The header that carries a send's dedup key.
pub(super) const SEND_KEY: &str = "scxml-send-key";

/// The request a send of the processor's type becomes, made by the session
/// `session_id`; `None` when the send names no target, so there is no
/// address to request.
pub(super) fn request(session_id: &str, send: &Effect) -> Option<Request> {
    let send = Fields::of(send)?;
    let target = match send.target? {
        Value::String(target) => target.clone(),
        other => text(other),
    };
    let named: Vec<(String, String)> = match send.event {
        // The reference names the event when its value is truthy, and
        // writes an undefined one as the word.
        None | Some(Value::Bool(false)) => Vec::new(),
        Some(Value::Undefined) => vec![(EVENT_NAME.to_owned(), "undefined".to_owned())],
        Some(event) => vec![(EVENT_NAME.to_owned(), text(event))],
    };
    let (address, content_type, body) = match send.data {
        Value::Map(map) => {
            let mut members: Vec<(&Key, &Value)> = map.iter().collect();
            members.sort_by(|(a, _), (b, _)| term_order(a, b));
            let mut pairs = named;
            pairs.extend(
                members
                    .into_iter()
                    .map(|(key, value)| (key_text(key), text(value))),
            );
            (target, FORM, form(&pairs))
        }
        Value::Undefined => (target, FORM, form(&named)),
        content => (with_query(target, &named), "text/plain", text(content)),
    };
    let headers = vec![
        ("content-type".to_owned(), content_type.to_owned()),
        (SEND_KEY.to_owned(), send_key(session_id, &send)),
    ];
    Some(Request::new("POST", address, headers, body))
}

/// The fields of an [`Effect::Send`] or an [`Effect::SendDelayed`] the
/// request is built from.
struct Fields<'a> {
    event: Option<&'a Value>,
    target: Option<&'a Value>,
    data: &'a Value,
    send_id: &'a str,
    content_index: usize,
    owner: BlockOwner,
    macrostep: u64,
    microstep: u64,
    round: u64,
    ordinal: Option<u64>,
}

impl<'a> Fields<'a> {
    /// The fields of a send, `None` for any other effect.
    fn of(send: &'a Effect) -> Option<Self> {
        match send {
            Effect::Send {
                event,
                target,
                data,
                send_id,
                ordinal,
                content_index,
                owner,
                macrostep,
                microstep,
                round,
                ..
            } => Some(Fields {
                event: event.as_ref(),
                target: target.as_ref(),
                data,
                send_id,
                content_index: *content_index,
                owner: *owner,
                macrostep: *macrostep,
                microstep: *microstep,
                round: *round,
                ordinal: *ordinal,
            }),
            Effect::SendDelayed {
                event,
                target,
                data,
                send_id,
                ordinal,
                content_index,
                owner,
                macrostep,
                microstep,
                round,
                ..
            } => Some(Fields {
                event: event.as_ref(),
                target: target.as_ref(),
                data,
                send_id,
                content_index: *content_index,
                owner: *owner,
                macrostep: *macrostep,
                microstep: *microstep,
                round: *round,
                ordinal: Some(*ordinal),
            }),
            _ => None,
        }
    }
}

/// The dedup key: the session, the send id, the three counters, the
/// content index, the owner and the ordinal, joined by `/`, the first two
/// percent-encoded.
fn send_key(session_id: &str, send: &Fields<'_>) -> String {
    let owner = match send.owner {
        BlockOwner::OnEntry { state, ordinal } => format!("onentry.{state}.{ordinal}"),
        BlockOwner::OnExit { state, ordinal } => format!("onexit.{state}.{ordinal}"),
        BlockOwner::Transition { transition } => format!("transition.{transition}"),
        BlockOwner::Finalize { state, invoke } => format!("finalize.{state}.{invoke}"),
        // A send never runs in an invoke's own block; the reference spells
        // no other owner, and an absent one is empty.
        _ => String::new(),
    };
    [
        percent(session_id, "%20"),
        percent(send.send_id, "%20"),
        send.macrostep.to_string(),
        send.microstep.to_string(),
        send.round.to_string(),
        send.content_index.to_string(),
        owner,
        send.ordinal
            .map(|ordinal| ordinal.to_string())
            .unwrap_or_default(),
    ]
    .join("/")
}

/// The reference runtime's order of two map keys, the order it walks a map
/// of up to 32 members in: integers, then `false`, `true` and undefined,
/// then strings, each kind in its own order.
fn term_order(a: &Key, b: &Key) -> Ordering {
    fn rank(key: &Key) -> u8 {
        match key {
            Key::Integer(_) => 0,
            Key::Bool(false) => 1,
            Key::Bool(true) => 2,
            Key::Undefined => 3,
            Key::String(_) => 4,
            _ => 5,
        }
    }
    rank(a).cmp(&rank(b)).then_with(|| a.cmp(b))
}

/// A map key as a form field's name.
fn key_text(key: &Key) -> String {
    match key {
        Key::String(text) => text.clone(),
        Key::Integer(number) => number.to_string(),
        Key::Bool(flag) => flag.to_string(),
        Key::Undefined => "undefined".to_owned(),
        other => format!("{other:?}"),
    }
}

/// A value as a parameter's text: a string as it is, `null` for null, the
/// empty string for undefined, a number or a boolean as its literal, and a
/// list or a map as JSON text.
pub(super) fn text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => "null".to_owned(),
        Value::Undefined => String::new(),
        Value::Bool(flag) => flag.to_string(),
        Value::Integer(number) => number.to_string(),
        Value::Float(number) if number.is_finite() => float_text(*number),
        value => json(value, Strict::Yes).unwrap_or_else(|| json_lenient(value)),
    }
}

/// A finite float in the reference's spelling, which the predicator
/// crate's tagged encoding writes.
fn float_text(number: f64) -> String {
    tagged::encode(&Value::Float(number)).unwrap_or_else(|_| number.to_string())
}

/// Whether [`json`] refuses a value with no JSON form.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Strict {
    Yes,
    No,
}

/// [`json`] for a value with no JSON form: never refused.
fn json_lenient(value: &Value) -> String {
    json(value, Strict::No).unwrap_or_default()
}

/// One step of an iterative write.
enum Task<'a> {
    Value(&'a Value),
    Text(&'static str),
    Key(String),
}

/// `value` as JSON text with no whitespace, a map's members in key order
/// and undefined written as null. Strictly, `None` when anything inside
/// has no JSON form (a date, an instant, a duration, a key that is not a
/// string, a float that is not finite). Leniently, a date, an instant or a
/// duration is written as its tagged encoding (null where that has none), a
/// key as its text and a float that is not finite as null.
fn json(value: &Value, strict: Strict) -> Option<String> {
    let mut out = String::new();
    let mut work = vec![Task::Value(value)];
    while let Some(task) = work.pop() {
        match task {
            Task::Text(text) => out.push_str(text),
            Task::Key(key) => {
                json_string(&key, &mut out);
                out.push(':');
            }
            Task::Value(value) => match value {
                Value::Null | Value::Undefined => out.push_str("null"),
                Value::Bool(flag) => out.push_str(if *flag { "true" } else { "false" }),
                Value::Integer(number) => out.push_str(&number.to_string()),
                Value::Float(number) if number.is_finite() => out.push_str(&float_text(*number)),
                Value::String(text) => json_string(text, &mut out),
                Value::List(items) => {
                    out.push('[');
                    work.push(Task::Text("]"));
                    for (index, item) in items.iter().enumerate().rev() {
                        work.push(Task::Value(item));
                        if index > 0 {
                            work.push(Task::Text(","));
                        }
                    }
                }
                Value::Map(map) => {
                    out.push('{');
                    work.push(Task::Text("}"));
                    let members: Vec<(&Key, &Value)> = map.iter().collect();
                    for (index, (key, item)) in members.into_iter().enumerate().rev() {
                        let key = match key {
                            Key::String(key) => key.clone(),
                            _ if strict == Strict::Yes => return None,
                            other => key_text(other),
                        };
                        work.push(Task::Value(item));
                        work.push(Task::Key(key));
                        if index > 0 {
                            work.push(Task::Text(","));
                        }
                    }
                }
                _ if strict == Strict::Yes => return None,
                Value::Float(_) => out.push_str("null"),
                other => out.push_str(&tagged::encode(other).unwrap_or_else(|_| "null".to_owned())),
            },
        }
    }
    Some(out)
}

/// `text` as a JSON string, escaped as the reference's encoder escapes it:
/// the quote, the backslash and the control characters, the five with
/// short forms short and the rest as `\u` and four uppercase hex digits.
fn json_string(text: &str, out: &mut String) {
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            control if u32::from(control) < 0x20 => {
                out.push_str(&format!("\\u{:04X}", u32::from(control)));
            }
            other => out.push(other),
        }
    }
    out.push('"');
}

/// Name and value pairs as a form body: each percent-encoded, a space as
/// `+`, joined by `=` and `&`.
fn form(pairs: &[(String, String)]) -> String {
    pairs
        .iter()
        .map(|(name, value)| format!("{}={}", percent(name, "+"), percent(value, "+")))
        .collect::<Vec<_>>()
        .join("&")
}

/// `target` with the pairs as a query, after a `&` when it already holds a
/// `?`, else after a `?`; `target` itself when there are none.
fn with_query(target: String, pairs: &[(String, String)]) -> String {
    if pairs.is_empty() {
        return target;
    }
    let separator = if target.contains('?') { '&' } else { '?' };
    format!("{target}{separator}{}", form(pairs))
}

/// Every byte of `text` outside the unreserved set (letters, digits, `-`,
/// `.`, `_` and `~`) as `%` and two uppercase hex digits, a space as
/// `space`.
fn percent(text: &str, space: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(char::from(byte));
            }
            b' ' => out.push_str(space),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests;
