use super::*;
use crate::datamodel::system_variables::EventType;
use crate::effect::BlockOwner;
use crate::interpreter::machine_state::Status;
use predicator_rs::{Key, Map, Value};

const OPEN: &str = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0""#;

fn machine(attributes: &str, body: &str) -> Machine {
    let text = format!("{OPEN}{attributes}>{body}</scxml>");
    match crate::compile(&text) {
        Ok(machine) => machine,
        Err(refusals) => panic!("{text}: {refusals:#?}"),
    }
}

fn options() -> StartOptions {
    StartOptions {
        session_id: String::from("s-1"),
        ..StartOptions::default()
    }
}

fn start(attributes: &str, body: &str) -> Stepped {
    initialize(&machine(attributes, body), options())
}

fn external(name: &str) -> Event {
    Event {
        name: name.to_owned(),
        event_type: EventType::External,
        sendid: None,
        origin: None,
        origintype: None,
        invokeid: None,
        data: Value::Undefined,
    }
}

fn step(stepped: &Stepped, name: &str) -> Stepped {
    handle_event(&stepped.state, &external(name)).expect("a running chart")
}

// The active states' ids, in document order.
fn active(state: &MachineState) -> Vec<String> {
    state
        .configuration
        .iter()
        .filter_map(|&index| state.chart().states[index].id.clone())
        .collect()
}

// Every `<log>`'s label and value, in order.
fn logs(effects: &[Effect]) -> Vec<(String, Value)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Log { label, value, .. } => Some((
                label.clone().unwrap_or_default(),
                value.clone().unwrap_or(Value::Undefined),
            )),
            _ => None,
        })
        .collect()
}

fn labels(effects: &[Effect]) -> Vec<String> {
    logs(effects).into_iter().map(|(label, _)| label).collect()
}

// Each `<log>`'s label with the counters it carries.
fn stamped(effects: &[Effect]) -> Vec<(String, u64, u64, u64)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Log {
                label,
                macrostep,
                microstep,
                round,
                ..
            } => Some((
                label.clone().unwrap_or_default(),
                *macrostep,
                *microstep,
                *round,
            )),
            _ => None,
        })
        .collect()
}

fn get(map: &Value, key: &str) -> Value {
    match map {
        Value::Map(map) => map
            .get(&Key::from(key))
            .cloned()
            .unwrap_or(Value::Undefined),
        _ => Value::Undefined,
    }
}

fn root(state: &MachineState, name: &str) -> Value {
    state
        .datamodel
        .get(&Key::from(name))
        .cloned()
        .unwrap_or(Value::Undefined)
}

// Sabotage: with initialize skipping begin_microstep, the entry log carries
// microstep 0 and this fails.
#[test]
fn starting_enters_the_initial_states_as_macrostep_one_microstep_one() {
    let started = start(
        r#" initial="b""#,
        r#"<datamodel><data id="x" expr="1"/></datamodel>
           <state id="a"/>
           <state id="b"><onentry><log label="b" expr="x"/></onentry>
             <state id="b1"><onentry><log label="b1"/></onentry></state></state>"#,
    );
    assert_eq!(active(&started.state), ["b", "b1"]);
    assert!(matches!(
        started.effects[0],
        Effect::DatamodelInit {
            macrostep: 1,
            microstep: 1,
            round: 0,
            ..
        }
    ));
    assert!(matches!(
        &started.effects[1],
        Effect::DatamodelChange {
            data_index: Some(0),
            new_value: Value::Integer(1),
            ..
        }
    ));
    assert_eq!(
        stamped(&started.effects),
        [(String::from("b"), 1, 1, 0), (String::from("b1"), 1, 1, 0)]
    );
    assert!(started.state.running);
    assert_eq!(started.state.status, Status::Running);
    assert_eq!(started.state.last_selection, None);
}

// Sabotage: with microstep running the transition content after entry, the
// order reads exit, enter, transition and this fails.
#[test]
fn a_microstep_exits_then_runs_the_transition_then_enters() {
    let started = start(
        "",
        r#"<state id="a"><onexit><log label="exit a"/></onexit>
             <transition event="e.go" target="b"><log label="transition"/></transition></state>
           <state id="b"><onentry><log label="enter b"/></onentry></state>"#,
    );
    let stepped = step(&started, "e.go");
    assert_eq!(active(&stepped.state), ["b"]);
    assert_eq!(
        stamped(&stepped.effects),
        [
            (String::from("exit a"), 2, 1, 0),
            (String::from("transition"), 2, 1, 0),
            (String::from("enter b"), 2, 1, 0),
        ]
    );
    assert_eq!(stepped.state.last_selection, Some(true));
    assert_eq!(step(&stepped, "e.none").state.last_selection, Some(false));
}

