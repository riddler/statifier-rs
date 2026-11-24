//! `Serialize` and `Deserialize` behind the `serde` feature, written by
//! hand: serde's derive macros are not used, so the feature adds `serde`
//! alone to the dependency graph.
//!
//! A [`Position`] is a map with the TypeScript sibling's keys in camel case,
//! each datamodel root the predicator crate's tagged text of its value. An
//! [`Effect`] and a [`BlockOwner`] are maps whose `kind` is the variant in
//! snake case, beside their fields in camel case; an optional field that is
//! `None` is left out, so a present `null` stays a value. A [`PathSegment`]
//! is `{"key": text}` or `{"index": integer}`, a [`RoundBudget`] its count
//! of rounds or `"infinity"`, a [`Status`] `"running"` or `"done"`.
//!
//! Every type is read through the predicator crate's own `Value`, so what
//! that reader refuses is refused here too: nesting past 100 levels,
//! counted from the type's own map, and an integer outside the 64-bit
//! signed range, which no counter, index or delay here then takes. A key a
//! type does not name is not read.

use crate::{
    ActiveInvocation, BlockOwner, Done, Effect, PathSegment, Position, RoundBudget, Status,
};
use predicator_rs::{Key, Map, Value, tagged};
use serde::de::{self, Deserializer};
use serde::ser::{self, SerializeMap, Serializer};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroU64;

/// The spelling of [`RoundBudget::Unbounded`].
const INFINITY: &str = "infinity";

/// The key a tagged value is read by, which no datamodel root may take.
const TAG: &str = "$type";

// ---- writing

/// A datamodel map written as the map value it is.
struct AsValue<'a>(&'a Map);

impl Serialize for AsValue<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        Value::Map(self.0.clone()).serialize(serializer)
    }
}

/// A position's datamodel: each root's name to its tagged text.
struct AsText<'a>(&'a Map);

impl Serialize for AsText<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut roots = serializer.serialize_map(Some(self.0.len()))?;
        for (key, value) in self.0.iter() {
            let root = match key {
                Key::String(root) if root != TAG => root,
                _ => {
                    return Err(ser::Error::custom(format!(
                        "datamodel: a root named {key:?} has no place in a JSON object"
                    )));
                }
            };
            let text = tagged::encode(value).map_err(|error| {
                ser::Error::custom(format!("datamodel.{root}: {}", error.reason()))
            })?;
            roots.serialize_entry(root, &text)?;
        }
        roots.end()
    }
}

impl Serialize for RoundBudget {
    /// A bounded budget is its count of rounds; no bound is `"infinity"`, as
    /// the reference and the TypeScript sibling spell it.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            RoundBudget::Rounds(rounds) => serializer.serialize_u64(rounds.get()),
            RoundBudget::Unbounded => serializer.serialize_str(INFINITY),
        }
    }
}

impl Serialize for Status {
    /// `"running"` or `"done"`.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(match self {
            Status::Running => "running",
            Status::Done => "done",
        })
    }
}

impl Serialize for ActiveInvocation {
    /// `state`, `invokeIndex` and `invokeId`.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(3))?;
        map.serialize_entry("state", &self.state)?;
        map.serialize_entry("invokeIndex", &self.invoke_index)?;
        map.serialize_entry("invokeId", &self.invoke_id)?;
        map.end()
    }
}

impl Serialize for Position {
    /// The TypeScript sibling's keys, in camel case.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(17))?;
        map.serialize_entry("identity", &self.identity)?;
        map.serialize_entry("configuration", &self.configuration)?;
        map.serialize_entry("enteredStates", &self.entered_states)?;
        map.serialize_entry("statesToInvoke", &self.states_to_invoke)?;
        map.serialize_entry("historyValues", &self.history_values)?;
        map.serialize_entry("activeInvocations", &self.active_invocations)?;
        map.serialize_entry("invokeCounter", &self.invoke_counter)?;
        map.serialize_entry("sendCounter", &self.send_counter)?;
        map.serialize_entry("timerCounter", &self.timer_counter)?;
        map.serialize_entry("datamodel", &AsText(&self.datamodel))?;
        map.serialize_entry("running", &self.running)?;
        map.serialize_entry("status", &self.status)?;
        map.serialize_entry("macrostep", &self.macrostep)?;
        map.serialize_entry("microstep", &self.microstep)?;
        map.serialize_entry("round", &self.round)?;
        map.serialize_entry("trace", &self.trace)?;
        map.serialize_entry("maxMacrostepRounds", &self.max_macrostep_rounds)?;
        map.end()
    }
}

