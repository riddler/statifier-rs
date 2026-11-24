use super::*;
use crate::effect::PathSegment;
use crate::machine::Machine;

const OPEN: &str = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="s">"#;

const COUNTERS: Counters = Counters {
    macrostep: 3,
    microstep: 2,
    round: 1,
};

fn machine(onentry: &str) -> Machine {
    let text = format!(
        r#"{OPEN}<state id="s"><onentry>{onentry}</onentry></state><state id="t"/></scxml>"#
    );
    match crate::compile(&text) {
        Ok(machine) => machine,
        Err(refusals) => panic!("{text}: {refusals:#?}"),
    }
}

fn data(roots: &[(&str, Value)]) -> Map {
    roots
        .iter()
        .map(|(root, value)| (*root, value.clone()))
        .collect()
}

// The state `s`'s `<onentry>` block run against `data` with `s` active.
fn run(onentry: &str, roots: &[(&str, Value)]) -> (Ran, Vec<usize>) {
    let machine = machine(onentry);
    let chart = machine.chart();
    let state = chart.id_to_index["s"];
    let configuration: BTreeSet<usize> = [state].into_iter().collect();
    let send_types = BTreeSet::new();
    let site = Site {
        chart,
        configuration: &configuration,
        owner: BlockOwner::OnEntry { state, ordinal: 0 },
        counters: COUNTERS,
        send_types: &send_types,
        routes: None,
    };
    let content = chart.states[state].onentry[0].content.clone();
    (
        execute_block(&site, data(roots), Sends::default(), &content),
        content,
    )
}

fn get<'a>(ran: &'a Ran, root: &str) -> Option<&'a Value> {
    ran.data.get(&Key::from(root))
}

fn names(ran: &Ran) -> Vec<String> {
    ran.raised
        .iter()
        .map(|raised| match raised {
            Raised::Event { name, .. } => name.clone(),
            Raised::Error { failure, .. } => format!("error:{}", kind(failure)),
        })
        .collect()
}

fn kind(failure: &Failure) -> String {
    match failure.data() {
        Value::Map(ref map) => match map.get(&Key::from("kind")) {
            Some(Value::String(kind)) => kind.clone(),
            other => panic!("{other:?}"),
        },
        other => panic!("{other:?}"),
    }
}

fn logged(ran: &Ran) -> Vec<Option<Value>> {
    ran.effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Log { value, .. } => Some(value.clone()),
            _ => None,
        })
        .collect()
}