// Sabotage: with the round dequeuing from the back of the queue, `e.two` is
// taken first and the order fails.
#[test]
fn a_macrostep_drains_raised_events_in_order_one_per_round() {
    let started = start(
        "",
        r#"<state id="a"><onentry><raise event="e.one"/><raise event="e.two"/></onentry>
             <transition event="e.one"><log label="one" expr="_event.name"/></transition>
             <transition event="e.two"><log label="two" expr="_event.type"/></transition></state>"#,
    );
    assert_eq!(
        logs(&started.effects),
        [
            (String::from("one"), Value::from("e.one")),
            (String::from("two"), Value::from("internal")),
        ]
    );
    assert_eq!(
        stamped(&started.effects)
            .into_iter()
            .map(|(_, _, microstep, round)| (microstep, round))
            .collect::<Vec<_>>(),
        [(2, 1), (3, 2)]
    );
    assert!(started.state.internal_queue.is_empty());
}

// Sabotage: with the round taking a queued internal event before the
// eventless transitions, `e.held` moves a to c and the chart ends in c.
#[test]
fn eventless_transitions_run_to_stability_before_any_internal_event() {
    let started = start(
        "",
        r#"<state id="a"><onentry><raise event="e.held"/></onentry>
             <transition target="b"/><transition event="e.held" target="c"/></state>
           <state id="b"><transition target="d"/></state>
           <state id="c"/>
           <state id="d"><transition event="e.held" target="e"/></state>
           <state id="e"/>"#,
    );
    assert_eq!(active(&started.state), ["e"]);
    assert_eq!(started.state.counters.microstep, 4);
    assert_eq!(started.state.counters.round, 4);
}

// Sabotage: with exit_interpreter keeping the internal queue, the pending
// `e.late` survives and the empty-queue assertion fails.
#[test]
fn a_top_level_final_stops_the_chart_with_its_donedata() {
    let started = start(
        "",
        r#"<datamodel><data id="x" expr="3"/></datamodel>
           <state id="a"><onexit><log label="exit a"/></onexit>
             <transition event="e.go" target="f"/></state>
           <final id="f"><onentry><raise event="e.late"/></onentry>
             <onexit><log label="exit f"/></onexit>
             <donedata><param name="p" expr="x + 1"/></donedata></final>"#,
    );
    let stopped = step(&started, "e.go");
    assert!(!stopped.state.running);
    assert_eq!(stopped.state.status, Status::Done);
    assert!(stopped.state.configuration.is_empty());
    assert!(stopped.state.internal_queue.is_empty());
    assert_eq!(labels(&stopped.effects), ["exit a", "exit f"]);
    let mut expected = Map::new();
    expected.insert("p", Value::from(4));
    let f = stopped.state.chart().id_to_index["f"];
    assert_eq!(
        stopped.effects.last(),
        Some(&Effect::Done {
            donedata: Some(Value::Map(expected)),
            donedata_error: None,
            configuration: vec![0, f],
            macrostep: 2,
            microstep: 1,
            round: 1,
        })
    );
    assert_eq!(
        handle_event(&stopped.state, &external("e.go")).err(),
        Some(NotRunning)
    );
    assert_eq!(
        deliver_internal(&stopped.state, external("e.go")).err(),
        Some(NotRunning)
    );
    assert_eq!(cancel(&stopped.state).err(), Some(NotRunning));
}