impl Serialize for Done {
    /// `donedata`, `configuration` and `activeLeafStates`.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(3))?;
        map.serialize_entry("donedata", &self.donedata)?;
        map.serialize_entry("configuration", &self.configuration)?;
        map.serialize_entry("activeLeafStates", &self.active_leaf_states)?;
        map.end()
    }
}

impl Serialize for PathSegment {
    /// `{"key": text}` or `{"index": integer}`.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(1))?;
        match self {
            PathSegment::Key(key) => map.serialize_entry("key", key)?,
            PathSegment::Index(index) => map.serialize_entry("index", index)?,
        }
        map.end()
    }
}

impl Serialize for BlockOwner {
    /// Its `kind` beside its fields.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        match self {
            BlockOwner::OnEntry { state, ordinal } | BlockOwner::OnExit { state, ordinal } => {
                let kind = if matches!(self, BlockOwner::OnEntry { .. }) {
                    "on_entry"
                } else {
                    "on_exit"
                };
                map.serialize_entry("kind", kind)?;
                map.serialize_entry("state", state)?;
                map.serialize_entry("ordinal", ordinal)?;
            }
            BlockOwner::Transition { transition } => {
                map.serialize_entry("kind", "transition")?;
                map.serialize_entry("transition", transition)?;
            }
            BlockOwner::Finalize { state, invoke } => {
                map.serialize_entry("kind", "finalize")?;
                map.serialize_entry("state", state)?;
                map.serialize_entry("invoke", invoke)?;
            }
        }
        map.end()
    }
}

/// An optional entry, left out when it is `None`.
fn optional<M: SerializeMap, T: Serialize>(
    map: &mut M,
    key: &str,
    value: Option<&T>,
) -> Result<(), M::Error> {
    match value {
        Some(value) => map.serialize_entry(key, value),
        None => Ok(()),
    }
}

/// The three counters every effect carries.
fn counters<M: SerializeMap>(map: &mut M, counters: [u64; 3]) -> Result<(), M::Error> {
    map.serialize_entry("macrostep", &counters[0])?;
    map.serialize_entry("microstep", &counters[1])?;
    map.serialize_entry("round", &counters[2])
}