// Sabotage: with the `<raise>` arm naming its event empty, the queued
// names differ and this fails.
#[test]
fn raise_queues_an_internal_event_named_as_written() {
    let (ran, content) = run(r#"<raise event="e.one"/><raise event="e.two"/>"#, &[]);
    assert_eq!(
        ran.raised,
        vec![
            Raised::Event {
                name: "e.one".to_owned(),
                content: content[0]
            },
            Raised::Event {
                name: "e.two".to_owned(),
                content: content[1]
            },
        ]
    );
    assert_eq!(ran.executed, content);
    assert!(ran.effects.is_empty());
}

// Sabotage: with log stamping the owner's ordinal as 1, the effect differs
// and this fails.
#[test]
fn log_answers_its_label_and_value_with_the_counters() {
    let (ran, content) = run(
        r#"<log label="x is" expr="x + 1"/><log label="bare"/>"#,
        &[("x", Value::from(1))],
    );
    let state = 1;
    assert_eq!(
        ran.effects,
        vec![
            Effect::Log {
                label: Some("x is".to_owned()),
                value: Some(Value::from(2)),
                content_index: content[0],
                owner: BlockOwner::OnEntry { state, ordinal: 0 },
                macrostep: 3,
                microstep: 2,
                round: 1,
            },
            Effect::Log {
                label: Some("bare".to_owned()),
                value: None,
                content_index: content[1],
                owner: BlockOwner::OnEntry { state, ordinal: 0 },
                macrostep: 3,
                microstep: 2,
                round: 1,
            },
        ]
    );
}

// Sabotage: with the runner going on past a failed node, the second log
// runs and this fails.
#[test]
fn a_failing_node_stops_the_block_and_raises_error_execution() {
    let (ran, content) = run(
        r#"<log expr="1"/><raise event="e.before"/><log expr="nowhere + 1"/><log expr="2"/>"#,
        &[],
    );
    assert_eq!(logged(&ran), vec![Some(Value::from(1))]);
    assert_eq!(names(&ran), ["e.before", "error:evaluator_error"]);
    assert_eq!(ran.executed, content[..3]);
    let Some(Raised::Error {
        content: origin, ..
    }) = ran.raised.last()
    else {
        panic!("an error");
    };
    assert_eq!(*origin, content[2]);
}

// Sabotage: with assign dropping the datamodel it wrote, the log after it
// reads no value and this fails.
#[test]
fn assign_writes_a_location_that_later_nodes_read() {
    let (ran, content) = run(
        r#"<assign location="a.b" expr="7"/><log expr="a.b"/>"#,
        &[("a", Value::Undefined)],
    );
    assert_eq!(logged(&ran), vec![Some(Value::from(7))]);
    assert_eq!(
        ran.effects[0],
        Effect::DatamodelChange {
            location_path: vec![
                PathSegment::Key("a".to_owned()),
                PathSegment::Key("b".to_owned())
            ],
            location_source: "a.b".to_owned(),
            new_value: Value::from(7),
            prior_value: Value::Undefined,
            data_index: None,
            content_index: Some(content[0]),
            owner: Some(BlockOwner::OnEntry {
                state: 1,
                ordinal: 0
            }),
            macrostep: 3,
            microstep: 2,
            round: 1,
        }
    );
    let (ran, _) = run(
        r#"<assign location="undeclared" expr="1"/><raise event="e.after"/>"#,
        &[],
    );
    assert_eq!(names(&ran), ["error:unbound_location"]);
    assert!(ran.effects.is_empty());
}

// Sabotage: with if_node taking the first branch whatever its condition,
// the `<if>` branch runs and this fails.
#[test]
fn if_elseif_and_else_run_the_first_branch_whose_condition_holds() {
    let chart = r#"<if cond="x == 1"><raise event="e.if"/>
        <elseif cond="x == 2"/><raise event="e.elseif"/>
        <else/><raise event="e.else"/></if>"#;
    for (x, expected) in [(1, "e.if"), (2, "e.elseif"), (3, "e.else")] {
        let (ran, _) = run(chart, &[("x", Value::from(x))]);
        assert_eq!(names(&ran), [expected], "x = {x}");
    }
    let (ran, _) = run(r#"<if cond="false"><raise event="e.no"/></if>"#, &[]);
    assert!(ran.raised.is_empty());
}

// Sabotage: with the pending failures dropped, the condition's
// error.execution never joins the queue and this fails.
#[test]
fn a_failed_if_condition_is_passed_over_and_raised_after_the_node() {
    let (ran, content) = run(
        r#"<if cond="nowhere"><raise event="e.if"/>
           <elseif cond="1"/><raise event="e.elseif"/>
           <else/><raise event="e.else"/></if><raise event="e.next"/>"#,
        &[],
    );
    assert_eq!(
        names(&ran),
        [
            "e.else",
            "error:evaluator_error",
            "error:non_boolean_cond",
            "e.next"
        ]
    );
    assert!(ran.raised[1..3].iter().all(
        |raised| matches!(raised, Raised::Error { content, .. } if *content == ran.executed[0])
    ));
    assert_eq!(ran.executed, content);
}

// Sabotage: with node keeping a failing composite node's effects, the log
// inside the `<if>` survives and this fails; with nested naming index 0,
// the inner index differs and this fails.
#[test]
fn a_failure_inside_an_if_names_the_inner_node_and_keeps_its_writes() {
    let (ran, top) = run(
        r#"<if cond="true"><assign location="x" expr="5"/><log expr="x"/><raise event="e.in"/>
           <log expr="nowhere"/></if><raise event="e.after"/>"#,
        &[("x", Value::from(0))],
    );
    assert_eq!(get(&ran, "x"), Some(&Value::from(5)));
    assert!(ran.effects.is_empty());
    assert_eq!(names(&ran), ["e.in", "error:nested_content"]);
    let Some(Raised::Error {
        failure: Failure::Nested { content, failure },
        ..
    }) = ran.raised.last()
    else {
        panic!("a nested failure");
    };
    assert!(matches!(**failure, Failure::Evaluation { .. }));
    assert!(matches!(
        ran.data.get(&Key::from("x")),
        Some(Value::Integer(5))
    ));
    // The failing `<log>` is the fourth node of the `<if>`'s one branch,
    // and a container is interned before its children.
    assert_eq!(*content, top[0] + 4);
}

// Sabotage: with foreach binding the index from 1, the logged indexes shift
// and this fails.
#[test]
fn foreach_binds_each_item_and_its_index() {
    let (ran, _) = run(
        r#"<foreach array="[10, 20, 30]" item="v" index="i"><log expr="v + i"/>
           <assign location="sum" expr="sum + v"/></foreach>"#,
        &[("sum", Value::from(0))],
    );
    assert_eq!(
        logged(&ran),
        vec![
            Some(Value::from(10)),
            Some(Value::from(21)),
            Some(Value::from(32))
        ]
    );
    assert_eq!(get(&ran, "sum"), Some(&Value::from(60)));
    assert_eq!(get(&ran, "v"), Some(&Value::from(30)));
    assert_eq!(get(&ran, "i"), Some(&Value::from(2)));
    // An empty list still declares the names, undefined.
    let (ran, _) = run(
        r#"<foreach array="[]" item="w" index="j"/>"#,
        &[("j", Value::from(4))],
    );
    assert_eq!(get(&ran, "w"), Some(&Value::Undefined));
    assert_eq!(get(&ran, "j"), Some(&Value::from(4)));
}

// Sabotage: with foreach skipping the item's name check, the system root
// is bound and this fails.
#[test]
fn foreach_refuses_a_bad_name_or_a_value_that_is_not_a_list() {
    for (body, expected) in [
        (r#"<foreach array="[1]" item="_x"/>"#, "system_variable"),
        (r#"<foreach array="[1]" item="a.b"/>"#, "illegal_item_name"),
        (
            r#"<foreach array="[1]" item="a" index="'i'"/>"#,
            "illegal_index_name",
        ),
        (r#"<foreach array="5" item="a"/>"#, "not_iterable"),
        (r#"<foreach array="nowhere" item="a"/>"#, "evaluator_error"),
    ] {
        let (ran, _) = run(body, &[]);
        assert_eq!(names(&ran), [format!("error:{expected}")], "{body}");
        assert!(get(&ran, "a").is_none(), "{body}");
    }
}

// Sabotage: with foreach restoring the datamodel the failing iteration
// began with, that iteration's write is lost and this fails.
#[test]
fn a_foreach_body_failure_keeps_earlier_iterations() {
    let (ran, _) = run(
        r#"<foreach array="[1, 0]" item="v"><assign location="seen" expr="v"/>
           <assign location="q" expr="1 / v"/></foreach>"#,
        &[("seen", Value::Null), ("q", Value::Null)],
    );
    assert_eq!(get(&ran, "seen"), Some(&Value::from(0)));
    assert_eq!(get(&ran, "q"), Some(&Value::from(1)));
    assert_eq!(names(&ran), ["error:nested_content"]);
    assert!(ran.effects.is_empty());
}

// Sabotage: with script dropping the program's datamodel, x stays 1 and
// this fails.
#[test]
fn script_runs_its_program_and_keeps_writes_before_a_failure() {
    let (ran, _) = run(
        r#"<script>x = x + 1; fresh = 2</script><log expr="x + fresh"/>"#,
        &[("x", Value::from(1))],
    );
    assert_eq!(logged(&ran), vec![Some(Value::from(4))]);
    let (ran, _) = run(
        r#"<script>x = 9; y = nowhere</script><raise event="e.after"/>"#,
        &[("x", Value::from(1))],
    );
    assert_eq!(get(&ran, "x"), Some(&Value::from(9)));
    assert_eq!(names(&ran), ["error:evaluator_error"]);
}
