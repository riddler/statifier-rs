//! The position a corpus step's `expect_position` states, rendered from the
//! crate's export and compared, as the reference's host case runner renders
//! and compares it (`render/1` and `compare/3` in
//! `lib/mix/statifier/corpus/position_expectation.ex` at the vendored tag).
//!
//! The rendering is built here, in the test tree, from what the crate
//! exposes: [`State::export_position`] answers the [`Position`], and the
//! rendering keeps exactly seven of its members, as JSON:
//!
//! - `configuration`, `entered_states` and `states_to_invoke`: each set of
//!   state ids as an array sorted ascending.
//! - `history_values`: an object from each history state's id to its
//!   recorded state ids, as an array sorted ascending.
//! - `active_invocations`: one object per live invocation, `state` and
//!   `index` (the `<invoke>`'s ordinal among its state's own), sorted by
//!   `state` and then by `index`; the invoke id is left out.
//! - `running`: the export's flag as it is.
//! - `datamodel`: the export's datamodel without `_event`, `_ioprocessors`
//!   and `_sessionid`, and without `_name` unless the document's `<scxml>`
//!   element carries a `name` attribute. A string, a number, a boolean and
//!   null are written as they are, undefined as null, a list as an array
//!   and a map with string keys as an object; any other value has no JSON
//!   form and is refused, naming its variable.
//!
//! Whether the `<scxml>` element carries a `name` is read from the
//! datamodel itself: the crate binds `_name` to that attribute's value, and
//! to undefined when there is none, at the start, and a chart cannot assign
//! a root whose name begins with an underscore. So `_name` is undefined at
//! every point of a drive exactly when the element carries no `name`.
//!
//! The comparison is of JSON values: two numbers agree when they are equal
//! in value (a float `2.0` agrees with an expected `2`), an array compares
//! element by element in order, an object member by member over the union
//! of both key sets, and a string, a boolean and null agree only with
//! themselves.

use serde_json::{Map as Object, Number, Value as Json};
use statifier::{Key, Map, Position, State, Value};
use std::collections::BTreeSet;

/// The datamodel roots the rendering leaves out in every case.
const LEFT_OUT_VARIABLES: [&str; 3] = ["_event", "_ioprocessors", "_sessionid"];

/// The datamodel root left out unless the document names itself.
const NAME: &str = "_name";

/// `state`'s exported position in the expectation's JSON form, or why it
/// cannot be rendered: the export's own refusal, or a datamodel value with
/// no JSON form.
pub fn render(state: &State) -> Result<Json, String> {
    members(state).map(Json::Object)
}

/// The rendering's members, as [`render`] answers them.
fn members(state: &State) -> Result<Object<String, Json>, String> {
    let position = state
        .export_position()
        .map_err(|refusal| format!("the export refused the position: {refusal}"))?;
    rendering(&position)
}

/// The members of a position already exported.
fn rendering(position: &Position) -> Result<Object<String, Json>, String> {
    let mut rendered = Object::new();
    rendered.insert("configuration".to_owned(), ids(&position.configuration));
    rendered.insert("entered_states".to_owned(), ids(&position.entered_states));
    rendered.insert(
        "states_to_invoke".to_owned(),
        ids(&position.states_to_invoke),
    );
    rendered.insert(
        "history_values".to_owned(),
        Json::Object(
            position
                .history_values
                .iter()
                .map(|(history, recorded)| (history.clone(), ids(recorded)))
                .collect(),
        ),
    );
    rendered.insert("active_invocations".to_owned(), invocations(position));
    rendered.insert("running".to_owned(), Json::Bool(position.running));
    rendered.insert("datamodel".to_owned(), datamodel(&position.datamodel)?);
    Ok(rendered)
}

/// A set of state ids as an array sorted ascending, the order a
/// `BTreeSet` iterates in.
fn ids(set: &BTreeSet<String>) -> Json {
    Json::Array(set.iter().map(|id| Json::String(id.clone())).collect())
}

/// The live invocations as `state` and `index`, sorted by both, the invoke
/// id left out.
fn invocations(position: &Position) -> Json {
    let mut owners: Vec<(&str, u64)> = position
        .active_invocations
        .iter()
        .map(|invocation| (invocation.state.as_str(), invocation.invoke_index))
        .collect();
    owners.sort_unstable();
    Json::Array(
        owners
            .into_iter()
            .map(|(state, index)| serde_json::json!({"state": state, "index": index}))
            .collect(),
    )
}

