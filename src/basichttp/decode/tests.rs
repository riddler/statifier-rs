// The decoding table, row by row. Each expected event is what the reference
// build at its tag answered for the same request (the differential in the
// request that added this file).

use super::*;

const FORM_TYPE: &str = "application/x-www-form-urlencoded";

fn received(
    method: &str,
    content_type: Option<&str>,
    body: &[u8],
    query: Option<&str>,
) -> Result<Received, DecodeError> {
    let address = match query {
        Some(query) => format!("http://h/s?{query}"),
        None => "http://h/s".to_owned(),
    };
    let headers = content_type
        .map(|content_type| vec![("Content-Type".to_owned(), content_type.to_owned())])
        .unwrap_or_default();
    decode(&Request::new(method, address, headers, body.to_vec()))
}

fn form(body: &str) -> Result<Received, DecodeError> {
    received("POST", Some(FORM_TYPE), body.as_bytes(), None)
}

fn event(name: &str, data: Value) -> Result<Received, DecodeError> {
    Ok(Received {
        name: name.to_owned(),
        data,
    })
}

fn data(members: &[(&str, Value)]) -> Value {
    let mut map = Map::new();
    for (name, value) in members {
        map.insert(*name, value.clone());
    }
    Value::Map(map)
}

// Sabotage: with the event name taken from the last _scxmleventname, the
// query-before-body row goes red.
#[test]
fn the_name_is_the_first_scxmleventname_else_the_method() {
    assert_eq!(
        form("_scxmleventname=test&param1=2"),
        event("test", data(&[("param1", Value::from(2))]))
    );
    assert_eq!(
        received(
            "POST",
            Some(FORM_TYPE),
            b"_scxmleventname=b&x=1",
            Some("_scxmleventname=q&y=2")
        ),
        event("q", data(&[("x", Value::from(1)), ("y", Value::from(2))]))
    );
    assert_eq!(
        form("_scxmleventname=a&_scxmleventname=b"),
        event("a", Value::Undefined)
    );
    assert_eq!(form("_scxmleventname="), event("", Value::Undefined));
    assert_eq!(form("_scxmleventname"), event("", Value::Undefined));
    assert_eq!(
        received(
            "POST",
            Some(FORM_TYPE),
            b"_scxmleventname=a",
            Some("_scxmleventname")
        ),
        event("", Value::Undefined)
    );
    assert_eq!(
        received("post", None, b"", None),
        event("HTTP.POST", Value::Undefined)
    );
    assert_eq!(
        received("pOsT", None, b"", None),
        event("HTTP.POST", Value::Undefined)
    );
}

// Sabotage: with the method compared without upper-casing, the `post` row
// of the test above goes red; with any method allowed, this goes red.
#[test]
fn only_post_is_allowed() {
    for method in ["GET", "PUT", "post\u{df}"] {
        assert_eq!(
            received(method, None, b"", None),
            Err(DecodeError::MethodNotAllowed {
                method: method.to_owned()
            })
        );
    }
    assert_eq!(
        decode(&Request::new(
            "GET",
            "http://h/s",
            vec![("scxml-send-key".to_owned(), "bad".to_owned())],
            Vec::new()
        )),
        Err(DecodeError::MethodNotAllowed {
            method: "GET".to_owned()
        })
    );
}

// Sabotage: with a text body kept as its raw text rather than read as
// content text, the `42` row goes red.
#[test]
fn a_text_body_is_the_data() {
    assert_eq!(
        received(
            "POST",
            Some("text/plain"),
            b"this is some content",
            Some("_scxmleventname=n&y=2")
        ),
        event("n", Value::from("this is some content"))
    );
    assert_eq!(
        received("POST", Some("text/plain"), b"  42 ", None),
        event("HTTP.POST", Value::from(42))
    );
    assert_eq!(
        received("POST", None, b"", None),
        event("HTTP.POST", Value::Undefined)
    );
    assert_eq!(
        received("POST", None, b"a \n  b", None),
        event("HTTP.POST", Value::from("a b"))
    );
    assert_eq!(
        received("POST", Some("application/json"), b"{\"a\": 1}", None),
        event("HTTP.POST", data(&[("a", Value::from(1))]))
    );
    assert_eq!(
        received("POST", Some(""), b"a=1", None),
        event("HTTP.POST", Value::from("a=1"))
    );
    assert_eq!(
        received(
            "POST",
            Some(" application/x-www-form-urlencoded"),
            b"a=1",
            None
        ),
        event("HTTP.POST", Value::from("a=1"))
    );
}

// Sabotage: with the content type compared without lower-casing, the first
// row goes red.
#[test]
fn a_form_body_is_named_by_its_content_type_in_any_case() {
    assert_eq!(
        received(
            "POST",
            Some("Application/X-WWW-Form-Urlencoded; charset=utf-8"),
            b"a=1",
            None
        ),
        event("HTTP.POST", data(&[("a", Value::from(1))]))
    );
    assert_eq!(
        received("POST", Some(FORM_TYPE), b"a=1", Some("")),
        event("HTTP.POST", data(&[("a", Value::from(1))]))
    );
}

