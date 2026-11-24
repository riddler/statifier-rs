use super::*;
use crate::Key;

const OPEN: &str = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0""#;

fn machine(attributes: &str, body: &str) -> Machine {
    let text = format!("{OPEN}{attributes}>{body}</scxml>");
    match crate::compile(&text) {
        Ok(machine) => machine,
        Err(refusals) => panic!("{text}: {refusals:#?}"),
    }
}

fn start(attributes: &str, body: &str) -> Moved {
    machine(attributes, body)
        .start(Options::new("s-1"))
        .expect("a start")
}

fn budget(rounds: u64) -> RoundBudget {
    RoundBudget::Rounds(NonZeroU64::new(rounds).expect("a budget above zero"))
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

fn text(value: &str) -> Value {
    Value::from(value)
}

// Sabotage: with `ids` leaving the ids in document order, this goes red.
#[test]
fn start_answers_the_active_state_ids_sorted_root_and_unnamed_left_out() {
    let started = start(
        r#" initial="p""#,
        r#"<parallel id="p"><state id="z"/><state id="b"/><state><state id="m"/></state></parallel>"#,
    );
    assert_eq!(started.state.configuration(), ["b", "m", "p", "z"]);
    assert!(started.state.done().is_none());
}

// Sabotage: with `step` moving the state it was given in place (a field
// written through a shared value), the first assertion goes red.
#[test]
fn step_answers_a_new_state_and_leaves_the_one_it_was_given() {
    let started = start(
        r#" initial="a""#,
        r#"<state id="a"><transition event="e.go" target="b"/></state><state id="b"/>"#,
    );
    let moved = started.state.step("e.go", Value::Undefined).unwrap();
    assert_eq!(started.state.configuration(), ["a"]);
    assert_eq!(moved.state.configuration(), ["b"]);
    let again = started.state.step("e.go", Value::Undefined).unwrap();
    assert_eq!(again.state.configuration(), ["b"]);
}

// Sabotage: with `perform_one` not recording the `Done` effect, `done` stays
// `None` and the step after it is not refused.
#[test]
fn a_stopped_chart_answers_its_donedata_and_refuses_another_event() {
    let started = start(
        r#" initial="a""#,
        r#"<state id="a"><transition event="e.go" target="f"/></state>
           <final id="f"><donedata><content expr="'x' + 'y'"/></donedata></final>"#,
    );
    let moved = started.state.step("e.go", Value::Undefined).unwrap();
    let done = moved.state.done().expect("a stopped chart");
    assert_eq!(done.donedata, text("xy"));
    assert_eq!(done.configuration, ["f"]);
    assert!(moved.state.configuration().is_empty());
    assert_eq!(
        moved.state.step("e.go", Value::Undefined).unwrap_err(),
        Error::NotRunning
    );
}

// Sabotage: with `is_built_in` answering false, start is not refused.
#[test]
fn start_refuses_a_built_in_send_type() {
    let mut options = Options::new("s-1");
    options.send_types = [SCXML_EVENT_PROCESSOR, "x-a", "scxml"]
        .into_iter()
        .map(String::from)
        .collect();
    let refused = Error::BuiltInSendTypes {
        types: vec![SCXML_EVENT_PROCESSOR.to_owned(), "scxml".to_owned()],
    };
    assert_eq!(
        machine("", r#"<state id="a"/>"#).start(options).err(),
        Some(refused.clone())
    );
    assert!(refused.to_string().contains("scxml"));
    let error: &dyn std::error::Error = &refused;
    assert!(error.source().is_none());
}

// Sabotage: with `start` dropping the host's datamodel, or `step` dropping
// the event's data, the values read back differ.
#[test]
fn host_values_pass_through_unchanged() {
    let machine = machine(
        r#" initial="a""#,
        r#"<datamodel><data id="x" expr="1"/></datamodel>
           <state id="a"><transition event="e.go" target="a">
             <log label="data" expr="_event.data"/><log label="x" expr="x"/>
           </transition></state>"#,
    );
    let mut options = Options::new("s-1");
    let mut datamodel = Map::new();
    let mut inner = Map::new();
    inner.insert("k", Value::List(vec![Value::from(1.5), Value::Null]));
    datamodel.insert("x", Value::Map(inner.clone()));
    options.datamodel = datamodel;
    let started = machine.start(options).unwrap();
    let Some(Effect::DatamodelInit { datamodel, .. }) = started.effects.first() else {
        panic!("{:#?}", started.effects);
    };
    assert_eq!(
        datamodel.get(&Key::from("x")),
        Some(&Value::Map(inner.clone()))
    );
    let data = Value::List(vec![text("p"), Value::Map(inner.clone())]);
    let moved = started.state.step("e.go", data.clone()).unwrap();
    assert_eq!(
        logs(&moved.effects),
        [
            (String::from("data"), data),
            (String::from("x"), Value::Map(inner))
        ]
    );
}

// Sabotage: with `route` dropping a send with no target, or one to the
// session's own address, the second state is never entered.
#[test]
fn a_send_to_the_session_itself_is_taken_as_an_external_event() {
    for target in ["", r##" target="#_scxml_s-1""##] {
        let started = start(
            r#" initial="a""#,
            &format!(
                r#"<state id="a"><onentry><send event="e.self" id="k"{target}/></onentry>
                   <transition event="e.self" target="b">
                     <log label="origin" expr="_event.origin"/>
                     <log label="type" expr="_event.origintype"/>
                     <log label="sendid" expr="_event.sendid"/>
                     <log label="eventtype" expr="_event.type"/>
                   </transition></state><state id="b"/>"#
            ),
        );
        assert_eq!(started.state.configuration(), ["b"], "{target}");
        assert_eq!(
            logs(&started.effects),
            [
                (String::from("origin"), text("#_scxml_s-1")),
                (String::from("type"), text(SCXML_EVENT_PROCESSOR)),
                (String::from("sendid"), text("k")),
                (String::from("eventtype"), text("external")),
            ]
        );
    }
}

// Sabotage: with `deliver_internal`'s effects acted on at once rather than
// after the batch, the log order changes.
#[test]
fn an_internal_send_runs_the_chart_after_the_batch_that_sent_it() {
    let started = start(
        r#" initial="a""#,
        r##"<state id="a"><onentry><send event="e.in" target="#_internal"/><log label="after"/></onentry>
           <transition event="e.in" target="b"><log label="taken"/>
             <log label="type" expr="_event.type"/></transition></state><state id="b"/>"##,
    );
    assert_eq!(started.state.configuration(), ["b"]);
    assert_eq!(
        logs(&started.effects),
        [
            (String::from("after"), Value::Undefined),
            (String::from("taken"), Value::Undefined),
            (String::from("type"), text("internal")),
        ]
    );
}

// Sabotage: with `next_due` ignoring the sequence among equal due times, or
// `advance` firing a timer not yet due, the order or the count changes.
#[test]
fn advance_fires_due_timers_earliest_first_then_in_scheduled_order() {
    let started = start(
        r#" initial="a""#,
        r#"<state id="a"><onentry>
             <send event="e.two" delay="20ms"/><send event="e.one" delay="10ms"/>
             <send event="e.three" delay="20ms"/><send event="e.four" delay="30ms"/>
           </onentry>
           <transition event="*"><log label="got" expr="_event.name"/></transition></state>"#,
    );
    assert_eq!(started.state.next_due_ms(), Some(10));
    let first = started.state.advance(20).unwrap();
    assert_eq!(first.state.now_ms(), 20);
    assert_eq!(
        logs(&first.effects),
        [
            (String::from("got"), text("e.one")),
            (String::from("got"), text("e.two")),
            (String::from("got"), text("e.three")),
        ]
    );
    assert_eq!(first.state.next_due_ms(), Some(30));
    assert_eq!(started.state.next_due_ms(), Some(10));
    let second = first.state.advance(9).unwrap();
    assert!(second.effects.is_empty());
    let third = second.state.advance(1).unwrap();
    assert_eq!(labels(&third.effects), ["got"]);
    assert_eq!(third.state.next_due_ms(), None);
}

// Sabotage: with the `Cancel` arm keeping every timer, the cancelled send
// fires.
#[test]
fn a_cancel_removes_the_pending_timer_it_names() {
    let started = start(
        r#" initial="a""#,
        r#"<state id="a"><onentry><send event="e.late" id="t" delay="1s"/>
           <send event="e.stop" delay="5ms"/></onentry>
           <transition event="e.stop"><cancel sendid="t"/></transition>
           <transition event="e.late" target="b"/></state><state id="b"/>"#,
    );
    let moved = started.state.advance(5).unwrap();
    assert_eq!(moved.state.next_due_ms(), None);
    let later = moved.state.advance(5_000).unwrap();
    assert_eq!(later.state.configuration(), ["a"]);
}

// Sabotage: with `registered` answering false, the registered send is
// routed to the session itself and the chart moves to `b`.
#[test]
fn a_send_of_a_registered_type_is_answered_as_an_effect_and_never_routed() {
    let machine = machine(
        r#" initial="a""#,
        r#"<state id="a"><onentry>
             <send event="e.out" type="x-host"/><send event="e.out" type="x-host" delay="1s"/>
             <log label="entry" expr="_ioprocessors['x-host']"/>
           </onentry><transition event="e.out" target="b"/></state><state id="b"/>"#,
    );
    let mut options = Options::new("s-1");
    options.send_types = BTreeSet::from([String::from("x-host")]);
    let started = machine.start(options).unwrap();
    assert_eq!(started.state.configuration(), ["a"]);
    assert_eq!(started.state.next_due_ms(), None);
    let sends = started
        .effects
        .iter()
        .filter(|effect| {
            matches!(effect, Effect::Send { send_type: Some(t), .. } | Effect::SendDelayed { send_type: Some(t), .. } if *t == text("x-host"))
        })
        .count();
    assert_eq!(sends, 2);
    assert_eq!(
        logs(&started.effects),
        [(String::from("entry"), Value::Map(Map::new()))]
    );
}

// Sabotage: with an unreachable fired send dropped rather than raising
// `error.communication`, the chart stays in `a`.
#[test]
fn a_delayed_send_to_an_unreachable_target_raises_error_communication_when_it_fires() {
    let started = start(
        r#" initial="a""#,
        r##"<state id="a"><onentry><send event="e.up" id="k" target="#_parent" delay="1ms"/></onentry>
           <transition event="error.communication" target="b">
             <log label="sendid" expr="_event.sendid"/></transition></state><state id="b"/>"##,
    );
    assert_eq!(started.state.configuration(), ["a"]);
    let moved = started.state.advance(1).unwrap();
    assert_eq!(moved.state.configuration(), ["b"]);
    assert_eq!(logs(&moved.effects), [(String::from("sendid"), text("k"))]);
}

// Sabotage: with the fired `#_internal` arm queuing on the external queue,
// the event's type reads `external`.
#[test]
fn a_fired_internal_send_is_delivered_onto_the_internal_queue() {
    let started = start(
        r#" initial="a""#,
        r##"<state id="a"><onentry><send event="e.in" targetexpr="'#_internal'" delay="1ms"/></onentry>
           <transition event="e.in" target="b"><log label="type" expr="_event.type"/></transition>
           </state><state id="b"/>"##,
    );
    let moved = started.state.advance(1).unwrap();
    assert_eq!(moved.state.configuration(), ["b"]);
    assert_eq!(
        logs(&moved.effects),
        [(String::from("type"), text("internal"))]
    );
}

// Sabotage: with `admits` allowing more counted events than the budget, the
// halt comes at a later macrostep.
#[test]
fn a_call_halts_when_the_events_it_sends_itself_spend_the_budget() {
    let machine = machine(
        r#" initial="a""#,
        r#"<state id="a"><onentry><send event="e.loop"/></onentry>
           <transition event="e.loop" target="a"/>
           <transition event="e.host" target="b"/></state><state id="b"/>"#,
    );
    let mut options = Options::new("s-1");
    options.round_budget = budget(3);
    let started = machine.start(options).unwrap();
    let Some(Effect::BudgetExhausted {
        budget, macrostep, ..
    }) = started.effects.last()
    else {
        panic!("{:#?}", started.effects);
    };
    assert_eq!(*budget, 3);
    // The start is macrostep 1, and each of the three events taken one more.
    assert_eq!(*macrostep, 4);
    // Halted: a host event is queued, not taken.
    let moved = started.state.step("e.host", Value::Undefined).unwrap();
    assert!(moved.effects.is_empty());
    assert_eq!(moved.state.configuration(), ["a"]);
}

// Sabotage: with `admits` always true for an internal send, the start never
// returns.
#[test]
fn a_call_halts_when_its_internal_sends_spend_the_budget() {
    let machine = machine(
        r#" initial="a""#,
        r##"<state id="a"><onentry><send event="e.loop" target="#_internal"/></onentry>
           <transition event="e.loop" target="a"/></state>"##,
    );
    let mut options = Options::new("s-1");
    options.round_budget = budget(2);
    let started = machine.start(options).unwrap();
    let exhausted: Vec<&Effect> = started
        .effects
        .iter()
        .filter(|effect| matches!(effect, Effect::BudgetExhausted { .. }))
        .collect();
    assert_eq!(exhausted.len(), 1);
    let Effect::BudgetExhausted {
        pending_internal_events,
        ..
    } = exhausted[0]
    else {
        unreachable!()
    };
    assert_eq!(pending_internal_events.len(), 1);
}

// Sabotage: with a macrostep's `BudgetExhausted` not halting the driver, the
// host event is taken and the chart moves to `b`.
#[test]
fn a_macrostep_that_spends_its_budget_halts_the_driver_and_timers_still_fire() {
    let machine = machine(
        r#" initial="a""#,
        r#"<state id="a"><onentry><raise event="e.r"/><send event="e.t" delay="1ms"/></onentry>
           <transition event="e.r" target="a"/>
           <transition event="e.host" target="b"/>
           <transition event="e.t"><log label="fired"/></transition></state><state id="b"/>"#,
    );
    let mut options = Options::new("s-1");
    options.round_budget = budget(4);
    let started = machine.start(options).unwrap();
    assert!(matches!(
        started.effects.last(),
        Some(Effect::BudgetExhausted { .. })
    ));
    let moved = started.state.step("e.host", Value::Undefined).unwrap();
    assert_eq!(moved.state.configuration(), ["a"]);
    assert!(moved.state.next_due_ms().is_some());
    let fired = moved.state.advance(1).unwrap();
    assert_eq!(fired.state.next_due_ms(), None);
    // The fired event is queued, as the host event is: a halted driver takes
    // none.
    assert!(logs(&fired.effects).is_empty());
}

// Sabotage: with `checked_add` replaced by a saturating add, no refusal.
#[test]
fn advance_refuses_a_duration_past_the_clock_and_moves_a_stopped_chart_only_in_time() {
    let started = start(r#" initial="a""#, r#"<state id="a"/>"#);
    let later = started.state.advance(10).unwrap();
    assert_eq!(
        later.state.advance(u64::MAX).unwrap_err(),
        Error::InvalidDuration { ms: u64::MAX }
    );
    assert!(
        Error::InvalidDuration { ms: 1 }
            .to_string()
            .contains("1 ms")
    );

    let stopped = start(r#" initial="f""#, r#"<final id="f"/>"#);
    assert!(stopped.state.done().is_some());
    let moved = stopped.state.advance(7).unwrap();
    assert_eq!(moved.state.now_ms(), 7);
    assert!(moved.effects.is_empty());
}

// Sabotage: with `Done` not clearing the timers, a timer stays pending after
// the chart stopped.
#[test]
fn a_chart_that_stops_discards_its_timers_and_its_queued_events() {
    let started = start(
        r#" initial="a""#,
        r#"<state id="a"><onentry><send event="e.late" delay="1s"/><send event="e.now"/>
           <send event="e.now"/></onentry>
           <transition event="e.now" target="f"/></state><final id="f"/>"#,
    );
    let done = started.state.done().expect("stopped");
    assert_eq!(done.configuration, ["f"]);
    assert_eq!(done.donedata, Value::Undefined);
    assert_eq!(started.state.next_due_ms(), None);
    assert!(started.state.external_queue.is_empty());
}

// Sabotage: with `present` keeping a null, the send's target is
// `Some(Value::Null)`.
#[test]
fn a_null_target_is_carried_as_no_target() {
    let machine = machine(
        r#" initial="a""#,
        r#"<state id="a"><onentry><send event="e.out" typeexpr="'x-host'" targetexpr="null"/></onentry></state>"#,
    );
    let mut options = Options::new("s-1");
    options.send_types = BTreeSet::from([String::from("x-host")]);
    let started = machine.start(options).unwrap();
    let target = started.effects.iter().find_map(|effect| match effect {
        Effect::Send { target, .. } => Some(target.clone()),
        _ => None,
    });
    assert_eq!(target, Some(None));
}

#[test]
fn the_round_budget_defaults_to_the_reference_and_converts() {
    assert_eq!(RoundBudget::default(), budget(DEFAULT_ROUNDS));
    assert_eq!(
        RoundBudget::Unbounded.interpreter(),
        interpreter::RoundBudget::Unbounded
    );
    assert!(RoundBudget::Unbounded.admits(u64::MAX));
    assert_eq!(RoundBudget::Unbounded.spent(), u64::MAX);
    let options = Options::new("s-1");
    assert_eq!(options.round_budget, RoundBudget::default());
    assert!(!options.trace);
    assert!(Error::NotRunning.to_string().contains("stopped"));
}

// Sabotage: with `start` ignoring an unbounded budget (a bound of 1), the
// eventless chain stops early and `c` is not reached.
#[test]
fn an_unbounded_budget_lets_a_long_macrostep_finish() {
    let machine = machine(
        r#" initial="a""#,
        r#"<state id="a"><transition target="b"/></state><state id="b"><transition target="c"/></state><state id="c"/>"#,
    );
    let mut options = Options::new("s-1");
    options.round_budget = RoundBudget::Unbounded;
    assert_eq!(machine.start(options).unwrap().state.configuration(), ["c"]);
}

// No driver call panics on a chart the corpus holds: each is started,
// sent an event and advanced past its timers, and each answer is a value.
// Sabotage: with `next_due` answering a position past the end, `remove`
// panics and this goes red.
#[test]
fn every_corpus_chart_starts_steps_and_advances_without_a_panic() {
    for (name, bytes) in crate::testing::corpus_documents() {
        let Ok(source) = String::from_utf8(bytes) else {
            continue;
        };
        let Ok(machine) = crate::compile(&source) else {
            continue;
        };
        let mut options = Options::new("s-1");
        options.round_budget = budget(100);
        let started = machine
            .start(options)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        let stepped = match started.state.step("e.go", Value::Undefined) {
            Ok(moved) => moved.state,
            Err(Error::NotRunning) => started.state,
            Err(error) => panic!("{name}: {error}"),
        };
        let advanced = stepped
            .advance(10_000)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert!(
            advanced.state.next_due_ms().is_none_or(|due| due > 10_000),
            "{name}"
        );
    }
}

// A halted driver drops a fired send to the internal queue or to a target it
// cannot reach: the chart is not run again, and no second budget effect.
// Sabotage: with `fired_internal` delivering whatever the halt, the advance
// runs the chart with a fresh budget and answers a second BudgetExhausted.
#[test]
fn a_halted_driver_drops_a_fired_internal_or_unreachable_send() {
    let machine = machine(
        r#" initial="idle""#,
        r##"<state id="idle"><onentry>
             <send event="e.up" target="#_parent" delay="10ms"/>
             <send event="e.in" targetexpr="'#_internal'" delay="10ms"/>
           </onentry><transition event="go" target="loop"/></state>
           <state id="loop"><transition target="loop2"/></state>
           <state id="loop2"><transition target="loop"/></state>"##,
    );
    let mut options = Options::new("s-1");
    options.round_budget = budget(5);
    let started = machine.start(options).unwrap();
    let stepped = started.state.step("go", Value::Undefined).unwrap();
    let halted = stepped.state.configuration();
    assert!(matches!(
        stepped.effects.last(),
        Some(Effect::BudgetExhausted { .. })
    ));
    let advanced = stepped.state.advance(20).unwrap();
    assert!(advanced.effects.is_empty(), "{:#?}", advanced.effects);
    assert_eq!(advanced.state.configuration(), halted);
    assert_eq!(advanced.state.next_due_ms(), None);
}

// A timer that reschedules itself due at once would fire forever; the call
// counts the timers it scheduled and fired, and halts past the budget.
// Sabotage: with `count_fire` counting nothing, advance(0) never returns.
#[test]
fn a_timer_rescheduled_due_at_once_spends_the_call_s_budget() {
    let machine = machine(
        r#" initial="a""#,
        r#"<state id="a"><onentry><send event="tick" delay="0ms"/></onentry>
           <transition event="tick"><send event="tick" delay="0ms"/></transition></state>"#,
    );
    let mut options = Options::new("s-1");
    options.round_budget = budget(3);
    let started = machine.start(options).unwrap();
    let advanced = started.state.advance(0).unwrap();
    let exhausted = advanced
        .effects
        .iter()
        .filter(|effect| matches!(effect, Effect::BudgetExhausted { .. }))
        .count();
    assert_eq!(exhausted, 1);
    // The start's timer and three it scheduled fire, then the fourth is
    // fired into the queue of a halted driver.
    let sent = advanced
        .effects
        .iter()
        .filter(|effect| matches!(effect, Effect::SendDelayed { .. }))
        .count();
    assert_eq!(sent, 4);
    assert_eq!(advanced.state.next_due_ms(), None);
    assert_eq!(advanced.state.now_ms(), 0);
}

// A heartbeat that reschedules itself a second later is bounded by the
// duration advanced, never by the budget: one long advance answers what ten
// short ones do.
// Sabotage: with `next_instant` keeping the count of fired timers when the
// clock moves on, the long advance halts with BudgetExhausted and leaves no timer.
#[test]
fn a_timer_rescheduled_with_a_delay_is_not_halted_however_time_is_split() {
    let machine = machine(
        r#" initial="a""#,
        r#"<state id="a"><onentry><send event="tick" delay="1000ms"/></onentry>
           <transition event="tick"><log label="tick"/><send event="tick" delay="1000ms"/></transition></state>"#,
    );
    let mut options = Options::new("s-1");
    options.round_budget = budget(3);
    let started = machine.start(options).unwrap();
    let long = started.state.advance(10_000).unwrap();
    assert!(
        !long
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::BudgetExhausted { .. })),
        "{:#?}",
        long.effects
    );
    assert_eq!(long.state.next_due_ms(), Some(11_000));
    let mut state = started.state;
    let mut effects = Vec::new();
    for _ in 0..10 {
        let moved = state.advance(1_000).unwrap();
        effects.extend(moved.effects);
        state = moved.state;
    }
    assert_eq!(effects, long.effects);
    assert_eq!(labels(&long.effects).len(), 10);
    assert_eq!(state.next_due_ms(), Some(11_000));
}

// A heartbeat whose tick also sends itself an event and sends one onto the
// internal queue at once: each count is kept at one virtual time, so one
// long advance answers what ten short ones do, and nothing halts.
// Sabotage: with `next_instant` keeping the count of events taken (or of
// internal sends), the long advance halts with BudgetExhausted.
#[test]
fn immediate_sends_of_a_heartbeat_are_counted_at_one_virtual_time() {
    let machine = machine(
        r#" initial="a""#,
        r##"<state id="a"><onentry><send event="tick" delay="1000ms"/></onentry>
           <transition event="tick"><send event="echo"/><send event="inner" target="#_internal"/>
             <send event="tick" delay="1000ms"/></transition>
           <transition event="echo"><log label="echo"/></transition>
           <transition event="inner"><log label="inner"/></transition></state>"##,
    );
    let mut options = Options::new("s-1");
    options.round_budget = budget(3);
    let started = machine.start(options).unwrap();
    let long = started.state.advance(10_000).unwrap();
    assert!(
        !long
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::BudgetExhausted { .. })),
        "{:#?}",
        long.effects
    );
    assert_eq!(labels(&long.effects).len(), 20);
    assert_eq!(long.state.next_due_ms(), Some(11_000));
    let mut state = started.state;
    let mut effects = Vec::new();
    for _ in 0..10 {
        let moved = state.advance(1_000).unwrap();
        effects.extend(moved.effects);
        state = moved.state;
    }
    assert_eq!(effects, long.effects);
    assert_eq!(state.next_due_ms(), Some(11_000));
}