/// The datamodel without the roots the rendering leaves out, each root's
/// value as JSON; the first root in sorted order whose value has no JSON
/// form is the refusal.
fn datamodel(datamodel: &Map) -> Result<Json, String> {
    let named = !matches!(
        datamodel.get(&Key::from(NAME)),
        None | Some(Value::Undefined)
    );
    let mut roots: Vec<(String, &Value)> = datamodel
        .iter()
        .map(|(key, value)| (key_text(key), value))
        .filter(|(name, _)| !LEFT_OUT_VARIABLES.contains(&name.as_str()))
        .filter(|(name, _)| named || name != NAME)
        .collect();
    roots.sort_by(|left, right| left.0.cmp(&right.0));
    let mut rendered = Object::new();
    for (name, value) in roots {
        let json = json_of(value).ok_or_else(|| {
            format!("the datamodel's {name} holds {value:?}, which has no JSON form here")
        })?;
        rendered.insert(name, json);
    }
    Ok(Json::Object(rendered))
}

/// A datamodel root's name as text.
fn key_text(key: &Key) -> String {
    match key {
        Key::String(text) => text.clone(),
        other => format!("{other:?}"),
    }
}

/// `value` as JSON, `None` when it has no JSON form: a float that is not
/// finite, a map with a key that is not a string, a date, an instant or a
/// duration.
fn json_of(value: &Value) -> Option<Json> {
    match value {
        Value::Null | Value::Undefined => Some(Json::Null),
        Value::Bool(flag) => Some(Json::Bool(*flag)),
        Value::Integer(number) => Some(Json::from(*number)),
        Value::Float(number) => Number::from_f64(*number).map(Json::Number),
        Value::String(text) => Some(Json::String(text.clone())),
        Value::List(items) => items
            .iter()
            .map(json_of)
            .collect::<Option<_>>()
            .map(Json::Array),
        Value::Map(map) => map
            .iter()
            .map(|(key, value)| match key {
                Key::String(text) => json_of(value).map(|json| (text.clone(), json)),
                _ => None,
            })
            .collect::<Option<Object<String, Json>>>()
            .map(Json::Object),
        _ => None,
    }
}

/// Nothing when `expected`, a step's `expect_position`, agrees with the
/// rendering of `state`; else the reason, naming the step by its one-based
/// `number` and its `event`, and each member that differs with both values.
pub fn compare(expected: &Json, state: &State, number: usize, event: &str) -> Result<(), String> {
    let label = format!("after step {number} ({event}), expect_position");
    let expected = expected
        .as_object()
        .ok_or_else(|| format!("{label} is not an object"))?;
    let actual =
        members(state).map_err(|reason| format!("{label} cannot be compared: {reason}"))?;
    let mut keys: Vec<&String> = expected.keys().chain(actual.keys()).collect();
    keys.sort();
    keys.dedup();
    let differing: Vec<String> = keys
        .into_iter()
        .filter(|key| !same_member(expected.get(*key), actual.get(*key)))
        .map(|key| {
            format!(
                "{key}: expected {}, but got {}",
                encode(expected.get(key)),
                encode(actual.get(key))
            )
        })
        .collect();
    if differing.is_empty() {
        Ok(())
    } else {
        Err(format!("{label} differs: {}", differing.join("; ")))
    }
}

/// Whether two members agree: both absent, or both present and [`same`].
fn same_member(left: Option<&Json>, right: Option<&Json>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => same(left, right),
        (None, None) => true,
        _ => false,
    }
}

/// Equality of two JSON values: numbers by value, arrays in order, objects
/// by their members, anything else only with itself.
pub fn same(left: &Json, right: &Json) -> bool {
    match (left, right) {
        (Json::Number(left), Json::Number(right)) => same_number(left, right),
        (Json::Array(left), Json::Array(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right)
                    .all(|(left, right)| same(left, right))
        }
        (Json::Object(left), Json::Object(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .all(|(key, value)| right.get(key).is_some_and(|other| same(value, other)))
        }
        _ => left == right,
    }
}

/// Two numbers equal in value: integers exactly, else as floats.
fn same_number(left: &Number, right: &Number) -> bool {
    if let (Some(left), Some(right)) = (left.as_i64(), right.as_i64()) {
        return left == right;
    }
    if let (Some(left), Some(right)) = (left.as_u64(), right.as_u64()) {
        return left == right;
    }
    match (left.as_f64(), right.as_f64()) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

/// A member's value in a disagreement: absent and null as `nothing`, as the
/// reference writes `nil`, else its JSON text.
fn encode(value: Option<&Json>) -> String {
    match value {
        None | Some(Json::Null) => "nothing".to_owned(),
        Some(value) => value.to_string(),
    }
}
