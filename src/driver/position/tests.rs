use super::*;
use crate::{Effect, Error, Options};
use std::num::NonZeroU64;

const OPEN: &str = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0""#;

fn machine(attributes: &str, body: &str) -> Machine {
    let text = format!("{OPEN}{attributes}>{body}</scxml>");
    match crate::compile(&text) {
        Ok(machine) => machine,
        Err(refusals) => panic!("{text}: {refusals:#?}"),
    }
}

fn set(ids: &[&str]) -> BTreeSet<String> {
    ids.iter().map(|id| (*id).to_owned()).collect()
}

fn budget(rounds: u64) -> RoundBudget {
    RoundBudget::Rounds(NonZeroU64::new(rounds).expect("a budget above zero"))
}

// A chart with a deep history inside `p`, left from `a2` so the history has
// recorded it.
fn with_history() -> (Machine, State) {
    let machine = machine(
        r#" initial="p""#,
        r#"<state id="p" initial="a"><history id="h" type="deep"><transition target="a"/></history>
             <state id="a" initial="a1"><state id="a1"><transition event="e.next" target="a2"/></state><state id="a2"/></state>
             <transition event="e.out" target="q"/></state>
           <state id="q"><transition event="e.back" target="h"/></state>"#,
    );
    let started = machine.start(Options::new("s-1")).unwrap();
    let next = started.state.step("e.next", Value::Undefined).unwrap();
    let out = next.state.step("e.out", Value::Undefined).unwrap();
    (machine, out.state)
}

// Sabotage: with the export naming the configuration as the entered
// states, this goes red.
#[test]
fn the_export_names_every_state_by_id_and_leaves_the_root_out() {
    let (_, state) = with_history();
    let position = state.export_position().unwrap();
    assert_eq!(position.configuration, set(&["q"]));
    assert_eq!(position.entered_states, set(&["a", "a1", "a2", "p", "q"]));
    assert_eq!(
        position.history_values,
        BTreeMap::from([("h".to_owned(), set(&["a2"]))])
    );
    assert!(position.states_to_invoke.is_empty());
    assert!(position.active_invocations.is_empty());
    assert_eq!(position.identity, Value::Null);
    assert_eq!(position.status, Status::Running);
    assert!(position.running);
    assert_eq!(
        (position.macrostep, position.microstep, position.round),
        (3, 1, 1)
    );
    assert_eq!(
        position.datamodel.get(&Key::from("_sessionid")),
        Some(&Value::from("s-1"))
    );
    assert_eq!(position.max_macrostep_rounds, RoundBudget::default());
    assert!(!position.trace);
}

// Sabotage: with the import leaving `history` empty, the restore enters
// `a1` (the default) and this goes red.
#[test]
fn an_imported_position_restores_its_history_and_moves_on_as_the_original_does() {
    let (machine, state) = with_history();
    let imported = machine
        .import_position(&state.export_position().unwrap())
        .unwrap();
    assert_eq!(imported.configuration(), ["q"]);
    assert_eq!(imported.export_position(), state.export_position());
    let back = imported.step("e.back", Value::Undefined).unwrap();
    assert_eq!(back.state.active_leaf_states(), ["a2"]);
    assert_eq!(
        back.state.export_position(),
        state
            .step("e.back", Value::Undefined)
            .unwrap()
            .state
            .export_position()
    );
}

// Sabotage: with the import re-adding nothing to `entered_states`, the late
// `<data>` of `b` binds again and resets `x`, and this goes red.
#[test]
fn an_imported_position_keeps_late_binding_on_the_states_already_entered() {
    let machine = machine(
        r#" binding="late" initial="a""#,
        r#"<state id="a"><transition event="e.go" target="b"/></state>
           <state id="b"><datamodel><data id="x" expr="1"/></datamodel>
             <onentry><assign location="x" expr="x + 1"/></onentry>
             <transition event="e.go" target="a"/></state>"#,
    );
    let started = machine.start(Options::new("s-1")).unwrap();
    let there = started.state.step("e.go", Value::Undefined).unwrap();
    let back = there.state.step("e.go", Value::Undefined).unwrap();
    let imported = machine
        .import_position(&back.state.export_position().unwrap())
        .unwrap();
    let again = imported.step("e.go", Value::Undefined).unwrap();
    let position = again.state.export_position().unwrap();
    assert_eq!(
        position.datamodel.get(&Key::from("x")),
        Some(&Value::from(3))
    );
}