// A fan-out of delayed timers that land on one later time: each was
// scheduled at an earlier time, so none is counted when it fires, and one
// long advance answers what the same time in two short ones does.
// Sabotage: with `next_instant` keeping the call's first sequence, the long
// advance counts the four fires and halts with BudgetExhausted.
#[test]
fn a_fan_out_of_delayed_timers_is_not_halted_however_time_is_split() {
    let machine = machine(
        r#" initial="a""#,
        r#"<state id="a"><onentry><send event="tick" delay="1000ms"/></onentry>
           <transition event="tick"><send event="x" delay="1000ms"/><send event="x" delay="1000ms"/>
             <send event="x" delay="1000ms"/><send event="x" delay="1000ms"/></transition>
           <transition event="x"><log label="x"/></transition></state>"#,
    );
    let mut options = Options::new("s-1");
    options.round_budget = budget(3);
    let started = machine.start(options).unwrap();
    let long = started.state.advance(2_000).unwrap();
    assert!(
        !long
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::BudgetExhausted { .. })),
        "{:#?}",
        long.effects
    );
    assert_eq!(labels(&long.effects).len(), 4);
    let first = started.state.advance(1_000).unwrap();
    let second = first.state.advance(1_000).unwrap();
    let mut effects = first.effects;
    effects.extend(second.effects);
    assert_eq!(effects, long.effects);
}