impl Serialize for Effect {
    /// Its `kind` beside its fields; a `None` field is left out.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        match self {
            Effect::Log {
                label,
                value,
                content_index,
                owner,
                macrostep,
                microstep,
                round,
            } => {
                map.serialize_entry("kind", "log")?;
                optional(&mut map, "label", label.as_ref())?;
                optional(&mut map, "value", value.as_ref())?;
                map.serialize_entry("contentIndex", content_index)?;
                map.serialize_entry("owner", owner)?;
                counters(&mut map, [*macrostep, *microstep, *round])?;
            }
            Effect::DatamodelChange {
                location_path,
                location_source,
                new_value,
                prior_value,
                data_index,
                content_index,
                owner,
                macrostep,
                microstep,
                round,
            } => {
                map.serialize_entry("kind", "datamodel_change")?;
                map.serialize_entry("locationPath", location_path)?;
                map.serialize_entry("locationSource", location_source)?;
                map.serialize_entry("newValue", new_value)?;
                map.serialize_entry("priorValue", prior_value)?;
                optional(&mut map, "dataIndex", data_index.as_ref())?;
                optional(&mut map, "contentIndex", content_index.as_ref())?;
                optional(&mut map, "owner", owner.as_ref())?;
                counters(&mut map, [*macrostep, *microstep, *round])?;
            }
            Effect::Send {
                event,
                target,
                send_type,
                data,
                send_id,
                id_from_author,
                ordinal,
                content_index,
                owner,
                macrostep,
                microstep,
                round,
            } => {
                map.serialize_entry("kind", "send")?;
                optional(&mut map, "event", event.as_ref())?;
                optional(&mut map, "target", target.as_ref())?;
                optional(&mut map, "sendType", send_type.as_ref())?;
                map.serialize_entry("data", data)?;
                map.serialize_entry("sendId", send_id)?;
                map.serialize_entry("idFromAuthor", id_from_author)?;
                optional(&mut map, "ordinal", ordinal.as_ref())?;
                map.serialize_entry("contentIndex", content_index)?;
                map.serialize_entry("owner", owner)?;
                counters(&mut map, [*macrostep, *microstep, *round])?;
            }
            Effect::SendDelayed {
                event,
                target,
                send_type,
                data,
                send_id,
                id_from_author,
                delay_ms,
                ordinal,
                content_index,
                owner,
                macrostep,
                microstep,
                round,
            } => {
                map.serialize_entry("kind", "send_delayed")?;
                optional(&mut map, "event", event.as_ref())?;
                optional(&mut map, "target", target.as_ref())?;
                optional(&mut map, "sendType", send_type.as_ref())?;
                map.serialize_entry("data", data)?;
                map.serialize_entry("sendId", send_id)?;
                map.serialize_entry("idFromAuthor", id_from_author)?;
                map.serialize_entry("delayMs", delay_ms)?;
                map.serialize_entry("ordinal", ordinal)?;
                map.serialize_entry("contentIndex", content_index)?;
                map.serialize_entry("owner", owner)?;
                counters(&mut map, [*macrostep, *microstep, *round])?;
            }
            Effect::DatamodelInit {
                datamodel,
                macrostep,
                microstep,
                round,
            } => {
                map.serialize_entry("kind", "datamodel_init")?;
                map.serialize_entry("datamodel", &AsValue(datamodel))?;
                counters(&mut map, [*macrostep, *microstep, *round])?;
            }
            Effect::BudgetExhausted {
                configuration,
                budget,
                pending_internal_events,
                macrostep,
                microstep,
                round,
            } => {
                map.serialize_entry("kind", "budget_exhausted")?;
                map.serialize_entry("configuration", configuration)?;
                map.serialize_entry("budget", budget)?;
                map.serialize_entry("pendingInternalEvents", pending_internal_events)?;
                counters(&mut map, [*macrostep, *microstep, *round])?;
            }
            Effect::Done {
                donedata,
                donedata_error,
                configuration,
                macrostep,
                microstep,
                round,
            } => {
                map.serialize_entry("kind", "done")?;
                optional(&mut map, "donedata", donedata.as_ref())?;
                optional(&mut map, "donedataError", donedata_error.as_ref())?;
                map.serialize_entry("configuration", configuration)?;
                counters(&mut map, [*macrostep, *microstep, *round])?;
            }
            Effect::Cancel {
                send_id,
                ordinal,
                content_index,
                owner,
                macrostep,
                microstep,
                round,
            } => {
                map.serialize_entry("kind", "cancel")?;
                map.serialize_entry("sendId", send_id)?;
                map.serialize_entry("ordinal", ordinal)?;
                map.serialize_entry("contentIndex", content_index)?;
                map.serialize_entry("owner", owner)?;
                counters(&mut map, [*macrostep, *microstep, *round])?;
            }
        }
        map.end()
    }
}

// ---- reading

/// A map's fields by name, taken one at a time; a name not taken is not
/// read.
struct Fields(BTreeMap<String, Value>);

impl Fields {
    /// The fields of `value`, which must be a map with string keys.
    fn of(value: Value, what: &str) -> Result<Fields, String> {
        let Value::Map(ref map) = value else {
            return Err(format!("{what} is not a map"));
        };
        let mut fields = BTreeMap::new();
        for (key, item) in map.iter() {
            if let Key::String(name) = key {
                fields.insert(name.clone(), item.clone());
            }
        }
        Ok(Fields(fields))
    }

    fn take(&mut self, name: &str) -> Result<Value, String> {
        self.0
            .remove(name)
            .ok_or_else(|| format!("missing field `{name}`"))
    }

    fn optional(&mut self, name: &str) -> Option<Value> {
        self.0.remove(name)
    }

    fn take_as<T>(
        &mut self,
        name: &str,
        read: impl Fn(&Value) -> Option<T>,
        what: &str,
    ) -> Result<T, String> {
        let value = self.take(name)?;
        read(&value).ok_or_else(|| format!("`{name}` is not {what}"))
    }

