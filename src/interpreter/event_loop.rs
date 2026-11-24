//! The specification's `interpret` and `mainEventLoop`, ported as the
//! reference ports them (`Statifier.Interpreter` in statifier-ex): a chart
//! started, one external event taken, or one event put on the internal
//! queue, each then run to a stable configuration or to the chart stopping.
//!
//! Every entry point is a pure function: it takes a [`MachineState`] by
//! reference and answers a new one with the effects the call produced, in
//! the order they were produced ([`Stepped`]), leaving the one it was given
//! as it was. Nothing is kept anywhere else.
//!
//! Where this departs from the pseudocode, as the reference does:
//!
//! - The outer `while running` loop is the caller's: the core takes one
//!   external event per call, and the driver keeps the external events
//!   still waiting.
//! - One round, [`microstep`], is a pass of the inner `while running and not
//!   macrostepDone` loop, hoisted so a position between two rounds is a
//!   value, and counted whether or not it moves anything.
//! - The inner loop is bounded. The pseudocode lets a macrostep run forever
//!   and presumes something outside can stop it; a pure function has
//!   nothing outside, so the rounds spend a budget
//!   ([`MachineState::max_macrostep_rounds`]), and a macrostep that spends
//!   it stops where the last round left it, the chart still running, and
//!   answers [`Effect::BudgetExhausted`] last.
//! - `returnDoneEvent` is an effect, [`Effect::Done`], answered last when
//!   the chart stops.
//!
//! The invoke pass the pseudocode runs at the end of each macrostep, and the
//! finalize and autoforward pass before an external event's selection,
//! arrive with `<invoke>`; until then a chart starts no invocation, and the
//! internal queue a stable macrostep leaves is always empty, so the
//! pseudocode's `continue` after the invoke pass never fires.

use super::binding;
use super::machine_state::{Event, MachineState, RoundBudget, StartOptions};
use super::selection::{Outcome, select_eventless_transitions, select_transitions};
use super::step;
use crate::datamodel::{Scope, execute};
use crate::effect::Effect;
use crate::machine::{Chart, Machine, Transition};
use std::collections::BTreeMap;

#[cfg(test)]
mod tests;

/// What a step answers: the position it leaves and its effects, in order.
#[derive(Debug, Clone)]
pub(crate) struct Stepped {
    /// The position.
    pub(crate) state: MachineState,
    /// The effects.
    pub(crate) effects: Vec<Effect>,
}

/// The refusal of a step on a chart that has stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NotRunning;

/// How one round ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Round {
    /// It took transitions, or an internal event: another round follows.
    Progressed,
    /// It found neither, or the chart no longer runs: the macrostep ends.
    Quiescent,
}

/// How a macrostep's rounds ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Folded {
    /// Stable, or the chart stopped.
    Quiescent,
    /// The budget, this many rounds, was spent.
    Exhausted(u64),
}

/// `interpret`: a fresh position over `machine`, its datamodel initialized,
/// the `<script>` children of `<scxml>` run in document order, the initial
/// states entered, and the macrostep run. This is macrostep 1, and the
/// initial entry is its microstep 1.
pub(crate) fn initialize(machine: &Machine, options: StartOptions) -> Stepped {
    let mut state = MachineState::new(machine, options);
    state.begin_macrostep();
    state.begin_microstep();
    let mut effects = Vec::new();
    binding::initialize(&mut state, &mut effects);
    run_global_scripts(&mut state);
    let chart = machine.chart();
    let synthesized;
    let initial = match chart.states[0].initial_transition {
        Some(index) => &chart.transitions[index],
        None => {
            synthesized = initial_transition(chart);
            &synthesized
        }
    };
    step::enter(&mut state, [initial], &mut effects);
    main_event_loop(&mut state, &mut effects);
    Stepped { state, effects }
}