// Sabotage: with `leaf_ids` keeping every active state, the first
// assert_eq goes red (the parallel and the compound state are answered).
#[test]
fn the_leaf_read_answers_the_active_atomic_states_sorted_unnamed_left_out() {
    let started = start(
        r#" initial="p""#,
        r#"<parallel id="p"><state id="z"/><state id="s"><state id="m"/></state>
             <state><state id="b"/></state></parallel>"#,
    );
    assert_eq!(started.state.active_leaf_states(), ["b", "m", "z"]);
    assert_eq!(started.state.configuration(), ["b", "m", "p", "s", "z"]);
}

// A stopped chart has no active state, so the leaf read answers none, as
// the reference's does; the leaves it stopped in are on `Done`.
// Sabotage: with `Done::active_leaf_states` built empty in `perform_one`,
// the last assert_eq goes red; with `active_leaf_states` answering the
// done leaves on a stopped chart, the assertion that it is empty goes red.
#[test]
fn a_stopped_chart_answers_no_leaf_and_its_done_the_leaves_it_stopped_in() {
    let started = start(
        r#" initial="a""#,
        r#"<state id="a"><transition event="e.go" target="f"/></state><final id="f"/>"#,
    );
    assert_eq!(started.state.active_leaf_states(), ["a"]);
    let moved = started.state.step("e.go", Value::Undefined).unwrap();
    assert!(moved.state.active_leaf_states().is_empty());
    assert!(moved.state.configuration().is_empty());
    let done = moved.state.done().expect("a stopped chart");
    assert_eq!(done.configuration, ["f"]);
    assert_eq!(done.active_leaf_states, ["f"]);
}