// Sabotage: with the donedata fold not reading the queue after it, the
// error is lost and donedata_error is None.
#[test]
fn a_failing_top_level_donedata_is_undefined_with_its_error_carried() {
    let started = start(
        "",
        r#"<final id="f"><donedata><content expr="nowhere + 1"/></donedata></final>"#,
    );
    match started.effects.last() {
        Some(Effect::Done {
            donedata,
            donedata_error,
            ..
        }) => {
            assert_eq!(donedata, &Some(Value::Undefined));
            assert_eq!(
                get(donedata_error.as_ref().expect("an error"), "kind"),
                Value::from("evaluator_error")
            );
        }
        other => panic!("{other:?}"),
    }
    let bare = start("", r#"<final id="f"/>"#);
    assert!(matches!(
        bare.effects.last(),
        Some(Effect::Done {
            donedata: Some(Value::Undefined),
            donedata_error: None,
            ..
        })
    ));
}

// Sabotage: with each round spending two of the budget, the fold halts at
// round 2 and this fails.
#[test]
fn a_chart_that_exceeds_the_round_budget_halts_as_a_value() {
    let chart = machine(
        "",
        r#"<state id="a"><onentry><raise event="e.loop"/></onentry>
             <transition event="e.loop" target="a"/></state>"#,
    );
    let started = initialize(
        &chart,
        StartOptions {
            max_macrostep_rounds: RoundBudget::Rounds(3),
            ..options()
        },
    );
    assert!(started.state.running);
    assert_eq!(started.state.status, Status::Running);
    let a = chart.chart().id_to_index["a"];
    match started.effects.last() {
        Some(Effect::BudgetExhausted {
            configuration,
            budget,
            pending_internal_events,
            macrostep,
            microstep,
            round,
        }) => {
            assert_eq!(configuration, &vec![0, a]);
            assert_eq!(*budget, 3);
            assert_eq!(pending_internal_events.len(), 1);
            assert_eq!(
                get(&pending_internal_events[0], "name"),
                Value::from("e.loop")
            );
            assert_eq!((*macrostep, *microstep, *round), (1, 4, 3));
        }
        other => panic!("{other:?}"),
    }
    assert!(
        !started
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::Done { .. }))
    );
    // The position resumes: a new external event starts a fresh budget.
    let again = step(&started, "e.other");
    assert!(matches!(
        again.effects.last(),
        Some(Effect::BudgetExhausted {
            macrostep: 2,
            round: 3,
            ..
        })
    ));
}

// Sabotage: with fold never exhausting a zero budget, the entry's raise is
// taken and the chart ends in b.
#[test]
fn a_zero_budget_takes_no_round_and_an_unbounded_one_never_exhausts() {
    let chart = machine(
        "",
        r#"<state id="a"><onentry><raise event="e.one"/></onentry>
             <transition event="e.one" target="b"/></state><state id="b"/>"#,
    );
    let zero = initialize(
        &chart,
        StartOptions {
            max_macrostep_rounds: RoundBudget::Rounds(0),
            ..options()
        },
    );
    assert_eq!(active(&zero.state), ["a"]);
    assert!(matches!(
        zero.effects.last(),
        Some(Effect::BudgetExhausted {
            budget: 0,
            round: 0,
            ..
        })
    ));
    let unbounded = initialize(
        &chart,
        StartOptions {
            max_macrostep_rounds: RoundBudget::Unbounded,
            ..options()
        },
    );
    assert_eq!(active(&unbounded.state), ["b"]);
    assert!(
        !unbounded
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::BudgetExhausted { .. }))
    );
}

// Sabotage: with run_block dropping the raised errors, no error.execution is
// queued and the chart stays in b.
#[test]
fn an_error_raised_mid_macrostep_is_an_internal_event_the_chart_takes() {
    let started = start(
        "",
        r#"<state id="a"><transition event="e.go" target="b"/></state>
           <state id="b"><onentry><assign location="undeclared" expr="1"/>
             <raise event="e.unreached"/></onentry>
             <transition event="error.execution" target="c">
               <log label="kind" expr="_event.data.kind"/>
               <log label="type" expr="_event.type"/></transition></state>
           <state id="c"/>"#,
    );
    let stepped = step(&started, "e.go");
    assert_eq!(active(&stepped.state), ["c"]);
    assert_eq!(
        logs(&stepped.effects),
        [
            (String::from("kind"), Value::from("unbound_location")),
            (String::from("type"), Value::from("platform")),
        ]
    );
}

// Sabotage: with select ignoring the evaluations, the failed condition
// raises nothing and the chart stays in a.
#[test]
fn a_condition_that_fails_raises_error_execution() {
    let started = start(
        "",
        r#"<state id="a"><transition event="e.go" cond="nowhere" target="b"/>
             <transition event="error.execution" target="c"/></state>
           <state id="b"/><state id="c"/>"#,
    );
    assert_eq!(active(&step(&started, "e.go").state), ["c"]);
}