// Sabotage: with the internal-queue check removed, the export answers a
// position and this goes red.
#[test]
fn the_export_refuses_a_state_whose_internal_queue_holds_an_event() {
    let machine = machine(
        r#" initial="a""#,
        r#"<state id="a"><onentry><raise event="e.again"/></onentry><transition event="e.again" target="a"/></state>"#,
    );
    let mut options = Options::new("s-1");
    options.round_budget = budget(5);
    let started = machine.start(options).unwrap();
    assert_eq!(
        started.state.export_position(),
        Err(ExportError::InternalQueueNotEmpty)
    );
    assert_eq!(
        ExportError::InternalQueueNotEmpty.to_string(),
        "the internal queue holds an event"
    );
}

// Sabotage: with the unnameable check letting a nameless state through, the
// export answers a position missing it and this goes red.
#[test]
fn the_export_refuses_a_state_with_no_id_naming_every_one() {
    let machine = machine(
        r#" initial="m""#,
        r#"<state><state><state id="m"/></state></state>"#,
    );
    let started = machine.start(Options::new("s-1")).unwrap();
    let refused = started.state.export_position().unwrap_err();
    assert_eq!(
        refused,
        ExportError::UnnameableStates {
            indexes: vec![1, 2]
        }
    );
    assert_eq!(
        refused.to_string(),
        "states with no id are in the position: 1, 2"
    );
}

// Sabotage: with the unknown-id check answering the first id only, the list
// is one long and this goes red.
#[test]
fn the_import_refuses_every_unknown_id_at_once_sorted() {
    let (machine, state) = with_history();
    let mut position = state.export_position().unwrap();
    position.configuration.insert("zz".to_owned());
    position
        .history_values
        .insert("hh".to_owned(), set(&["a2", "yy"]));
    position.active_invocations.push(ActiveInvocation {
        state: "ww".to_owned(),
        invoke_index: 0,
        invoke_id: "i".to_owned(),
    });
    let refused = machine.import_position(&position).unwrap_err();
    assert_eq!(
        refused,
        ImportError::UnknownStateIds {
            ids: vec!["hh".into(), "ww".into(), "yy".into(), "zz".into()]
        }
    );
    assert_eq!(
        refused.to_string(),
        "the chart holds no state with these ids: hh, ww, yy, zz"
    );
}

// Sabotage: with the session id read whatever its type, the import answers
// a state and this goes red.
#[test]
fn the_import_refuses_a_datamodel_with_no_session_id() {
    let (machine, state) = with_history();
    let mut position = state.export_position().unwrap();
    position.datamodel.insert("_sessionid", Value::from(1));
    let refused = machine.import_position(&position).unwrap_err();
    assert_eq!(refused, ImportError::NoSessionId);
    assert_eq!(
        refused.to_string(),
        "the datamodel's _sessionid is not a string"
    );
    position.datamodel.remove(&Key::from("_sessionid"));
    assert_eq!(
        machine.import_position(&position).unwrap_err(),
        ImportError::NoSessionId
    );
}

// A history value the chart cannot restore is refused as a value, before
// it can send the entry walk round in a circle: a key that is not a
// history, a recorded history (itself among them), and a recorded state
// outside the history's parent.
// Sabotage: with `restorable` answering true, the import of a key that is
// not a history answers a state and this goes red.
#[test]
fn the_import_refuses_a_history_value_the_chart_cannot_restore() {
    let (machine, state) = with_history();
    let refused = |key: &str, recorded: &[&str]| {
        let mut position = state.export_position().unwrap();
        position.history_values = BTreeMap::from([(key.to_owned(), set(recorded))]);
        machine.import_position(&position).unwrap_err()
    };
    for (key, recorded) in [
        ("a", &["a1"][..]),
        ("h", &["h"][..]),
        ("h", &["a2", "q"][..]),
        ("h", &["p"][..]),
    ] {
        assert_eq!(
            refused(key, recorded),
            ImportError::InvalidHistoryValue {
                history: key.to_owned()
            },
            "{key} {recorded:?}"
        );
    }
    assert_eq!(
        refused("h", &["h"]).to_string(),
        "the history value recorded for \"h\" cannot be restored"
    );
}