// A nested final ends its parent's region, not the chart: the leaf read is
// the live configuration's.
// Sabotage: with `active_leaf_states` answering the done leaves (none while
// the chart runs) in place of the live read, the first assert_eq goes red.
#[test]
fn the_leaf_read_of_a_running_chart_holds_a_nested_final() {
    let started = start(
        r#" initial="s""#,
        r#"<state id="s"><transition event="done.state.s" target="t"/>
             <state id="a"><transition event="e.go" target="f"/></state><final id="f"/></state>
           <state id="t"/>"#,
    );
    assert_eq!(started.state.active_leaf_states(), ["a"]);
    let moved = started.state.step("e.go", Value::Undefined).unwrap();
    assert_eq!(moved.state.active_leaf_states(), ["t"]);
}

// The leaf count counts a leaf with no id, which the leaf read leaves out,
// running and once stopped, and a stopped chart counts no live leaf.
// Sabotage: with `leaf_count` counting only the states with an id, the
// assert_eq on 3 goes red; with `Done::active_leaf_count` built from the
// leaf read's length in `perform_one`, the assert_eq on 1 goes red.
#[test]
fn the_leaf_count_counts_a_leaf_with_no_id_running_and_stopped() {
    let started = start(
        r#" initial="p""#,
        r#"<parallel id="p"><state id="z"/><state/><state><state id="b"/></state></parallel>"#,
    );
    assert_eq!(started.state.active_leaf_states(), ["b", "z"]);
    assert_eq!(started.state.active_leaf_count(), 3);
    let stopped = start("", "<final/>");
    let done = stopped.state.done().expect("a stopped chart");
    assert!(done.active_leaf_states.is_empty());
    assert_eq!(done.active_leaf_count, 1);
    assert_eq!(stopped.state.active_leaf_count(), 0);
}

