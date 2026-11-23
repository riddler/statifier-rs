//! Exit and entry, ported function for function from the pseudocode as the
//! reference ports it (`Statifier.Interpreter.ExitEntry`); each function
//! names the one it ports. Which transitions fire and which states each one
//! leaves is selection's, and the entry set is entering's; this module
//! answers, for a set of transitions that fire, what leaving and entering
//! states means: the states to exit in exit order with the history
//! recorded before any of them leaves, and the states to enter in entry
//! order with, for each, the default-entry and history content to run and
//! the completion events its entry raises.
//!
//! It runs no executable content. The pseudocode's `exitStates` and
//! `enterStates` interleave the configuration's moves with content; here
//! [`exit_states`] and [`enter_states`] answer the moves as values
//! ([`Exit`], [`Entry`]), and the step that calls them runs each state's
//! `<onexit>`, `<onentry>` and transition content, evaluates a final
//! state's `<donedata>` and queues the completion events, state by state,
//! in the order given. No content moves the configuration, so the answers
//! do not depend on what the content does.
//!
//! Exit order is descending index and entry order ascending, which is
//! document order: a descendant leaves before its ancestors and enters
//! after them, and a parallel's regions leave in reverse document order and
//! enter in document order.

use super::entering::compute_entry_set;
use super::selection::compute_exit_set;
use super::{History, Snapshot};
use crate::document::HistoryType;
use std::collections::BTreeSet;

/// What [`exit_states`] answered.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Exit {
    /// The states to exit, in exit order, each to run its `<onexit>`
    /// blocks and leave.
    pub(crate) states: Vec<usize>,
    /// The recorded history, every history child of an exiting state
    /// recorded from the configuration as it stood before any state left.
    pub(crate) history: History,
    /// The configuration once every state has left.
    pub(crate) configuration: BTreeSet<usize>,
}

/// A completion event an entry raises.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Completion {
    /// `done.state.` and the completed state's id.
    pub(crate) name: String,
    /// The index of the final state whose entry raised it: the event's
    /// origin.
    pub(crate) final_state: usize,
    /// True when the event carries that final state's `<donedata>` (its
    /// parent's event), false when it carries no data (a parallel's).
    pub(crate) donedata: bool,
}

/// One state's entry, as [`enter_states`] answers it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Arrival {
    /// The state.
    pub(crate) index: usize,
    /// The index of its `<initial>` transition when it was entered by
    /// default and its document wrote one.
    pub(crate) initial_content: Option<usize>,
    /// The index of the default transition a history child registered on
    /// it.
    pub(crate) history_content: Option<usize>,
    /// The completion events its entry raises, in the order they are
    /// raised.
    pub(crate) completions: Vec<Completion>,
    /// True for a final state whose parent is the root: its entry ends the
    /// execution and raises nothing.
    pub(crate) stops: bool,
}

/// What [`enter_states`] answered.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Entry {
    /// An arrival per state to enter, in entry order, each to join the
    /// configuration and run, in order, its `<onentry>` blocks, its initial
    /// content, its history content and then its completions.
    pub(crate) states: Vec<Arrival>,
    /// The configuration once every state has entered.
    pub(crate) configuration: BTreeSet<usize>,
}

/// `exitStates`, its moves: the exit set in exit order, the history
/// recorded over it and the configuration it leaves.
pub(crate) fn exit_states(snapshot: &Snapshot<'_>, transitions: &[usize]) -> Exit {
    let exit_set = compute_exit_set(snapshot, transitions);
    let states: Vec<usize> = exit_set.iter().rev().copied().collect();
    Exit {
        history: record_history_values(snapshot, &states),
        configuration: snapshot
            .configuration
            .difference(&exit_set)
            .copied()
            .collect(),
        states,
    }
}

