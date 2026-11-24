//! An event's data, normalized as the reference normalizes it
//! (`Statifier.EventData` in statifier-ex): one function per shape a
//! payload arrives in.

use predicator_rs::{Context, Map, OnUnbound, Options, Value};

/// A `<content>` body's text as data: blank text is no data
/// ([`Value::Undefined`]); text the predicator language reads as a literal
/// (a number, a string, a list, a map) is that value, read against an empty
/// context that refuses every name; anything else is the trimmed text with
/// each run of whitespace one space.
pub(crate) fn text(text: &str) -> Value {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Value::Undefined;
    }
    let options = Options::new().on_unbound(OnUnbound::Error);
    match predicator_rs::compile(trimmed)
        .and_then(|instructions| predicator_rs::evaluate(&instructions, &Context::new(), &options))
    {
        Ok(value) => value,
        Err(_) => Value::from(space_normalized(trimmed)),
    }
}

/// `namelist` and `<param>` values, name and value pairs in document order,
/// as data: a map, the last of a repeated name kept, or no data
/// ([`Value::Undefined`]) when there are none.
pub(crate) fn params(pairs: Vec<(&str, Value)>) -> Value {
    if pairs.is_empty() {
        return Value::Undefined;
    }
    let mut map = Map::new();
    for (name, value) in pairs {
        map.insert(name, value);
    }
    Value::Map(map)
}

/// `text` with each run of ASCII whitespace (space, tab, line feed, line
/// tabulation, form feed, carriage return) one space.
fn space_normalized(text: &str) -> String {
    let mut normalized = String::with_capacity(text.len());
    let mut in_space = false;
    for c in text.chars() {
        if matches!(c, ' ' | '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r') {
            if !in_space {
                normalized.push(' ');
            }
            in_space = true;
        } else {
            normalized.push(c);
            in_space = false;
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    // Sabotage: with text answering the trimmed text whatever it reads as,
    // the number stays a string and this fails.
    #[test]
    fn text_reads_a_literal_else_normalizes_its_whitespace() {
        assert_eq!(text("  \n "), Value::Undefined);
        assert_eq!(text(" 42 "), Value::from(42));
        assert_eq!(text("'q'"), Value::from("q"));
        assert_eq!(text(" a \t\n  b\u{0B}c "), Value::from("a b c"));
    }

    // Sabotage: with params keeping the first of a repeated name, the map
    // holds 1 and this fails.
    #[test]
    fn params_keep_the_last_of_a_repeated_name() {
        assert_eq!(params(Vec::new()), Value::Undefined);
        let mut expected = Map::new();
        expected.insert("a", Value::from(2));
        expected.insert("b", Value::Null);
        assert_eq!(
            params(vec![
                ("a", Value::from(1)),
                ("b", Value::Null),
                ("a", Value::from(2))
            ]),
            Value::Map(expected)
        );
    }
}
