//! The accepts check: a declaration of the event names a chart accepts,
//! compared with the chart's event vocabulary, as the reference compares
//! them (`Statifier.Chart.check_accepts/2` and `events/1` in statifier-ex).
//!
//! The vocabulary is every event descriptor on a transition whose source
//! state can be active, each as the document writes it and each once, in
//! transition order: states in document order, a state's own transitions
//! before its children's. "Can be active" is a static rule over the chart's
//! structure, the specification's entry rules read with no condition and no
//! event: the root is entered by its default; a state entered by its
//! default enters its `initial` states as targets (a compound state or the
//! root), every child region by its default (a parallel state), or its
//! default transition's targets as targets (a history); a state entered as
//! a target is entered by its default, enters each of its proper ancestors,
//! and, for each parallel ancestor, enters by its default every region that
//! holds none of the targets; and every transition of an entered state
//! enters its targets as targets. The rule over-counts and never
//! under-counts.

use crate::interpreter::name_match::{name_match, tokenize};
use crate::machine::{Chart, Machine};
use std::collections::BTreeSet;

/// What [`Machine::check_accepts`] answers: the declared names the chart can
/// never select on, and the descriptors it listens for that the declaration
/// does not state.
///
/// ```
/// let machine = statifier::compile(
///     r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0">
///          <state id="a"><transition event="e.go" target="b"/></state>
///          <state id="b"/>
///        </scxml>"#,
/// )?;
/// let check = machine.check_accepts(Some(&["e.go.now", "e.stop"]));
/// assert_eq!(check.unreachable, ["e.stop"]);
/// assert!(check.undeclared.is_empty());
/// # Ok::<(), Vec<statifier::Refusal>>(())
/// ```
#[non_exhaustive]
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct AcceptsCheck {
    /// Each declared name that no descriptor in the chart's vocabulary
    /// matches, in the declaration's order and each once: a name the
    /// declaration promises and the chart can never select on.
    pub unreachable: Vec<String>,
    /// Each descriptor in the chart's vocabulary that matches no declared
    /// name, as the document writes it, in the vocabulary's order: a name
    /// the chart listens for that the declaration does not state.
    pub undeclared: Vec<String>,
}

impl Machine {
    /// Compares `declared`, the event names the chart is declared to
    /// accept, with the chart's event vocabulary: every event descriptor on
    /// a transition whose source state some path enters, as the document
    /// writes it, each once, states in document order and a state's own
    /// transitions before its children's. A pattern is a descriptor as
    /// written (`*`, `e.*`, `e.`), never expanded, and a descriptor on a
    /// state no path enters is not in the vocabulary.
    ///
    /// A descriptor matches a declared name as transition selection matches
    /// an event, on token boundaries: a declared `e.go` is matched by `e.go`,
    /// `e.*`, `e.`, `e` and `*`, and not by `e.gone` or `e.go.now`. A
    /// declared name is a name, never a pattern: a `*` in it is an ordinary
    /// token, so a declared `e.*` is matched by `e` but not by `e.go`.
    ///
    /// `None` is no declaration: the computed vocabulary is the contract,
    /// which cannot disagree with itself, so both lists are empty. An empty
    /// declaration declares that the chart accepts nothing: no name is
    /// unreachable and every descriptor is undeclared. Asked with one name,
    /// `unreachable` is the membership answer: empty when some descriptor in
    /// the vocabulary matches it.
    ///
    /// It reads the compiled chart alone, runs nothing and refuses nothing;
    /// which list a host refuses a deployment on, if either, is the host's
    /// decision.
    ///
    /// ```
    /// let machine = statifier::compile(
    ///     r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0">
    ///          <state id="a">
    ///            <transition event="e.go" target="b"/>
    ///            <transition event="e.*"/>
    ///          </state>
    ///          <state id="b"><transition event="x"/></state>
    ///          <state id="c"><transition event="y"/></state>
    ///        </scxml>"#,
    /// )?;
    ///
    /// // No declaration: nothing to disagree with.
    /// let none = machine.check_accepts(None);
    /// assert!(none.unreachable.is_empty() && none.undeclared.is_empty());
    ///
    /// // An empty declaration: every descriptor some path reaches is
    /// // undeclared, and `y`, on a state no path enters, is not one.
    /// let empty = machine.check_accepts(Some(&[]));
    /// assert_eq!(empty.undeclared, ["e.go", "e.*", "x"]);
    ///
    /// let check = machine.check_accepts(Some(&["e.go", "y"]));
    /// assert_eq!(check.unreachable, ["y"]);
    /// assert_eq!(check.undeclared, ["x"]);
    /// # Ok::<(), Vec<statifier::Refusal>>(())
    /// ```
    pub fn check_accepts(&self, declared: Option<&[&str]>) -> AcceptsCheck {
        let Some(declared) = declared else {
            return AcceptsCheck::default();
        };
        let vocabulary = vocabulary(self.chart());
        let mut seen = BTreeSet::new();
        let names: Vec<(&str, Vec<&str>)> = declared
            .iter()
            .filter(|name| seen.insert(**name))
            .map(|name| (*name, tokenize(name)))
            .collect();
        let descriptors: Vec<Vec<String>> = vocabulary
            .iter()
            .map(|(_, tokens)| (*tokens).clone())
            .collect();
        let unreachable = names
            .iter()
            .filter(|(_, tokens)| !name_match(&descriptors, tokens))
            .map(|(name, _)| (*name).to_owned())
            .collect();
        let undeclared = vocabulary
            .into_iter()
            .filter(|(_, tokens)| {
                !names
                    .iter()
                    .any(|(_, name)| name_match(std::slice::from_ref(*tokens), name))
            })
            .map(|(descriptor, _)| descriptor)
            .collect();
        AcceptsCheck {
            unreachable,
            undeclared,
        }
    }
}