// Sabotage: with the duplicate check removed, the second invocation
// replaces the first and this goes red.
#[test]
fn the_import_refuses_two_invocations_of_one_invoke_and_carries_the_rest() {
    let (machine, state) = with_history();
    let mut position = state.export_position().unwrap();
    position.states_to_invoke = set(&["q"]);
    position.invoke_counter = 2;
    position.active_invocations = vec![
        ActiveInvocation {
            state: "q".to_owned(),
            invoke_index: 0,
            invoke_id: "i1".to_owned(),
        },
        ActiveInvocation {
            state: "q".to_owned(),
            invoke_index: 1,
            invoke_id: "i2".to_owned(),
        },
    ];
    let carried = machine.import_position(&position).unwrap();
    assert_eq!(carried.export_position(), Ok(position.clone()));

    position.active_invocations[1].invoke_index = 0;
    let refused = machine.import_position(&position).unwrap_err();
    assert_eq!(
        refused,
        ImportError::DuplicateInvocation {
            state: "q".to_owned(),
            invoke_index: 0
        }
    );
    assert_eq!(
        refused.to_string(),
        "two invocations name invoke 0 of \"q\""
    );
}

// Sabotage: with the import's done left `None` for a stopped position, the
// first assertion goes red.
#[test]
fn a_stopped_position_imports_stopped() {
    let machine = machine(
        r#" initial="a""#,
        r#"<state id="a"><transition event="e.go" target="f"/></state>
           <final id="f"><donedata><param name="n" expr="1"/></donedata></final>"#,
    );
    let started = machine.start(Options::new("s-1")).unwrap();
    let stopped = started.state.step("e.go", Value::Undefined).unwrap().state;
    let position = stopped.export_position().unwrap();
    assert_eq!(position.status, Status::Done);
    assert!(!position.running);
    let imported = machine.import_position(&position).unwrap();
    let done = imported.done().expect("a stopped chart");
    assert_eq!(done.donedata, Value::Undefined);
    assert!(done.configuration.is_empty());
    assert_eq!(done.active_leaf_count, 0);
    assert_eq!(
        imported.step("e.go", Value::Undefined).unwrap_err(),
        Error::NotRunning
    );
    assert_eq!(imported.export_position(), Ok(position));
}

// The import reads the registered send types from `_ioprocessors`, so a
// send of one is still answered as an effect for the host, and keeps the
// trace flag, the budget and the session id.
// Sabotage: with `send_types` answering none, the send is refused with
// error.execution and no Send effect is answered.
#[test]
fn the_import_keeps_the_send_types_the_trace_flag_the_budget_and_the_session() {
    let machine = machine(
        r#" initial="a""#,
        r#"<state id="a"><transition event="e.go" target="b"/></state>
           <state id="b"><onentry><send type="x-example" event="e.out" target="somewhere"/></onentry></state>"#,
    );
    let mut options = Options::new("s-7");
    options.send_types = set(&["x-example"]);
    options.trace = true;
    options.round_budget = RoundBudget::Unbounded;
    let started = machine.start(options).unwrap();
    let position = started.state.export_position().unwrap();
    assert!(position.trace);
    assert_eq!(position.max_macrostep_rounds, RoundBudget::Unbounded);
    let imported = machine.import_position(&position).unwrap();
    assert_eq!(imported.session_id, "s-7");
    assert_eq!(imported.core.send_types, set(&["x-example"]));
    let moved = imported.step("e.go", Value::Undefined).unwrap();
    assert!(moved.effects.iter().any(|effect| matches!(
        effect,
        Effect::Send { send_type: Some(Value::String(send_type)), .. } if send_type == "x-example"
    )));
    assert_eq!(imported.export_position(), Ok(position));
}

// Sabotage: with `send_types` keeping a built-in spelling, the set holds
// "scxml" and this goes red.
#[test]
fn the_send_types_leave_out_every_built_in_spelling_and_a_missing_entry() {
    let mut ioprocessors = Map::new();
    ioprocessors.insert("scxml", Value::Map(Map::new()));
    ioprocessors.insert(
        "http://www.w3.org/TR/scxml/#SCXMLEventProcessor",
        Value::Map(Map::new()),
    );
    ioprocessors.insert("x-one", Value::Map(Map::new()));
    ioprocessors.insert(1, Value::Map(Map::new()));
    let mut datamodel = Map::new();
    datamodel.insert("_ioprocessors", Value::Map(ioprocessors));
    assert_eq!(send_types(&datamodel), set(&["x-one"]));
    assert!(send_types(&Map::new()).is_empty());
}

