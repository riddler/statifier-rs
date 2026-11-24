//! The specification's `microstep`, `exitStates`, `enterStates` and
//! `exitInterpreter` with their content run, as the reference runs them
//! (`Statifier.Interpreter.microstep/2`, `exit_interpreter/1` and
//! `Statifier.Interpreter.ExitEntry` in statifier-ex).
//!
//! Which states leave and enter, in which order, is
//! [`exit_entry`](super::exit_entry)'s answer; this module walks those moves
//! state by state against a [`MachineState`]. Each exiting state runs its
//! `<onexit>` blocks while it is still active and then leaves. Each entering
//! state joins the configuration, binds its own `<data>` on its first entry
//! under late binding, runs its `<onentry>` blocks, its initial and
//! default-history content, and then raises its completion events, its
//! parent's carrying the final state's `<donedata>`; a top-level final
//! clears `running` instead.

use super::binding;
use super::exit_entry::{self, Arrival, Entry};
use super::machine_state::{Event, MachineState, Status};
use crate::datamodel::event_data;
use crate::datamodel::{ERROR_EXECUTION, Scope, evaluate, evaluate_deferred};
use crate::effect::{BlockOwner, Effect};
use crate::machine::{Expr, Transition};
use predicator_rs::Value;

/// `microstep`: the states the transitions leave exit, each transition's
/// content runs in the order given, and the states they reach enter.
pub(crate) fn microstep(
    state: &mut MachineState,
    transitions: &[usize],
    effects: &mut Vec<Effect>,
) {
    exit_states(state, transitions, effects);
    let machine = state.machine.clone();
    for &transition in transitions {
        let content = &machine.chart().transitions[transition].content;
        state.run_block(BlockOwner::Transition { transition }, content, effects);
    }
    let entry = exit_entry::enter_states(&state.snapshot(), transitions);
    arrive_all(state, entry, effects);
}

/// `enterStates` over transitions given as values: the root's initial
/// transition, which no document wrote, enters this way.
pub(crate) fn enter<'t>(
    state: &mut MachineState,
    transitions: impl IntoIterator<Item = &'t Transition>,
    effects: &mut Vec<Effect>,
) {
    let entry = exit_entry::enter_states_of(&state.snapshot(), transitions);
    arrive_all(state, entry, effects);
}

/// `exitStates`: the history recorded from the configuration before
/// anything leaves, then each state of the exit set, in exit order, runs
/// its `<onexit>` blocks and leaves.
fn exit_states(state: &mut MachineState, transitions: &[usize], effects: &mut Vec<Effect>) {
    let exit = exit_entry::exit_states(&state.snapshot(), transitions);
    state.history = exit.history;
    for index in exit.states {
        depart(state, index, effects);
    }
}

// One state leaving: its `<onexit>` blocks in document order, then out of
// the configuration.
fn depart(state: &mut MachineState, index: usize, effects: &mut Vec<Effect>) {
    let machine = state.machine.clone();
    for (ordinal, block) in machine.chart().states[index].onexit.iter().enumerate() {
        state.run_block(
            BlockOwner::OnExit {
                state: index,
                ordinal,
            },
            &block.content,
            effects,
        );
    }
    state.configuration.remove(&index);
}

// Each arrival, in entry order.
fn arrive_all(state: &mut MachineState, entry: Entry, effects: &mut Vec<Effect>) {
    for arrival in entry.states {
        arrive(state, &arrival, effects);
    }
}

