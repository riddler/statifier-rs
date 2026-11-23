//! The interpreter's algorithm, ported function for function from the
//! specification's appendix pseudocode as the reference ports it
//! (`Statifier.Interpreter` in statifier-ex): which transitions an event
//! enables, which states they leave and which they enter.
//!
//! Every function here is a pure query over a [`Snapshot`], the two globals
//! the pseudocode keeps beside the chart: the configuration and the
//! recorded history. None of them changes the snapshot or runs executable
//! content; each answers a plain value the step that calls it acts on.
//!
//! Every set of state indexes is a [`BTreeSet`], and every map keyed by a
//! state a [`BTreeMap`]: the compiler interns states in document order, so
//! ascending index order is document order, and no answer depends on a
//! hash.

mod entering;
mod exit_entry;
mod name_match;
mod selection;

#[cfg(test)]
mod tests;

use crate::machine::Chart;
use std::collections::{BTreeMap, BTreeSet};

/// The recorded history: for each history state that has recorded one, the
/// states it restores, by the history state's index.
pub(crate) type History = BTreeMap<usize, BTreeSet<usize>>;

/// What selection, exit and entry read of an execution: the chart, the
/// indexes of the active states and the recorded history.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Snapshot<'a> {
    /// The compiled chart.
    pub(crate) chart: &'a Chart,
    /// The indexes of the active states.
    pub(crate) configuration: &'a BTreeSet<usize>,
    /// The recorded history.
    pub(crate) history: &'a History,
}

impl Snapshot<'_> {
    /// Whether the state at `index` is active.
    pub(crate) fn is_active(&self, index: usize) -> bool {
        self.configuration.contains(&index)
    }
}