// A tiny deterministic generator, so the arbitrary positions below are the
// same on every run.
struct Draw(u64);

impl Draw {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn pick(&mut self, ids: &[String]) -> BTreeSet<String> {
        ids.iter()
            .filter(|_| self.next() % 3 == 0)
            .cloned()
            .collect()
    }
}

// No import panics, and no imported state panics when it is stepped and
// advanced: every corpus chart takes positions whose fields name arbitrary
// subsets of its states, unknown ids among them, with arbitrary counters and
// either status; each import answers a state or a refusal.
// Sabotage: with the unknown-id check removed and an unknown id read as an
// index past the chart's last state, a step indexes past the states, panics,
// and this goes red.
#[test]
fn no_arbitrary_position_panics_on_import_or_after_it() {
    let mut draw = Draw(0x9e37_79b9_7f4a_7c15);
    let mut imported = 0;
    for (name, bytes) in crate::testing::corpus_documents() {
        let Ok(source) = String::from_utf8(bytes) else {
            continue;
        };
        let Ok(machine) = crate::compile(&source) else {
            continue;
        };
        let mut options = Options::new("s-1");
        options.round_budget = budget(50);
        let Ok(started) = machine.start(options) else {
            continue;
        };
        let Ok(base) = started.state.export_position() else {
            continue;
        };
        let mut ids: Vec<String> = machine.chart().id_to_index.keys().cloned().collect();
        ids.push("unknown".to_owned());
        for _ in 0..8 {
            let mut position = base.clone();
            position.configuration = draw.pick(&ids);
            position.entered_states = draw.pick(&ids);
            position.states_to_invoke = draw.pick(&ids);
            position.history_values = draw
                .pick(&ids)
                .into_iter()
                .map(|key| (key, draw.pick(&ids)))
                .collect();
            position.running = draw.next() % 4 != 0;
            position.status = if draw.next() % 4 == 0 {
                Status::Done
            } else {
                Status::Running
            };
            position.macrostep = draw.next();
            position.microstep = draw.next();
            position.round = draw.next();
            position.send_counter = draw.next();
            position.timer_counter = draw.next();
            let Ok(state) = machine.import_position(&position) else {
                continue;
            };
            imported += 1;
            for event in ["e.go", "e.next", "done.state.s", "error.execution"] {
                let _ = state.step(event, Value::Undefined);
            }
            let _ = state
                .advance(10_000)
                .and_then(|moved| moved.state.step("e.go", Value::Undefined));
            let _ = state.export_position();
            let _ = name;
        }
    }
    assert!(
        imported > 100,
        "only {imported} arbitrary positions imported"
    );
}

#[cfg(feature = "serde")]
mod serde {
    use super::*;
    use crate::{BlockOwner, PathSegment};
    use serde_json::{Value as Json, json};

    // The keys of an exported position in the TypeScript sibling,
    // statifier-ts v0.4.0: `ExportedPosition` in `src/position.ts`, the
    // identity beside the reference's required keys (`POSITION_FIELDS` in
    // `src/driver-shape.ts`).
    const SIBLING_KEYS: [&str; 17] = [
        "identity",
        "configuration",
        "enteredStates",
        "statesToInvoke",
        "historyValues",
        "activeInvocations",
        "invokeCounter",
        "sendCounter",
        "timerCounter",
        "datamodel",
        "running",
        "status",
        "macrostep",
        "microstep",
        "round",
        "trace",
        "maxMacrostepRounds",
    ];

    // Sabotage: with `Position`'s writer naming `entered_states` in snake
    // case, the keys differ and this goes red.
    #[test]
    fn the_json_form_has_the_typescript_siblings_keys() {
        let (_, state) = with_history();
        let json = serde_json::to_value(state.export_position().unwrap()).unwrap();
        let mut keys: Vec<&str> = json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        let mut sibling: Vec<&str> = SIBLING_KEYS.to_vec();
        sibling.sort_unstable();
        assert_eq!(keys, sibling);
    }

