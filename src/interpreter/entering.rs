//! The entry set, ported function for function from the pseudocode as the
//! reference ports it (`Statifier.Interpreter.ExitEntry`'s entry-set
//! half): what a set of transitions enters. Each function adds to an
//! [`EntrySet`] the caller owns, where the pseudocode writes into its three
//! out-parameters; none reads or changes the configuration.

use super::Snapshot;
use super::selection::{get_effective_target_states, get_transition_domain};
use std::collections::{BTreeMap, BTreeSet};

/// The pseudocode's three `computeEntrySet` out-parameters, one value.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct EntrySet {
    /// The states to enter.
    pub(crate) states_to_enter: BTreeSet<usize>,
    /// The compound states among them entered by default rather than as a
    /// target, whose `<initial>` content runs on entry.
    pub(crate) states_for_default_entry: BTreeSet<usize>,
    /// By a history's parent, the index of the history's default
    /// transition, whose content runs because the history had recorded
    /// nothing.
    pub(crate) default_history_content: BTreeMap<usize, usize>,
}

/// `computeEntrySet`: what `transitions` enter. For each, every written
/// target enters with its descendants, then every effective target (a
/// history resolved to what it stands for) enters its ancestors up to the
/// transition's domain.
pub(crate) fn compute_entry_set(snapshot: &Snapshot<'_>, transitions: &[usize]) -> EntrySet {
    let mut entry_set = EntrySet::default();
    for &index in transitions {
        let transition = &snapshot.chart.transitions[index];
        for &target in &transition.targets {
            add_descendant_states_to_enter(snapshot, target, &mut entry_set);
        }
        let domain = get_transition_domain(snapshot, transition);
        for target in get_effective_target_states(snapshot, transition) {
            add_ancestor_states_to_enter(snapshot, target, domain, &mut entry_set);
        }
    }
    entry_set
}

/// `addDescendantStatesToEnter`: `index` and what enters with it, added to
/// `entry_set`. A history restores its recorded value or, having recorded
/// nothing, follows its default transition and registers that transition's
/// content on its parent. A compound state is marked as entered by default
/// and enters its initial targets. A parallel enters every region no state
/// already in the set lies inside. Any other state enters alone.
///
/// A history whose default transition reaches back to itself through other
/// histories of the same parent enters nothing on the second visit, so the
/// walk always ends; the reference never finishes it.
pub(crate) fn add_descendant_states_to_enter(
    snapshot: &Snapshot<'_>,
    index: usize,
    entry_set: &mut EntrySet,
) {
    descend(snapshot, index, entry_set, &mut Vec::new());
}

// `addDescendantStatesToEnter` with `visiting`, the histories whose default
// transition is being entered.
fn descend(
    snapshot: &Snapshot<'_>,
    index: usize,
    entry_set: &mut EntrySet,
    visiting: &mut Vec<usize>,
) {
    let chart = snapshot.chart;
    if chart.is_history(index) {
        enter_history_target(snapshot, index, entry_set, visiting);
        return;
    }
    entry_set.states_to_enter.insert(index);
    if chart.is_compound(index) {
        entry_set.states_for_default_entry.insert(index);
        let targets = initial_targets(snapshot, index);
        enter_targets(snapshot, &targets, Some(index), entry_set, visiting);
    } else if chart.is_parallel(index) {
        enter_uncovered_regions(snapshot, index, entry_set, visiting);
    }
}

/// `addAncestorStatesToEnter`: every proper ancestor of `index` up to but
/// not including `ancestor` enters, each parallel among them with its
/// regions not yet covered. `None` bounds nothing.
pub(crate) fn add_ancestor_states_to_enter(
    snapshot: &Snapshot<'_>,
    index: usize,
    ancestor: Option<usize>,
    entry_set: &mut EntrySet,
) {
    ascend(snapshot, index, ancestor, entry_set, &mut Vec::new());
}

// `addAncestorStatesToEnter` with the histories being entered.
fn ascend(
    snapshot: &Snapshot<'_>,
    index: usize,
    ancestor: Option<usize>,
    entry_set: &mut EntrySet,
    visiting: &mut Vec<usize>,
) {
    let chart = snapshot.chart;
    for state in chart
        .proper_ancestors(index)
        .take_while(|&state| Some(state) != ancestor)
    {
        entry_set.states_to_enter.insert(state);
        if chart.is_parallel(state) {
            enter_uncovered_regions(snapshot, state, entry_set, visiting);
        }
    }
}

// The history arm: the recorded value when there is one, else the default
// transition's targets with its content registered on the history's
// parent; either way the targets enter up to that parent.
fn enter_history_target(
    snapshot: &Snapshot<'_>,
    history: usize,
    entry_set: &mut EntrySet,
    visiting: &mut Vec<usize>,
) {
    let chart = snapshot.chart;
    let parent = chart.states[history].parent;
    if let Some(recorded) = snapshot.history.get(&history) {
        let targets: Vec<usize> = recorded.iter().copied().collect();
        enter_targets(snapshot, &targets, parent, entry_set, visiting);
        return;
    }
    let Some(default) = chart.states[history].history_default else {
        return;
    };
    if visiting.contains(&history) {
        return;
    }
    if let Some(parent) = parent {
        entry_set.default_history_content.insert(parent, default);
    }
    visiting.push(history);
    let targets = &chart.transitions[default].targets;
    enter_targets(snapshot, targets, parent, entry_set, visiting);
    visiting.pop();
}

// Every target's descendants, then every target's ancestors up to
// `ancestor`.
fn enter_targets(
    snapshot: &Snapshot<'_>,
    targets: &[usize],
    ancestor: Option<usize>,
    entry_set: &mut EntrySet,
    visiting: &mut Vec<usize>,
) {
    for &target in targets {
        descend(snapshot, target, entry_set, visiting);
    }
    for &target in targets {
        ascend(snapshot, target, ancestor, entry_set, visiting);
    }
}

// A compound state's default-entry targets: its `<initial>` transition's
// when the document wrote one, else the default the compiler resolved from
// the `initial` attribute or the first child.
fn initial_targets(snapshot: &Snapshot<'_>, index: usize) -> Vec<usize> {
    let chart = snapshot.chart;
    let state = &chart.states[index];
    match state.initial_transition {
        Some(transition) => chart.transitions[transition].targets.clone(),
        None => state.initial.clone(),
    }
}

// Every region of the parallel `index` that no state already in the set
// lies inside enters with its descendants. A history child is never a
// region.
fn enter_uncovered_regions(
    snapshot: &Snapshot<'_>,
    index: usize,
    entry_set: &mut EntrySet,
    visiting: &mut Vec<usize>,
) {
    let chart = snapshot.chart;
    for region in chart.child_states(index) {
        let covered = entry_set
            .states_to_enter
            .iter()
            .any(|&state| chart.is_descendant(state, region));
        if !covered {
            descend(snapshot, region, entry_set, visiting);
        }
    }
}