    fn optional_as<T>(
        &mut self,
        name: &str,
        read: impl Fn(&Value) -> Option<T>,
        what: &str,
    ) -> Result<Option<T>, String> {
        match self.optional(name) {
            None => Ok(None),
            Some(value) => read(&value)
                .map(Some)
                .ok_or_else(|| format!("`{name}` is not {what}")),
        }
    }

    fn u64(&mut self, name: &str) -> Result<u64, String> {
        self.take_as(name, as_u64, "a count")
    }

    fn usize(&mut self, name: &str) -> Result<usize, String> {
        self.take_as(name, as_usize, "an index")
    }

    fn string(&mut self, name: &str) -> Result<String, String> {
        self.take_as(name, as_string, "a string")
    }

    fn bool(&mut self, name: &str) -> Result<bool, String> {
        self.take_as(
            name,
            |value| match value {
                Value::Bool(flag) => Some(*flag),
                _ => None,
            },
            "a boolean",
        )
    }

    fn owner(&mut self) -> Result<BlockOwner, String> {
        owner_of(self.take("owner")?)
    }

    fn counters(&mut self) -> Result<[u64; 3], String> {
        Ok([
            self.u64("macrostep")?,
            self.u64("microstep")?,
            self.u64("round")?,
        ])
    }
}

fn as_u64(value: &Value) -> Option<u64> {
    match value {
        Value::Integer(number) => u64::try_from(*number).ok(),
        _ => None,
    }
}

fn as_usize(value: &Value) -> Option<usize> {
    as_u64(value).and_then(|number| usize::try_from(number).ok())
}

fn as_string(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        _ => None,
    }
}

fn list_of<T>(value: &Value, read: impl Fn(&Value) -> Option<T>) -> Option<Vec<T>> {
    match value {
        Value::List(items) => items.iter().map(read).collect(),
        _ => None,
    }
}

fn id_set(value: &Value) -> Option<BTreeSet<String>> {
    list_of(value, as_string).map(|ids| ids.into_iter().collect())
}

fn budget_of(value: &Value) -> Option<RoundBudget> {
    match value {
        Value::String(text) if text == INFINITY => Some(RoundBudget::Unbounded),
        _ => as_u64(value)
            .and_then(NonZeroU64::new)
            .map(RoundBudget::Rounds),
    }
}

fn status_of(value: &Value) -> Option<Status> {
    match value {
        Value::String(text) if text == "running" => Some(Status::Running),
        Value::String(text) if text == "done" => Some(Status::Done),
        _ => None,
    }
}

fn invocation_of(value: Value) -> Result<ActiveInvocation, String> {
    let mut fields = Fields::of(value, "an active invocation")?;
    Ok(ActiveInvocation {
        state: fields.string("state")?,
        invoke_index: fields.u64("invokeIndex")?,
        invoke_id: fields.string("invokeId")?,
    })
}

fn history_of(value: Value) -> Result<BTreeMap<String, BTreeSet<String>>, String> {
    Fields::of(value, "`historyValues`")?
        .0
        .into_iter()
        .map(|(history, recorded)| {
            id_set(&recorded)
                .map(|recorded| (history.clone(), recorded))
                .ok_or_else(|| format!("`historyValues.{history}` is not a list of ids"))
        })
        .collect()
}

fn datamodel_of(value: Value) -> Result<Map, String> {
    let mut data = Map::new();
    for (root, text) in Fields::of(value, "`datamodel`")?.0 {
        let Value::String(ref text) = text else {
            return Err(format!("`datamodel.{root}` is not tagged text"));
        };
        let value = tagged::decode(text)
            .map_err(|error| format!("`datamodel.{root}`: {}", error.reason()))?;
        data.insert(root, value);
    }
    Ok(data)
}

