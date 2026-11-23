use super::entering::{
    EntrySet, add_ancestor_states_to_enter, add_descendant_states_to_enter, compute_entry_set,
};
use super::exit_entry::{Arrival, Completion, enter_states, exit_states, is_in_final_state};
use super::name_match::{name_match, tokenize};
use super::selection::{
    Evaluation, Outcome, compute_exit_set, get_effective_target_states, get_transition_domain,
    remove_conflicting_transitions, select_eventless_transitions, select_transitions,
};
use super::{History, Snapshot};
use crate::datamodel::Failure;
use crate::machine::{Chart, Machine};
use predicator_rs::{Map, Value};
use std::collections::BTreeSet;

const OPEN: &str = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0""#;

fn machine(attributes: &str, body: &str) -> Machine {
    let text = format!("{OPEN}{attributes}>{body}</scxml>");
    match crate::compile(&text) {
        Ok(machine) => machine,
        Err(refusals) => panic!("{text}: {refusals:#?}"),
    }
}

fn at(chart: &Chart, id: &str) -> usize {
    chart.id_to_index[id]
}

fn set(chart: &Chart, ids: &[&str]) -> BTreeSet<usize> {
    ids.iter().map(|id| at(chart, id)).collect()
}

fn ids(chart: &Chart, indexes: impl IntoIterator<Item = usize>) -> Vec<String> {
    indexes
        .into_iter()
        .map(|index| chart.states[index].id.clone().unwrap_or_default())
        .collect()
}

// The transition at `position` among the state `id`'s own.
fn transition(chart: &Chart, id: &str, position: usize) -> usize {
    chart.states[at(chart, id)].transitions[position]
}

// The default transition of the history `id`.
fn history_default(chart: &Chart, id: &str) -> usize {
    chart.states[at(chart, id)]
        .history_default
        .expect("a default")
}

