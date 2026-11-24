use super::super::{Effect, Error, Machine, Moved, Options, RoundBudget};
use predicator_rs::Value;
use std::num::NonZeroU64;

const OPEN: &str = r##"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0""##;

fn machine(attributes: &str, body: &str) -> Machine {
    let text = format!("{OPEN}{attributes}>{body}</scxml>");
    match crate::compile(&text) {
        Ok(machine) => machine,
        Err(refusals) => panic!("{text}: {refusals:#?}"),
    }
}

// What `run` answers, or `None` when it has not answered within twenty
// seconds: a call that never returns fails the test rather than hanging it.
fn within_seconds<T: Send + 'static>(run: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(run());
    });
    receiver
        .recv_timeout(std::time::Duration::from_secs(20))
        .ok()
}

fn start(attributes: &str, body: &str) -> Moved {
    machine(attributes, body)
        .start(Options::new("s-1"))
        .expect("a start")
}

// An `<invoke>` of inline content whose child is `child`.
fn invoke(attributes: &str, child: &str, rest: &str) -> String {
    format!(
        r##"<invoke{attributes}><content><scxml version="1.0" {child}</scxml></content>{rest}</invoke>"##
    )
}

// Every effect's own form, each child's unwrapped, with the session it came
// from: `None` for the session the host started.
fn flat(effects: &[Effect]) -> Vec<(Option<String>, Effect)> {
    effects
        .iter()
        .map(|effect| match effect {
            Effect::Child { session_id, effect } => (Some(session_id.clone()), (**effect).clone()),
            other => (None, other.clone()),
        })
        .collect()
}

// Every `<log>`'s label, with the session it came from.
fn logs(effects: &[Effect]) -> Vec<(Option<String>, String)> {
    flat(effects)
        .into_iter()
        .filter_map(|(session, effect)| match effect {
            Effect::Log { label, .. } => Some((session, label.unwrap_or_default())),
            _ => None,
        })
        .collect()
}

fn ours(label: &str) -> (Option<String>, String) {
    (None, label.to_owned())
}

fn child(session: &str, label: &str) -> (Option<String>, String) {
    (Some(session.to_owned()), label.to_owned())
}

// The child's session id is its parent's, a dot and the invoke id, and its
// `_sessionid` reads it; every effect it answers is among the call's,
// wrapped, in the order made, after the invoke effect that started it.
// Sabotage: with `report` answering a child's effect unwrapped, the child's
// log reads as the parent's and this goes red.
#[test]
fn a_child_runs_as_its_own_session_and_its_effects_are_tagged() {
    let started = start(
        r##" initial="a""##,
        &format!(
            r##"<state id="a">{}</state>"##,
            invoke(
                r##" id="i1""##,
                r##"initial="c"><state id="c"><onentry><log label="c" expr="_sessionid"/></onentry></state>"##,
                ""
            )
        ),
    );
    let effects = flat(&started.effects);
    let invoke_at = effects
        .iter()
        .position(|(session, effect)| session.is_none() && matches!(effect, Effect::Invoke { .. }))
        .expect("an invoke effect");
    let log_at = effects
        .iter()
        .position(|(_, effect)| matches!(effect, Effect::Log { .. }))
        .expect("the child's log");
    assert!(invoke_at < log_at);
    assert_eq!(effects[log_at].0.as_deref(), Some("s-1.i1"));
    assert!(matches!(
        &effects[log_at].1,
        Effect::Log { value: Some(value), .. } if *value == Value::from("s-1.i1")
    ));
}

// A child that reaches a top-level final returns `done.invoke.<id>` carrying
// its donedata, its invoke id and the parent's own address; the parent takes
// it and the invocation is retired.
// Sabotage: with `done_event` carrying no data, `_event.data.n` is undefined
// and the parent goes to `bad`.
#[test]
fn a_child_that_stops_returns_its_donedata_in_done_invoke() {
    let started = start(
        r##" initial="a""##,
        &format!(
            r##"<state id="a">{}
                 <transition event="done.invoke.i1" cond="_event.data.n == 7 and _event.invokeid == 'i1' and _event.origin == '#_scxml_s-1'" target="good"/>
                 <transition event="*" target="bad"/></state>
               <final id="good"/><final id="bad"/>"##,
            invoke(
                r##" id="i1""##,
                r##"initial="f"><final id="f"><donedata><param name="n" expr="7"/></donedata></final>"##,
                ""
            )
        ),
    );
    let done = started.state.done().expect("the parent stopped");
    assert_eq!(done.configuration, ["good"]);
    assert!(started.state.invocations.is_empty());
}