fn position_of(value: Value) -> Result<Position, String> {
    let mut fields = Fields::of(value, "a position")?;
    let active_invocations = match fields.take("activeInvocations")? {
        Value::List(ref items) => items
            .iter()
            .cloned()
            .map(invocation_of)
            .collect::<Result<_, _>>()?,
        _ => return Err("`activeInvocations` is not a list".to_owned()),
    };
    Ok(Position {
        identity: fields.optional("identity").unwrap_or(Value::Null),
        configuration: fields.take_as("configuration", id_set, "a list of ids")?,
        entered_states: fields.take_as("enteredStates", id_set, "a list of ids")?,
        states_to_invoke: fields.take_as("statesToInvoke", id_set, "a list of ids")?,
        history_values: history_of(fields.take("historyValues")?)?,
        active_invocations,
        invoke_counter: fields.u64("invokeCounter")?,
        send_counter: fields.u64("sendCounter")?,
        timer_counter: fields.u64("timerCounter")?,
        datamodel: datamodel_of(fields.take("datamodel")?)?,
        running: fields.bool("running")?,
        status: fields.take_as("status", status_of, "\"running\" or \"done\"")?,
        macrostep: fields.u64("macrostep")?,
        microstep: fields.u64("microstep")?,
        round: fields.u64("round")?,
        trace: fields.bool("trace")?,
        max_macrostep_rounds: fields.take_as(
            "maxMacrostepRounds",
            budget_of,
            "a count above zero or \"infinity\"",
        )?,
    })
}

fn done_of(value: Value) -> Result<Done, String> {
    let mut fields = Fields::of(value, "a done answer")?;
    let ids = |value: &Value| list_of(value, as_string);
    Ok(Done {
        donedata: fields.take("donedata")?,
        configuration: fields.take_as("configuration", ids, "a list of ids")?,
        active_leaf_states: fields.take_as("activeLeafStates", ids, "a list of ids")?,
    })
}

fn segment_of(value: &Value) -> Option<PathSegment> {
    let Value::Map(map) = value else {
        return None;
    };
    if map.len() != 1 {
        return None;
    }
    match (map.get(&Key::from("key")), map.get(&Key::from("index"))) {
        (Some(Value::String(key)), None) => Some(PathSegment::Key(key.clone())),
        (None, Some(Value::Integer(index))) => Some(PathSegment::Index(*index)),
        _ => None,
    }
}

fn owner_of(value: Value) -> Result<BlockOwner, String> {
    let mut fields = Fields::of(value, "a block owner")?;
    match fields.string("kind")?.as_str() {
        "on_entry" => Ok(BlockOwner::OnEntry {
            state: fields.usize("state")?,
            ordinal: fields.usize("ordinal")?,
        }),
        "on_exit" => Ok(BlockOwner::OnExit {
            state: fields.usize("state")?,
            ordinal: fields.usize("ordinal")?,
        }),
        "transition" => Ok(BlockOwner::Transition {
            transition: fields.usize("transition")?,
        }),
        "finalize" => Ok(BlockOwner::Finalize {
            state: fields.usize("state")?,
            invoke: fields.usize("invoke")?,
        }),
        kind => Err(format!("a block owner of kind {kind:?} is unknown")),
    }
}

