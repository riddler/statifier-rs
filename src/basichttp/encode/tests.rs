// The encoding table, row by row. Each expected text is what the reference
// build at its tag answered for the same send (the differential in the
// request that added this file), except the rows named as this crate's own.

use super::*;
use predicator_rs::{Date, Map};

const FORM_TYPE: &str = "application/x-www-form-urlencoded";

fn send(event: Option<&str>, target: Option<&str>, data: Value) -> Effect {
    Effect::Send {
        event: event.map(Value::from),
        target: target.map(Value::from),
        send_type: Some(Value::from("basichttp")),
        data,
        send_id: "send_1".to_owned(),
        id_from_author: false,
        ordinal: Some(1),
        content_index: 2,
        owner: BlockOwner::OnEntry {
            state: 1,
            ordinal: 0,
        },
        macrostep: 0,
        microstep: 1,
        round: 2,
    }
}

fn map(members: Vec<(Key, Value)>) -> Value {
    let mut map = Map::new();
    for (key, value) in members {
        map.insert(key, value);
    }
    Value::Map(map)
}

fn strings(members: &[(&str, Value)]) -> Value {
    map(members
        .iter()
        .map(|(key, value)| (Key::from(*key), value.clone()))
        .collect())
}

/// The address, the content type and the body text of a send's request.
fn encoded(send: &Effect) -> (String, String, String) {
    let request = request("s-1", send).expect("a target");
    let content_type = request
        .headers
        .iter()
        .find(|(name, _)| name == "content-type")
        .map(|(_, value)| value.clone())
        .expect("a content type");
    (
        request.address,
        content_type,
        String::from_utf8(request.body).expect("UTF-8"),
    )
}

fn body(data: Value) -> String {
    encoded(&send(None, Some("http://h/s"), data)).2
}

// Sabotage: with the event name left out of the form, the first row goes red.
#[test]
fn a_form_body_carries_the_event_name_first() {
    let target = Some("http://h/s");
    assert_eq!(
        encoded(&send(Some("test"), target, Value::Undefined)),
        (
            "http://h/s".to_owned(),
            FORM_TYPE.to_owned(),
            "_scxmleventname=test".to_owned()
        )
    );
    assert_eq!(body(Value::Undefined), "");
    assert_eq!(
        encoded(&send(
            Some("test"),
            target,
            strings(&[("param1", Value::from(1))])
        ))
        .2,
        "_scxmleventname=test&param1=1"
    );
    assert_eq!(
        encoded(&send(Some("test"), target, Value::Map(Map::new()))).2,
        "_scxmleventname=test"
    );
    assert_eq!(
        encoded(&send(
            Some("test"),
            target,
            strings(&[("_scxmleventname", Value::from("other"))])
        ))
        .2,
        "_scxmleventname=test&_scxmleventname=other"
    );
}

// Sabotage: with a space percent-encoded as %20 in a form, this goes red.
#[test]
fn form_fields_are_percent_encoded_in_key_order() {
    assert_eq!(
        body(strings(&[
            ("a b", Value::from("x&y=z")),
            ("\u{e9}", Value::from("~-._*'()!/?:@+")),
            ("k", Value::from("line\nbreak\ttab")),
        ])),
        "a+b=x%26y%3Dz&k=line%0Abreak%09tab&%C3%A9=~-._%2A%27%28%29%21%2F%3F%3A%40%2B"
    );
    assert_eq!(
        body(strings(&[("s", Value::from("\u{e9}\u{1F600}"))])),
        "s=%C3%A9%F0%9F%98%80"
    );
    assert_eq!(body(strings(&[("k", Value::from(""))])), "k=");
    assert_eq!(body(strings(&[("k", Value::from("a\u{0}b"))])), "k=a%00b");
}