    // A position in the TypeScript sibling's JSON form, its identity an
    // object and every datamodel root tagged text, reads and imports here.
    // Sabotage: with `datamodel_of` keeping each root's text as a string,
    // `x` is the string "1" and this goes red.
    #[test]
    fn a_position_in_the_siblings_form_reads_and_imports() {
        let (machine, _) = with_history();
        let sibling = json!({
            "identity": {"contentHash": "abc", "name": null, "version": "3"},
            "configuration": ["q"],
            "enteredStates": ["a", "a1", "a2", "p", "q"],
            "statesToInvoke": [],
            "historyValues": {"h": ["a2"]},
            "activeInvocations": [{"state": "q", "invokeIndex": 0, "invokeId": "q.i"}],
            "invokeCounter": 1,
            "sendCounter": 0,
            "timerCounter": 0,
            "datamodel": {
                "_sessionid": "\"s-9\"",
                "_name": "{\"$type\":\"undefined\"}",
                "_event": "{\"$type\":\"undefined\"}",
                "_ioprocessors": "{}",
                "x": "1"
            },
            "running": true,
            "status": "running",
            "macrostep": 3,
            "microstep": 1,
            "round": 2,
            "trace": false,
            "maxMacrostepRounds": "infinity",
            "extra": "a key that is not required is not read"
        });
        let position: Position = serde_json::from_value(sibling).unwrap();
        assert_eq!(
            position.datamodel.get(&Key::from("x")),
            Some(&Value::from(1))
        );
        assert_eq!(position.max_macrostep_rounds, RoundBudget::Unbounded);
        let state = machine.import_position(&position).unwrap();
        assert_eq!(state.configuration(), ["q"]);
        assert_eq!(state.session_id, "s-9");
        let back = state.step("e.back", Value::Undefined).unwrap();
        assert_eq!(back.state.active_leaf_states(), ["a2"]);
    }

    // Sabotage: with `position_of` taking `identity` as a required field, the
    // position without one is refused and this goes red.
    #[test]
    fn a_position_round_trips_through_json_text_and_needs_no_identity() {
        let (_, state) = with_history();
        let position = state.export_position().unwrap();
        let text = serde_json::to_string(&position).unwrap();
        assert_eq!(serde_json::from_str::<Position>(&text).unwrap(), position);
        let mut json: Json = serde_json::from_str(&text).unwrap();
        assert_eq!(
            json["datamodel"]["_name"],
            json!("{\"$type\":\"undefined\"}")
        );
        assert_eq!(json["maxMacrostepRounds"], json!(10_000));
        assert_eq!(json["status"], json!("running"));
        json.as_object_mut().unwrap().remove("identity");
        assert_eq!(
            serde_json::from_value::<Position>(json).unwrap().identity,
            Value::Null
        );
    }

    // Sabotage: with `budget_of` reading any string as no bound, "forever"
    // reads and this goes red.
    #[test]
    fn a_round_budget_is_a_count_above_zero_or_infinity() {
        assert_eq!(serde_json::to_string(&budget(7)).unwrap(), "7");
        assert_eq!(
            serde_json::to_string(&RoundBudget::Unbounded).unwrap(),
            "\"infinity\""
        );
        assert_eq!(serde_json::from_str::<RoundBudget>("7").unwrap(), budget(7));
        assert_eq!(
            serde_json::from_str::<RoundBudget>("\"infinity\"").unwrap(),
            RoundBudget::Unbounded
        );
        for refused in ["0", "-1", "\"forever\"", "1.5", "null"] {
            assert!(
                serde_json::from_str::<RoundBudget>(refused).is_err(),
                "{refused}"
            );
        }
    }

    // Sabotage: with `AsText` writing an empty text for a value it cannot
    // encode, the NaN serializes and this goes red.
    #[test]
    fn a_datamodel_the_tagged_text_cannot_hold_is_refused_as_an_error() {
        let (_, state) = with_history();
        let mut position = state.export_position().unwrap();
        position.datamodel.insert("x", Value::Float(f64::NAN));
        assert!(serde_json::to_string(&position).is_err());
        let mut position = state.export_position().unwrap();
        position.datamodel.insert(1, Value::from(1));
        assert!(serde_json::to_string(&position).is_err());
        let text = serde_json::to_string(&state.export_position().unwrap()).unwrap();
        let bad = text.replacen("\"_sessionid\":\"\\\"s-1\\\"\"", "\"_sessionid\":\"[\"", 1);
        assert_ne!(bad, text);
        assert!(serde_json::from_str::<Position>(&bad).is_err());
    }