fn effect_of(value: Value) -> Result<Effect, String> {
    let mut fields = Fields::of(value, "an effect")?;
    let kind = fields.string("kind")?;
    let effect = match kind.as_str() {
        "log" => {
            let label = fields.optional_as("label", as_string, "a string")?;
            let value = fields.optional("value");
            let content_index = fields.usize("contentIndex")?;
            let owner = fields.owner()?;
            let [macrostep, microstep, round] = fields.counters()?;
            Effect::Log {
                label,
                value,
                content_index,
                owner,
                macrostep,
                microstep,
                round,
            }
        }
        "datamodel_change" => {
            let location_path = fields.take_as(
                "locationPath",
                |value| list_of(value, segment_of),
                "a location path",
            )?;
            let location_source = fields.string("locationSource")?;
            let new_value = fields.take("newValue")?;
            let prior_value = fields.take("priorValue")?;
            let data_index = fields.optional_as("dataIndex", as_usize, "an index")?;
            let content_index = fields.optional_as("contentIndex", as_usize, "an index")?;
            let owner = fields.optional("owner").map(owner_of).transpose()?;
            let [macrostep, microstep, round] = fields.counters()?;
            Effect::DatamodelChange {
                location_path,
                location_source,
                new_value,
                prior_value,
                data_index,
                content_index,
                owner,
                macrostep,
                microstep,
                round,
            }
        }
        "send" | "send_delayed" => {
            let event = fields.optional("event");
            let target = fields.optional("target");
            let send_type = fields.optional("sendType");
            let data = fields.take("data")?;
            let send_id = fields.string("sendId")?;
            let id_from_author = fields.bool("idFromAuthor")?;
            let content_index = fields.usize("contentIndex")?;
            let owner = fields.owner()?;
            if kind == "send" {
                let ordinal = fields.optional_as("ordinal", as_u64, "a count")?;
                let [macrostep, microstep, round] = fields.counters()?;
                Effect::Send {
                    event,
                    target,
                    send_type,
                    data,
                    send_id,
                    id_from_author,
                    ordinal,
                    content_index,
                    owner,
                    macrostep,
                    microstep,
                    round,
                }
            } else {
                let delay_ms = fields.take_as(
                    "delayMs",
                    |value| match value {
                        Value::Integer(delay) => Some(*delay),
                        _ => None,
                    },
                    "an integer",
                )?;
                let ordinal = fields.u64("ordinal")?;
                let [macrostep, microstep, round] = fields.counters()?;
                Effect::SendDelayed {
                    event,
                    target,
                    send_type,
                    data,
                    send_id,
                    id_from_author,
                    delay_ms,
                    ordinal,
                    content_index,
                    owner,
                    macrostep,
                    microstep,
                    round,
                }
            }
        }
        "datamodel_init" => {
            let Value::Map(ref datamodel) = fields.take("datamodel")? else {
                return Err("`datamodel` is not a map".to_owned());
            };
            let datamodel = datamodel.clone();
            let [macrostep, microstep, round] = fields.counters()?;
            Effect::DatamodelInit {
                datamodel,
                macrostep,
                microstep,
                round,
            }
        }
        "budget_exhausted" => {
            let configuration = fields.take_as(
                "configuration",
                |value| list_of(value, as_usize),
                "a list of indexes",
            )?;
            let budget = fields.u64("budget")?;
            let pending_internal_events = fields.take_as(
                "pendingInternalEvents",
                |value| list_of(value, |item| Some(item.clone())),
                "a list",
            )?;
            let [macrostep, microstep, round] = fields.counters()?;
            Effect::BudgetExhausted {
                configuration,
                budget,
                pending_internal_events,
                macrostep,
                microstep,
                round,
            }
        }
        "done" => {
            let donedata = fields.optional("donedata");
            let donedata_error = fields.optional("donedataError");
            let configuration = fields.take_as(
                "configuration",
                |value| list_of(value, as_usize),
                "a list of indexes",
            )?;
            let [macrostep, microstep, round] = fields.counters()?;
            Effect::Done {
                donedata,
                donedata_error,
                configuration,
                macrostep,
                microstep,
                round,
            }
        }
        "cancel" => {
            let send_id = fields.take("sendId")?;
            let ordinal = fields.u64("ordinal")?;
            let content_index = fields.usize("contentIndex")?;
            let owner = fields.owner()?;
            let [macrostep, microstep, round] = fields.counters()?;
            Effect::Cancel {
                send_id,
                ordinal,
                content_index,
                owner,
                macrostep,
                microstep,
                round,
            }
        }
        _ => return Err(format!("an effect of kind {kind:?} is unknown")),
    };
    Ok(effect)
}

/// `Deserialize` for a type read through a value: the value first, then
/// the type from it, a refusal as the deserializer's error.
macro_rules! read_through_value {
    ($($type:ty => $read:expr),* $(,)?) => {
        $(
            impl<'de> Deserialize<'de> for $type {
                fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                    let value = Value::deserialize(deserializer)?;
                    $read(value).map_err(de::Error::custom)
                }
            }
        )*
    };
}

read_through_value! {
    Position => position_of,
    Effect => effect_of,
    BlockOwner => owner_of,
    Done => done_of,
    ActiveInvocation => invocation_of,
    PathSegment => |value: Value| segment_of(&value).ok_or("not a path segment"),
    RoundBudget => |value: Value| budget_of(&value).ok_or("not a count above zero or \"infinity\""),
    Status => |value: Value| status_of(&value).ok_or("not \"running\" or \"done\""),
}