// Sabotage: with arrive binding on every entry, the second entry rebinds y
// and the log reads 1.
#[test]
fn late_binding_binds_a_state_s_data_on_its_first_entry_only() {
    let started = start(
        r#" binding="late""#,
        r#"<state id="a"><transition event="e.go" target="b"/></state>
           <state id="b"><datamodel><data id="y" expr="1"/></datamodel>
             <onentry><log label="y" expr="y"/><assign location="y" expr="y + 1"/></onentry>
             <transition event="e.back" target="a"/></state>"#,
    );
    assert_eq!(root(&started.state, "y"), Value::Undefined);
    assert!(
        started.state.datamodel.contains_key(&Key::from("y")),
        "declared when the chart starts"
    );
    let entered = step(&started, "e.go");
    assert!(matches!(
        entered.effects[0],
        Effect::DatamodelChange {
            data_index: Some(0),
            owner: None,
            content_index: None,
            ..
        }
    ));
    let again = step(&step(&entered, "e.back"), "e.go");
    assert_eq!(logs(&again.effects), [(String::from("y"), Value::from(2))]);
}

// Sabotage: with initialize binding every root `<data>`, the host's x is
// overwritten by 1.
#[test]
fn the_host_s_value_wins_over_a_root_data_and_a_src_raises() {
    let chart = machine(
        "",
        r#"<datamodel><data id="x" expr="1"/><data id="z" src="file:z.json"/></datamodel>
           <state id="a"><transition event="error.execution" target="b">
             <log label="kind" expr="_event.data.kind"/></transition></state>
           <state id="b"/>"#,
    );
    let mut host = Map::new();
    host.insert("x", Value::from(9));
    let started = initialize(
        &chart,
        StartOptions {
            datamodel: host,
            ..options()
        },
    );
    assert_eq!(root(&started.state, "x"), Value::from(9));
    assert_eq!(root(&started.state, "z"), Value::Undefined);
    assert_eq!(
        logs(&started.effects),
        [(String::from("kind"), Value::from("src"))]
    );
    assert_eq!(root(&started.state, "_sessionid"), Value::from("s-1"));
}

// Sabotage: with the completion's donedata skipped, `_event.data` is
// undefined and the first log fails.
#[test]
fn a_nested_final_raises_done_state_with_its_donedata_then_the_parallel_s() {
    let started = start(
        "",
        r#"<parallel id="p">
             <state id="r1"><final id="f1"><donedata><content>21</content></donedata></final></state>
             <state id="r2"><final id="f2"/></state>
             <transition event="done.state.r1"><log label="r1" expr="_event.data"/></transition>
             <transition event="done.state.p" target="x"><log label="p" expr="_event.type"/></transition>
           </parallel><state id="x"/>"#,
    );
    assert_eq!(
        logs(&started.effects),
        [
            (String::from("r1"), Value::from(21)),
            (String::from("p"), Value::from("platform")),
        ]
    );
    assert_eq!(active(&started.state), ["x"]);
}

// Sabotage: with cancel skipping exit_interpreter, no onexit runs and the
// log list is empty.
#[test]
fn cancel_exits_every_state_and_answers_done_with_no_donedata() {
    let started = start(
        "",
        r#"<state id="a"><onexit><log label="exit a"/></onexit></state>"#,
    );
    let cancelled = cancel(&started.state).expect("a running chart");
    assert_eq!(labels(&cancelled.effects), ["exit a"]);
    assert!(matches!(
        cancelled.effects.last(),
        Some(Effect::Done {
            donedata: None,
            donedata_error: None,
            ..
        })
    ));
    assert_eq!(cancelled.state.status, Status::Done);
}

// Sabotage: with deliver_internal not running the loop, the event stays
// queued and the chart stays in a.
#[test]
fn an_event_delivered_internally_is_taken_in_the_same_call() {
    let started = start(
        "",
        r#"<state id="a"><transition event="e.back" target="b"/></state><state id="b"/>"#,
    );
    let delivered = deliver_internal(&started.state, Event::internal("e.back")).expect("running");
    assert_eq!(active(&delivered.state), ["b"]);
    assert_eq!(delivered.state.counters.macrostep, 1);
}

// Sabotage: with microstep skipping the transition content, x stays 1 and
// the first assertion fails; the given position is untouched by
// construction, since a step only borrows it.
#[test]
fn a_step_leaves_the_position_it_was_given_as_it_was() {
    let started = start(
        "",
        r#"<datamodel><data id="x" expr="1"/></datamodel>
           <state id="a"><transition event="e.go" target="b">
             <assign location="x" expr="x + 1"/></transition></state><state id="b"/>"#,
    );
    let before = started.state.clone();
    let stepped = step(&started, "e.go");
    assert_eq!(root(&stepped.state, "x"), Value::from(2));
    assert_eq!(root(&started.state, "x"), Value::from(1));
    assert_eq!(started.state.configuration, before.configuration);
    assert_eq!(started.state.counters, before.counters);
    // The same event against the same position answers the same.
    let replayed = step(&started, "e.go");
    assert_eq!(replayed.effects, stepped.effects);
    assert_eq!(replayed.state.datamodel, stepped.state.datamodel);
}

