//! The invocations a session holds and the child sessions behind them, as
//! the reference's session runs a built-in `scxml` invocation
//! (`Statifier.Session`, `Statifier.Session.Invocations`,
//! `Statifier.Invoke.Source` and `Statifier.Invoke.Answer` in statifier-ex),
//! with a child's [`State`] held in its parent's [`Invocation`] in place of
//! a process. The driver's module documentation says what each does.
//!
//! A child runs inside a call of its own ([`Call::within`]): it starts on
//! its parent's clock and timer sequence, and its state, the timer sequence
//! it leaves, its effects and what it sent its parent go back to the parent
//! once it has run. The count of timers fired at one virtual time is the
//! whole tree's, so a child that keeps rescheduling a timer due at once
//! halts as a session does.

#[cfg(test)]
mod tests;

use super::{Call, Invocation, Mail, RoundBudget, State};
use crate::datamodel::system_variables::{EventType, SCXML_EVENT_PROCESSOR, scxml_location};
use crate::effect::Effect;
use crate::interpreter::{self, Event, Routes, StartOptions, is_scxml_type};
use crate::machine::Machine;
use predicator_rs::{Key, Map, Value};
use std::collections::BTreeSet;

impl Call {
    /// The child under `invoke_id` run by `run` in a call of its own, then
    /// its state, timer sequence, effects and mail taken back. Nothing runs
    /// for an id that names no live invocation.
    pub(super) fn within(&mut self, invoke_id: &str, run: impl FnOnce(&mut Call)) {
        let Some(mut invocation) = self.state.invocations.remove(invoke_id) else {
            return;
        };
        let mut child = invocation.child;
        child.now_ms = self.state.now_ms;
        child.sequence = self.state.sequence;
        let mut call = Call::open(child);
        call.first_sequence = self.first_sequence;
        call.fired = self.fired;
        run(&mut call);
        self.fired = call.fired;
        let (child, effects, outbox) = call.finish();
        self.state.sequence = child.sequence;
        self.effects.extend(effects);
        invocation.child = child;
        self.state
            .invocations
            .insert(invoke_id.to_owned(), invocation);
        self.mailbox.extend(outbox);
    }

    /// An [`Effect::Invoke`] acted on: a type other than the SCXML type
    /// raises `error.execution`; content that is not markup, or does not
    /// compile, raises `error.communication`; otherwise the child is
    /// started, recorded live under its invoke id and run to a stable
    /// configuration. Answers the effects the raised error's run left.
    pub(super) fn start_child(
        &mut self,
        invoke_id: &str,
        invoke_type: Option<&Value>,
        params: &Value,
        content: Option<&Value>,
    ) -> Vec<Effect> {
        if !is_scxml_type(invoke_type) {
            return self.deliver_internal(Event::platform("error.execution", Value::Undefined));
        }
        let machine = match content {
            Some(Value::String(source)) => crate::compile_inline(source),
            _ => None,
        };
        let Some(machine) = machine else {
            return self.deliver_internal(Event::platform("error.communication", Value::Undefined));
        };
        let session_id = format!("{}.{invoke_id}", self.state.session_id);
        let send_types = self.state.core.send_types.clone();
        let ioprocessors = send_types
            .iter()
            .map(|send_type| (send_type.as_str(), Value::Map(Map::new())))
            .collect();
        let started = interpreter::initialize(
            &machine,
            StartOptions {
                session_id: session_id.clone(),
                datamodel: seed(params, &machine),
                send_types,
                ioprocessors,
                routes: Some(Routes {
                    sessions: BTreeSet::from([session_id.clone()]),
                    parent: true,
                    invokes: BTreeSet::new(),
                }),
                max_macrostep_rounds: RoundBudget::default().interpreter(),
            },
        );
        let mut child = State::fresh(started.state, session_id, RoundBudget::default(), false);
        child.invoked_as = Some(invoke_id.to_owned());
        self.state.invocations.insert(
            invoke_id.to_owned(),
            Invocation {
                completed: false,
                child,
            },
        );
        self.within(invoke_id, |call| {
            call.perform(started.effects);
            call.drain();
        });
        Vec::new()
    }