/// `exitStates`' history-recording loop: for every history child of every
/// state in `states_to_exit`, in that order, its value, each overwriting
/// what the history had recorded. A shallow history records the state's
/// active children, a deep one its active atomic descendants, both read
/// from the snapshot's configuration.
pub(crate) fn record_history_values(snapshot: &Snapshot<'_>, states_to_exit: &[usize]) -> History {
    let chart = snapshot.chart;
    let mut history = snapshot.history.clone();
    for &state in states_to_exit {
        for &child in &chart.states[state].history_children {
            let deep = chart.states[child].history_type == Some(HistoryType::Deep);
            let value = snapshot
                .configuration
                .iter()
                .copied()
                .filter(|&active| {
                    if deep {
                        chart.is_atomic(active) && chart.is_descendant(active, state)
                    } else {
                        chart.states[active].parent == Some(state)
                    }
                })
                .collect();
            history.insert(child, value);
        }
    }
    history
}

/// `enterStates`, its moves: the entry set in entry order, each state with
/// the content its entry runs and the completion events it raises, read
/// against the configuration as the states before it have joined it.
/// `snapshot` is the one the exit left: its configuration and history after
/// [`exit_states`].
pub(crate) fn enter_states(snapshot: &Snapshot<'_>, transitions: &[usize]) -> Entry {
    let chart = snapshot.chart;
    let entry_set = compute_entry_set(snapshot, transitions);
    let mut configuration = snapshot.configuration.clone();
    let mut states = Vec::with_capacity(entry_set.states_to_enter.len());
    for &index in &entry_set.states_to_enter {
        configuration.insert(index);
        let entered = Snapshot {
            configuration: &configuration,
            ..*snapshot
        };
        let stops = chart.is_final(index) && chart.states[index].parent == Some(0);
        states.push(Arrival {
            index,
            initial_content: entry_set
                .states_for_default_entry
                .contains(&index)
                .then_some(chart.states[index].initial_transition)
                .flatten(),
            history_content: entry_set.default_history_content.get(&index).copied(),
            completions: if stops {
                Vec::new()
            } else {
                completion_events(&entered, index)
            },
            stops,
        });
    }
    Entry {
        states,
        configuration,
    }
}

// A final state's completion events: `done.state.` and its parent's id,
// carrying its `<donedata>`, then, when the parent's own parent is a
// parallel every region of which is now in a final state, `done.state.` and
// that parallel's id, carrying no data. A parent with no id raises nothing,
// its parallel's event included; a parallel with no id raises nothing for
// itself.
fn completion_events(snapshot: &Snapshot<'_>, index: usize) -> Vec<Completion> {
    let chart = snapshot.chart;
    if !chart.is_final(index) {
        return Vec::new();
    }
    let Some(parent) = chart.states[index].parent else {
        return Vec::new();
    };
    let Some(parent_id) = id(snapshot, parent) else {
        return Vec::new();
    };
    let mut completions = vec![Completion {
        name: format!("done.state.{parent_id}"),
        final_state: index,
        donedata: true,
    }];
    let grandparent = chart.states[parent].parent;
    if let Some(grandparent) = grandparent.filter(|&state| chart.is_parallel(state)) {
        let complete = chart
            .child_states(grandparent)
            .all(|region| is_in_final_state(snapshot, region));
        if let Some(grandparent_id) = id(snapshot, grandparent).filter(|_| complete) {
            completions.push(Completion {
                name: format!("done.state.{grandparent_id}"),
                final_state: index,
                donedata: false,
            });
        }
    }
    completions
}

// The state's id, when it has a non-empty one.
fn id<'a>(snapshot: &Snapshot<'a>, index: usize) -> Option<&'a str> {
    snapshot.chart.states[index]
        .id
        .as_deref()
        .filter(|id| !id.is_empty())
}

/// `isInFinalState`: a compound state is in a final state when an active
/// child is a final; a parallel when every region is, recursively.
/// Anything else, a final itself included, is not.
pub(crate) fn is_in_final_state(snapshot: &Snapshot<'_>, index: usize) -> bool {
    let chart = snapshot.chart;
    if chart.is_compound(index) {
        chart
            .child_states(index)
            .any(|child| snapshot.is_active(child) && chart.is_final(child))
    } else if chart.is_parallel(index) {
        chart
            .child_states(index)
            .all(|region| is_in_final_state(snapshot, region))
    } else {
        false
    }
}