// Sabotage: with a float written by Rust's own Display (1e20 as
// 100000000000000000000), this goes red.
#[test]
fn scalar_values_are_written_as_their_literals() {
    assert_eq!(
        body(strings(&[
            ("n", Value::Null),
            ("u", Value::Undefined),
            ("t", Value::from(true)),
            ("f", Value::from(false)),
            ("x1", Value::from(1.5)),
            ("x2", Value::from(1.0e20)),
            ("x3", Value::from(0.1)),
            ("x4", Value::from(-0.0)),
            ("x5", Value::from(123_456_789.0)),
            ("x6", Value::from(9_007_199_254_740_992.0)),
            ("x7", Value::from(1.5e-7)),
            ("x8", Value::from(9_007_199_254_740_993_i64)),
            ("x9", Value::from(-5)),
            ("y1", Value::from(1.0e16)),
            ("y2", Value::from(0.001)),
            ("y3", Value::from(1.0e-5)),
        ])),
        "f=false&n=null&t=true&u=&x1=1.5&x2=1.0e20&x3=0.1&x4=-0.0&x5=123456789.0\
         &x6=9.007199254740992e15&x7=1.5e-7&x8=9007199254740993&x9=-5&y1=1.0e16\
         &y2=0.001&y3=1.0e-5"
    );
}

// Sabotage: with undefined inside a list written as the empty string rather
// than null, the first row goes red.
#[test]
fn lists_and_maps_are_written_as_json() {
    assert_eq!(
        body(strings(&[(
            "l",
            Value::List(vec![
                Value::from(1),
                Value::from("a"),
                Value::Null,
                Value::Undefined,
                Value::List(vec![Value::from(true)]),
                Value::Map(Map::new()),
            ])
        )])),
        "l=%5B1%2C%22a%22%2Cnull%2Cnull%2C%5Btrue%5D%2C%7B%7D%5D"
    );
    assert_eq!(
        body(strings(&[(
            "m",
            strings(&[
                ("k", Value::from("v\"\\\n\u{1}/</\u{e9}\u{2028}\u{7f}")),
                ("z", Value::List(Vec::new())),
                ("a", strings(&[("u", Value::Undefined)])),
            ])
        )])),
        "m=%7B%22a%22%3A%7B%22u%22%3Anull%7D%2C%22k%22%3A%22v%5C%22%5C%5C%5Cn%5Cu0001%2F%3C%2F\
         %C3%A9%E2%80%A8%7F%22%2C%22z%22%3A%5B%5D%7D"
    );
}

// Sabotage: with a text body's control characters other than the five short
// forms written raw, this goes red.
#[test]
fn a_content_value_is_a_text_body_written_as_its_text() {
    let text_body = |data: Value| {
        let (address, content_type, body) = encoded(&send(None, Some("http://h/s"), data));
        assert_eq!(address, "http://h/s");
        assert_eq!(content_type, "text/plain");
        body
    };
    assert_eq!(
        text_body(Value::from("this is some content")),
        "this is some content"
    );
    assert_eq!(text_body(Value::from(42)), "42");
    assert_eq!(
        text_body(Value::List(vec![Value::from(1), Value::from(2)])),
        "[1,2]"
    );
    assert_eq!(text_body(Value::Null), "null");
    assert_eq!(text_body(Value::from(1.0)), "1.0");
    assert_eq!(text_body(Value::from(true)), "true");
    assert_eq!(
        text_body(Value::List(vec![Value::from("a"), Value::from(1.5e300)])),
        "[\"a\",1.5e300]"
    );
    assert_eq!(
        text_body(Value::List(
            ["\u{1f}", "\u{8}", "\u{c}", "\r", "\u{7f}", "\u{a0}"]
                .into_iter()
                .map(Value::from)
                .collect()
        )),
        "[\"\\u001F\",\"\\b\",\"\\f\",\"\\r\",\"\u{7f}\",\"\u{a0}\"]"
    );
    assert_eq!(
        text_body(Value::List(vec![
            Value::from(-0.0),
            Value::from(1.0e21),
            Value::from(100.0),
            Value::from(1.0e15),
            Value::from(123.456),
            Value::from(5.0e-324),
            Value::from(1.797_693_134_862_315_7e308),
        ])),
        "[-0.0,1.0e21,100.0,1.0e15,123.456,5.0e-324,1.7976931348623157e308]"
    );
}

// Sabotage: with the query always joined by `?`, the first row goes red.
#[test]
fn a_text_body_carries_the_event_name_in_the_query() {
    let address =
        |event: &str, target: &str, data: Value| encoded(&send(Some(event), Some(target), data)).0;
    assert_eq!(
        address("test", "http://h/s?x=1", Value::from("c")),
        "http://h/s?x=1&_scxmleventname=test"
    );
    assert_eq!(
        address("e v&", "http://h/s", Value::from(42)),
        "http://h/s?_scxmleventname=e+v%26"
    );
    assert_eq!(
        address("a b", "http://h/s#frag", Value::from("x")),
        "http://h/s#frag?_scxmleventname=a+b"
    );
    assert_eq!(
        address("e", "http://h/s", Value::from("")),
        "http://h/s?_scxmleventname=e"
    );
}

