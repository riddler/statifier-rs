use super::*;

fn data(roots: &[(&str, Value)]) -> Map {
    roots
        .iter()
        .map(|(root, value)| (*root, value.clone()))
        .collect()
}

fn map(entries: &[(&str, Value)]) -> Value {
    Value::Map(data(entries))
}

fn key(name: &str) -> PathSegment {
    PathSegment::Key(name.to_owned())
}

fn written(data: &Map, source: &str, value: Value) -> Written {
    match write_location(data, source, value) {
        Ok(written) => written,
        Err(failure) => panic!("{source}: {failure:?}"),
    }
}

fn reason(data: &Map, source: &str) -> LocationReason {
    match write_location(data, source, Value::from(1)) {
        Err(Failure::Location { reason, .. }) => reason,
        other => panic!("{source}: {other:?}"),
    }
}

// Sabotage: with resolve answering the root alone, the dotted and indexed
// paths lose their segments and this fails; with string_literal refusing an
// escape other than \n, \t, \r, \\, \' and \", `a['\q']` is a computed key and
// this fails.
#[test]
fn resolves_a_root_a_dotted_path_an_index_and_a_name_index() {
    let data = data(&[
        ("a", Value::Null),
        ("i", Value::from(2)),
        ("k", Value::from("p")),
    ]);
    assert_eq!(resolve("a", &data), Ok(vec![key("a")]));
    assert_eq!(
        resolve(" a . b . c ", &data),
        Ok(vec![key("a"), key("b"), key("c")])
    );
    assert_eq!(
        resolve("a[0][-1]['x'][\"y\\\"z\"]", &data),
        Ok(vec![
            key("a"),
            PathSegment::Index(0),
            PathSegment::Index(-1),
            key("x"),
            key("y\"z"),
        ])
    );
    assert_eq!(
        resolve(r#"a['\q']["\$"]"#, &data),
        Ok(vec![key("a"), key("q"), key("$")])
    );
    assert_eq!(
        resolve("a[i].b[k]", &data),
        Ok(vec![key("a"), PathSegment::Index(2), key("b"), key("p")])
    );
}

// Sabotage: with bracket reading a computed bracket as a bare name, a
// computed key reads as an undefined name and this fails; with escape
// letting a numeric escape through, `a['\u0041']` is a computed key and
// this fails; with root_literal skipping the escaped character unread, as
// before it read escapes, `'\u'[k]` resolves its key and fails on the
// undefined `k` and this fails.
#[test]
fn refuses_what_is_not_a_location() {
    let data = data(&[("a", Value::Null), ("f", Value::from(1.5))]);
    for source in [
        "", "1", "'a'", "a +", "a.", "a[0", "a.true", "true", "f(x)", "a b",
    ] {
        assert_eq!(
            reason(&data, source),
            LocationReason::NotAssignable,
            "{source}"
        );
    }
    for source in [r"a['\u0041']", r"a['\U']", r"'\u'[k]"] {
        assert_eq!(
            reason(&data, source),
            LocationReason::NotAssignable,
            "{source}"
        );
    }
    for source in ["a[i + 1]", "a[1.5]", "a[true]", "a[[0]]"] {
        assert_eq!(
            reason(&data, source),
            LocationReason::ComputedKey,
            "{source}"
        );
    }
    assert_eq!(
        reason(&data, "a[missing]"),
        LocationReason::UndefinedVariable("missing".to_owned())
    );
    assert_eq!(
        reason(&data, "a[f]"),
        LocationReason::InvalidKey(Value::from(1.5))
    );
    // The later bracket's failure is the one reported, as the reference
    // resolves keys last to first.
    assert_eq!(
        reason(&data, "a[x][y]"),
        LocationReason::UndefinedVariable("y".to_owned())
    );
}

// Sabotage: with the system-variable check dropped, `_event.x` reaches the
// root check and answers an unbound location instead.
#[test]
fn refuses_a_system_root_and_an_undeclared_root() {
    let data = data(&[("_event", Value::Undefined), ("a", Value::from(1))]);
    assert_eq!(
        write_location(&data, "_event.x", Value::from(1)),
        Err(Failure::SystemVariable("_event".to_owned()))
    );
    assert_eq!(
        write_location(&data, "_fresh", Value::from(1)),
        Err(Failure::SystemVariable("_fresh".to_owned()))
    );
    assert_eq!(
        write_location(&data, "b.c", Value::from(1)),
        Err(Failure::UnboundLocation("b.c".to_owned()))
    );
}

// Sabotage: with vivify answering a map before an index too, `a.l[1]`
// builds a map and this fails.
#[test]
fn creates_missing_intermediate_containers() {
    let data = data(&[("a", Value::Undefined)]);
    let after = written(&data, "a.m.l[1]", Value::from("v"));
    assert_eq!(
        after.data.get(&Key::from("a")),
        Some(&map(&[(
            "m",
            map(&[("l", Value::List(vec![Value::Undefined, Value::from("v")]))])
        )]))
    );
    assert_eq!(after.prior_value, Value::Undefined);
    assert_eq!(after.new_value, Value::from("v"));
}

// Sabotage: with read_path answering the root's value, the prior value of
// `a.b` is the whole map and this fails.
#[test]
fn reports_the_prior_value_at_the_full_path() {
    let data = data(&[("a", map(&[("b", Value::from(1))]))]);
    let after = written(&data, "a.b", Value::from(2));
    assert_eq!(after.prior_value, Value::from(1));
    assert_eq!(after.path, vec![key("a"), key("b")]);
    assert_eq!(
        after.data.get(&Key::from("a")),
        Some(&map(&[("b", Value::from(2))]))
    );
    let list = data_with_list();
    assert_eq!(
        written(&list, "l[1]", Value::from(9)).prior_value,
        Value::from(20)
    );
    assert_eq!(
        written(&list, "l[5]", Value::from(9)).prior_value,
        Value::Undefined
    );
}

fn data_with_list() -> Map {
    data(&[("l", Value::List(vec![Value::from(10), Value::from(20)]))])
}

// Sabotage: with put_in reading a negative list index as 0, the negative
// index writes and this fails.
#[test]
fn never_destroys_a_value_to_make_room() {
    let data = data(&[
        ("n", Value::from(5)),
        ("l", Value::List(vec![Value::from(1)])),
        ("m", map(&[])),
    ]);
    assert_eq!(reason(&data, "n.x"), LocationReason::NotAContainer);
    assert_eq!(reason(&data, "l.x"), LocationReason::NotAContainer);
    assert_eq!(reason(&data, "l['x']"), LocationReason::NotAContainer);
    assert_eq!(reason(&data, "l[-1]"), LocationReason::InvalidIndex(-1));
    assert_eq!(reason(&data, "l[-1].x"), LocationReason::InvalidIndex(-1));
    assert_eq!(
        reason(&data, "l[1048576]"),
        LocationReason::InvalidIndex(MAX_LIST_LENGTH)
    );
    // An integer key against a map writes an integer key.
    let after = written(&data, "m[3]", Value::from("x"));
    let mut expected = Map::new();
    expected.insert(3_i64, Value::from("x"));
    assert_eq!(after.data.get(&Key::from("m")), Some(&Value::Map(expected)));
    // The leaf is overwritten whatever it holds.
    let after = written(&data, "l[0]", map(&[]));
    assert_eq!(
        after.data.get(&Key::from("l")),
        Some(&Value::List(vec![map(&[])]))
    );
}

// Sabotage: with the reason's spelling answering a constant, the failure's
// data loses its reason and this fails.
#[test]
fn a_location_failure_reads_as_event_data() {
    let failure = Failure::Location {
        source: "a[x]".to_owned(),
        reason: LocationReason::UndefinedVariable("x".to_owned()),
    };
    let Value::Map(ref data) = failure.data() else {
        panic!("a map");
    };
    assert_eq!(
        data.get(&Key::from("kind")),
        Some(&Value::from("evaluator_error"))
    );
    assert_eq!(
        data.get(&Key::from("type")),
        Some(&Value::from("location_error"))
    );
    assert_eq!(
        data.get(&Key::from("reason")),
        Some(&Value::from("undefined_variable"))
    );
    for reason in [
        LocationReason::NotAssignable,
        LocationReason::ComputedKey,
        LocationReason::InvalidKey(Value::Null),
        LocationReason::NotAContainer,
        LocationReason::InvalidIndex(-1),
    ] {
        assert!(!reason.message().is_empty());
        assert!(!reason.as_str().is_empty());
    }
}