/// `mainEventLoop`'s external-event body: a new macrostep, `_event` set to
/// `event`, the transitions it enables selected and taken, then the
/// macrostep run. The answered position's `last_selection` says whether the
/// event selected a transition. A chart that has stopped refuses.
pub(crate) fn handle_event(state: &MachineState, event: &Event) -> Result<Stepped, NotRunning> {
    if !state.running {
        return Err(NotRunning);
    }
    let mut state = state.clone();
    state.begin_macrostep();
    state.put_event(event);
    let mut effects = Vec::new();
    let transitions = select(&mut state, Some(&event.name));
    run_selected(&mut state, &transitions, &mut effects);
    main_event_loop(&mut state, &mut effects);
    state.last_selection = Some(!transitions.is_empty());
    Ok(Stepped { state, effects })
}

/// The door back onto the internal queue: `event` queued as an in-loop
/// raise queues it, then the macrostep run. The driver routes a send to
/// `#_internal` here, and an error it found delivering a send. A chart that
/// has stopped refuses.
pub(crate) fn deliver_internal(state: &MachineState, event: Event) -> Result<Stepped, NotRunning> {
    if !state.running {
        return Err(NotRunning);
    }
    let mut state = state.clone();
    state.enqueue(event);
    let mut effects = Vec::new();
    main_event_loop(&mut state, &mut effects);
    Ok(Stepped { state, effects })
}

/// `mainEventLoop`'s cancel path: `running` cleared, then `exitInterpreter`.
/// A chart that has stopped refuses.
// No driver call cancels a chart, or steps one round or one macrostep
// alone, yet; the tests reach the port through it.
#[cfg(test)]
pub(crate) fn cancel(state: &MachineState) -> Result<Stepped, NotRunning> {
    if !state.running {
        return Err(NotRunning);
    }
    let mut state = state.clone();
    state.running = false;
    let mut effects = Vec::new();
    step::exit_interpreter(&mut state, &mut effects);
    Ok(Stepped { state, effects })
}

/// One round: the round counter advances; then, while the chart runs, the
/// eventless transitions are taken when any are enabled, else the oldest
/// internal event is taken, set as `_event`, and the transitions it enables
/// taken. A round that finds neither, or a chart no longer running, is
/// [`Round::Quiescent`].
// No driver call cancels a chart, or steps one round or one macrostep
// alone, yet; the tests reach the port through it.
#[cfg(test)]
pub(crate) fn microstep(state: &MachineState) -> (Stepped, Round) {
    let mut state = state.clone();
    let mut effects = Vec::new();
    let round = round(&mut state, &mut effects);
    (Stepped { state, effects }, round)
}

/// The rounds folded until the macrostep is stable, the chart stops or the
/// budget is spent; a spent budget answers [`Effect::BudgetExhausted`] last.
// No driver call cancels a chart, or steps one round or one macrostep
// alone, yet; the tests reach the port through it.
#[cfg(test)]
pub(crate) fn macrostep(state: &MachineState) -> Stepped {
    let mut state = state.clone();
    let mut effects = Vec::new();
    let folded = fold(&mut state, &mut effects);
    terminal(&state, folded, &mut effects);
    Stepped { state, effects }
}

// `mainEventLoop`'s body after the caller's own selection: the rounds run,
// the effect that ended them, and, once the chart has stopped,
// `exitInterpreter`.
fn main_event_loop(state: &mut MachineState, effects: &mut Vec<Effect>) {
    let folded = fold(state, effects);
    terminal(state, folded, effects);
    if !state.running {
        step::exit_interpreter(state, effects);
    }
}

// The rounds, each that progresses charged against the budget; reaching a
// stable configuration costs nothing beyond the rounds already charged.
fn fold(state: &mut MachineState, effects: &mut Vec<Effect>) -> Folded {
    let budget = state.max_macrostep_rounds;
    let mut left = budget;
    loop {
        if let (RoundBudget::Rounds(0), RoundBudget::Rounds(spent)) = (left, budget) {
            return Folded::Exhausted(spent);
        }
        if round(state, effects) == Round::Quiescent {
            return Folded::Quiescent;
        }
        if let RoundBudget::Rounds(rounds) = left {
            left = RoundBudget::Rounds(rounds.saturating_sub(1));
        }
    }
}

