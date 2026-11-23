//! Transition selection, ported function for function from the
//! pseudocode as the reference ports it
//! (`Statifier.Interpreter.Selection`); each function names the one it
//! ports.
//!
//! The domain half ([`get_effective_target_states`],
//! [`get_transition_domain`] and [`compute_exit_set`], with `findLCCA` the
//! chart's own [`Chart::lcca`](crate::machine::Chart::lcca)) answers which
//! states a transition leaves; exit and entry read it too. The selection
//! half ([`select_transitions`], [`select_eventless_transitions`] and
//! [`remove_conflicting_transitions`]) answers which transitions fire.
//!
//! The two selection walks evaluate conditions against one evaluation site
//! built per round from the datamodel the caller passes and the snapshot's
//! configuration. A condition that fails, or answers something other than
//! true or false, leaves its transition unselected: the walks answer,
//! beside the transitions, every written condition they evaluated and how
//! it came out ([`Selected::evaluations`]), in walk order, and the caller
//! raises an `error.execution` for each that failed.

use super::Snapshot;
use super::name_match::{name_match, tokenize};
use crate::datamodel::{Failure, Scope, evaluate_cond};
use crate::machine::Transition;
use predicator_rs::Map;
use std::collections::BTreeSet;

/// What a selection round answered.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Selected {
    /// The indexes of the transitions that fire, in the order the round
    /// took them.
    pub(crate) transitions: Vec<usize>,
    /// Every written condition the round evaluated, in walk order.
    pub(crate) evaluations: Vec<Evaluation>,
}

/// One written condition, evaluated. A transition with no condition is
/// never recorded.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Evaluation {
    /// The transition's index.
    pub(crate) transition: usize,
    /// How it came out.
    pub(crate) outcome: Outcome,
}

/// How a written condition came out.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Outcome {
    /// It answered true: the transition is enabled.
    Enabled,
    /// It answered false.
    Disabled,
    /// It failed, or answered something other than true or false: the
    /// transition is not enabled and the failure raises `error.execution`.
    Error(Failure),
}

/// `getEffectiveTargetStates`: the transition's targets in the order
/// written, each history target resolved to its recorded value when it has
/// one, else to its default transition's own effective targets. The list is
/// not deduplicated; no caller's answer changes on a repeat.
///
/// A history whose default transition reaches back to itself through other
/// histories of the same parent resolves to nothing on the second visit, so
/// the resolution always ends; the reference never finishes it.
pub(crate) fn get_effective_target_states(
    snapshot: &Snapshot<'_>,
    transition: &Transition,
) -> Vec<usize> {
    let mut targets = Vec::new();
    effective_targets(snapshot, &transition.targets, &mut Vec::new(), &mut targets);
    targets
}

// Each of `targets`' effective states, appended to `out`; `visiting` holds
// the histories whose default transition is being resolved.
fn effective_targets(
    snapshot: &Snapshot<'_>,
    targets: &[usize],
    visiting: &mut Vec<usize>,
    out: &mut Vec<usize>,
) {
    let chart = snapshot.chart;
    for &target in targets {
        if !chart.is_history(target) {
            out.push(target);
        } else if let Some(recorded) = snapshot.history.get(&target) {
            out.extend(recorded.iter().copied());
        } else if !visiting.contains(&target) {
            if let Some(default) = chart.states[target].history_default {
                visiting.push(target);
                effective_targets(snapshot, &chart.transitions[default].targets, visiting, out);
                visiting.pop();
            }
        }
    }
}

/// `getTransitionDomain`: the state whose active descendants the transition
/// exits, `None` when its effective targets are empty. An internal
/// transition whose source is compound and whose every effective target is
/// a proper descendant of it has its source as its domain; any other has the
/// least common compound ancestor of its source and its effective targets.
pub(crate) fn get_transition_domain(
    snapshot: &Snapshot<'_>,
    transition: &Transition,
) -> Option<usize> {
    let targets = get_effective_target_states(snapshot, transition);
    if targets.is_empty() {
        return None;
    }
    let chart = snapshot.chart;
    let source = transition.source;
    if transition.transition_type == crate::document::TransitionType::Internal
        && chart.is_compound(source)
        && targets
            .iter()
            .all(|&target| chart.is_descendant(target, source))
    {
        return Some(source);
    }
    let mut indexes = Vec::with_capacity(targets.len() + 1);
    indexes.push(source);
    indexes.extend(targets);
    chart.lcca(&indexes)
}

/// `computeExitSet`: every active state that is a proper descendant of some
/// transition's domain, across `transitions`. A transition with no written
/// target contributes nothing, and neither does one whose written targets
/// resolve to no domain. Exit order is the set's reverse.
pub(crate) fn compute_exit_set(snapshot: &Snapshot<'_>, transitions: &[usize]) -> BTreeSet<usize> {
    let chart = snapshot.chart;
    let mut exit_set = BTreeSet::new();
    for &index in transitions {
        let transition = &chart.transitions[index];
        if transition.targets.is_empty() {
            continue;
        }
        if let Some(domain) = get_transition_domain(snapshot, transition) {
            exit_set.extend(
                snapshot
                    .configuration
                    .iter()
                    .copied()
                    .filter(|&state| chart.is_descendant(state, domain)),
            );
        }
    }
    exit_set
}