/// The chart's event vocabulary: each descriptor joined back on its dots,
/// with its tokens, each once, in transition order over the states some
/// path enters, history pseudo-states left out.
fn vocabulary(chart: &Chart) -> Vec<(String, &Vec<String>)> {
    let entered = entered_states(chart);
    let mut transitions: Vec<usize> = entered
        .into_iter()
        .filter(|&index| !chart.is_history(index))
        .flat_map(|index| chart.states[index].transitions.iter().copied())
        .collect();
    transitions.sort_unstable();
    let mut seen = BTreeSet::new();
    transitions
        .into_iter()
        .flat_map(|index| chart.transitions[index].events.iter())
        .map(|tokens| (tokens.join("."), tokens))
        .filter(|(descriptor, _)| seen.insert(descriptor.clone()))
        .collect()
}

/// One item of the entry walk.
enum Entry {
    /// Enter a state by its default.
    Default(usize),
    /// Enter one transition's (or one `initial`'s) targets as targets.
    Targets(Vec<usize>),
}

/// The least set of state indexes closed under the entry rule, as a
/// worklist: each state's default expansion and each state's own
/// transitions are queued once, so the walk ends on any chart, cycles
/// included.
fn entered_states(chart: &Chart) -> BTreeSet<usize> {
    let mut entered = BTreeSet::new();
    let mut defaulted = BTreeSet::new();
    let mut work = vec![Entry::Default(0)];
    while let Some(item) = work.pop() {
        match item {
            Entry::Default(index) => {
                if defaulted.insert(index) {
                    mark_entered(chart, index, &mut entered, &mut work);
                    work.extend(default_entry(chart, index));
                }
            }
            Entry::Targets(targets) => {
                let mut ancestors = BTreeSet::new();
                for &target in &targets {
                    for ancestor in chart.proper_ancestors(target) {
                        if ancestors.insert(ancestor) {
                            mark_entered(chart, ancestor, &mut entered, &mut work);
                            work.extend(untargeted_regions(chart, ancestor, &targets));
                        }
                    }
                }
                work.extend(targets.into_iter().map(Entry::Default));
            }
        }
    }
    entered
}

/// What entering `index` by its default enters next: a parallel state's
/// regions by their defaults, a history's default transition's targets as
/// targets, and otherwise the state's `initial` states as targets; nothing
/// for an atomic state.
fn default_entry(chart: &Chart, index: usize) -> Vec<Entry> {
    let state = &chart.states[index];
    if chart.is_parallel(index) {
        chart.child_states(index).map(Entry::Default).collect()
    } else if chart.is_history(index) {
        state
            .history_default
            .map(|transition| Entry::Targets(chart.transitions[transition].targets.clone()))
            .into_iter()
            .collect()
    } else if state.initial.is_empty() {
        Vec::new()
    } else {
        vec![Entry::Targets(state.initial.clone())]
    }
}

/// A parallel ancestor's regions that hold none of `targets`, each to be
/// entered by its default; nothing for any other ancestor.
fn untargeted_regions(chart: &Chart, ancestor: usize, targets: &[usize]) -> Vec<Entry> {
    if !chart.is_parallel(ancestor) {
        return Vec::new();
    }
    chart
        .child_states(ancestor)
        .filter(|&region| {
            !targets
                .iter()
                .any(|&target| target == region || chart.is_descendant(target, region))
        })
        .map(Entry::Default)
        .collect()
}

/// Marks `index` entered; the first time only, queues each of its
/// transitions' targets, as targets, when it has any.
fn mark_entered(chart: &Chart, index: usize, entered: &mut BTreeSet<usize>, work: &mut Vec<Entry>) {
    if entered.insert(index) {
        work.extend(
            chart.states[index]
                .transitions
                .iter()
                .map(|&transition| &chart.transitions[transition].targets)
                .filter(|targets| !targets.is_empty())
                .map(|targets| Entry::Targets(targets.clone())),
        );
    }
}

#[cfg(test)]
mod tests;