// `f` over a snapshot of `chart` whose configuration is `active` and whose
// history is `history`.
fn with<T>(
    chart: &Chart,
    active: &[&str],
    history: &History,
    f: impl FnOnce(&Snapshot<'_>) -> T,
) -> T {
    let configuration = set(chart, active);
    f(&Snapshot {
        chart,
        configuration: &configuration,
        history,
    })
}

fn data(roots: &[(&str, Value)]) -> Map {
    let mut map = Map::new();
    for (root, value) in roots {
        map.insert(*root, value.clone());
    }
    map
}

// Sabotage: with descriptor_match comparing only the first token, `a.b`
// matches `a.c` and the table goes red.
#[test]
fn the_name_matcher_follows_section_3_13_on_token_boundaries() {
    // (the descriptors as written in an `event` attribute, the event name,
    // whether they match)
    let table: &[(&str, &str, bool)] = &[
        ("a", "a", true),
        ("a", "a.b", true),
        ("a", "a.b.c", true),
        ("a.b", "a.b", true),
        ("a.b", "a.b.c", true),
        ("a.b", "a", false),
        ("a.b", "a.c", false),
        ("a", "ab", false),
        ("error", "errors.execution", false),
        ("error", "error.execution", true),
        ("*", "a", true),
        ("*", "a.b", true),
        ("*", "", true),
        ("a.*", "a", true),
        ("a.*", "a.b", true),
        ("a.*", "ab", false),
        ("a.", "a", true),
        ("a.", "a.b", true),
        ("a.*.c", "a.b.c", false),
        ("a.*.c", "a.*.c", true),
        ("b", "a.b", false),
        ("a b", "b.c", true),
        ("a b", "c", false),
        ("a.b.c", "a.b", false),
        ("A", "a", false),
    ];
    for &(written, event, expected) in table {
        let descriptors: Vec<Vec<String>> = written
            .split_whitespace()
            .map(|descriptor| descriptor.split('.').map(str::to_owned).collect())
            .collect();
        assert_eq!(
            name_match(&descriptors, &tokenize(event)),
            expected,
            "{written:?} against {event:?}"
        );
    }
}

// Sabotage: with tokenize dropping empty parts, the empty name splits to
// no tokens and this fails.
#[test]
fn an_event_name_splits_on_every_dot_keeping_empty_parts() {
    assert_eq!(tokenize("a.b"), vec!["a", "b"]);
    assert_eq!(tokenize(""), vec![""]);
    assert_eq!(tokenize("a..b."), vec!["a", "", "b", ""]);
}

const NESTED: &str = r#"
  <state id="p" initial="q">
    <state id="q"><state id="q1"/><state id="q2"/></state>
    <parallel id="r"><state id="r1"/><state id="r2"/></parallel>
    <history id="h"><transition target="q"/></history>
  </state>
  <final id="f"/>"#;

// Sabotage: with is_descendant using <= for its lower bound, a state is its
// own descendant and the first assertion fails.
#[test]
fn the_chart_answers_the_structural_queries() {
    let machine = machine("", NESTED);
    let chart = machine.chart();
    let [p, q, q1, r, r1, r2, h, f] =
        ["p", "q", "q1", "r", "r1", "r2", "h", "f"].map(|id| at(chart, id));
    assert!(!chart.is_descendant(p, p));
    assert!(chart.is_descendant(q1, p));
    assert!(!chart.is_descendant(p, q1));
    assert!(!chart.is_descendant(f, p));
    assert!(chart.is_compound(0) && chart.is_compound(p) && chart.is_compound(q));
    assert!(!chart.is_compound(r) && chart.is_parallel(r));
    assert!(chart.is_atomic(q1) && chart.is_atomic(f) && !chart.is_atomic(p));
    assert!(chart.is_final(f) && !chart.is_final(q1));
    assert!(chart.is_history(h) && !chart.is_history(q));
    assert_eq!(chart.child_states(p).collect::<Vec<_>>(), vec![q, r]);
    assert_eq!(
        chart.proper_ancestors(q1).collect::<Vec<_>>(),
        vec![q, p, 0]
    );
    assert_eq!(chart.proper_ancestors(0).count(), 0);
    assert_eq!(chart.lcca(&[r1, r2]), Some(p));
    assert_eq!(chart.lcca(&[q1, r1]), Some(p));
    assert_eq!(chart.lcca(&[q1, q]), Some(p));
    assert_eq!(chart.lcca(&[]), None);
}

const HISTORIES: &str = r#"
  <state id="p" initial="a">
    <history id="hs"><transition target="b"/></history>
    <history id="hd" type="deep"><transition target="b2"/></history>
    <state id="a"/>
    <state id="b" initial="b1"><state id="b1"/><state id="b2"/></state>
    <transition event="e.shallow" target="hs"/>
    <transition event="e.deep" target="hd"/>
  </state>
  <state id="o"><transition event="e.back" target="hs"/></state>"#;

// Sabotage: with effective_targets ignoring the recorded value, the
// recorded history resolves to its default and the second assertion fails.
#[test]
fn effective_targets_resolve_a_history_to_its_record_else_its_default() {
    let machine = machine("", HISTORIES);
    let chart = machine.chart();
    let back = &chart.transitions[transition(chart, "o", 0)];
    with(chart, &["o"], &History::new(), |snapshot| {
        assert_eq!(
            get_effective_target_states(snapshot, back),
            vec![at(chart, "b")]
        );
    });
    let mut history = History::new();
    history.insert(at(chart, "hs"), set(chart, &["a"]));
    with(chart, &["o"], &history, |snapshot| {
        assert_eq!(
            get_effective_target_states(snapshot, back),
            vec![at(chart, "a")]
        );
    });
}

const CYCLE: &str = r#"
  <state id="p" initial="a">
    <history id="h1"><transition target="h2"/></history>
    <history id="h2"><transition target="h1"/></history>
    <state id="a"/>
  </state>
  <state id="o"><transition event="e.go" target="h1"/></state>"#;

// Sabotage: with effective_targets no longer checking `visiting`, the
// resolution recurses until the stack overflows.
#[test]
fn a_history_default_that_reaches_itself_resolves_to_nothing() {
    let machine = machine("", CYCLE);
    let chart = machine.chart();
    let go = transition(chart, "o", 0);
    with(chart, &["o"], &History::new(), |snapshot| {
        assert!(get_effective_target_states(snapshot, &chart.transitions[go]).is_empty());
        assert_eq!(
            get_transition_domain(snapshot, &chart.transitions[go]),
            None
        );
        assert!(compute_exit_set(snapshot, &[go]).is_empty());
    });
}

// Sabotage: with descend no longer checking `visiting` in
// enter_history_target, the entry set recurses until the stack overflows.
#[test]
fn entering_a_history_default_that_reaches_itself_ends() {
    let machine = machine("", CYCLE);
    let chart = machine.chart();
    let go = transition(chart, "o", 0);
    let entry_set = with(chart, &["o"], &History::new(), |snapshot| {
        compute_entry_set(snapshot, &[go])
    });
    assert!(entry_set.states_to_enter.is_empty());
    assert_eq!(
        entry_set.default_history_content.get(&at(chart, "p")),
        Some(&history_default(chart, "h2"))
    );
}

const DOMAINS: &str = r#"
  <state id="p" initial="q">
    <state id="q" initial="q1">
      <state id="q1"><transition event="e.up" target="q"/></state>
      <state id="q2"/>
      <transition event="e.in" type="internal" target="q2"/>
      <transition event="e.out" target="q2"/>
      <transition event="e.self" type="internal" target="q"/>
    </state>
    <parallel id="r">
      <state id="r1"><transition event="e.across" target="r2"/></state>
      <state id="r2"/>
    </parallel>
    <state id="s"><transition event="e.none"/></state>
  </state>"#;

// Sabotage: with get_transition_domain dropping the internal test, the
// internal transition's domain becomes p and the first assertion fails.
#[test]
fn a_transition_s_domain_is_its_source_only_when_internal_into_a_compound() {
    let machine = machine("", DOMAINS);
    let chart = machine.chart();
    with(chart, &["p", "q", "q1"], &History::new(), |snapshot| {
        let domain = |id, position| {
            get_transition_domain(
                snapshot,
                &chart.transitions[transition(chart, id, position)],
            )
        };
        assert_eq!(domain("q", 0), Some(at(chart, "q")));
        assert_eq!(domain("q", 1), Some(at(chart, "p")));
        assert_eq!(domain("q", 2), Some(at(chart, "p")));
        assert_eq!(domain("q1", 0), Some(at(chart, "p")));
        assert_eq!(domain("s", 0), None);
    });
}

// Sabotage: with Chart::lcca accepting a parallel candidate, the domain of
// a transition between two regions becomes r and this fails.
#[test]
fn a_parallel_is_never_the_least_common_compound_ancestor() {
    let machine = machine("", DOMAINS);
    let chart = machine.chart();
    with(
        chart,
        &["p", "r", "r1", "r2"],
        &History::new(),
        |snapshot| {
            let across = transition(chart, "r1", 0);
            assert_eq!(
                get_transition_domain(snapshot, &chart.transitions[across]),
                Some(at(chart, "p"))
            );
            assert_eq!(
                compute_exit_set(snapshot, &[across]),
                set(chart, &["r", "r1", "r2"])
            );
        },
    );
}

// Sabotage: with compute_exit_set filtering on `state >= domain` (the
// domain itself kept when active), q joins the internal transition's exit
// set and the first assertion fails.
#[test]
fn the_exit_set_is_the_active_proper_descendants_of_each_domain() {
    let machine = machine("", DOMAINS);
    let chart = machine.chart();
    with(chart, &["p", "q", "q1"], &History::new(), |snapshot| {
        let internal = transition(chart, "q", 0);
        let external = transition(chart, "q", 1);
        let targetless = transition(chart, "s", 0);
        assert_eq!(compute_exit_set(snapshot, &[internal]), set(chart, &["q1"]));
        assert_eq!(
            compute_exit_set(snapshot, &[external]),
            set(chart, &["q", "q1"])
        );
        assert!(compute_exit_set(snapshot, &[targetless]).is_empty());
        assert_eq!(
            compute_exit_set(snapshot, &[internal, external]),
            set(chart, &["q", "q1"])
        );
    });
}

const SELECTION: &str = r#"
  <parallel id="p">
    <state id="a" initial="a1">
      <state id="a1">
        <transition event="e" cond="x > 5" target="a2"/>
        <transition event="e.go" target="a2"/>
        <transition event="e" target="a1"/>
      </state>
      <state id="a2"/>
      <transition event="e.go" target="a1"/>
      <transition event="f" target="a1"/>
    </state>
    <state id="b" initial="b1">
      <state id="b1"><transition event="e.go" cond="x" target="b2"/><transition target="b2" cond="x > 1"/></state>
      <state id="b2"/>
    </state>
    <transition event="e.leave" target="p"/>
  </parallel>"#;

// Sabotage: with first_matching_transition searching the ancestors before
// the state itself, a's own `e.go` transition wins and the first assertion
// fails.
#[test]
fn selection_takes_the_first_enabled_transition_from_the_leaf_outward() {
    let machine = machine("", SELECTION);
    let chart = machine.chart();
    let data = data(&[("x", Value::from(1))]);
    with(
        chart,
        &["p", "a", "a1", "b", "b1"],
        &History::new(),
        |snapshot| {
            let selected = select_transitions(snapshot, &data, "e.go");
            assert_eq!(selected.transitions, vec![transition(chart, "a1", 1)]);
            assert_eq!(
                selected.evaluations,
                vec![
                    Evaluation {
                        transition: transition(chart, "a1", 0),
                        outcome: Outcome::Disabled,
                    },
                    Evaluation {
                        transition: transition(chart, "b1", 0),
                        outcome: Outcome::Error(Failure::NonBooleanCond(Value::from(1))),
                    },
                ]
            );
            let other = select_transitions(snapshot, &data, "f");
            assert_eq!(other.transitions, vec![transition(chart, "a", 1)]);
            assert!(
                select_transitions(snapshot, &data, "g.nothing")
                    .transitions
                    .is_empty()
            );
        },
    );
}

// Sabotage: with select_transitions passing a condition that failed as
// enabled, the failing transition is selected and this fails.
#[test]
fn a_failed_condition_leaves_its_transition_unselected_and_is_recorded() {
    let machine = machine("", SELECTION);
    let chart = machine.chart();
    with(
        chart,
        &["p", "a", "a1", "b", "b1"],
        &History::new(),
        |snapshot| {
            let selected = select_transitions(snapshot, &Map::new(), "e");
            assert_eq!(selected.transitions, vec![transition(chart, "a1", 2)]);
            assert!(matches!(
                selected.evaluations.as_slice(),
                [Evaluation {
                    outcome: Outcome::Error(Failure::Evaluation { .. }),
                    ..
                }]
            ));
        },
    );
}

// Sabotage: with is_enabled letting an eventless walk take a transition
// that names an event, `e.go`'s transitions join the eventless round and
// this fails.
#[test]
fn the_eventless_walk_takes_only_transitions_with_no_event() {
    let machine = machine("", SELECTION);
    let chart = machine.chart();
    with(
        chart,
        &["p", "a", "a1", "b", "b1"],
        &History::new(),
        |snapshot| {
            let selected = select_eventless_transitions(snapshot, &data(&[("x", Value::from(2))]));
            assert_eq!(selected.transitions, vec![transition(chart, "b1", 1)]);
            assert_eq!(selected.evaluations.len(), 1);
            let none = select_eventless_transitions(snapshot, &data(&[("x", Value::from(0))]));
            assert!(none.transitions.is_empty());
            assert_eq!(none.evaluations[0].outcome, Outcome::Disabled);
        },
    );
}

// Sabotage: with remove_conflicting_transitions keeping a candidate that
// conflicts with a kept one it cannot preempt, the repeat survives and the
// second assertion fails. round's first-occurrence filter is a second guard:
// a repeat also conflicts with itself, so dropping either alone is unseen.
#[test]
fn a_transition_two_states_select_is_taken_once() {
    let machine = machine("", SELECTION);
    let chart = machine.chart();
    with(
        chart,
        &["p", "a", "a2", "b", "b2"],
        &History::new(),
        |snapshot| {
            let selected = select_transitions(snapshot, &Map::new(), "e.leave");
            assert_eq!(selected.transitions, vec![transition(chart, "p", 0)]);
            let leave = transition(chart, "p", 0);
            assert_eq!(
                remove_conflicting_transitions(snapshot, &[leave, leave]),
                vec![leave]
            );
        },
    );
}

const CONFLICTS: &str = r#"
  <state id="top" initial="p">
    <parallel id="p">
      <state id="a">
        <state id="a1"><transition event="e" target="a2"/><transition event="f" target="z"/></state>
        <state id="a2"/>
      </state>
      <state id="b">
        <state id="b1"><transition event="e" target="b2"/><transition event="f" target="b2"/></state>
        <state id="b2"/>
      </state>
    </parallel>
    <state id="z"/>
    <transition event="g" target="z"/>
  </state>"#;

// Sabotage: with remove_conflicting_transitions dropping the later of two
// transitions whatever their exit sets, region b's transition is lost and
// the first assertion fails.
#[test]
fn transitions_in_separate_regions_do_not_conflict() {
    let machine = machine("", CONFLICTS);
    let chart = machine.chart();
    with(
        chart,
        &["top", "p", "a", "a1", "b", "b1"],
        &History::new(),
        |snapshot| {
            let selected = select_transitions(snapshot, &Map::new(), "e");
            assert_eq!(
                selected.transitions,
                vec![transition(chart, "a1", 0), transition(chart, "b1", 0)]
            );
            // a1's `f` leaves the parallel, so it exits b1 too: the later one
            // conflicts and its source is no descendant of a1, so it goes.
            let selected = select_transitions(snapshot, &Map::new(), "f");
            assert_eq!(selected.transitions, vec![transition(chart, "a1", 1)]);
        },
    );
}

// Sabotage: with the preempt branch dropping the candidate instead of the
// kept transition, the ancestor's transition survives and this fails.
#[test]
fn a_descendant_sourced_transition_preempts_a_conflicting_ancestor_s() {
    let machine = machine("", CONFLICTS);
    let chart = machine.chart();
    with(
        chart,
        &["top", "p", "a", "a1", "b", "b1"],
        &History::new(),
        |snapshot| {
            let ancestor = transition(chart, "top", 0);
            let descendant = transition(chart, "a1", 1);
            assert_eq!(
                remove_conflicting_transitions(snapshot, &[ancestor, descendant]),
                vec![descendant]
            );
            assert_eq!(
                remove_conflicting_transitions(snapshot, &[descendant, ancestor]),
                vec![descendant]
            );
        },
    );
}

const ENTRY: &str = r#"
  <state id="p">
    <initial><transition target="q2"/></initial>
    <state id="q1"/>
    <state id="q2" initial="q21"><state id="q21"/><state id="q22"/></state>
    <parallel id="r">
      <state id="r1"><state id="r11"/><state id="r12"/></state>
      <state id="r2"/>
    </parallel>
  </state>
  <state id="o">
    <transition event="e.p" target="p"/>
    <transition event="e.r" target="r"/>
    <transition event="e.r12" target="r12"/>
    <transition event="e.q21" target="q21"/>
  </state>"#;

// Sabotage: with descend no longer flagging a compound state for default
// entry, states_for_default_entry is empty and the second assertion fails.
#[test]
fn a_compound_target_enters_its_default_flagged_for_default_entry() {
    let machine = machine("", ENTRY);
    let chart = machine.chart();
    let entry_set = with(chart, &["o"], &History::new(), |snapshot| {
        compute_entry_set(snapshot, &[transition(chart, "o", 0)])
    });
    assert_eq!(entry_set.states_to_enter, set(chart, &["p", "q2", "q21"]));
    assert_eq!(entry_set.states_for_default_entry, set(chart, &["p", "q2"]));
    assert!(entry_set.default_history_content.is_empty());
}

// Sabotage: with enter_uncovered_regions skipping the covered test, r1's
// default child r11 joins the set beside r12 and the second assertion
// fails.
#[test]
fn a_parallel_enters_every_region_not_already_covered() {
    let machine = machine("", ENTRY);
    let chart = machine.chart();
    with(chart, &["o"], &History::new(), |snapshot| {
        let whole = compute_entry_set(snapshot, &[transition(chart, "o", 1)]);
        assert_eq!(
            whole.states_to_enter,
            set(chart, &["p", "r", "r1", "r11", "r2"])
        );
        let deep = compute_entry_set(snapshot, &[transition(chart, "o", 2)]);
        assert_eq!(
            deep.states_to_enter,
            set(chart, &["p", "r", "r1", "r12", "r2"])
        );
        assert!(deep.states_for_default_entry.is_empty());
    });
}

// Sabotage: with ascend including the bound itself, p joins the set and the
// first assertion fails.
#[test]
fn ancestors_enter_up_to_but_not_including_the_bound() {
    let machine = machine("", ENTRY);
    let chart = machine.chart();
    with(chart, &[], &History::new(), |snapshot| {
        let mut bounded = EntrySet::default();
        add_ancestor_states_to_enter(
            snapshot,
            at(chart, "q21"),
            Some(at(chart, "p")),
            &mut bounded,
        );
        assert_eq!(bounded.states_to_enter, set(chart, &["q2"]));
        let mut unbounded = EntrySet::default();
        add_ancestor_states_to_enter(snapshot, at(chart, "q21"), None, &mut unbounded);
        assert_eq!(
            unbounded.states_to_enter,
            [0, at(chart, "p"), at(chart, "q2")].into_iter().collect()
        );
        let mut descendants = EntrySet::default();
        add_descendant_states_to_enter(snapshot, at(chart, "q1"), &mut descendants);
        assert_eq!(descendants.states_to_enter, set(chart, &["q1"]));
    });
}

// Sabotage: with enter_history_target skipping the registration, no default
// history content is recorded and the first assertion fails.
#[test]
fn a_history_enters_its_record_else_its_default_with_its_content() {
    let machine = machine("", HISTORIES);
    let chart = machine.chart();
    let back = transition(chart, "o", 0);
    let unrecorded = with(chart, &["o"], &History::new(), |snapshot| {
        compute_entry_set(snapshot, &[back])
    });
    assert_eq!(
        unrecorded.default_history_content.get(&at(chart, "p")),
        Some(&history_default(chart, "hs"))
    );
    assert_eq!(unrecorded.states_to_enter, set(chart, &["p", "b", "b1"]));
    let mut history = History::new();
    history.insert(at(chart, "hs"), set(chart, &["a"]));
    let recorded = with(chart, &["o"], &history, |snapshot| {
        compute_entry_set(snapshot, &[back])
    });
    assert!(recorded.default_history_content.is_empty());
    assert_eq!(recorded.states_to_enter, set(chart, &["p", "a"]));
}

// Sabotage: with record_history_values recording a shallow history's
// atomic descendants, hs records b1 rather than b and this fails.
#[test]
fn exiting_records_shallow_and_deep_history_before_any_state_leaves() {
    let machine = machine("", HISTORIES);
    let chart = machine.chart();
    let leave = with(chart, &["p", "b", "b2"], &History::new(), |snapshot| {
        exit_states(snapshot, &[transition(chart, "p", 0)])
    });
    // The shallow transition targets hs inside p: its domain is the root,
    // so p and everything in it leaves, descendants first.
    assert_eq!(
        ids(chart, leave.states.iter().copied()),
        vec!["b2", "b", "p"]
    );
    assert_eq!(leave.history[&at(chart, "hs")], set(chart, &["b"]));
    assert_eq!(leave.history[&at(chart, "hd")], set(chart, &["b2"]));
    assert!(leave.configuration.is_empty());
}

// Sabotage: with exit_states answering the exit set in ascending order, p
// leaves before its descendants and this fails.
#[test]
fn exit_leaves_descendants_first_and_keeps_what_it_does_not_exit() {
    let machine = machine("", DOMAINS);
    let chart = machine.chart();
    let leave = with(chart, &["p", "q", "q1"], &History::new(), |snapshot| {
        exit_states(snapshot, &[transition(chart, "q", 1)])
    });
    assert_eq!(ids(chart, leave.states.iter().copied()), vec!["q1", "q"]);
    assert_eq!(leave.configuration, set(chart, &["p"]));
    assert!(leave.history.is_empty());
}

// Sabotage: with initial_content ignoring states_for_default_entry, p
// entered as an ancestor reports its `<initial>` transition and the last
// assertion fails.
#[test]
fn entry_answers_each_state_in_document_order_with_its_content() {
    let machine = machine("", ENTRY);
    let chart = machine.chart();
    let entry = with(chart, &["o"], &History::new(), |snapshot| {
        enter_states(snapshot, &[transition(chart, "o", 0)])
    });
    let p = at(chart, "p");
    assert_eq!(
        entry.states,
        vec![
            Arrival {
                index: p,
                initial_content: chart.states[p].initial_transition,
                history_content: None,
                completions: Vec::new(),
                stops: false,
            },
            Arrival {
                index: at(chart, "q2"),
                initial_content: None,
                history_content: None,
                completions: Vec::new(),
                stops: false,
            },
            Arrival {
                index: at(chart, "q21"),
                initial_content: None,
                history_content: None,
                completions: Vec::new(),
                stops: false,
            },
        ]
    );
    assert!(chart.states[p].initial_transition.is_some());
    assert_eq!(entry.configuration, set(chart, &["o", "p", "q2", "q21"]));
    let ancestor = with(chart, &["o"], &History::new(), |snapshot| {
        enter_states(snapshot, &[transition(chart, "o", 3)])
    });
    assert_eq!(ancestor.states[0].index, p);
    assert_eq!(ancestor.states[0].initial_content, None);
}

// Sabotage: with enter_states dropping the history content, the arrival of
// p carries none and this fails.
#[test]
fn entry_carries_the_default_history_content_on_the_history_s_parent() {
    let machine = machine("", HISTORIES);
    let chart = machine.chart();
    let entry = with(chart, &["o"], &History::new(), |snapshot| {
        enter_states(snapshot, &[transition(chart, "o", 0)])
    });
    let p = &entry.states[0];
    assert_eq!(p.index, at(chart, "p"));
    assert_eq!(p.history_content, Some(history_default(chart, "hs")));
    assert_eq!(p.initial_content, None);
}

const DONE: &str = r#"
  <state id="top" initial="p">
    <parallel id="p">
      <state id="a"><state id="a1"><transition event="e" target="af"/></state><final id="af"/></state>
      <state id="b"><state id="b1"><transition event="e" target="bf"/></state><final id="bf"/></state>
    </parallel>
    <transition event="e.end" target="end"/>
  </state>
  <final id="end"/>"#;

// Sabotage: with completion_events skipping the parallel's event, entering
// the second region's final raises only done.state.b and this fails.
#[test]
fn entering_finals_raises_done_state_for_the_parent_then_a_complete_parallel() {
    let machine = machine("", DONE);
    let chart = machine.chart();
    let [af, bf] = ["af", "bf"].map(|id| at(chart, id));
    let one = with(
        chart,
        &["top", "p", "a", "a1", "b", "b1"],
        &History::new(),
        |snapshot| enter_states(snapshot, &[transition(chart, "a1", 0)]),
    );
    assert_eq!(
        one.states[0].completions,
        vec![Completion {
            name: "done.state.a".into(),
            final_state: af,
            donedata: true,
        }]
    );
    let both = with(
        chart,
        &["top", "p", "a", "a1", "b", "b1"],
        &History::new(),
        |snapshot| {
            enter_states(
                snapshot,
                &[transition(chart, "a1", 0), transition(chart, "b1", 0)],
            )
        },
    );
    assert_eq!(both.states[0].completions.len(), 1);
    assert_eq!(
        both.states[1].completions,
        vec![
            Completion {
                name: "done.state.b".into(),
                final_state: bf,
                donedata: true,
            },
            Completion {
                name: "done.state.p".into(),
                final_state: bf,
                donedata: false,
            },
        ]
    );
}

// Sabotage: with stops testing the parent against the final's own index,
// the top-level final does not stop and this fails.
#[test]
fn a_top_level_final_stops_the_execution_and_raises_nothing() {
    let machine = machine("", DONE);
    let chart = machine.chart();
    let entry = with(
        chart,
        &["top", "p", "a", "a1", "b", "b1"],
        &History::new(),
        |snapshot| enter_states(snapshot, &[transition(chart, "top", 0)]),
    );
    assert_eq!(
        entry.states,
        vec![Arrival {
            index: at(chart, "end"),
            initial_content: None,
            history_content: None,
            completions: Vec::new(),
            stops: true,
        }]
    );
}

// Sabotage: with is_in_final_state answering a parallel with `any` in place
// of `all`, the half-finished parallel is in a final state and this fails.
#[test]
fn a_parallel_is_in_a_final_state_only_when_every_region_is() {
    let machine = machine("", DONE);
    let chart = machine.chart();
    let [a, p, af] = ["a", "p", "af"].map(|id| at(chart, id));
    with(
        chart,
        &["top", "p", "a", "af", "b", "b1"],
        &History::new(),
        |snapshot| {
            assert!(is_in_final_state(snapshot, a));
            assert!(!is_in_final_state(snapshot, p));
            assert!(!is_in_final_state(snapshot, af));
        },
    );
    with(
        chart,
        &["top", "p", "a", "af", "b", "bf"],
        &History::new(),
        |snapshot| {
            assert!(is_in_final_state(snapshot, p));
        },
    );
}