    // No byte prefix of a position's JSON text, and no text with one byte
    // changed, panics on the way in: each answers a position or an error.
    // Sabotage: with `datamodel_of` unwrapping the decode it makes, an
    // altered datamodel text panics and this goes red.
    #[test]
    fn no_truncated_or_altered_json_text_panics() {
        let (machine, state) = with_history();
        let text = serde_json::to_string(&state.export_position().unwrap()).unwrap();
        for end in 0..text.len() {
            if let Some(prefix) = text.get(..end) {
                let _ = serde_json::from_str::<Position>(prefix);
            }
        }
        let bytes = text.as_bytes();
        for at in 0..bytes.len() {
            for replacement in *b"0\"{]x" {
                let mut altered = bytes.to_vec();
                altered[at] = replacement;
                if let Ok(position) = serde_json::from_slice::<Position>(&altered) {
                    if let Ok(state) = machine.import_position(&position) {
                        let _ = state.step("e.back", Value::Undefined);
                    }
                }
            }
        }
    }

    // Every effect a chart answers, the done answer among them, goes
    // through JSON text and back unchanged, a present null apart from an
    // absent value.
    // Sabotage: with the `Log` writer writing a `None` value as null, it is
    // read back as `Some(Null)`, and this goes red.
    #[test]
    fn effects_and_done_round_trip_through_json_text() {
        let machine = machine(
            r#" initial="a""#,
            r#"<datamodel><data id="n" expr="null"/></datamodel>
               <state id="a"><onentry><log label="none"/><log label="null" expr="n"/>
                 <assign location="n" expr="[1]"/><assign location="n[0]" expr="2"/>
                 <send event="e.later" delay="5ms" id="t"/><cancel sendid="t"/>
                 <send type="x-example" event="e.out"/></onentry>
                 <transition event="e.go" target="f"/></state>
               <final id="f"><donedata><param name="k" expr="n"/></donedata></final>"#,
        );
        let mut options = Options::new("s-1");
        options.send_types = set(&["x-example"]);
        let started = machine.start(options).unwrap();
        let stopped = started.state.step("e.go", Value::Undefined).unwrap();
        let effects: Vec<Effect> = started.effects.into_iter().chain(stopped.effects).collect();
        for kind in [
            "log",
            "datamodel_change",
            "send",
            "send_delayed",
            "datamodel_init",
            "done",
            "cancel",
        ] {
            let found = effects
                .iter()
                .any(|effect| serde_json::to_value(effect).unwrap()["kind"] == json!(kind));
            assert!(found, "{kind}");
        }
        let text = serde_json::to_string(&effects).unwrap();
        assert_eq!(serde_json::from_str::<Vec<Effect>>(&text).unwrap(), effects);
        let logs: Vec<&Effect> = effects
            .iter()
            .filter(|effect| matches!(effect, Effect::Log { .. }))
            .collect();
        assert_eq!(serde_json::to_value(logs[1]).unwrap()["value"], Json::Null);
        assert!(
            serde_json::to_value(logs[0])
                .unwrap()
                .get("value")
                .is_none()
        );
        let done = stopped.state.done().unwrap();
        let text = serde_json::to_string(done).unwrap();
        assert!(text.contains("activeLeafStates"));
        assert!(text.contains("\"activeLeafCount\":1"));
        assert_eq!(&serde_json::from_str::<Done>(&text).unwrap(), done);
        assert!(
            serde_json::from_str::<Effect>(
                r#"{"kind":"datamodel_init","datamodel":1,"macrostep":0,"microstep":0,"round":0}"#
            )
            .is_err()
        );
    }

