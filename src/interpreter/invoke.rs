//! The invoke passes, as the reference ports them (`run_invoke_pass`,
//! `invoke_one`, `generate_invoke_id`, `apply_invoke_passes`,
//! `apply_finalize` and `auto_assign_finalize` in `Statifier.Interpreter`,
//! and the cancel of a state's invocations in
//! `Statifier.Interpreter.ExitEntry`, statifier-ex): starting the
//! invocations of the states a macrostep entered, the finalize and
//! autoforward pass an external event meets before its selection, and the
//! cancel of a state's live invocations as it exits.
//!
//! The core starts no session. Starting an invocation answers an
//! [`Effect::Invoke`] and, for the SCXML type, records it live in
//! `active_invocations`; forwarding an event answers an
//! [`Effect::Autoforward`]; a state that exits answers an
//! [`Effect::CancelInvoke`] per live invocation it owns. Running the child,
//! delivering to it and turning what it answers into events are the
//! driver's. An event an invocation sent is an ordinary external event
//! here: its `invokeid` decides only which `<finalize>` runs.
//!
//! No host declares the invoke types it runs, so every invocation is judged
//! as the reference judges one in a session that declared none: the effect
//! is answered for any type, and only the SCXML type is recorded live.

#[cfg(test)]
mod tests;

use super::machine_state::{Event, MachineState};
use crate::datamodel::location::{Written, write_location};
use crate::datamodel::{Failure, Scope, evaluate, evaluate_deferred, event_data};
use crate::effect::{BlockOwner, Effect};
use crate::machine::{Deferred, Expr, Invoke, ParamKind};
use predicator_rs::{Key, Value};

/// The URI of the SCXML invoke type.
const SCXML_TYPE: &str = "http://www.w3.org/TR/scxml/";

/// Whether the SCXML processor runs an invocation of `invoke_type` itself:
/// none, the short form `scxml`, or the type's URI with or without its
/// trailing slash.
pub(crate) fn is_scxml_type(invoke_type: Option<&Value>) -> bool {
    match invoke_type {
        None => true,
        Some(Value::String(text)) => {
            text == "scxml"
                || text == SCXML_TYPE
                || Some(text.as_str()) == SCXML_TYPE.strip_suffix('/')
        }
        Some(_) => false,
    }
}

/// An `<invoke>`'s arguments, resolved.
struct Resolved {
    invoke_type: Option<Value>,
    src: Option<Value>,
    params: Value,
    content: Option<Value>,
}

/// The key an invocation is live under: its state's index and its own among
/// the state's `<invoke>` elements.
fn key(state: usize, invoke: &Invoke) -> (usize, u64) {
    (state, u64::try_from(invoke.index).unwrap_or(u64::MAX))
}

/// `for state in statesToInvoke.sort(entryOrder): for inv in
/// state.invoke.sort(documentOrder): invoke(inv)`, then
/// `statesToInvoke.clear()`. State indexes ascend in document order, the
/// order the reference sorts the states to invoke in. An invocation whose
/// start fails raises `error.execution` and answers nothing; its siblings
/// are unaffected.
pub(crate) fn invoke_pass(state: &mut MachineState, effects: &mut Vec<Effect>) {
    let order = std::mem::take(&mut state.states_to_invoke);
    let machine = state.machine.clone();
    for index in order {
        let Some(node) = machine.chart().states.get(index) else {
            continue;
        };
        for invoke in &node.invoke {
            invoke_one(state, index, invoke, effects);
        }
    }
}

