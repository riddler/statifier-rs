use super::*;
use crate::datamodel::ERROR_COMMUNICATION;
use crate::effect::{BlockOwner, PathSegment};
use crate::interpreter::content::{Counters, Raised, Ran, execute_block};
use crate::machine::Machine;
use predicator_rs::{Key, Map};

const OPEN: &str = r##"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="s">"##;

const COUNTERS: Counters = Counters {
    macrostep: 3,
    microstep: 2,
    round: 1,
};

const OWNER: BlockOwner = BlockOwner::OnEntry {
    state: 1,
    ordinal: 0,
};

/// What a block runs against besides its content.
struct Given<'a> {
    roots: &'a [(&'a str, Value)],
    send_types: &'a [&'a str],
    routes: Option<Routes>,
    sends: Sends,
}

const NOTHING: Given<'static> = Given {
    roots: &[],
    send_types: &[],
    routes: None,
    sends: Sends {
        send_counter: 0,
        timer_counter: 0,
    },
};

fn machine(onentry: &str) -> Machine {
    let text = format!(
        r##"{OPEN}<state id="s"><onentry>{onentry}</onentry></state><state id="t"/></scxml>"##
    );
    match crate::compile(&text) {
        Ok(machine) => machine,
        Err(refusals) => panic!("{text}: {refusals:#?}"),
    }
}

// The state `s`'s `<onentry>` block run with `s` active.
fn run(onentry: &str, given: &Given<'_>) -> (Ran, Vec<usize>) {
    let machine = machine(onentry);
    let chart = machine.chart();
    let state = chart.id_to_index["s"];
    let configuration: BTreeSet<usize> = [state].into_iter().collect();
    let send_types: BTreeSet<String> = given.send_types.iter().map(|t| (*t).to_owned()).collect();
    let site = Site {
        chart,
        configuration: &configuration,
        owner: BlockOwner::OnEntry { state, ordinal: 0 },
        counters: COUNTERS,
        send_types: &send_types,
        routes: given.routes.as_ref(),
    };
    let data: Map = given
        .roots
        .iter()
        .map(|(root, value)| (*root, value.clone()))
        .collect();
    let content = chart.states[state].onentry[0].content.clone();
    (execute_block(&site, data, given.sends, &content), content)
}

fn routes(sessions: &[&str], parent: bool, invokes: &[&str]) -> Routes {
    Routes {
        sessions: sessions.iter().map(|id| (*id).to_owned()).collect(),
        parent,
        invokes: invokes.iter().map(|id| (*id).to_owned()).collect(),
    }
}

/// The one immediate send a block made: its target, send id, ordinal and
/// data.
fn sent(ran: &Ran) -> (Option<Value>, String, Option<u64>, Value) {
    match ran.effects.as_slice() {
        [
            Effect::Send {
                target,
                send_id,
                ordinal,
                data,
                ..
            },
        ] => (target.clone(), send_id.clone(), *ordinal, data.clone()),
        other => panic!("one send: {other:#?}"),
    }
}

/// The refusal a block raised: its event name, send id and data's kind.
fn refused(ran: &Ran) -> (&'static str, Value, Value) {
    let [Raised::Error { failure, .. }] = ran.raised.as_slice() else {
        panic!("one error: {:#?}", ran.raised);
    };
    (
        failure.event_name(),
        field(&failure.event(), "sendid"),
        field(&failure.data(), "kind"),
    )
}

/// The value at `key` in a map value, null when it is not one or holds none.
fn field(value: &Value, key: &str) -> Value {
    match value {
        Value::Map(map) => map.get(&Key::from(key)).cloned().unwrap_or(Value::Null),
        _ => Value::Null,
    }
}

fn map(pairs: &[(&str, Value)]) -> Value {
    Value::Map(pairs.iter().map(|(k, v)| (*k, v.clone())).collect())
}