// Sabotage: with the members of a map of more than 32 keys taken in reverse
// order, this goes red. The reference walks such a map in its runtime's
// hash order instead (the README's differences section).
#[test]
fn a_map_of_more_than_32_members_keeps_key_order() {
    let members: Vec<(Key, Value)> = (1..=40)
        .map(|number: i64| (Key::from(format!("k{number:02}")), Value::from(number)))
        .collect();
    let expected_form: Vec<String> = (1..=40)
        .map(|number| format!("k{number:02}={number}"))
        .collect();
    assert_eq!(body(map(members.clone())), expected_form.join("&"));
    let expected_json: Vec<String> = (1..=40)
        .map(|number| format!("%22k{number:02}%22%3A{number}"))
        .collect();
    assert_eq!(
        body(strings(&[("m", map(members))])),
        format!("m=%7B{}%7D", expected_json.join("%2C"))
    );
}

// Sabotage: with form fields taken in the predicator map's own key order
// (undefined, booleans, integers, strings), this goes red.
#[test]
fn form_fields_of_mixed_keys_follow_the_reference_term_order() {
    assert_eq!(
        body(map(vec![
            (Key::from(1), Value::from("a")),
            (Key::from(true), Value::from("b")),
            (Key::from("s"), Value::from("c")),
            (Key::from(false), Value::from("d")),
            (Key::Undefined, Value::from("e")),
            (Key::from(-3), Value::from("f")),
        ])),
        "-3=f&1=a&false=d&true=b&undefined=e&s=c"
    );
}

// This crate's own rows: a value with no JSON form. The reference writes
// its runtime's inspect text (`[1, ~D[2026-08-06]]`, `%{1 => 2}`).
// Sabotage: with the lenient writer refusing a date, the first row goes red.
#[test]
fn a_value_with_no_json_form_is_written_leniently() {
    let date = Value::from(Date::new(2026, 8, 6).expect("a date"));
    assert_eq!(
        encoded(&send(
            None,
            Some("http://h/s"),
            Value::List(vec![Value::from(1), date.clone()])
        ))
        .2,
        r#"[1,{"$type":"date","value":"2026-08-06"}]"#
    );
    assert_eq!(
        encoded(&send(None, Some("http://h/s"), date)).2,
        r#"{"$type":"date","value":"2026-08-06"}"#
    );
    assert_eq!(
        body(strings(&[("m", map(vec![(Key::from(1), Value::from(2))]))])),
        "m=%7B%221%22%3A2%7D"
    );
    assert_eq!(
        encoded(&send(None, Some("http://h/s"), Value::from(f64::NAN))).2,
        "null"
    );
}

// A value nested far deeper than any recursion could walk is written, not
// a crash: the writer keeps a work list. Sabotage: with a list's closing
// bracket dropped, this goes red.
#[test]
fn a_deep_value_is_written_without_recursion() {
    let mut value = Value::from(0);
    for _ in 0..200_000 {
        value = Value::List(vec![value]);
    }
    let text = text(&value);
    assert_eq!(text.len(), 400_001);
    assert!(text.starts_with("[[") && text.ends_with("]]"));
}