/// `invoke(inv)`: the type, the source, the `namelist` and `<param>` values
/// in order, then `<content>`, the first failure abandoning this
/// invocation; then the id minted or read, `idlocation` written, the
/// invocation recorded live for the SCXML type, and the effect, after the
/// write's [`Effect::DatamodelChange`].
fn invoke_one(state: &mut MachineState, index: usize, invoke: &Invoke, effects: &mut Vec<Effect>) {
    let resolved = resolve(
        &Scope {
            data: &state.datamodel,
            chart: state.machine.chart(),
            configuration: &state.configuration,
        },
        invoke,
    );
    let resolved = match resolved {
        Ok(resolved) => resolved,
        Err(failure) => return state.raise_failure(&failure),
    };
    let invoke_id = mint(state, index, invoke);
    let written = match &invoke.idlocation {
        Some(location) => {
            match write_location(&state.datamodel, location, Value::from(invoke_id.as_str())) {
                Ok(written) => Some((location, written)),
                Err(failure) => return state.raise_failure(&failure),
            }
        }
        None => None,
    };
    let owner = BlockOwner::Invoke {
        state: index,
        invoke: invoke.index,
    };
    if let Some((location, written)) = written {
        state.datamodel = written.data.clone();
        effects.push(change(state, location, written, owner));
    }
    if is_scxml_type(resolved.invoke_type.as_ref()) {
        state
            .active_invocations
            .insert(key(index, invoke), invoke_id.clone());
    }
    effects.push(Effect::Invoke {
        invoke_id,
        invoke_type: resolved.invoke_type,
        src: resolved.src,
        params: resolved.params,
        content: resolved.content,
        autoforward: invoke.autoforward,
        state_index: index,
        invoke_index: invoke.index,
        macrostep: state.counters.macrostep,
        microstep: state.counters.microstep,
        round: state.counters.round,
    });
}

/// The arguments in the reference's order, the first failure answered.
fn resolve(scope: &Scope<'_>, invoke: &Invoke) -> Result<Resolved, Failure> {
    let optional = |expr: Option<&Expr>| -> Result<Option<Value>, Failure> {
        Ok(expr
            .map(|expr| evaluate(scope, expr))
            .transpose()?
            .filter(|value| !matches!(value, Value::Null)))
    };
    let invoke_type = optional(invoke.invoke_type.as_ref())?;
    let src = optional(invoke.src.as_ref())?;
    let mut pairs = Vec::with_capacity(invoke.namelist.len() + invoke.params.len());
    for param in invoke.namelist.iter().chain(&invoke.params) {
        pairs.push((param.name.as_str(), evaluate_deferred(scope, &param.expr)?));
    }
    let content = match &invoke.content {
        None => None,
        Some(Expr::Static(Value::String(text))) => Some(event_data::text(text)),
        Some(expr) => Some(evaluate(scope, expr)?),
    };
    Ok(Resolved {
        invoke_type,
        src,
        params: event_data::params(pairs),
        content,
    })
}

/// `generate_invoke_id`: the author's id, which consumes no counter; else
/// `inv_` and the counter plus one, after the state's id and a dot when the
/// state has one.
fn mint(state: &mut MachineState, index: usize, invoke: &Invoke) -> String {
    if let Some(id) = &invoke.id {
        return id.clone();
    }
    state.invoke_counter = state.invoke_counter.saturating_add(1);
    let platform_id = format!("inv_{}", state.invoke_counter);
    match state
        .machine
        .chart()
        .states
        .get(index)
        .and_then(|node| node.id.as_deref())
    {
        Some(owner) => format!("{owner}.{platform_id}"),
        None => platform_id,
    }
}

/// A write no content node made: an `idlocation`'s, or an empty
/// `<finalize>`'s.
fn change(state: &MachineState, source: &str, written: Written, owner: BlockOwner) -> Effect {
    Effect::DatamodelChange {
        location_path: written.path,
        location_source: source.to_owned(),
        new_value: written.new_value,
        prior_value: written.prior_value,
        data_index: None,
        content_index: None,
        owner: Some(owner),
        macrostep: state.counters.macrostep,
        microstep: state.counters.microstep,
        round: state.counters.round,
    }
}