// An event a child sends to `#_parent` reaches its parent carrying the
// invoke id, and one the parent sends to `#_<invokeid>` reaches the child.
// Sabotage: with `to_parent` not stamping the invoke id, the parent's
// condition fails and it never sends `e.pong`; the child never logs.
#[test]
fn parent_and_child_exchange_events() {
    let started = start(
        r##" initial="a""##,
        &format!(
            r##"<state id="a">{}
                 <transition event="e.ping" cond="_event.invokeid == 'i1'"><send event="e.pong" target="#_i1"/></transition>
               </state>"##,
            invoke(
                r##" id="i1""##,
                r##"initial="c"><state id="c"><onentry><send event="e.ping" target="#_parent"/></onentry>
                     <transition event="e.pong" target="d"/></state>
                   <state id="d"><onentry><log label="pong"/></onentry></state>"##,
                ""
            )
        ),
    );
    assert_eq!(logs(&started.effects), [child("s-1.i1", "pong")]);
}

// An `autoforward` invocation is handed every external event its parent
// takes, unchanged; one without autoforward is not.
// Sabotage: with the driver ignoring Effect::Autoforward, the child never
// sees `e.go` and does not log.
#[test]
fn an_autoforward_invocation_is_handed_each_external_event() {
    let child_chart = r##"initial="c"><state id="c"><transition event="e.go" cond="_event.data == 5"><log label="seen"/></transition></state>"##;
    let started = start(
        r##" initial="a""##,
        &format!(
            r##"<state id="a">{}{}</state>"##,
            invoke(r##" id="on" autoforward="true""##, child_chart, ""),
            invoke(r##" id="off""##, child_chart, "")
        ),
    );
    let stepped = started.state.step("e.go", Value::from(5)).unwrap();
    assert_eq!(logs(&stepped.effects), [child("s-1.on", "seen")]);
    assert!(stepped.effects.iter().any(|effect| matches!(
        effect,
        Effect::Autoforward { invoke_id, .. } if invoke_id == "on"
    )));
}

// A `<finalize>` runs on an event from its invocation before the parent's
// selection, `_event` already set; an empty one writes the returned values
// to its namelist and `<param location>` entries.
// Sabotage: with finalize_autoforward never matching an invoke id, x and y
// stay 0 and the parent goes to `bad`.
#[test]
fn finalize_runs_on_an_event_from_its_invocation() {
    let started = start(
        r##" initial="a""##,
        &format!(
            r##"<datamodel><data id="x" expr="0"/><data id="y" expr="0"/><data id="z" expr="0"/></datamodel>
               <state id="a">{}{}{}
                 <transition event="e.t" cond="x == 1 and y == 2 and z == 3" target="good"/>
                 <transition event="e.t" target="bad"/></state>
               <final id="good"/><final id="bad"/>"##,
            invoke(
                r##" id="i1""##,
                r##"initial="c"><state id="c"><onentry><send event="e.r" target="#_parent"><param name="v" expr="1"/></send></onentry></state>"##,
                r##"<finalize><assign location="x" expr="_event.data.v"/></finalize>"##
            ),
            invoke(
                r##" id="i2" namelist="y""##,
                r##"initial="c"><state id="c"><onentry><send event="e.s" target="#_parent"><param name="y" expr="2"/></send></onentry></state>"##,
                "<finalize/>"
            ),
            invoke(
                r##" id="i3""##,
                r##"initial="c"><state id="c"><onentry><send event="e.t" target="#_parent"><param name="w" expr="3"/></send></onentry></state>"##,
                r##"<param name="w" location="z"/><finalize/>"##
            )
        ),
    );
    assert_eq!(
        started.state.done().map(|done| done.configuration.clone()),
        Some(vec!["good".to_owned()])
    );
}

// The invoking state's exit cancels the invocation: the child's `<onexit>`
// runs, it returns no done event, and what it had sent its parent but the
// parent had not taken is discarded.
// Sabotage: with cancel_child leaving the invocation live, the child's
// queued `e.late` is taken and the parent goes to `bad`.
#[test]
fn exit_cancels_the_invocation_and_stops_its_child() {
    let started = start(
        r##" initial="a""##,
        &format!(
            r##"<state id="a"><onentry><send event="e.out"/></onentry>{}
                 <transition event="e.out" target="b"/></state>
               <state id="b"><transition event="e.late" target="bad"/><transition event="done.invoke.*" target="bad"/></state>
               <final id="bad"/>"##,
            invoke(
                r##" id="i1""##,
                r##"initial="c"><state id="c"><onentry><send event="e.late" target="#_parent"/></onentry>
                     <onexit><log label="child-exit"/></onexit></state>"##,
                ""
            )
        ),
    );
    assert_eq!(started.state.configuration(), ["b"]);
    assert_eq!(logs(&started.effects), [child("s-1.i1", "child-exit")]);
    assert!(started.state.invocations.is_empty());
    assert!(started.effects.iter().any(|effect| matches!(
        effect,
        Effect::CancelInvoke { invoke_id, .. } if invoke_id == "i1"
    )));
}

// A child's delayed send fires on its parent's clock, through
// `State::advance` on the parent; a cancelled child's pending timer never
// fires.
// Sabotage: with next_due_ms reading only the session's own timers, the
// parent reports no due timer and this goes red.
#[test]
fn a_childs_timers_fire_on_its_parents_clock() {
    let started = start(
        r##" initial="a""##,
        &format!(
            r##"<state id="a">{}<transition event="e.tick" target="b"/></state>
               <state id="b"/>"##,
            invoke(
                r##" id="i1""##,
                r##"initial="c"><state id="c"><onentry><send event="e.tick" target="#_parent" delay="50ms"/></onentry></state>"##,
                ""
            )
        ),
    );
    assert_eq!(started.state.next_due_ms(), Some(50));
    let early = started.state.advance(49).unwrap();
    assert_eq!(early.state.configuration(), ["a"]);
    let due = early.state.advance(1).unwrap();
    assert_eq!(due.state.configuration(), ["b"]);
    assert_eq!(due.state.next_due_ms(), None);
}

// A child inherits its parent's registered send types: a send of one is
// answered as an effect, never refused.
// Sabotage: with start_child passing the child no send types, the child's
// send raises error.execution and it logs `refused`.
#[test]
fn a_child_inherits_its_parents_send_types() {
    let mut options = Options::new("s-1");
    options.send_types = ["x-host"].into_iter().map(String::from).collect();
    let started = machine(
        r##" initial="a""##,
        &format!(
            r##"<state id="a">{}</state>"##,
            invoke(
                r##" id="i1""##,
                r##"initial="c"><state id="c"><onentry><send type="x-host" event="e.out"/></onentry>
                     <transition event="error.*"><log label="refused"/></transition></state>"##,
                ""
            )
        ),
    )
    .start(options)
    .unwrap();
    assert!(logs(&started.effects).is_empty());
    assert!(flat(&started.effects).iter().any(|(session, effect)| {
        session.as_deref() == Some("s-1.i1") && matches!(effect, Effect::Send { .. })
    }));
}

// A child's datamodel is seeded with the params a root `<data>` names,
// the rest dropped.
// Sabotage: with seed keeping every param, `extra` is bound in the child
// and it logs `leaked`.
#[test]
fn a_childs_root_data_takes_the_params_it_names() {
    let started = start(
        r##" initial="a""##,
        &format!(
            r##"<state id="a">{}</state>"##,
            invoke(
                r##" id="i1""##,
                r##"initial="c"><datamodel><data id="v" expr="0"/></datamodel>
                     <state id="c"><onentry>
                       <if cond="v == 4"><log label="seeded"/></if>
                       <if cond="extra == 1"><log label="leaked"/></if>
                     </onentry></state>"##,
                r##"<param name="v" expr="4"/><param name="extra" expr="1"/>"##
            )
        ),
    );
    assert_eq!(logs(&started.effects), [child("s-1.i1", "seeded")]);
}

// An invocation of another type raises error.execution and starts nothing;
// one with no content, or content that does not compile, raises
// error.communication.
// Sabotage: with start_child starting any type, the `x-other` invocation
// raises nothing and `execution` is not logged.
#[test]
fn an_invocation_that_cannot_start_raises_an_error() {
    let started = start(
        r##" initial="a""##,
        r##"<state id="a"><invoke type="x-other"/>
             <transition event="error.execution"><log label="execution"/></transition></state>"##,
    );
    assert_eq!(logs(&started.effects), [ours("execution")]);
    for body in [
        r##"<invoke src="file.scxml"/>"##,
        r##"<invoke><content>not markup</content></invoke>"##,
        r##"<invoke><content><scxml version="1.0" initial="nowhere"/></content></invoke>"##,
    ] {
        let started = start(
            r##" initial="a""##,
            &format!(
                r##"<state id="a">{body}<transition event="error.communication"><log label="communication"/></transition></state>"##
            ),
        );
        assert_eq!(logs(&started.effects), [ours("communication")], "{body}");
    }
}

// A child that stops is marked completed: its state's exit, taken before
// its done event, does not run it again, and the done event is still taken.
// Sabotage: with take_mail ignoring the completion notice, the exit cancels
// the stopped child and the done event is discarded; `done` is not logged.
#[test]
fn a_completed_child_is_not_cancelled_and_its_done_event_arrives() {
    let started = start(
        r##" initial="a""##,
        &format!(
            r##"<state id="p" initial="a"><state id="a">{}<transition event="e.first" target="b"/></state>
                 <state id="b"/>
                 <transition event="done.invoke.i1"><log label="done"/></transition></state>"##,
            invoke(
                r##" id="i1""##,
                r##"initial="c"><state id="c"><onentry><send event="e.first" target="#_parent"/></onentry>
                     <transition target="f"/></state><final id="f"/>"##,
                ""
            )
        ),
    );
    assert_eq!(started.state.configuration(), ["b", "p"]);
    assert_eq!(logs(&started.effects), [ours("done")]);
}

// A child whose budget a timer due at once spends halts: the parent's
// advance returns. The child starts with the default budget whatever its
// parent's.
// Sabotage: with `within` not handing the child the parent's fired count,
// each fire in a fresh child call counts from zero and advance does not
// return within the deadline.
#[test]
fn a_child_rescheduling_a_timer_due_at_once_halts() {
    let mut options = Options::new("s-1");
    options.round_budget = RoundBudget::Rounds(NonZeroU64::new(3).unwrap());
    let started = machine(
        r##" initial="a""##,
        &format!(
            r##"<state id="a">{}</state>"##,
            invoke(
                r##" id="i1""##,
                r##"initial="c"><state id="c"><onentry><send event="e.t" delay="0ms"/></onentry>
                     <transition event="e.t"><send event="e.t" delay="0ms"/></transition></state>"##,
                ""
            )
        ),
    )
    .start(options)
    .unwrap();
    let state = started.state;
    let advanced = within_seconds(move || state.advance(0));
    assert!(advanced.is_some(), "advance did not return");
    let advanced = advanced.unwrap().unwrap();
    assert!(flat(&advanced.effects).iter().any(|(session, effect)| {
        session.as_deref() == Some("s-1.i1")
            && matches!(effect, Effect::BudgetExhausted { budget: 10_000, .. })
    }));
}

// A stopped parent discards what its children sent and its invocations;
// a step on it is refused. The child answers the parent's last send, from
// `a`'s `<onexit>`, in the macrostep that stops the parent.
// Sabotage: with the done handler keeping the mailbox, the stopped parent
// still holds the child's `e.pong` and the first assertion goes red.
#[test]
fn a_stopped_parent_keeps_no_invocation_and_no_mail() {
    let started = start(
        r##" initial="a""##,
        &format!(
            r##"<state id="a">{}<onexit><send event="e.ping" target="#_i1"/></onexit>
                 <transition event="e.stop" target="f"/></state><final id="f"/>"##,
            invoke(
                r##" id="i1""##,
                r##"initial="c"><state id="c"><transition event="e.ping"><send event="e.pong" target="#_parent"/></transition></state>"##,
                ""
            )
        ),
    );
    let stopped = started.state.step("e.stop", Value::Undefined).unwrap();
    assert!(stopped.state.mailbox.is_empty());
    assert!(stopped.state.invocations.is_empty());
    assert_eq!(
        stopped.state.step("e.go", Value::Undefined).unwrap_err(),
        Error::NotRunning
    );
}

// An invoked child's own invocation is a grandchild whose effects are
// tagged with its own session id, and whose done event reaches the child.
// Sabotage: with `within` wrapping the child's effects again, the
// grandchild's log is tagged `s-1.i1` and this goes red.
#[test]
fn a_grandchild_is_tagged_with_its_own_session() {
    let started = start(
        r##" initial="a""##,
        r##"<state id="a"><invoke id="i1"><content><scxml version="1.0" initial="c">
             <state id="c"><invoke id="i2"><content><scxml version="1.0" initial="g">
               <state id="g"><onentry><log label="g"/></onentry><transition target="gf"/></state><final id="gf"/>
             </scxml></content></invoke>
             <transition event="done.invoke.i2"><log label="c"/></transition></state>
           </scxml></content></invoke></state>"##,
    );
    assert_eq!(
        logs(&started.effects),
        [child("s-1.i1.i2", "g"), child("s-1.i1", "c")]
    );
}