// Sabotage: with every empty segment dropped, the `&&a=1&&` row goes red.
#[test]
fn form_pairs_split_as_the_reference_splits_them() {
    assert_eq!(
        form("a=1&a=2"),
        event("HTTP.POST", data(&[("a", Value::from(2))]))
    );
    assert_eq!(
        form("a"),
        event("HTTP.POST", data(&[("a", Value::Undefined)]))
    );
    assert_eq!(
        form("a=&b"),
        event(
            "HTTP.POST",
            data(&[("a", Value::Undefined), ("b", Value::Undefined)])
        )
    );
    assert_eq!(
        form("&&a=1&&"),
        event(
            "HTTP.POST",
            data(&[("", Value::Undefined), ("a", Value::from(1))])
        )
    );
    assert_eq!(
        form("a=1&"),
        event("HTTP.POST", data(&[("a", Value::from(1))]))
    );
    assert_eq!(
        form("&"),
        event("HTTP.POST", data(&[("", Value::Undefined)]))
    );
    assert_eq!(form(""), event("HTTP.POST", Value::Undefined));
    assert_eq!(
        form("a=b=c"),
        event("HTTP.POST", data(&[("a", Value::from("b=c"))]))
    );
    assert_eq!(
        form("=v"),
        event("HTTP.POST", data(&[("", Value::from("v"))]))
    );
    assert_eq!(
        form("a%3Db=c%26d"),
        event("HTTP.POST", data(&[("a=b", Value::from("c&d"))]))
    );
}

// Sabotage: with a malformed escape decoded as a zero byte rather than
// kept, the `%zz` row goes red.
#[test]
fn form_values_are_percent_decoded() {
    assert_eq!(
        form("k+1=%20v+w%2B"),
        event("HTTP.POST", data(&[("k 1", Value::from("v w+"))]))
    );
    assert_eq!(
        form("a=%zz"),
        event("HTTP.POST", data(&[("a", Value::from("%zz"))]))
    );
    assert_eq!(
        form("a=%"),
        event("HTTP.POST", data(&[("a", Value::from("%"))]))
    );
    assert_eq!(
        form("a=%2&b=%2g&c=%%41&d=%41%4"),
        event(
            "HTTP.POST",
            data(&[
                ("a", Value::from("%2")),
                ("b", Value::from("%2g")),
                ("c", Value::from("%A")),
                ("d", Value::from("A%4")),
            ])
        )
    );
    assert_eq!(
        form("a=%c3%a9"),
        event("HTTP.POST", data(&[("a", Value::from("\u{e9}"))]))
    );
    assert_eq!(
        form("x=%C3%A9&y=caf%C3%A9+au+lait"),
        event(
            "HTTP.POST",
            data(&[
                ("x", Value::from("\u{e9}")),
                ("y", Value::from("caf\u{e9} au lait"))
            ])
        )
    );
}

// Sabotage: with form values kept as strings, the `'q'` and `[1,2]` cells go
// red.
#[test]
fn form_values_are_read_as_content_text_is() {
    assert_eq!(
        form("x=%27q%27&y=a++b&z=%5B1%2C2%5D&w=null&v=+"),
        event(
            "HTTP.POST",
            data(&[
                ("v", Value::Undefined),
                ("w", Value::Null),
                ("x", Value::from("q")),
                ("y", Value::from("a b")),
                ("z", Value::List(vec![Value::from(1), Value::from(2)])),
            ])
        )
    );
}

// Sabotage: with a text body that is not UTF-8 read as empty, the second
// row goes red.
#[test]
fn a_part_that_is_not_utf8_is_refused() {
    let body = Err(DecodeError::NotUtf8 { part: Part::Body });
    let query = Err(DecodeError::NotUtf8 { part: Part::Query });
    assert_eq!(form("a=%C3"), body);
    assert_eq!(received("POST", Some("text/plain"), &[255], None), body);
    assert_eq!(received("POST", None, b"", Some("a=%FF")), query);
    assert_eq!(
        received("POST", Some("text/plain"), b"1", Some("a=%FF")),
        query
    );
    assert_eq!(
        received(
            "POST",
            Some("text/plain"),
            &[255],
            Some("_scxmleventname=%FF")
        ),
        query
    );
}

// Sabotage: with the key's field count unchecked, the seven-field row goes
// red.
#[test]
fn the_send_key_is_checked_for_its_shape_only() {
    let keyed = |key: &str| {
        decode(&Request::new(
            "POST",
            "http://h/s",
            vec![("SCXML-Send-Key".to_owned(), key.to_owned())],
            "a",
        ))
    };
    for key in ["a/b/1/2/3/4/onentry.1.0/5", "a/%zz/1/2/3/4/o/5", "///////"] {
        assert_eq!(keyed(key), event("HTTP.POST", Value::from("a")));
    }
    for key in [
        "a/b/1/2/3/4/onentry.1.0",
        "a/%FF/1/2/3/4/o/5",
        "a/b/c/d/e/f/g/h/i",
    ] {
        assert_eq!(
            keyed(key),
            Err(DecodeError::MalformedSendKey {
                value: key.to_owned()
            })
        );
    }
}

// Sabotage: with the query read past a `#`, the fragment joins the value.
#[test]
fn the_query_ends_at_a_fragment() {
    assert_eq!(
        decode(&Request::new("POST", "http://h/s?a=1#f", Vec::new(), "")),
        event("HTTP.POST", Value::Undefined)
    );
    assert_eq!(
        decode(&Request::new(
            "POST",
            "http://h/s?_scxmleventname=n#f",
            Vec::new(),
            ""
        )),
        event("n", Value::Undefined)
    );
}