// Sabotage: with microstep dropping its round's effects, the stepped effects
// miss the log and the comparison fails.
#[test]
fn stepping_round_by_round_reaches_what_the_fold_reaches() {
    let started = start(
        "",
        r#"<state id="a"><transition event="e.go" target="b"/></state>
           <state id="b"><onentry><raise event="e.one"/></onentry>
             <transition target="c"/></state>
           <state id="c"><transition event="e.one" target="d"><log label="d"/></transition></state>
           <state id="d"/>"#,
    );
    let mut opened = started.state.clone();
    opened.begin_macrostep();
    opened.put_event(&external("e.go"));
    let selected = select(&mut opened, Some("e.go"));
    let mut entered_effects = Vec::new();
    run_selected(&mut opened, &selected, &mut entered_effects);

    let folded = macrostep(&opened);
    let mut position = opened;
    let mut effects = Vec::new();
    loop {
        let (stepped, round) = microstep(&position);
        position = stepped.state;
        effects.extend(stepped.effects);
        if round == Round::Quiescent {
            break;
        }
    }
    assert_eq!(effects, folded.effects);
    assert_eq!(position.configuration, folded.state.configuration);
    assert_eq!(position.counters, folded.state.counters);
    assert_eq!(active(&position), ["d"]);
}

// Sabotage: with run_global_scripts skipped, x stays 1 and the entry log
// reads 1.
#[test]
fn root_scripts_run_before_entry_and_a_failing_one_raises() {
    let started = start(
        "",
        r#"<datamodel><data id="x" expr="1"/></datamodel>
           <script>x = x + 10</script><script>nowhere = nowhere + 1</script>
           <state id="a"><onentry><log label="x" expr="x"/></onentry>
             <transition event="error.execution" target="b"/></state><state id="b"/>"#,
    );
    assert_eq!(
        logs(&started.effects),
        [(String::from("x"), Value::from(11))]
    );
    assert_eq!(active(&started.state), ["b"]);
}

// Sabotage: with depart leaving each state in the configuration, In('c')
// reads true in p's onexit and the second log fails.
#[test]
fn exit_runs_onexit_blocks_in_exit_order_while_each_state_is_active() {
    let started = start(
        "",
        r#"<state id="p"><onexit><log label="p" expr="In('c')"/></onexit>
             <state id="c"><onexit><log label="c" expr="In('c')"/></onexit>
               <transition event="e.go" target="q"/></state></state>
           <state id="q"/>"#,
    );
    let stepped = step(&started, "e.go");
    assert_eq!(
        logs(&stepped.effects),
        [
            (String::from("c"), Value::Bool(true)),
            (String::from("p"), Value::Bool(false)),
        ]
    );
    let owners: Vec<BlockOwner> = stepped
        .effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Log { owner, .. } => Some(*owner),
            _ => None,
        })
        .collect();
    let chart = stepped.state.chart();
    assert_eq!(
        owners,
        [
            BlockOwner::OnExit {
                state: chart.id_to_index["c"],
                ordinal: 0
            },
            BlockOwner::OnExit {
                state: chart.id_to_index["p"],
                ordinal: 0
            },
        ]
    );
}

// Sabotage: with a panic where a round finds the internal queue empty,
// every corpus chart that quiesces panics and this fails.
#[test]
fn every_corpus_chart_starts_and_takes_an_event_without_a_panic() {
    let mut started = 0;
    for (name, bytes) in crate::testing::corpus_documents() {
        let Ok(text) = String::from_utf8(bytes) else {
            continue;
        };
        let Ok(chart) = crate::compile(&text) else {
            continue;
        };
        let stepped = initialize(
            &chart,
            StartOptions {
                max_macrostep_rounds: RoundBudget::Rounds(1_000),
                ..options()
            },
        );
        if stepped.state.running {
            let again = handle_event(&stepped.state, &external("e.go"));
            assert!(again.is_ok(), "{name}");
        } else {
            assert_eq!(stepped.state.status, Status::Done, "{name}");
        }
        started += 1;
    }
    assert!(started > 300, "{started} charts started");
}
