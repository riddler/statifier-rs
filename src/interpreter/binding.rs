//! The `<data>` binding, as the reference binds
//! (`Statifier.Interpreter.Datamodel` in statifier-ex; the specification's
//! sections 5.3.2, 5.3.3 and B.2.2, its appendix giving no procedure body).
//!
//! When the chart starts every `<data>` id the host did not supply is
//! declared, as undefined, and what binds then binds: every `<data>` under
//! early binding, the root's own under late binding. A root `<data>` whose
//! id the host's datamodel held keeps the host's value. Under late binding
//! a state's own `<data>` binds when the state is first entered, before its
//! `<onentry>`; the caller decides that it is the first entry.
//!
//! One evaluation site serves a whole pass, since no `<data>` may read
//! another bound in the same pass. A value that fails (an `expr` that did
//! not compile, a `src`, which nothing here fetches, or an evaluation that
//! fails) leaves the id as it was and raises `error.execution`.

use super::machine_state::MachineState;
use crate::datamodel::{Failure, Scope, evaluate};
use crate::document::Binding;
use crate::effect::{Effect, PathSegment};
use crate::machine::DataValue;
use predicator_rs::{Key, Value};

/// `interpret`'s datamodel preamble: every `<data>` id declared, the
/// [`Effect::DatamodelInit`] of the datamodel as it then stands, and what
/// binds when the chart starts bound, each binding answering an
/// [`Effect::DatamodelChange`] in `<data>` order.
pub(crate) fn initialize(state: &mut MachineState, effects: &mut Vec<Effect>) {
    let machine = state.machine.clone();
    let chart = machine.chart();
    let host_ids: Vec<Key> = state.datamodel.keys().cloned().collect();
    for data in &chart.data {
        let key = Key::from(data.id.as_str());
        if !state.datamodel.contains_key(&key) {
            state.datamodel.insert(key, Value::Undefined);
        }
    }
    effects.push(Effect::DatamodelInit {
        datamodel: state.datamodel.clone(),
        macrostep: state.counters.macrostep,
        microstep: state.counters.microstep,
        round: state.counters.round,
    });
    let root = &chart.states[0].data;
    let mut indexes: Vec<usize> = match chart.binding {
        Binding::Early => (0..chart.data.len()).collect(),
        Binding::Late => root.clone(),
    };
    indexes.sort_unstable();
    indexes.retain(|index| {
        !(root.contains(index) && host_ids.contains(&Key::from(chart.data[*index].id.as_str())))
    });
    bind_all(state, &indexes, effects);
}

/// `enterStates`' per-state binding: the state's own `<data>` bound under
/// late binding. Nothing binds under early binding, which bound everything
/// when the chart started, nor for the root, whose own bound then too:
/// binding it again would overwrite a value the host supplied.
pub(crate) fn enter_state(state: &mut MachineState, index: usize, effects: &mut Vec<Effect>) {
    let machine = state.machine.clone();
    let chart = machine.chart();
    if chart.binding == Binding::Early || index == 0 {
        return;
    }
    let mut indexes = chart.states[index].data.clone();
    indexes.sort_unstable();
    bind_all(state, &indexes, effects);
}

// Each of `indexes`, in order, bound against one site over the datamodel as
// the pass found it; the failures raised after, in the same order.
fn bind_all(state: &mut MachineState, indexes: &[usize], effects: &mut Vec<Effect>) {
    if indexes.is_empty() {
        return;
    }
    let machine = state.machine.clone();
    let chart = machine.chart();
    let before = state.datamodel.clone();
    let scope = Scope {
        data: &before,
        chart,
        configuration: &state.configuration,
    };
    let mut failures = Vec::new();
    for &index in indexes {
        let data = &chart.data[index];
        let value = match &data.value {
            DataValue::Expr(expr) => evaluate(&scope, expr),
            DataValue::Src(src) => Err(Failure::Src(src.clone())),
            DataValue::Invalid(refusal) => Err(Failure::Compile(refusal.clone())),
        };
        match value {
            Ok(value) => {
                let key = Key::from(data.id.as_str());
                let prior_value = state
                    .datamodel
                    .get(&key)
                    .cloned()
                    .unwrap_or(Value::Undefined);
                state.datamodel.insert(key, value.clone());
                effects.push(Effect::DatamodelChange {
                    location_path: vec![PathSegment::Key(data.id.clone())],
                    location_source: data.id.clone(),
                    new_value: value,
                    prior_value,
                    data_index: Some(index),
                    content_index: None,
                    owner: None,
                    macrostep: state.counters.macrostep,
                    microstep: state.counters.microstep,
                    round: state.counters.round,
                });
            }
            Err(failure) => failures.push(failure),
        }
    }
    for failure in &failures {
        state.raise_failure(failure);
    }
}