/// `selectTransitions`: the transitions the event named `name` enables, at
/// most one per active atomic state, the states taken in document order.
/// Each state is searched, then each proper ancestor outward, for the first
/// transition in document order whose descriptors match the name and whose
/// condition holds, so a descendant's transition preempts an ancestor's.
/// The first occurrence of each transition is kept, and the list passes
/// through [`remove_conflicting_transitions`]. Conditions are evaluated
/// against `data`.
pub(crate) fn select_transitions(snapshot: &Snapshot<'_>, data: &Map, name: &str) -> Selected {
    let tokens = tokenize(name);
    round(snapshot, data, Some(&tokens))
}

/// `selectEventlessTransitions`: [`select_transitions`]'s walk over the
/// transitions that name no event. A transition naming one is never taken
/// here, and an eventless one never by [`select_transitions`].
pub(crate) fn select_eventless_transitions(snapshot: &Snapshot<'_>, data: &Map) -> Selected {
    round(snapshot, data, None)
}

/// `removeConflictingTransitions`: the enabled transitions, filtered to a
/// set no two of which exit a common state. Taken in the order given, each
/// is checked against those kept so far: on a conflict, one whose source is
/// a descendant of the kept one's source preempts it and keeps checking;
/// otherwise the later one is dropped and the check stops. A surviving
/// transition is appended after every kept one it preempted is removed.
pub(crate) fn remove_conflicting_transitions(
    snapshot: &Snapshot<'_>,
    enabled: &[usize],
) -> Vec<usize> {
    let chart = snapshot.chart;
    let mut filtered: Vec<usize> = Vec::new();
    for &candidate in enabled {
        let candidate_exit = compute_exit_set(snapshot, &[candidate]);
        let mut preempted = Vec::new();
        let mut kept = true;
        for &other in &filtered {
            let other_exit = compute_exit_set(snapshot, &[other]);
            if candidate_exit.is_disjoint(&other_exit) {
                continue;
            }
            if chart.is_descendant(
                chart.transitions[candidate].source,
                chart.transitions[other].source,
            ) {
                preempted.push(other);
            } else {
                kept = false;
                break;
            }
        }
        if kept {
            filtered.retain(|index| !preempted.contains(index));
            filtered.push(candidate);
        }
    }
    filtered
}

// The round both selection walks share: every active atomic state in
// document order, the written conditions recorded in walk order, the first
// occurrence of each transition kept and the conflicts removed. `tokens`
// is `None` for the eventless walk.
fn round(snapshot: &Snapshot<'_>, data: &Map, tokens: Option<&[&str]>) -> Selected {
    let chart = snapshot.chart;
    let scope = Scope {
        data,
        chart,
        configuration: snapshot.configuration,
    };
    let mut evaluations = Vec::new();
    let mut enabled: Vec<usize> = Vec::new();
    for atomic in snapshot
        .configuration
        .iter()
        .copied()
        .filter(|&state| chart.is_atomic(state))
    {
        let found = std::iter::once(atomic)
            .chain(chart.proper_ancestors(atomic))
            .find_map(|state| first_matching_transition(&scope, state, tokens, &mut evaluations));
        if let Some(transition) = found.filter(|transition| !enabled.contains(transition)) {
            enabled.push(transition);
        }
    }
    Selected {
        transitions: remove_conflicting_transitions(snapshot, &enabled),
        evaluations,
    }
}

// A state's own transitions in document order, stopping at the first
// enabled one, so a later sibling's condition is never evaluated once an
// earlier one is taken.
fn first_matching_transition(
    scope: &Scope<'_>,
    state: usize,
    tokens: Option<&[&str]>,
    evaluations: &mut Vec<Evaluation>,
) -> Option<usize> {
    let chart = scope.chart;
    chart.states[state]
        .transitions
        .iter()
        .copied()
        .find(|&index| is_enabled(scope, index, &chart.transitions[index], tokens, evaluations))
}

// The eventless predicate when `tokens` is `None`, the event-matched one
// otherwise; both end in the transition's condition (`conditionMatch`),
// which, when written, is recorded in `evaluations`.
fn is_enabled(
    scope: &Scope<'_>,
    index: usize,
    transition: &Transition,
    tokens: Option<&[&str]>,
    evaluations: &mut Vec<Evaluation>,
) -> bool {
    let matches = match tokens {
        None => transition.events.is_empty(),
        Some(tokens) => !transition.events.is_empty() && name_match(&transition.events, tokens),
    };
    if !matches {
        return false;
    }
    let Some(cond) = transition.cond.as_ref() else {
        return true;
    };
    let (enabled, outcome) = match evaluate_cond(scope, Some(cond)) {
        Ok(true) => (true, Outcome::Enabled),
        Ok(false) => (false, Outcome::Disabled),
        Err(failure) => (false, Outcome::Error(failure)),
    };
    evaluations.push(Evaluation {
        transition: index,
        outcome,
    });
    enabled
}
