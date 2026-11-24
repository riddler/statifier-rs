//! A received request read as an external event, as the reference reads
//! one (`Statifier.Send.BasicHTTP.decode/1` in statifier-ex).

use super::encode::{EVENT_NAME, FORM, SEND_KEY};
use super::{DecodeError, Part, Received, Request};
use crate::datamodel::event_data;
use predicator_rs::{Map, Value};

/// The event a request carries, or why it carries none: the method, then
/// the query, then the body, then the dedup key header, are judged in that
/// order.
pub(super) fn decode(request: &Request) -> Result<Received, DecodeError> {
    let method = request.method.to_uppercase();
    if method != "POST" {
        return Err(DecodeError::MethodNotAllowed {
            method: request.method.clone(),
        });
    }
    let query = match query(&request.address) {
        Some(query) => pairs(query.as_bytes(), Part::Query)?,
        None => Vec::new(),
    };
    let form = header(request, "content-type")
        .is_some_and(|content_type| content_type.to_lowercase().starts_with(FORM));
    let (body, text) = if form {
        (pairs(&request.body, Part::Body)?, None)
    } else {
        let text = String::from_utf8(request.body.clone())
            .map_err(|_| DecodeError::NotUtf8 { part: Part::Body })?;
        (Vec::new(), Some(text))
    };
    if let Some(key) = header(request, SEND_KEY) {
        check_send_key(key)?;
    }
    let (names, params): (Vec<_>, Vec<_>) = query
        .into_iter()
        .chain(body)
        .partition(|(name, _)| name == EVENT_NAME);
    let name = match names.into_iter().next() {
        Some((_, value)) => value.unwrap_or_default(),
        None => format!("HTTP.{method}"),
    };
    let data = match text {
        Some(text) => event_data::text(&text),
        None if params.is_empty() => Value::Undefined,
        None => {
            let mut map = Map::new();
            for (name, value) in params {
                let value = value.map_or(Value::Undefined, |value| event_data::text(&value));
                map.insert(name, value);
            }
            Value::Map(map)
        }
    };
    Ok(Received { name, data })
}

/// The query of an address: what follows its first `?`, up to a `#`.
fn query(address: &str) -> Option<&str> {
    let (_, query) = address.split_once('?')?;
    Some(query.split('#').next().unwrap_or(query))
}

/// The value of the first header named `name`, in any case.
fn header<'a>(request: &'a Request, name: &str) -> Option<&'a str> {
    request
        .headers
        .iter()
        .find(|(header, _)| header.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

/// The name and value pairs of a form-encoded text: split at each `&`, the
/// last part dropped when it is empty, each part split at its first `=`
/// (`None` for a part with none), and each half decoded. A half that is
/// not UTF-8 once decoded refuses the whole `part`.
fn pairs(encoded: &[u8], part: Part) -> Result<Vec<(String, Option<String>)>, DecodeError> {
    let mut segments: Vec<&[u8]> = encoded.split(|byte| *byte == b'&').collect();
    if segments.last().is_some_and(|last| last.is_empty()) {
        segments.pop();
    }
    let utf8 = |bytes: Vec<u8>| String::from_utf8(bytes).map_err(|_| DecodeError::NotUtf8 { part });
    segments
        .into_iter()
        .map(|segment| {
            let (name, value) = match segment.iter().position(|byte| *byte == b'=') {
                Some(at) => (
                    segment.get(..at).unwrap_or_default(),
                    Some(segment.get(at + 1..).unwrap_or_default()),
                ),
                None => (segment, None),
            };
            let name = utf8(unpercent(name, true))?;
            let value = value
                .map(|value| utf8(unpercent(value, true)))
                .transpose()?;
            Ok((name, value))
        })
        .collect()
}

/// `bytes` with each `%` and two hex digits decoded, and each `+` a space
/// when `plus` is set; a `%` not followed by two hex digits is kept as it
/// is.
fn unpercent(bytes: &[u8], plus: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while let Some(&byte) = bytes.get(at) {
        let escaped = match (byte, bytes.get(at + 1), bytes.get(at + 2)) {
            (b'%', Some(&high), Some(&low)) => hex(high).zip(hex(low)),
            _ => None,
        };
        match escaped {
            Some((high, low)) => {
                out.push(high * 16 + low);
                at += 3;
            }
            None => {
                out.push(if plus && byte == b'+' { b' ' } else { byte });
                at += 1;
            }
        }
    }
    out
}

/// The value of one hex digit.
fn hex(digit: u8) -> Option<u8> {
    char::from(digit)
        .to_digit(16)
        .and_then(|value| u8::try_from(value).ok())
}

/// A dedup key header's shape: eight fields joined by `/`, the second, the
/// send id, UTF-8 once its percent escapes are decoded. The key sets no
/// field of the event.
fn check_send_key(key: &str) -> Result<(), DecodeError> {
    let fields: Vec<&str> = key.split('/').collect();
    let well_formed = fields.len() == 8
        && fields
            .get(1)
            .is_some_and(|send_id| String::from_utf8(unpercent(send_id.as_bytes(), false)).is_ok());
    if well_formed {
        Ok(())
    } else {
        Err(DecodeError::MalformedSendKey {
            value: key.to_owned(),
        })
    }
}

#[cfg(test)]
mod tests;