/// `for state in configuration: for inv in state.invoke: if inv.invokeid ==
/// externalEvent.invokeid: applyFinalize(inv, externalEvent); if
/// inv.autoforward: send(inv.id, externalEvent)`, run after `_event` is set
/// and before selection. The configuration is walked in document order and
/// each state's invocations in document order; one that is not live is
/// passed over. A live invocation that matches and forwards does both, its
/// `<finalize>` first.
pub(crate) fn finalize_autoforward(
    state: &mut MachineState,
    event: &Event,
    effects: &mut Vec<Effect>,
) {
    if state.active_invocations.is_empty() {
        return;
    }
    let machine = state.machine.clone();
    let configuration: Vec<usize> = state.configuration.iter().copied().collect();
    for index in configuration {
        let Some(node) = machine.chart().states.get(index) else {
            continue;
        };
        for invoke in &node.invoke {
            let Some(invoke_id) = state.active_invocations.get(&key(index, invoke)).cloned() else {
                continue;
            };
            if event.invokeid.as_deref() == Some(invoke_id.as_str()) {
                finalize(state, index, invoke, event, effects);
            }
            if invoke.autoforward {
                effects.push(Effect::Autoforward {
                    invoke_id,
                    state_index: index,
                    event: event.value(),
                    macrostep: state.counters.macrostep,
                    microstep: state.counters.microstep,
                    round: state.counters.round,
                });
            }
        }
    }
}

/// `applyFinalize`: no `<finalize>` does nothing; an empty one writes the
/// returned values back ([`auto_assign`]); any other runs as a block owned
/// by [`BlockOwner::Finalize`].
fn finalize(
    state: &mut MachineState,
    index: usize,
    invoke: &Invoke,
    event: &Event,
    effects: &mut Vec<Effect>,
) {
    let Some(block) = &invoke.finalize else {
        return;
    };
    let owner = BlockOwner::Finalize {
        state: index,
        invoke: invoke.index,
    };
    if block.content.is_empty() {
        auto_assign(state, invoke, &event.data, owner, effects);
    } else {
        state.run_block(owner, &block.content, effects);
    }
}

/// An empty `<finalize>`: each `namelist` entry and each `<param
/// location>` whose name the event's data, a map, carries is written as by
/// `<assign>` with that value, each write answering an
/// [`Effect::DatamodelChange`] in the order written. A write that fails
/// raises `error.execution` and leaves the other writes standing.
fn auto_assign(
    state: &mut MachineState,
    invoke: &Invoke,
    data: &Value,
    owner: BlockOwner,
    effects: &mut Vec<Effect>,
) {
    let Value::Map(data) = data else {
        return;
    };
    for param in invoke.namelist.iter().chain(&invoke.params) {
        let (ParamKind::Location, Deferred::Ready(Expr::Compiled { source, .. })) =
            (param.kind, &param.expr)
        else {
            continue;
        };
        let Some(value) = data.get(&Key::from(param.name.as_str())) else {
            continue;
        };
        match write_location(&state.datamodel, source, value.clone()) {
            Ok(written) => {
                state.datamodel = written.data.clone();
                effects.push(change(state, source, written, owner));
            }
            Err(failure) => state.raise_failure(&failure),
        }
    }
}

/// `for inv in s.invoke: cancelInvoke(inv)`: an [`Effect::CancelInvoke`]
/// per live invocation of the state at `index`, in document order, each no
/// longer live after it. Run after the state's `<onexit>` blocks and before
/// it leaves the configuration.
pub(crate) fn cancel_invocations(
    state: &mut MachineState,
    index: usize,
    effects: &mut Vec<Effect>,
) {
    let machine = state.machine.clone();
    let Some(node) = machine.chart().states.get(index) else {
        return;
    };
    for invoke in &node.invoke {
        if let Some(invoke_id) = state.active_invocations.remove(&key(index, invoke)) {
            effects.push(Effect::CancelInvoke {
                invoke_id,
                state_index: index,
                macrostep: state.counters.macrostep,
                microstep: state.counters.microstep,
                round: state.counters.round,
            });
        }
    }
}