    /// An [`Effect::CancelInvoke`] acted on: the invocation retired, then
    /// its child stopped, its active states' `<onexit>` handlers run, its
    /// effects answered and what it sent its parent discarded. An
    /// invocation whose child already said it completed is left for its
    /// done event, and an id that names nothing live is a no-op.
    pub(super) fn cancel_child(&mut self, invoke_id: &str) {
        match self.state.invocations.get(invoke_id) {
            Some(invocation) if !invocation.completed => {}
            _ => return,
        }
        let Some(invocation) = self.state.invocations.remove(invoke_id) else {
            return;
        };
        let mut child = invocation.child;
        if !child.core.running {
            return;
        }
        child.cancelled = true;
        child.now_ms = self.state.now_ms;
        child.sequence = self.state.sequence;
        let Ok(stopped) = interpreter::cancel(&child.core) else {
            return;
        };
        let mut call = Call::open(child);
        call.first_sequence = self.first_sequence;
        call.fired = self.fired;
        call.state.core = stopped.state;
        call.perform(stopped.effects);
        self.fired = call.fired;
        let (child, effects, _) = call.finish();
        self.state.sequence = child.sequence;
        self.effects.extend(effects);
    }

    /// An event for the live child under `invoke_id`: it joins the child's
    /// external queue and the child runs to a stable configuration. Nothing
    /// reaches an invocation that is not live, or a child that has stopped.
    pub(super) fn deliver(&mut self, invoke_id: &str, event: Event) {
        let running = self
            .state
            .invocations
            .get(invoke_id)
            .is_some_and(|invocation| invocation.child.core.running);
        if running {
            self.within(invoke_id, |call| {
                call.enqueue_own(event, false);
                call.drain();
            });
        }
    }

    /// A send to `#_parent`: the event, carrying the invoke id, into the
    /// parent's mailbox, counted there when the send had no delay.
    pub(super) fn post_to_parent(&mut self, mut event: Event, counted: bool) {
        let Some(invoke_id) = self.state.invoked_as.clone() else {
            return;
        };
        event.invokeid = Some(invoke_id.clone());
        self.outbox
            .push((Mail::Event { invoke_id, event }, counted));
    }

    /// A child whose batch stops it tells its parent it completed, ahead of
    /// everything the batch sends; a cancelled child does not.
    pub(super) fn announce(&mut self, batch: &[Effect]) {
        if self.state.cancelled
            || !batch
                .iter()
                .any(|effect| matches!(effect, Effect::Done { .. }))
        {
            return;
        }
        if let Some(invoke_id) = self.state.invoked_as.clone() {
            self.outbox.push((Mail::Completed { invoke_id }, false));
        }
    }

    /// A child that stopped on its own sends its parent its donedata; one
    /// that was cancelled sends nothing.
    pub(super) fn return_done(&mut self, donedata: Value) {
        if self.state.cancelled {
            return;
        }
        if let Some(invoke_id) = self.state.invoked_as.clone() {
            self.outbox.push((
                Mail::Done {
                    invoke_id,
                    donedata,
                },
                false,
            ));
        }
    }

    /// One mailbox entry; one whose invocation is no longer live is
    /// discarded. A completion notice marks the invocation completed; an
    /// event is taken as an external event; and the done event, built here,
    /// retires the invocation once taken.
    pub(super) fn take_mail(&mut self, mail: Mail) {
        let invoke_id = mail.invoke_id().to_owned();
        if !self.state.invocations.contains_key(&invoke_id) {
            return;
        }
        match mail {
            Mail::Completed { .. } => {
                if let Some(invocation) = self.state.invocations.get_mut(&invoke_id) {
                    invocation.completed = true;
                }
            }
            Mail::Event { event, .. } => self.take(event),
            Mail::Done { donedata, .. } => {
                let event = done_event(&self.state.session_id, &invoke_id, donedata);
                self.take(event);
                self.state.invocations.remove(&invoke_id);
            }
        }
    }
}

/// The reference's `Invoke.Answer.done/4`: `done.invoke.` and the invoke
/// id, its data the child's donedata, from the parent's own address.
fn done_event(session_id: &str, invoke_id: &str, donedata: Value) -> Event {
    Event {
        name: format!("done.invoke.{invoke_id}"),
        event_type: EventType::External,
        sendid: None,
        origin: Some(scxml_location(session_id)),
        origintype: Some(SCXML_EVENT_PROCESSOR.to_owned()),
        invokeid: Some(invoke_id.to_owned()),
        data: donedata,
    }
}

/// The reference's `Invocations.seed_datamodel/2`: the params a root
/// `<data>` of the child names, the rest dropped.
fn seed(params: &Value, machine: &Machine) -> Map {
    let mut seeded = Map::new();
    let Value::Map(params) = params else {
        return seeded;
    };
    let chart = machine.chart();
    let root_data = chart
        .states
        .first()
        .map_or(&[][..], |root| root.data.as_slice());
    for &index in root_data {
        let Some(data) = chart.data.get(index) else {
            continue;
        };
        let key = Key::from(data.id.as_str());
        if let Some(value) = params.get(&key) {
            seeded.insert(key, value.clone());
        }
    }
    seeded
}