// Sabotage: with the send id left unescaped in the key, the first row goes
// red.
#[test]
fn the_send_key_names_the_send() {
    let key = |session_id: &str, send: &Effect| {
        request(session_id, send)
            .expect("a target")
            .headers
            .into_iter()
            .find(|(name, _)| name == SEND_KEY)
            .map(|(_, value)| value)
            .expect("a key")
    };
    let mut named = send(None, Some("http://h/s"), Value::Undefined);
    if let Effect::Send {
        send_id, ordinal, ..
    } = &mut named
    {
        *send_id = "id/x y~".to_owned();
        *ordinal = Some(3);
    }
    assert_eq!(
        key("s 1/\u{e9}", &named),
        "s%201%2F%C3%A9/id%2Fx%20y~/0/1/2/2/onentry.1.0/3"
    );
    let owned = |owner: BlockOwner, ordinal: Option<u64>| {
        let mut sent = send(None, Some("http://h/s"), Value::Undefined);
        if let Effect::Send {
            owner: slot,
            ordinal: ordinal_slot,
            ..
        } = &mut sent
        {
            *slot = owner;
            *ordinal_slot = ordinal;
        }
        key("s-1", &sent)
    };
    assert_eq!(
        owned(
            BlockOwner::OnExit {
                state: 2,
                ordinal: 1
            },
            None
        ),
        "s-1/send_1/0/1/2/2/onexit.2.1/"
    );
    assert_eq!(
        owned(
            BlockOwner::Finalize {
                state: 3,
                invoke: 0
            },
            Some(1)
        ),
        "s-1/send_1/0/1/2/2/finalize.3.0/1"
    );
    let mut transition = send(None, Some("http://h/s"), Value::Undefined);
    if let Effect::Send {
        owner,
        macrostep,
        microstep,
        round,
        content_index,
        ..
    } = &mut transition
    {
        *owner = BlockOwner::Transition { transition: 4 };
        *macrostep = 7;
        *microstep = 8;
        *round = 9;
        *content_index = 10;
    }
    assert_eq!(
        key("s-1", &transition),
        "s-1/send_1/7/8/9/10/transition.4/1"
    );
    let delayed = Effect::SendDelayed {
        event: None,
        target: Some(Value::from("http://h/s")),
        send_type: Some(Value::from("basichttp")),
        data: Value::Undefined,
        send_id: "d".to_owned(),
        id_from_author: true,
        delay_ms: 10,
        ordinal: 5,
        content_index: 0,
        owner: BlockOwner::OnEntry {
            state: 1,
            ordinal: 0,
        },
        macrostep: 0,
        microstep: 0,
        round: 0,
    };
    assert_eq!(key("s-1", &delayed), "s-1/d/0/0/0/0/onentry.1.0/5");
}

// Sabotage: with a send of no target answered as a request to the address
// its data writes (the empty string here), this goes red.
#[test]
fn a_send_with_no_target_has_no_request() {
    assert_eq!(
        request("s-1", &send(Some("e"), None, Value::Undefined)),
        None
    );
    assert_eq!(
        request(
            "s-1",
            &Effect::Cancel {
                send_id: Value::from("x"),
                ordinal: 1,
                content_index: 0,
                owner: BlockOwner::OnEntry {
                    state: 1,
                    ordinal: 0
                },
                macrostep: 0,
                microstep: 0,
                round: 0,
            }
        ),
        None
    );
    let (address, _, _) = encoded(&send(None, Some("http://h/s"), Value::Undefined));
    assert_eq!(address, "http://h/s");
    let mut numbered = send(None, None, Value::Undefined);
    if let Effect::Send { target, .. } = &mut numbered {
        *target = Some(Value::from(7));
    }
    assert_eq!(
        request("s-1", &numbered).map(|request| request.address),
        Some("7".to_owned())
    );
}

// The event name follows the reference's rule: named when its value is
// truthy, an undefined one written as the word, a false one left out. The
// reference build answered each row the same. Sabotage: with a false event
// named like any other value, the first row reads `_scxmleventname=false`.
#[test]
fn the_event_name_is_left_out_only_when_false() {
    let with_event = |event: Value, data: Value| {
        let mut sent = send(None, Some("http://h/s"), data);
        if let Effect::Send { event: slot, .. } = &mut sent {
            *slot = Some(event);
        }
        let (address, _, body) = encoded(&sent);
        (address, body)
    };
    let form = |event: Value| with_event(event, Value::Undefined).1;
    assert_eq!(form(Value::from(false)), "");
    assert_eq!(form(Value::Undefined), "_scxmleventname=undefined");
    assert_eq!(form(Value::from(true)), "_scxmleventname=true");
    assert_eq!(form(Value::from(5)), "_scxmleventname=5");
    assert_eq!(form(Value::from(1.5)), "_scxmleventname=1.5");
    assert_eq!(
        with_event(Value::from(false), Value::from("c")),
        ("http://h/s".to_owned(), "c".to_owned())
    );
    assert_eq!(
        with_event(Value::Undefined, Value::from("c")),
        (
            "http://h/s?_scxmleventname=undefined".to_owned(),
            "c".to_owned()
        )
    );
    // This crate's own row: the reference's processor fails on a list.
    assert_eq!(
        form(Value::List(vec![Value::from(1), Value::from(2)])),
        "_scxmleventname=%5B1%2C2%5D"
    );
}