    // Every effect variant and block owner kind goes through JSON text and
    // back, and what the reader refuses it answers as an error naming why.
    // Sabotage: with `owner_of` reading `on_exit` as `OnEntry`, the round
    // trip of the second effect goes red.
    #[test]
    fn every_effect_and_owner_kind_round_trips_and_a_bad_one_is_refused() {
        let owners = [
            BlockOwner::OnEntry {
                state: 1,
                ordinal: 0,
            },
            BlockOwner::OnExit {
                state: 2,
                ordinal: 1,
            },
            BlockOwner::Transition { transition: 3 },
            BlockOwner::Finalize {
                state: 4,
                invoke: 0,
            },
        ];
        let mut effects: Vec<Effect> = owners
            .iter()
            .map(|owner| Effect::Cancel {
                send_id: Value::from("t"),
                ordinal: 1,
                content_index: 0,
                owner: *owner,
                macrostep: 1,
                microstep: 2,
                round: 3,
            })
            .collect();
        effects.push(Effect::BudgetExhausted {
            configuration: vec![0, 1],
            budget: 5,
            pending_internal_events: vec![Value::from("e")],
            macrostep: 1,
            microstep: 1,
            round: 5,
        });
        effects.push(Effect::DatamodelChange {
            location_path: vec![PathSegment::Key("a".into()), PathSegment::Index(-1)],
            location_source: "a[-1]".into(),
            new_value: Value::Undefined,
            prior_value: Value::Null,
            data_index: Some(0),
            content_index: None,
            owner: None,
            macrostep: 1,
            microstep: 1,
            round: 1,
        });
        let text = serde_json::to_string(&effects).unwrap();
        assert_eq!(serde_json::from_str::<Vec<Effect>>(&text).unwrap(), effects);
        let refused = |text: &str| {
            serde_json::from_str::<Effect>(text)
                .unwrap_err()
                .to_string()
        };
        assert!(refused(r#"{"kind":"other"}"#).contains("unknown"));
        assert!(
            refused(r#"{"kind":"budget_exhausted"}"#).contains("missing field `configuration`")
        );
        assert!(refused(r#"[]"#).contains("not a map"));
        assert!(
            refused(r#"{"kind":"cancel","sendId":"t","ordinal":-1}"#)
                .contains("`ordinal` is not a count")
        );
        assert!(
            serde_json::from_str::<BlockOwner>(r#"{"kind":"elsewhere"}"#)
                .unwrap_err()
                .to_string()
                .contains("unknown")
        );
        for bad in [r#"{"key":1}"#, r#"{"key":"a","index":0}"#, r#""a""#] {
            assert!(serde_json::from_str::<PathSegment>(bad).is_err(), "{bad}");
        }
        assert!(serde_json::from_str::<Status>(r#""stopped""#).is_err());
        assert_eq!(
            serde_json::from_str::<Status>(r#""done""#).unwrap(),
            Status::Done
        );
        let invocation = ActiveInvocation {
            state: "a".into(),
            invoke_index: 2,
            invoke_id: "a.i".into(),
        };
        let text = serde_json::to_string(&invocation).unwrap();
        assert_eq!(text, r#"{"state":"a","invokeIndex":2,"invokeId":"a.i"}"#);
        assert_eq!(
            serde_json::from_str::<ActiveInvocation>(&text).unwrap(),
            invocation
        );
    }

    // A position whose fields have the wrong shape is refused naming the
    // field.
    // Sabotage: with `history_of` keeping a value that is not a list as an
    // empty set, the second refusal reads and this goes red.
    #[test]
    fn a_position_of_the_wrong_shape_is_refused_naming_the_field() {
        let (_, state) = with_history();
        let json = serde_json::to_value(state.export_position().unwrap()).unwrap();
        let refused = |field: &str, value: Json| {
            let mut json = json.clone();
            json[field] = value;
            serde_json::from_value::<Position>(json)
                .unwrap_err()
                .to_string()
        };
        assert!(
            refused("activeInvocations", json!({})).contains("`activeInvocations` is not a list")
        );
        assert!(refused("historyValues", json!({"h": 1})).contains("`historyValues.h`"));
        assert!(refused("datamodel", json!({"x": 1})).contains("`datamodel.x` is not tagged text"));
        assert!(refused("status", json!("halted")).contains("`status`"));
        assert!(refused("running", json!(1)).contains("`running` is not a boolean"));
        assert!(refused("configuration", json!("a")).contains("`configuration`"));
        assert!(refused("maxMacrostepRounds", json!(0)).contains("`maxMacrostepRounds`"));
        let mut missing = json.clone();
        missing.as_object_mut().unwrap().remove("round");
        assert!(
            serde_json::from_value::<Position>(missing)
                .unwrap_err()
                .to_string()
                .contains("missing field `round`")
        );
    }
}