// A start with `x-host` registered.
fn start_hosting(attributes: &str, body: &str) -> Moved {
    let mut options = Options::new("s-1");
    options.send_types = BTreeSet::from([String::from("x-host")]);
    machine(attributes, body).start(options).expect("a start")
}

// The first send of `x-host` among `effects`, immediate or delayed.
fn handed(effects: &[Effect]) -> Effect {
    effects
        .iter()
        .find(|effect| {
            matches!(effect, Effect::Send { send_type: Some(t), .. } | Effect::SendDelayed { send_type: Some(t), .. } if *t == text("x-host"))
        })
        .cloned()
        .expect("a send handed to the host")
}

const FAILING: &str = r#"<state id="a"><onentry>
       <send event="e.out" type="x-host"/><send event="e.out" type="x-host" id="named" delay="1s"/>
     </onentry>
     <transition event="error.communication">
       <log label="sendid" expr="_event.sendid"/><log label="type" expr="_event.type"/>
       <log label="data" expr="_event.data"/><log label="origin" expr="_event.origin"/>
     </transition></state>"#;

// A report raises error.communication as a platform event, run within the
// call: its sendid the send's id, generated or named, no data, no origin,
// and the reason written nowhere.
// Sabotage: with `report_send_failed` setting no sendid, the first sendid
// log reads undefined.
#[test]
fn a_failed_send_report_raises_error_communication_with_the_sends_id() {
    let started = start_hosting(r#" initial="a""#, FAILING);
    let immediate = handed(&started.effects);
    let reported = started
        .state
        .report_send_failed(&immediate, Some("unreachable"))
        .unwrap();
    assert_eq!(
        logs(&reported.effects),
        [
            (String::from("sendid"), text("send_1")),
            (String::from("type"), text("platform")),
            (String::from("data"), Value::Undefined),
            (String::from("origin"), Value::Undefined),
        ]
    );
    let delayed = started
        .effects
        .iter()
        .find(|effect| matches!(effect, Effect::SendDelayed { .. }))
        .cloned()
        .expect("the delayed send handed");
    let reported = reported.state.report_send_failed(&delayed, None).unwrap();
    assert_eq!(
        logs(&reported.effects)[0],
        (String::from("sendid"), text("named"))
    );
    assert_eq!(reported.state.configuration(), ["a"]);
}

// A report runs the chart to a stable configuration, then takes the
// external queue as a step does.
// Sabotage: with `fail_at` not draining after the delivery, the queued
// e.next is not taken and the chart stays in b.
#[test]
fn a_failed_send_report_runs_the_chart_and_takes_its_external_queue() {
    let started = start_hosting(
        r#" initial="a""#,
        r#"<state id="a"><onentry><send event="e.out" type="x-host"/></onentry>
             <transition event="error.communication" target="b"/></state>
           <state id="b"><onentry><send event="e.next"/></onentry>
             <transition event="e.next" target="c"/></state>
           <state id="c"/>"#,
    );
    let reported = started
        .state
        .report_send_failed(&handed(&started.effects), None)
        .unwrap();
    assert_eq!(reported.state.configuration(), ["c"]);
    assert_eq!(started.state.configuration(), ["a"]);
}

// A report after the chart stopped answers NotRunning; one while a spent
// budget has halted it answers Halted; neither queues anything.
// Sabotage: with the halted check removed, the halted report answers Ok and
// the error.communication waits on the internal queue.
#[test]
fn a_failed_send_report_is_refused_by_a_stopped_or_halted_chart() {
    let stopped = start_hosting(
        r#" initial="a""#,
        r#"<state id="a"><onentry><send event="e.out" type="x-host"/></onentry>
             <transition event="e.go" target="f"/></state><final id="f"/>"#,
    );
    let send = handed(&stopped.effects);
    let done = stopped.state.step("e.go", Value::Undefined).unwrap();
    assert_eq!(
        done.state.report_send_failed(&send, None).unwrap_err(),
        Error::NotRunning
    );

    let machine = machine(
        r#" initial="a""#,
        r#"<state id="a"><onentry><send event="e.out" type="x-host"/><raise event="e.r"/></onentry>
             <transition event="e.r" target="a"/></state>"#,
    );
    let mut options = Options::new("s-1");
    options.send_types = BTreeSet::from([String::from("x-host")]);
    options.round_budget = budget(4);
    let halted = machine.start(options).unwrap();
    assert!(halted.state.halted);
    let queued = halted.state.core.internal_queue.len();
    assert_eq!(
        halted
            .state
            .report_send_failed(&handed(&halted.effects), None)
            .unwrap_err(),
        Error::Halted
    );
    assert_eq!(halted.state.core.internal_queue.len(), queued);
    assert!(Error::Halted.to_string().contains("halted"));
}

// Any effect that is not a send of a registered type handed to the host is
// refused as a value: a log, a send of the SCXML type the driver routed, a
// send of a type the session did not register, and a child wrapper around a
// log.
// Sabotage: with the registered-type check removed, the unregistered send
// is answered Ok.
#[test]
fn a_failed_send_report_refuses_an_effect_that_is_not_a_handed_send() {
    let started = start_hosting(
        r#" initial="a""#,
        r#"<state id="a"><onentry><log label="l"/><send event="e.self" delay="1s"/>
             <send event="e.out" type="x-host"/></onentry></state>"#,
    );
    let log = started
        .effects
        .iter()
        .find(|effect| matches!(effect, Effect::Log { .. }))
        .cloned()
        .unwrap();
    let routed = started
        .effects
        .iter()
        .find(|effect| {
            matches!(
                effect,
                Effect::SendDelayed {
                    send_type: None,
                    ..
                }
            )
        })
        .cloned()
        .unwrap();
    let mut unregistered = handed(&started.effects);
    if let Effect::Send { send_type, .. } = &mut unregistered {
        *send_type = Some(text("x-other"));
    }
    let wrapped = Effect::Child {
        session_id: String::from("s-1"),
        effect: Box::new(log.clone()),
    };
    for effect in [log, routed, unregistered, wrapped] {
        assert_eq!(
            started.state.report_send_failed(&effect, None).unwrap_err(),
            Error::NotAHandedSend,
            "{effect:?}"
        );
    }
    assert!(Error::NotAHandedSend.to_string().contains("registered"));
}

// A chart that logs the `_ioprocessors` entry of `x-a` and `x-b`, then
// invokes a child that logs its own.
const ENTRIES: &str = r#"<state id="a">
    <onentry>
      <log label="a" expr="_ioprocessors['x-a']"/>
      <log label="b" expr="_ioprocessors['x-b']"/>
    </onentry>
    <invoke id="i1"><content><scxml version="1.0" initial="c"><state id="c"><onentry>
      <log label="child-a" expr="_ioprocessors['x-a']"/>
      <log label="child-b" expr="_ioprocessors['x-b']"/>
    </onentry></state></scxml></content></invoke>
  </state>"#;

fn entry_logs(effects: &[Effect]) -> Vec<(String, Value)> {
    effects
        .iter()
        .flat_map(|effect| match effect {
            Effect::Child { effect, .. } => logs(std::slice::from_ref(&**effect)),
            other => logs(std::slice::from_ref(other)),
        })
        .collect()
}

fn location(text: &str) -> Value {
    let mut map = Map::new();
    map.insert("location", Value::from(text));
    Value::Map(map)
}

// Sabotage: with every registered type's entry written empty, as before
// hosts could supply one, the `a` and `b` rows go red.
#[test]
fn a_host_supplies_the_ioprocessors_entry_of_each_registered_type() {
    let mut fixed = Map::new();
    fixed.insert("queue", text("q1"));
    let mut options = Options::new("s-1");
    options.send_types = ["x-a", "x-b", "x-c"]
        .into_iter()
        .map(String::from)
        .collect();
    options
        .ioprocessors
        .insert("x-a".to_owned(), IoProcessorEntry::Fixed(fixed.clone()));
    options.ioprocessors.insert(
        "x-b".to_owned(),
        IoProcessorEntry::SessionLocation("http://h/base".to_owned()),
    );
    let started = machine(r#" initial="a""#, ENTRIES)
        .start(options.clone())
        .expect("a start");
    assert_eq!(
        entry_logs(&started.effects),
        vec![
            ("a".to_owned(), Value::Map(fixed.clone())),
            ("b".to_owned(), location("http://h/base/s-1")),
            ("child-a".to_owned(), Value::Map(fixed)),
            ("child-b".to_owned(), location("http://h/base/s-1.i1")),
        ]
    );
    let position = started.state.export_position().expect("an export");
    let Some(Value::Map(entries)) = position.datamodel.get(&Key::from("_ioprocessors")) else {
        panic!("an _ioprocessors map");
    };
    assert_eq!(
        entries.get(&Key::from("x-c")),
        Some(&Value::Map(Map::new()))
    );
    // The options the host passed are its own: start takes them by value,
    // and the copy it kept is as it was.
    assert_eq!(options.ioprocessors.len(), 2);
}

// Sabotage: with the unregistered entry dropped silently, start answers a
// chart and this goes red.
#[test]
fn an_entry_for_a_type_not_registered_is_refused() {
    let mut options = Options::new("s-1");
    options.send_types = ["x-a"].into_iter().map(String::from).collect();
    for send_type in ["x-z", "scxml", "x-y"] {
        options
            .ioprocessors
            .insert(send_type.to_owned(), IoProcessorEntry::Fixed(Map::new()));
    }
    let refused = machine(r#" initial="a""#, r#"<state id="a"/>"#)
        .start(options.clone())
        .map(|_| ())
        .unwrap_err();
    assert_eq!(
        refused,
        Error::UnregisteredEntries {
            types: vec!["scxml".to_owned(), "x-y".to_owned(), "x-z".to_owned()]
        }
    );
    assert_eq!(
        refused.to_string(),
        "an _ioprocessors entry names a send type not registered: scxml, x-y, x-z"
    );
    options.send_types.insert("scxml".to_owned());
    assert!(matches!(
        machine(r#" initial="a""#, r#"<state id="a"/>"#).start(options),
        Err(Error::BuiltInSendTypes { .. })
    ));
}

// A state imported from a position keeps the entries its datamodel carries
// and holds none of the host's to give a child. Sabotage: with the import
// inventing an entry for a child, the `child-b` row reads a location.
#[test]
fn an_imported_state_gives_a_child_empty_entries() {
    let mut options = Options::new("s-1");
    options.send_types = ["x-a", "x-b"].into_iter().map(String::from).collect();
    options.ioprocessors.insert(
        "x-b".to_owned(),
        IoProcessorEntry::SessionLocation("http://h/base".to_owned()),
    );
    let chart = machine(
        r#" initial="w""#,
        &format!(r#"<state id="w"><transition event="e.go" target="a"/></state>{ENTRIES}"#),
    );
    let started = chart.start(options).expect("a start");
    let position = started.state.export_position().expect("an export");
    let imported = chart.import_position(&position).expect("an import");
    let moved = imported.step("e.go", Value::Undefined).expect("a step");
    assert_eq!(
        entry_logs(&moved.effects),
        vec![
            ("a".to_owned(), Value::Map(Map::new())),
            ("b".to_owned(), location("http://h/base/s-1")),
            ("child-a".to_owned(), Value::Map(Map::new())),
            ("child-b".to_owned(), Value::Map(Map::new())),
        ]
    );
}