// The effect that ends a macrostep: [`Effect::BudgetExhausted`] when it
// spent its budget, nothing otherwise.
fn terminal(state: &MachineState, folded: Folded, effects: &mut Vec<Effect>) {
    if let Folded::Exhausted(budget) = folded {
        effects.push(budget_exhausted(state, budget));
    }
}

/// [`Effect::BudgetExhausted`] for the position as it stands and a budget
/// of `budget` rounds: the driver answers the same effect when a call
/// spends its own count of the events it takes.
pub(crate) fn budget_exhausted(state: &MachineState, budget: u64) -> Effect {
    Effect::BudgetExhausted {
        configuration: state.configuration.iter().copied().collect(),
        budget,
        pending_internal_events: state.pending_internal_events(),
        macrostep: state.counters.macrostep,
        microstep: state.counters.microstep,
        round: state.counters.round,
    }
}

// One pass of the inner loop, as [`microstep`].
fn round(state: &mut MachineState, effects: &mut Vec<Effect>) -> Round {
    state.begin_round();
    if !state.running {
        return Round::Quiescent;
    }
    let eventless = select(state, None);
    if !eventless.is_empty() {
        run_selected(state, &eventless, effects);
        return Round::Progressed;
    }
    let Some(event) = state.internal_queue.pop_front() else {
        return Round::Quiescent;
    };
    state.put_event(&event);
    let transitions = select(state, Some(&event.name));
    run_selected(state, &transitions, effects);
    Round::Progressed
}

// A selection for the event named `name`, the eventless one for `None`: the
// transitions it takes, with an `error.execution` queued per written
// condition that failed, in walk order.
fn select(state: &mut MachineState, name: Option<&str>) -> Vec<usize> {
    let selected = match name {
        Some(name) => select_transitions(&state.snapshot(), &state.datamodel, name),
        None => select_eventless_transitions(&state.snapshot(), &state.datamodel),
    };
    for evaluation in &selected.evaluations {
        if let Outcome::Error(failure) = &evaluation.outcome {
            state.raise_failure(failure);
        }
    }
    selected.transitions
}

// The tail every selection shares: when it took anything, a new microstep
// that takes it.
fn run_selected(state: &mut MachineState, transitions: &[usize], effects: &mut Vec<Effect>) {
    if transitions.is_empty() {
        return;
    }
    state.begin_microstep();
    step::microstep(state, transitions, effects);
}

// `executeGlobalScriptElement`: each `<script>` child of `<scxml>`, in
// document order, against the datamodel the one before it left. A script
// that did not compile, or whose program fails, raises `error.execution`,
// keeping what a failing program wrote.
fn run_global_scripts(state: &mut MachineState) {
    let machine = state.machine.clone();
    let chart = machine.chart();
    for script in &chart.global_scripts {
        let executed = execute(
            &Scope {
                data: &state.datamodel,
                chart,
                configuration: &state.configuration,
            },
            script,
        );
        state.datamodel = executed.data;
        if let Some(failure) = executed.failure {
            state.raise_failure(&failure);
        }
    }
}

// The root's initial transition as the reference synthesizes it when the
// document wrote no `<initial>`: from the root to its default targets,
// external, with no content.
fn initial_transition(chart: &Chart) -> Transition {
    Transition {
        source: 0,
        targets: chart.states[0].initial.clone(),
        events: Vec::new(),
        cond: None,
        cond_location: None,
        transition_type: crate::document::TransitionType::External,
        content: Vec::new(),
        location: chart.location,
        attribute_locations: BTreeMap::new(),
    }
}