// One state entering: it joins the configuration, binds its own `<data>` on
// its first entry, runs its `<onentry>` blocks, its initial content and its
// default-history content, then raises its completions, or, for a
// top-level final, clears `running`.
fn arrive(state: &mut MachineState, arrival: &Arrival, effects: &mut Vec<Effect>) {
    let index = arrival.index;
    state.configuration.insert(index);
    if state.entered_states.insert(index) {
        binding::enter_state(state, index, effects);
    }
    let machine = state.machine.clone();
    let chart = machine.chart();
    for (ordinal, block) in chart.states[index].onentry.iter().enumerate() {
        state.run_block(
            BlockOwner::OnEntry {
                state: index,
                ordinal,
            },
            &block.content,
            effects,
        );
    }
    for transition in [arrival.initial_content, arrival.history_content]
        .into_iter()
        .flatten()
    {
        let content = &chart.transitions[transition].content;
        state.run_block(BlockOwner::Transition { transition }, content, effects);
    }
    if arrival.stops {
        state.running = false;
        return;
    }
    for completion in &arrival.completions {
        let data = if completion.donedata {
            donedata(state, completion.final_state)
        } else {
            Value::Undefined
        };
        state.enqueue(Event::platform(&completion.name, data));
    }
}

/// `exitInterpreter`: every active state leaves in exit order, running its
/// `<onexit>` blocks, with no history recorded; the top-level final among
/// them, if any, yields the donedata. [`Effect::Done`] comes last, carrying
/// the configuration as it stood and the data of the first
/// `error.execution` the donedata raised. The status becomes
/// [`Status::Done`] and the internal queue is emptied, taking any event the
/// exits raised with it: nothing can take an event once the chart has
/// stopped, so a stopped chart is quiescent.
pub(crate) fn exit_interpreter(state: &mut MachineState, effects: &mut Vec<Effect>) {
    let machine = state.machine.clone();
    let chart = machine.chart();
    let at_exit: Vec<usize> = state.configuration.iter().copied().collect();
    let mut donedata_value = None;
    let mut donedata_error = None;
    for &index in at_exit.iter().rev() {
        depart(state, index, effects);
        if chart.is_final(index) && chart.states[index].parent == Some(0) {
            let queued = state.internal_queue.len();
            donedata_value = Some(donedata(state, index));
            donedata_error = state
                .internal_queue
                .iter()
                .skip(queued)
                .find(|event| event.name == ERROR_EXECUTION)
                .map(|event| event.data.clone());
        }
    }
    effects.push(Effect::Done {
        donedata: donedata_value,
        donedata_error,
        configuration: at_exit,
        macrostep: state.counters.macrostep,
        microstep: state.counters.microstep,
        round: state.counters.round,
    });
    state.status = Status::Done;
    state.internal_queue.clear();
}

/// `returnDoneEvent`'s `s.donedata`: the final state's `<donedata>` folded to
/// the data its event carries. None, or one with neither `<content>` nor a
/// `<param>`, is no data. `<content>` text is read as event data reads text;
/// a `<content expr>` that fails raises `error.execution` and is no data.
/// `<param>`s are evaluated in document order against one site, each that
/// fails raising its own `error.execution` and dropped, the rest a map, or
/// no data when none is left.
pub(crate) fn donedata(state: &mut MachineState, index: usize) -> Value {
    let machine = state.machine.clone();
    let Some(donedata) = &machine.chart().states[index].donedata else {
        return Value::Undefined;
    };
    let scope = Scope {
        data: &state.datamodel,
        chart: machine.chart(),
        configuration: &state.configuration,
    };
    let mut failures = Vec::new();
    let value = match &donedata.expr {
        Some(Expr::Static(Value::String(text))) => event_data::text(text),
        Some(expr) => evaluate(&scope, expr).unwrap_or_else(|failure| {
            failures.push(failure);
            Value::Undefined
        }),
        None => {
            let mut pairs = Vec::with_capacity(donedata.params.len());
            for param in &donedata.params {
                match evaluate_deferred(&scope, &param.expr) {
                    Ok(value) => pairs.push((param.name.as_str(), value)),
                    Err(failure) => failures.push(failure),
                }
            }
            event_data::params(pairs)
        }
    };
    for failure in &failures {
        state.raise_failure(failure);
    }
    value
}