// Sabotage: with id_from_author always true, the effect differs and this
// fails.
#[test]
fn send_answers_an_immediate_send_with_every_field() {
    let (ran, content) = run(r##"<send event="e.go" target="#_internal"/>"##, &NOTHING);
    assert_eq!(
        ran.effects,
        vec![Effect::Send {
            event: Some(Value::from("e.go")),
            target: Some(Value::from("#_internal")),
            send_type: None,
            data: Value::Undefined,
            send_id: "send_1".to_owned(),
            id_from_author: false,
            ordinal: None,
            content_index: content[0],
            owner: OWNER,
            macrostep: 3,
            microstep: 2,
            round: 1,
        }]
    );
    assert!(ran.raised.is_empty());
    assert_eq!(ran.sends.send_counter, 1);
    assert_eq!(ran.sends.timer_counter, 0);
}

// Sabotage: with parse_target reading `_internal` as invalid, the send is
// refused and this fails.
#[test]
fn an_internal_target_in_either_spelling_is_a_send_whatever_the_routes() {
    for target in ["#_internal", "_internal"] {
        let given = Given {
            routes: Some(Routes::default()),
            ..NOTHING
        };
        let (ran, _) = run(&format!(r##"<send event="e" target="{target}"/>"##), &given);
        assert_eq!(sent(&ran).0, Some(Value::from(target)), "{target}");
    }
}

// Sabotage: with Routes::reachable answering false for the session itself,
// a send with no target is refused and this fails.
#[test]
fn no_target_is_the_session_itself_and_always_reachable() {
    let given = Given {
        routes: Some(Routes::default()),
        ..NOTHING
    };
    let (ran, _) = run(r##"<send event="e"/>"##, &given);
    assert_eq!(sent(&ran).0, None);
    let (ran, _) = run(r##"<send event="e" targetexpr="null"/>"##, &given);
    // A null target is one spelling of no target, carried as `None`.
    assert_eq!(sent(&ran).0, None);
}

// Sabotage: with Routes::reachable answering true for the parent whatever
// it declares, the unreachable parent is sent and this fails.
#[test]
fn a_parent_target_is_judged_against_the_declared_routes() {
    for target in ["#_parent", "_parent"] {
        let body = format!(r##"<send event="e" target="{target}"/>"##);
        let (ran, _) = run(&body, &NOTHING);
        assert_eq!(sent(&ran).0, Some(Value::from(target)));
        let reachable = Given {
            routes: Some(routes(&[], true, &[])),
            ..NOTHING
        };
        let (ran, _) = run(&body, &reachable);
        assert_eq!(sent(&ran).0, Some(Value::from(target)));
        let unreachable = Given {
            routes: Some(Routes::default()),
            ..NOTHING
        };
        let (ran, _) = run(&body, &unreachable);
        assert!(ran.effects.is_empty());
        assert_eq!(
            refused(&ran),
            (
                ERROR_COMMUNICATION,
                Value::from("send_1"),
                Value::from("unreachable_target")
            )
        );
    }
}

// Sabotage: with parse_target never taking the `#_scxml_` arm, the session
// id is read as an invoke id, the declared session is unreachable and this
// fails.
#[test]
fn session_and_invoke_targets_are_judged_against_the_declared_routes() {
    let given = Given {
        routes: Some(routes(&["other"], false, &["child"])),
        ..NOTHING
    };
    for target in ["#_scxml_other", "#_child"] {
        let (ran, _) = run(&format!(r##"<send event="e" target="{target}"/>"##), &given);
        assert_eq!(sent(&ran).0, Some(Value::from(target)), "{target}");
    }
    for target in ["#_scxml_nobody", "#_nobody", "#_scxml_"] {
        let (ran, _) = run(&format!(r##"<send event="e" target="{target}"/>"##), &given);
        assert_eq!(refused(&ran).0, ERROR_COMMUNICATION, "{target}");
    }
}

// Sabotage: with reject_reason judging the route of a delayed send, the
// unreachable parent is refused and this fails.
#[test]
fn a_delayed_send_is_not_judged_against_the_routes() {
    let given = Given {
        routes: Some(Routes::default()),
        ..NOTHING
    };
    let (ran, _) = run(
        r##"<send event="e" target="#_parent" delay="1.5s"/>"##,
        &given,
    );
    assert!(matches!(
        ran.effects.as_slice(),
        [Effect::SendDelayed {
            delay_ms: 1500,
            ordinal: 1,
            ..
        }]
    ));
}

// Sabotage: with parse_target reading an unrecognized target as the
// session itself, the invalid target is sent and this fails.
#[test]
fn an_invalid_target_or_an_unsupported_type_raises_error_execution() {
    for (body, kind) in [
        (
            r##"<send event="e" target="elsewhere"/>"##,
            "invalid_target",
        ),
        (r##"<send event="e" targetexpr="5"/>"##, "invalid_target"),
        (r##"<send event="e" target="#_"/>"##, "invalid_target"),
        (r##"<send event="e" type="other"/>"##, "unsupported_type"),
        (r##"<send event="e" typeexpr="7"/>"##, "unsupported_type"),
    ] {
        let (ran, _) = run(body, &NOTHING);
        assert!(ran.effects.is_empty(), "{body}");
        assert_eq!(
            refused(&ran),
            ("error.execution", Value::from("send_1"), Value::from(kind)),
            "{body}"
        );
        assert_eq!(ran.sends.send_counter, 1, "{body}");
    }
}

// Sabotage: with classify not reading the SCXML processor's URI as built
// in, the URI registered here stamps an ordinal and this fails.
#[test]
fn the_scxml_processor_types_are_built_in() {
    for send_type in ["scxml", SCXML_EVENT_PROCESSOR] {
        let given = Given {
            send_types: &[SCXML_EVENT_PROCESSOR],
            ..NOTHING
        };
        let (ran, _) = run(
            &format!(r##"<send event="e" type="{send_type}" target="#_internal"/>"##),
            &given,
        );
        assert_eq!(sent(&ran).2, None, "{send_type}");
    }
}

// Sabotage: with a registered type stamping no ordinal, the ordinal is
// None and this fails.
#[test]
fn a_registered_type_sends_to_its_own_route_with_an_ordinal() {
    let given = Given {
        send_types: &["x-queue"],
        routes: Some(Routes::default()),
        sends: Sends {
            send_counter: 4,
            timer_counter: 9,
        },
        ..NOTHING
    };
    let (ran, _) = run(
        r##"<send event="e" type="x-queue" target="anywhere at all"/>"##,
        &given,
    );
    assert_eq!(
        sent(&ran),
        (
            Some(Value::from("anywhere at all")),
            "send_5".to_owned(),
            Some(10),
            Value::Undefined
        )
    );
    assert_eq!(
        ran.sends,
        Sends {
            send_counter: 5,
            timer_counter: 10
        }
    );
}

// Sabotage: with an author's id advancing the send counter, the generated
// id after it is send_2 and this fails.
#[test]
fn an_author_id_is_used_as_written_and_mints_nothing() {
    let (ran, _) = run(
        r##"<send event="a" id="mine"/><send event="b"/><send event="c" delay="10ms"/>"##,
        &NOTHING,
    );
    let ids: Vec<(String, bool)> = ran
        .effects
        .iter()
        .map(|effect| match effect {
            Effect::Send {
                send_id,
                id_from_author,
                ..
            }
            | Effect::SendDelayed {
                send_id,
                id_from_author,
                ..
            } => (send_id.clone(), *id_from_author),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(
        ids,
        [
            ("mine".to_owned(), true),
            ("send_1".to_owned(), false),
            ("send_2".to_owned(), false)
        ]
    );
}

// Sabotage: with the idlocation write's change dropped, the block answers
// the send alone and this fails.
#[test]
fn idlocation_is_written_through_the_location_writer_before_the_send() {
    let (ran, content) = run(
        r##"<send event="e" idlocation="ids.latest" delay="1s"/><log expr="ids.latest"/>"##,
        &Given {
            roots: &[("ids", map(&[]))],
            ..NOTHING
        },
    );
    assert_eq!(
        ran.effects[0],
        Effect::DatamodelChange {
            location_path: vec![
                PathSegment::Key("ids".to_owned()),
                PathSegment::Key("latest".to_owned())
            ],
            location_source: "ids.latest".to_owned(),
            new_value: Value::from("send_1"),
            prior_value: Value::Undefined,
            data_index: None,
            content_index: Some(content[0]),
            owner: Some(OWNER),
            macrostep: 3,
            microstep: 2,
            round: 1,
        }
    );
    assert!(matches!(
        &ran.effects[1],
        Effect::SendDelayed { send_id, id_from_author: true, .. } if send_id == "send_1"
    ));
    assert!(matches!(
        &ran.effects[2],
        Effect::Log { value: Some(Value::String(id)), .. } if id == "send_1"
    ));
}

// Sabotage: with a refused send restoring the datamodel it began with, the
// idlocation write is lost and this fails.
#[test]
fn a_refused_send_keeps_its_minted_id_and_its_idlocation_write() {
    let (ran, _) = run(
        r##"<send event="e" target="elsewhere" idlocation="id"/><raise event="e.after"/>"##,
        &Given {
            roots: &[("id", Value::Null)],
            ..NOTHING
        },
    );
    assert!(ran.effects.is_empty());
    assert_eq!(
        ran.data
            .get(&Key::from("id"))
            .cloned()
            .unwrap_or(Value::Null),
        Value::from("send_1")
    );
    assert_eq!(ran.sends.send_counter, 1);
    assert_eq!(ran.raised.len(), 1);
}

// Sabotage: with the delay's failure dropped, the send with a bad delay
// is sent now and this fails.
#[test]
fn an_argument_failure_discards_the_send_before_its_id_is_minted() {
    for body in [
        r##"<send eventexpr="nowhere"/>"##,
        r##"<send event="e" targetexpr="nowhere"/>"##,
        r##"<send event="e" typeexpr="nowhere"/>"##,
        r##"<send event="e" delay="soon"/>"##,
        r##"<send event="e" delayexpr="5"/>"##,
        r##"<send event="e" namelist="nowhere"/>"##,
        r##"<send event="e" namelist="a[0"/>"##,
        r##"<send event="e"><param name="p" expr="nowhere"/></send>"##,
        r##"<send event="e"><content expr="nowhere"/></send>"##,
        r##"<send event="e" idlocation="undeclared"/>"##,
    ] {
        let (ran, _) = run(body, &NOTHING);
        assert!(ran.effects.is_empty(), "{body}");
        assert_eq!(ran.sends, Sends::default(), "{body}");
        let [Raised::Error { failure, .. }] = ran.raised.as_slice() else {
            panic!("{body}: {:#?}", ran.raised);
        };
        assert_eq!(failure.event_name(), "error.execution", "{body}");
        assert!(!matches!(failure, Failure::SendRejected { .. }), "{body}");
    }
}

// Sabotage: with payload ignoring the namelist, the data holds only `p`
// and this fails.
#[test]
fn namelist_and_params_are_the_data_in_document_order() {
    let (ran, _) = run(
        r##"<send event="e" namelist="a b"/><send event="e"><param name="p" expr="a + 1"/>
           <param name="q" location="b"/></send>"##,
        &Given {
            roots: &[("a", Value::from(1)), ("b", Value::Undefined)],
            ..NOTHING
        },
    );
    let data: Vec<&Value> = ran
        .effects
        .iter()
        .map(|effect| match effect {
            Effect::Send { data, .. } => data,
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(
        data,
        [
            &map(&[("a", Value::from(1)), ("b", Value::Undefined)]),
            &map(&[("p", Value::from(2)), ("q", Value::Undefined)])
        ]
    );
}

// Sabotage: with static content kept as its raw text, the number reads as
// the string " 42 " and this fails.
#[test]
fn content_is_the_data_its_text_or_expression_gives() {
    for (content, expected) in [
        ("<content> 42 </content>", Value::from(42)),
        (
            "<content>  some\n  words </content>",
            Value::from("some words"),
        ),
        ("<content/>", Value::Undefined),
        (
            r##"<content expr="[a, 2]"/>"##,
            Value::List(vec![Value::from(1), Value::from(2)]),
        ),
    ] {
        let (ran, _) = run(
            &format!(r##"<send event="e">{content}</send>"##),
            &Given {
                roots: &[("a", Value::from(1))],
                ..NOTHING
            },
        );
        assert_eq!(sent(&ran).3, expected, "{content}");
    }
}

// Sabotage: with nested failures keeping the rejection's kind, the nested
// unreachable send raises error.communication and this fails.
#[test]
fn a_refusal_inside_an_if_raises_error_execution_without_a_send_id() {
    let (ran, _) = run(
        r##"<if cond="true"><send event="e" target="#_parent"/></if>"##,
        &Given {
            routes: Some(Routes::default()),
            ..NOTHING
        },
    );
    let [Raised::Error { failure, .. }] = ran.raised.as_slice() else {
        panic!("{:#?}", ran.raised);
    };
    assert_eq!(failure.event_name(), "error.execution");
    assert_eq!(field(&failure.event(), "sendid"), Value::Undefined);
    assert_eq!(ran.sends.send_counter, 1);
}

// Sabotage: with cancel not advancing the timer counter, both ordinals are
// 3 and this fails.
#[test]
fn cancel_names_its_send_id_with_an_ordinal() {
    let (ran, content) = run(
        r##"<cancel sendid="mine"/><cancel sendidexpr="'send_' + 2"/>"##,
        &Given {
            sends: Sends {
                send_counter: 0,
                timer_counter: 3,
            },
            ..NOTHING
        },
    );
    assert_eq!(
        ran.effects,
        vec![
            Effect::Cancel {
                send_id: Value::from("mine"),
                ordinal: 4,
                content_index: content[0],
                owner: OWNER,
                macrostep: 3,
                microstep: 2,
                round: 1,
            },
            Effect::Cancel {
                send_id: Value::from("send_2"),
                ordinal: 5,
                content_index: content[1],
                owner: OWNER,
                macrostep: 3,
                microstep: 2,
                round: 1,
            },
        ]
    );
    let (ran, _) = run(r##"<cancel sendidexpr="nowhere"/>"##, &NOTHING);
    assert!(ran.effects.is_empty());
    assert_eq!(ran.sends.timer_counter, 0);
    assert_eq!(ran.raised.len(), 1);
}

// Sabotage: with the unsupported type's value put under `target`, the
// data differs and this fails.
#[test]
fn a_refusal_carries_its_value_as_data() {
    let (ran, _) = run(r##"<send event="e" type="other"/>"##, &NOTHING);
    let [Raised::Error { failure, .. }] = ran.raised.as_slice() else {
        panic!("one error");
    };
    assert_eq!(
        failure.data(),
        map(&[
            ("kind", Value::from("unsupported_type")),
            ("type", Value::from("other"))
        ])
    );
}
