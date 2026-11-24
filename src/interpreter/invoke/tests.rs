//! The invoke passes, through the loop.

use crate::effect::{BlockOwner, Effect};
use crate::{Moved, Options};
use predicator_rs::{Key, Map, Value};

const OPEN: &str = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0""#;

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
    let text = format!("{OPEN}{attributes}>{body}</scxml>");
    match crate::compile(&text) {
        Ok(machine) => machine.start(Options::new("s-1")).expect("a start"),
        Err(refusals) => panic!("{text}: {refusals:#?}"),
    }
}

// The parent's own invoke effects, each as (invoke id, state index, invoke
// index).
fn invokes(effects: &[Effect]) -> Vec<(String, usize, usize)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Invoke {
                invoke_id,
                state_index,
                invoke_index,
                ..
            } => Some((invoke_id.clone(), *state_index, *invoke_index)),
            _ => None,
        })
        .collect()
}

fn labels(effects: &[Effect]) -> Vec<String> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Log { label, .. } => label.clone(),
            _ => None,
        })
        .collect()
}

// A generated invoke id is `inv_` and the session's invoke counter, after
// the state's id and a dot when it has one; an author's id consumes no
// count. States invoke in document order, each state's in document order.
// Sabotage: with mint advancing the counter for an author's id too, the
// anonymous state's id is `inv_3` and this goes red.
#[test]
fn invoke_ids_are_generated_from_one_counter_or_read_as_written() {
    let started = start(
        r#" initial="p""#,
        r#"<parallel id="p">
             <state id="a"><invoke type="x-t"/><invoke id="mine" type="x-t"/><invoke type="x-t"/></state>
             <state><invoke type="x-t"/></state>
           </parallel>"#,
    );
    assert_eq!(
        invokes(&started.effects),
        [
            ("a.inv_1".to_owned(), 2, 0),
            ("mine".to_owned(), 2, 1),
            ("a.inv_2".to_owned(), 2, 2),
            ("inv_3".to_owned(), 3, 0),
        ]
    );
}

// The invoke effect carries the resolved type, source, params and content;
// a type other than the SCXML type is not recorded live.
// Sabotage: with resolve dropping the namelist, the first invocation's
// params are none and this goes red.
#[test]
fn the_invoke_effect_carries_its_resolved_arguments() {
    let started = start(
        r#" initial="a""#,
        r#"<datamodel><data id="n" expr="2"/></datamodel>
           <state id="a"><invoke typeexpr="'x-' + 't'" namelist="n"><content expr="n + 1"/></invoke>
             <invoke id="i2" type="x-t"><param name="p" expr="1"/><content>  some  text </content></invoke>
             <invoke type="x-t" srcexpr="'s.' + 'x'"/></state>"#,
    );
    let mut params = Map::new();
    params.insert(Key::from("n"), Value::from(2));
    let first = started
        .effects
        .iter()
        .find(|effect| matches!(effect, Effect::Invoke { .. }))
        .expect("an invoke");
    assert!(matches!(
        first,
        Effect::Invoke { invoke_type: Some(t), src: None, params: p, content: Some(c), autoforward: false, .. }
            if *t == Value::from("x-t") && *p == Value::Map(params.clone()) && *c == Value::from(3)
    ));
    let all: Vec<&Effect> = started
        .effects
        .iter()
        .filter(|effect| matches!(effect, Effect::Invoke { .. }))
        .collect();
    assert!(matches!(
        all[1],
        Effect::Invoke { content: Some(c), params: Value::Map(_), .. } if *c == Value::from("some text")
    ));
    assert!(matches!(
        all[2],
        Effect::Invoke { src: Some(s), content: None, params: Value::Undefined, .. } if *s == Value::from("s.x")
    ));
    assert!(
        started
            .state
            .export_position()
            .unwrap()
            .active_invocations
            .is_empty()
    );
}

// `idlocation` is written with the invoke id before the invoke effect,
// owned by the `<invoke>`.
// Sabotage: with invoke_one leaving the datamodel unwritten, `where` is
// undefined and the log's value is not the id.
#[test]
fn idlocation_is_written_before_the_invoke_effect() {
    let started = start(
        r#" initial="a""#,
        r#"<datamodel><data id="where"/></datamodel>
           <state id="a"><invoke type="x-t" idlocation="where"/><onentry><log label="l" expr="1"/></onentry>
             <transition event="e.go"><log label="where" expr="where"/></transition></state>"#,
    );
    let change_at = started
        .effects
        .iter()
        .position(|effect| {
            matches!(effect, Effect::DatamodelChange { owner: Some(BlockOwner::Invoke { state: 1, invoke: 0 }), new_value, .. }
                if *new_value == Value::from("a.inv_1"))
        })
        .expect("the idlocation write");
    let invoke_at = started
        .effects
        .iter()
        .position(|effect| matches!(effect, Effect::Invoke { .. }))
        .expect("an invoke");
    assert_eq!(change_at + 1, invoke_at);
    let stepped = started.state.step("e.go", Value::Undefined).unwrap();
    assert!(stepped.effects.iter().any(|effect| matches!(
        effect,
        Effect::Log { value: Some(value), .. } if *value == Value::from("a.inv_1")
    )));
}

// An argument that fails abandons its invocation alone: no effect, one
// error.execution, and its sibling still starts. The error re-enters the
// loop at once, in the same call.
// Sabotage: with main_event_loop ending after the invoke pass whatever the
// queue holds, the error waits unrun and `error` is not logged.
#[test]
fn a_failing_argument_abandons_its_invocation_and_raises_at_once() {
    let started = start(
        r#" initial="a""#,
        r#"<state id="a"><invoke type="x-t" srcexpr="nothing.here"/><invoke id="ok"><content><scxml version="1.0"><state id="c"/></scxml></content></invoke>
             <transition event="error.execution"><log label="error"/></transition></state>"#,
    );
    assert_eq!(invokes(&started.effects), [("ok".to_owned(), 1, 1)]);
    assert_eq!(labels(&started.effects), ["error"]);
}

// A state entered and exited in one macrostep never invokes: its exit
// takes it off the states to invoke.
// Sabotage: with exit_states leaving the states to invoke as they were,
// `a`'s invocation starts after it has exited and this goes red.
#[test]
fn a_state_left_in_the_macrostep_that_entered_it_does_not_invoke() {
    let started = start(
        r#" initial="a""#,
        r#"<state id="a"><invoke id="never" type="x-t"/><transition target="b"/></state>
           <state id="b"><invoke id="kept" type="x-t"/></state>"#,
    );
    assert_eq!(invokes(&started.effects), [("kept".to_owned(), 2, 0)]);
}

// Invocations whose start keeps failing hand each other control forever;
// the macrostep's budget spans the invoke pass's re-entries and ends it.
// Sabotage: with main_event_loop starting each re-entry on a fresh budget,
// the start does not return within the deadline.
#[test]
fn the_budget_spans_the_invoke_pass_re_entries() {
    let started = within_seconds(|| {
        start(
            r#" initial="a""#,
            r#"<state id="a"><invoke srcexpr="nothing.here"/><transition event="error.execution" target="b"/></state>
               <state id="b"><invoke srcexpr="nothing.here"/><transition event="error.execution" target="a"/></state>"#,
        )
    });
    assert!(started.is_some(), "the start did not return");
    let started = started.unwrap();
    assert!(matches!(
        started.effects.last(),
        Some(Effect::BudgetExhausted { .. })
    ));
}
