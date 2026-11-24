//! The in-memory driver: what a host calls to execute a chart.
//!
//! The interpreter takes one external event at a time and answers its
//! effects as data; the driver is the small loop around it that a session
//! process is in the reference (`Statifier.Session` and
//! `Statifier.Session.Effects` in statifier-ex), with no process, no thread,
//! no real clock and no I/O. It keeps the external queue, routes every
//! `<send>` of the SCXML Event I/O Processor as the reference's session
//! routes it, keeps each delayed one as a pending timer on a virtual clock,
//! and answers every send of a type the host registered as an effect for
//! the host to deliver: it never calls host code.
//!
//! Every call takes a [`State`] by reference and answers a new one with the
//! effects the call produced, in the order they were produced ([`Moved`]);
//! the state it was given is unchanged.
//!
//! Routing, as the reference's session routes a send of the SCXML Event I/O
//! Processor: no target, and the session's own `#_scxml_` address, join the
//! session's external queue; `#_internal` is delivered onto the internal
//! queue and the chart runs to a stable configuration again, after the
//! effects of the batch that sent it, as the reference defers it. A driver
//! session has no parent and starts no invocation, so `#_parent`, another
//! session's address and an invocation's name are unreachable: the
//! interpreter refuses an immediate send to one with `error.communication`,
//! and a delayed one raises the same event when its timer fires.
//!
//! The virtual clock starts at 0. A delayed send becomes a pending timer due
//! at the clock's time plus its delay; [`State::advance`] moves the clock and
//! fires every timer due by the new time, earliest first and, at one due
//! time, in the order they were scheduled, each run to completion before the
//! next. A `<cancel>` removes every pending timer under its send id. When the
//! chart stops, its pending timers and its queued external events are
//! discarded.
//!
//! A macrostep that spends its round budget halts the driver as the
//! reference halts its session: the chart stays where the macrostep left it
//! and is never run again. Later external events are queued and not taken.
//! A pending timer still fires: a send to the session itself joins the
//! external queue, untaken, and a send to the internal queue or to a target
//! the driver cannot reach is dropped, with nothing run and nothing raised,
//! as the reference's halted session drops a fired delayed send.
//!
//! One call also spends the budget on three counts the reference's
//! session, which never returns, has no need of, each kept at one virtual
//! time and started over whenever [`State::advance`] moves the clock on:
//! the events it takes that an immediate send to the session itself queued
//! in the same call, the immediate sends it delivers onto the internal
//! queue, and the timers it fires that were scheduled at that same virtual
//! time in the same call. A call that would count one more than the budget
//! on any of them halts the same way, answering [`Effect::BudgetExhausted`].
//! Under a bounded budget every call therefore returns, a chart that keeps
//! rescheduling a timer due at once included, while timers with a delay, a
//! chain or a fan-out, are bounded by the duration advanced and never halt
//! on these counts, however the host splits time. Under
//! [`RoundBudget::Unbounded`] no count ever halts the driver, and such a
//! call may never return. A timer scheduled by an earlier call, or at an
//! earlier virtual time, is never counted.

#[cfg(test)]
mod tests;

use crate::datamodel::system_variables::{EventType, SCXML_EVENT_PROCESSOR, scxml_location};
use crate::effect::Effect;
use crate::interpreter::{self, Event, MachineState, Route, Routes, StartOptions};
use crate::machine::Machine;
use predicator_rs::{Map, Value};
use std::collections::{BTreeSet, VecDeque};
use std::fmt;
use std::num::NonZeroU64;

/// The round budget when the host sets none, the reference's
/// (`max_macrostep_rounds` in `Statifier.MachineState`).
const DEFAULT_ROUNDS: u64 = 10_000;

/// The most rounds one macrostep may take.
///
/// The interpreter's loop runs a macrostep until its configuration is
/// stable; a chart whose eventless transitions or raised events never let
/// it settle would run forever, so each macrostep spends at most this many
/// rounds and then stops where the last round left it, answering
/// [`Effect::BudgetExhausted`].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RoundBudget {
    /// At most this many rounds.
    Rounds(NonZeroU64),
    /// No bound: a macrostep that never reaches a stable configuration
    /// never returns, as the specification's loop never does, and nor does
    /// the call that started it.
    Unbounded,
}

impl Default for RoundBudget {
    /// 10000 rounds, the reference's budget.
    fn default() -> Self {
        RoundBudget::Rounds(NonZeroU64::new(DEFAULT_ROUNDS).unwrap_or(NonZeroU64::MIN))
    }
}

impl RoundBudget {
    fn interpreter(self) -> interpreter::RoundBudget {
        match self {
            RoundBudget::Rounds(rounds) => interpreter::RoundBudget::Rounds(rounds.get()),
            RoundBudget::Unbounded => interpreter::RoundBudget::Unbounded,
        }
    }

    /// Whether `count` is within the budget: always, for no bound.
    fn admits(self, count: u64) -> bool {
        match self {
            RoundBudget::Rounds(rounds) => count <= rounds.get(),
            RoundBudget::Unbounded => true,
        }
    }

    /// The budget as [`Effect::BudgetExhausted`] carries it; only a bounded
    /// budget is ever spent.
    fn spent(self) -> u64 {
        match self {
            RoundBudget::Rounds(rounds) => rounds.get(),
            RoundBudget::Unbounded => u64::MAX,
        }
    }
}

/// What [`Machine::start`] takes.
///
/// The struct is `#[non_exhaustive]`: a host builds one with
/// [`Options::new`] and sets the fields it wants.
///
/// ```
/// use statifier::{Map, Options, Value};
///
/// let mut options = Options::new("session-1");
/// let mut datamodel = Map::new();
/// datamodel.insert("limit", Value::from(3));
/// options.datamodel = datamodel;
/// options.send_types = ["x-example"].into_iter().map(String::from).collect();
/// # let _ = options;
/// ```
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    /// The session's id, minted by the host: `_sessionid` reads it, and
    /// `#_scxml_` followed by it is the session's own address.
    pub session_id: String,
    /// Starting datamodel values, under the system variables. A root
    /// `<data>` whose id is here keeps the host's value.
    pub datamodel: Map,
    /// The send types the host delivers itself. A `<send>` of one of them
    /// is answered as an effect and never routed or scheduled here, delayed
    /// or not; `_ioprocessors` holds an empty entry for each. A built-in
    /// spelling, `scxml` or the SCXML Event I/O Processor's URI, is refused.
    pub send_types: BTreeSet<String>,
    /// The round budget of each macrostep, and of the two counts one call
    /// spends it on.
    pub round_budget: RoundBudget,
    /// Whether the chart answers trace effects. This version emits none,
    /// whatever the flag says.
    pub trace: bool,
}

impl Options {
    /// The options for a session with this id, and every other field at its
    /// default: no starting values, no registered send type, the default
    /// round budget and no trace.
    pub fn new(session_id: impl Into<String>) -> Self {
        Options {
            session_id: session_id.into(),
            datamodel: Map::new(),
            send_types: BTreeSet::new(),
            round_budget: RoundBudget::default(),
            trace: false,
        }
    }
}

/// Why a driver call was refused.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The chart has stopped: it takes no more events.
    NotRunning,
    /// [`State::advance`] was given a duration that moves the virtual clock
    /// past its largest value.
    #[non_exhaustive]
    InvalidDuration {
        /// The duration given, in milliseconds.
        ms: u64,
    },
    /// [`Options::send_types`] names a built-in type, which is never handed
    /// to a host.
    #[non_exhaustive]
    BuiltInSendTypes {
        /// The built-in spellings named, sorted.
        types: Vec<String>,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotRunning => formatter.write_str("the chart has stopped"),
            Error::InvalidDuration { ms } => write!(
                formatter,
                "a duration of {ms} ms moves the clock past its largest value"
            ),
            Error::BuiltInSendTypes { types } => write!(
                formatter,
                "built-in send types cannot be registered: {}",
                types.join(", ")
            ),
        }
    }
}

impl std::error::Error for Error {}

/// What a call that moves a chart answers: the new state, and the effects
/// the call produced, in the order they were produced.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct Moved {
    /// The new state.
    pub state: State,
    /// The effects, in order.
    pub effects: Vec<Effect>,
}

/// What a stopped chart answers ([`State::done`]).
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub struct Done {
    /// The top-level final state's `<donedata>`: [`Value::Undefined`] when
    /// it has none, or when evaluating it failed.
    pub donedata: Value,
    /// The ids of the states that were active when the chart stopped, root
    /// excluded, sorted; a state with no id is left out.
    pub configuration: Vec<String>,
}

/// A delayed send waiting on the virtual clock.
#[derive(Debug, Clone)]
struct Timer {
    /// The send id a `<cancel>` names it by.
    send_id: String,
    /// The virtual time it fires at, in milliseconds.
    due_ms: u64,
    /// The order it was scheduled in: two timers due together fire in this
    /// order.
    sequence: u64,
    /// The send it delivers.
    send: Delivery,
}

/// What the driver keeps of a send to route it: the target it names and
/// the event a delivery carries.
#[derive(Debug, Clone)]
struct Delivery {
    target: Option<Value>,
    event: Event,
}

/// A chart in execution: an immutable value, as cheap to clone as its
/// datamodel. Each call that moves it answers a new state and leaves this
/// one unchanged.
///
/// ```
/// use statifier::{Options, Value};
///
/// let machine = statifier::compile(
///     r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="a">
///          <state id="a"><transition event="e.go" target="b"/></state>
///          <final id="b"/>
///        </scxml>"#,
/// )
/// .map_err(|refusals| format!("{refusals:?}"))?;
/// let started = machine.start(Options::new("s-1"))?;
/// assert_eq!(started.state.configuration(), ["a"]);
///
/// let moved = started.state.step("e.go", Value::Undefined)?;
/// assert_eq!(started.state.configuration(), ["a"]);
/// assert_eq!(moved.state.done().map(|done| done.configuration.clone()), Some(vec!["b".to_owned()]));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone)]
pub struct State {
    core: MachineState,
    session_id: String,
    round_budget: RoundBudget,
    now_ms: u64,
    timers: Vec<Timer>,
    sequence: u64,
    external_queue: VecDeque<Event>,
    halted: bool,
    done: Option<Done>,
    /// The ids of the atomic states that were active when the chart
    /// stopped, sorted; empty while it runs.
    stopped_leaves: Vec<String>,
}

impl Machine {
    /// Starts the chart: its datamodel bound, its `<scxml>` scripts run, its
    /// initial states entered, and the whole run to a stable configuration,
    /// with every event it sent itself taken.
    ///
    /// Refused with [`Error::BuiltInSendTypes`] when `options` registers a
    /// built-in send type.
    pub fn start(&self, options: Options) -> Result<Moved, Error> {
        let built_in: Vec<String> = options
            .send_types
            .iter()
            .filter(|send_type| is_built_in(send_type))
            .cloned()
            .collect();
        if !built_in.is_empty() {
            return Err(Error::BuiltInSendTypes { types: built_in });
        }
        let ioprocessors = options
            .send_types
            .iter()
            .map(|send_type| (send_type.as_str(), Value::Map(Map::new())))
            .collect();
        let routes = Routes {
            sessions: BTreeSet::from([options.session_id.clone()]),
            ..Routes::default()
        };
        let started = interpreter::initialize(
            self,
            StartOptions {
                session_id: options.session_id.clone(),
                datamodel: options.datamodel,
                send_types: options.send_types,
                ioprocessors,
                routes: Some(routes),
                max_macrostep_rounds: options.round_budget.interpreter(),
            },
        );
        let mut call = Call::open(State {
            core: started.state,
            session_id: options.session_id,
            round_budget: options.round_budget,
            now_ms: 0,
            timers: Vec::new(),
            sequence: 0,
            external_queue: VecDeque::new(),
            halted: false,
            done: None,
            stopped_leaves: Vec::new(),
        });
        call.perform(started.effects);
        call.drain();
        Ok(call.close())
    }
}

impl State {
    /// Sends the chart one external event named `name`, carrying `data`
    /// ([`Value::Undefined`] for none), and runs it, and every event it
    /// sends itself, to a stable configuration. A driver that a spent
    /// budget halted queues the event without taking it.
    ///
    /// Refused with [`Error::NotRunning`] once the chart has stopped.
    pub fn step(&self, name: &str, data: Value) -> Result<Moved, Error> {
        if !self.core.running {
            return Err(Error::NotRunning);
        }
        let mut call = Call::open(self.clone());
        call.external.push_back(Queued {
            event: Event {
                name: name.to_owned(),
                event_type: EventType::External,
                sendid: None,
                origin: None,
                origintype: None,
                invokeid: None,
                data,
            },
            counted: false,
        });
        call.drain();
        Ok(call.close())
    }

    /// Moves the virtual clock forward `ms` milliseconds, firing every
    /// pending timer due by then, earliest first and, at one due time, in
    /// the order they were scheduled, each run to completion before the
    /// next. A stopped chart has no timer, so only its clock moves.
    ///
    /// Refused with [`Error::InvalidDuration`] when the clock would pass its
    /// largest value.
    pub fn advance(&self, ms: u64) -> Result<Moved, Error> {
        let until = self
            .now_ms
            .checked_add(ms)
            .ok_or(Error::InvalidDuration { ms })?;
        let mut call = Call::open(self.clone());
        while let Some(timer) = call.next_due(until) {
            if timer.due_ms > call.state.now_ms {
                call.next_instant();
            }
            call.state.now_ms = timer.due_ms;
            call.count_fire(&timer);
            let effects = call.route(timer.send, false);
            call.perform(effects);
            call.drain();
        }
        call.state.now_ms = until;
        Ok(call.close())
    }

    /// The ids of the active states, root excluded, sorted; a state with no
    /// id is left out. A stopped chart has none.
    pub fn configuration(&self) -> Vec<String> {
        ids(&self.core, self.core.configuration.iter().copied())
    }

    /// The ids of the active leaf states, the active states with no child
    /// state, sorted; a state with no id is left out. It is the reference's
    /// `Statifier.active_leaf_states/1`, the set its conformance harness
    /// compares with a case's expected configuration. A final state is a
    /// leaf, and a history state is never active.
    ///
    /// A stopped chart answers the leaves of the configuration it stopped
    /// in, which [`Done::configuration`] carries whole, where
    /// [`State::configuration`] answers none.
    ///
    /// ```
    /// use statifier::{Options, Value};
    ///
    /// let machine = statifier::compile(
    ///     r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="p">
    ///          <parallel id="p">
    ///            <state id="a"><transition event="e.go" target="f"/></state>
    ///            <state id="b"/>
    ///          </parallel>
    ///          <final id="f"/>
    ///        </scxml>"#,
    /// )
    /// .map_err(|refusals| format!("{refusals:?}"))?;
    /// let started = machine.start(Options::new("s-1"))?;
    /// assert_eq!(started.state.configuration(), ["a", "b", "p"]);
    /// assert_eq!(started.state.active_leaf_states(), ["a", "b"]);
    ///
    /// let stopped = started.state.step("e.go", Value::Undefined)?;
    /// assert!(stopped.state.configuration().is_empty());
    /// assert_eq!(stopped.state.active_leaf_states(), ["f"]);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn active_leaf_states(&self) -> Vec<String> {
        if self.done.is_some() {
            return self.stopped_leaves.clone();
        }
        leaf_ids(&self.core, self.core.configuration.iter().copied())
    }

    /// `None` while the chart runs; once it has stopped, its donedata and
    /// the configuration it stopped in.
    pub fn done(&self) -> Option<&Done> {
        self.done.as_ref()
    }

    /// The virtual clock, in milliseconds since the chart started.
    pub fn now_ms(&self) -> u64 {
        self.now_ms
    }

    /// The virtual time the earliest pending timer fires at, `None` when no
    /// timer is pending.
    pub fn next_due_ms(&self) -> Option<u64> {
        self.timers.iter().map(|timer| timer.due_ms).min()
    }
}

/// Whether `send_type` is a built-in spelling of the SCXML Event I/O
/// Processor, which a host never registers.
fn is_built_in(send_type: &str) -> bool {
    send_type == "scxml" || send_type == SCXML_EVENT_PROCESSOR
}

/// The ids of `indexes`, sorted; a state with no id, the root among them,
/// is left out.
fn ids(core: &MachineState, indexes: impl Iterator<Item = usize>) -> Vec<String> {
    let chart = core.chart();
    let mut ids: Vec<String> = indexes
        .filter_map(|index| chart.states.get(index).and_then(|state| state.id.clone()))
        .collect();
    ids.sort();
    ids
}

/// The ids of the atomic states among `indexes`, sorted; a state with no id
/// is left out.
fn leaf_ids(core: &MachineState, indexes: impl Iterator<Item = usize>) -> Vec<String> {
    let chart = core.chart();
    ids(
        core,
        indexes.filter(|&index| index < chart.states.len() && chart.is_atomic(index)),
    )
}

/// An external event waiting, and whether this call counts it against the
/// budget when it takes it.
#[derive(Debug)]
struct Queued {
    event: Event,
    counted: bool,
}

/// One call's working form: the state being moved, the effects answered so
/// far, and the call's own counts.
struct Call {
    state: State,
    external: VecDeque<Queued>,
    effects: Vec<Effect>,
    /// Counted events taken in this call at the current virtual time.
    taken: u64,
    /// Immediate internal sends delivered in this call at the current
    /// virtual time.
    sent_internal: u64,
    /// Whether this call has spent its budget: nothing more runs the chart.
    spent: bool,
    /// The sequence of the first timer this call schedules at the current
    /// virtual time: a timer at or past it was scheduled at this time in
    /// this call, as a host that advanced only to this time would see it.
    first_sequence: u64,
    /// Timers scheduled in this call that it fired at the current virtual
    /// time.
    fired: u64,
}

impl Call {
    /// A call over `state`: every event already queued is one an earlier
    /// call queued, and is not counted.
    fn open(mut state: State) -> Self {
        let first_sequence = state.sequence;
        let external = std::mem::take(&mut state.external_queue)
            .into_iter()
            .map(|event| Queued {
                event,
                counted: false,
            })
            .collect();
        Call {
            state,
            external,
            effects: Vec::new(),
            taken: 0,
            sent_internal: 0,
            spent: false,
            first_sequence,
            fired: 0,
        }
    }

    /// The state the call leaves, and its effects.
    fn close(mut self) -> Moved {
        self.state.external_queue = self.external.into_iter().map(|q| q.event).collect();
        Moved {
            state: self.state,
            effects: self.effects,
        }
    }

    /// The earliest timer due by `until`, the first scheduled among equals,
    /// taken off the pending list.
    fn next_due(&mut self, until: u64) -> Option<Timer> {
        let position = self
            .state
            .timers
            .iter()
            .enumerate()
            .filter(|(_, timer)| timer.due_ms <= until)
            .min_by_key(|(_, timer)| (timer.due_ms, timer.sequence))
            .map(|(position, _)| position)?;
        Some(self.state.timers.remove(position))
    }

    /// Takes the external queue one event per macrostep while the chart
    /// runs and the driver is not halted. A counted event the budget has no
    /// room for is left queued, and the driver halts.
    fn drain(&mut self) {
        while self.state.core.running && !self.state.halted {
            let Some(queued) = self.external.pop_front() else {
                return;
            };
            if queued.counted {
                let taken = self.taken.saturating_add(1);
                if !self.state.round_budget.admits(taken) {
                    self.external.push_front(queued);
                    self.halt();
                    return;
                }
                self.taken = taken;
            }
            if let Ok(stepped) = interpreter::handle_event(&self.state.core, &queued.event) {
                self.state.core = stepped.state;
                self.perform(stepped.effects);
            }
        }
    }

    /// The clock has moved on to a later virtual time: the call's three
    /// counts start over, so each bounds only what happens at one time, and
    /// a chain of timers with a delay is bounded by the duration advanced.
    /// A timer scheduled at an earlier time is no longer counted when it
    /// fires, as a call that started at this time would not count it.
    fn next_instant(&mut self) {
        self.first_sequence = self.state.sequence;
        self.taken = 0;
        self.sent_internal = 0;
        self.fired = 0;
    }

    /// Counts a timer scheduled at the current virtual time in this call
    /// against the budget as it fires. Halts the driver when the budget has
    /// no room for it; a halted driver runs nothing, so its fires are not
    /// counted.
    fn count_fire(&mut self, timer: &Timer) {
        if self.state.halted || timer.sequence < self.first_sequence {
            return;
        }
        self.fired = self.fired.saturating_add(1);
        if !self.state.round_budget.admits(self.fired) {
            self.halt();
        }
    }

    /// Halts the driver for the call's budget, answering the spent-budget
    /// effect unless a macrostep already halted it.
    fn halt(&mut self) {
        self.spent = true;
        if self.state.halted {
            return;
        }
        let budget = self.state.round_budget.spent();
        self.perform(vec![interpreter::budget_exhausted(
            &self.state.core,
            budget,
        )]);
    }

    /// Acts on a batch of effects in order, each answered as it is acted
    /// on. A delivery onto the internal queue runs the chart at once, and
    /// the effects that run answers are acted on after the batch, as the
    /// reference defers them.
    fn perform(&mut self, effects: Vec<Effect>) {
        let mut batch = effects;
        while !batch.is_empty() {
            let mut deferred = Vec::new();
            for effect in batch {
                deferred.extend(self.perform_one(&effect));
                self.effects.push(effect);
            }
            batch = deferred;
        }
    }

    fn perform_one(&mut self, effect: &Effect) -> Vec<Effect> {
        match effect {
            Effect::Send {
                event,
                target,
                send_type,
                data,
                send_id,
                id_from_author,
                ..
            } if send_type.is_none() || !self.registered(send_type.as_ref()) => {
                let delivery = self.delivery(event, target, data, send_id, *id_from_author);
                self.route(delivery, true)
            }
            Effect::SendDelayed {
                event,
                target,
                send_type,
                data,
                send_id,
                id_from_author,
                delay_ms,
                ..
            } if send_type.is_none() || !self.registered(send_type.as_ref()) => {
                let send = self.delivery(event, target, data, send_id, *id_from_author);
                let delay = u64::try_from(*delay_ms).unwrap_or(0);
                self.state.timers.push(Timer {
                    send_id: send_id.clone(),
                    due_ms: self.state.now_ms.saturating_add(delay),
                    sequence: self.state.sequence,
                    send,
                });
                self.state.sequence = self.state.sequence.saturating_add(1);
                Vec::new()
            }
            Effect::Cancel { send_id, .. } => {
                if let Value::String(send_id) = send_id {
                    self.state.timers.retain(|timer| timer.send_id != *send_id);
                }
                Vec::new()
            }
            Effect::Done {
                donedata,
                configuration,
                ..
            } => {
                self.state.done = Some(Done {
                    donedata: donedata.clone().unwrap_or(Value::Undefined),
                    configuration: ids(&self.state.core, configuration.iter().copied()),
                });
                self.state.stopped_leaves =
                    leaf_ids(&self.state.core, configuration.iter().copied());
                self.state.timers.clear();
                self.external.clear();
                Vec::new()
            }
            Effect::BudgetExhausted { .. } => {
                self.state.halted = true;
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// Whether `send_type` names a type the host registered.
    fn registered(&self, send_type: Option<&Value>) -> bool {
        matches!(send_type, Some(Value::String(send_type)) if self.state.core.send_types.contains(send_type))
    }

    /// The event a delivered send carries, the reference's
    /// `Statifier.Send.Event.build/3`: from the session's own address through
    /// the SCXML Event I/O Processor, with the send id only when the
    /// document named it.
    fn delivery(
        &self,
        event: &Option<Value>,
        target: &Option<Value>,
        data: &Value,
        send_id: &str,
        id_from_author: bool,
    ) -> Delivery {
        let name = match event {
            Some(Value::String(name)) => name.clone(),
            _ => String::new(),
        };
        Delivery {
            target: target.clone(),
            event: Event {
                name,
                event_type: EventType::External,
                sendid: id_from_author.then(|| send_id.to_owned()),
                origin: Some(scxml_location(&self.state.session_id)),
                origintype: Some(SCXML_EVENT_PROCESSOR.to_owned()),
                invokeid: None,
                data: data.clone(),
            },
        }
    }

    /// Routes a send of the SCXML Event I/O Processor; `immediate` when it
    /// was sent with no delay, which counts what it queues for the session
    /// itself and what it delivers onto the internal queue.
    fn route(&mut self, send: Delivery, immediate: bool) -> Vec<Effect> {
        match interpreter::parse_target(send.target.as_ref()) {
            Route::Own => self.enqueue_own(send.event, immediate),
            Route::Session(id) if id == self.state.session_id => {
                self.enqueue_own(send.event, immediate)
            }
            Route::Internal => {
                let mut event = send.event;
                event.event_type = EventType::Internal;
                event.origin = None;
                event.origintype = None;
                if !immediate {
                    return self.fired_internal(event);
                }
                self.sent_internal = self.sent_internal.saturating_add(1);
                if self.state.round_budget.admits(self.sent_internal) {
                    self.deliver_internal(event)
                } else {
                    self.spend(event)
                }
            }
            _ => {
                let mut event = Event::platform("error.communication", Value::Undefined);
                event.sendid = send.event.sendid;
                self.fired_internal(event)
            }
        }
    }

    /// A fired timer's delivery onto the internal queue, its own event or
    /// the `error.communication` an unreachable target raises: dropped by a
    /// halted driver, with nothing run and nothing raised, as the
    /// reference's halted session drops a fired delayed send; delivered
    /// otherwise.
    fn fired_internal(&mut self, event: Event) -> Vec<Effect> {
        if self.state.halted {
            return Vec::new();
        }
        self.deliver_internal(event)
    }

    /// An event for the session itself joins its external queue, counted
    /// when an immediate send queued it.
    fn enqueue_own(&mut self, event: Event, counted: bool) -> Vec<Effect> {
        self.external.push_back(Queued { event, counted });
        Vec::new()
    }

    /// The reference's `Interpreter.deliver_internal/5`: the event joins the
    /// internal queue and the chart runs to a stable configuration,
    /// answering the effects that run left. A stopped chart takes nothing,
    /// and once the call has spent its budget the event waits there unrun.
    fn deliver_internal(&mut self, event: Event) -> Vec<Effect> {
        if !self.state.core.running {
            return Vec::new();
        }
        if self.spent {
            self.state.core.enqueue(event);
            return Vec::new();
        }
        match interpreter::deliver_internal(&self.state.core, event) {
            Ok(stepped) => {
                self.state.core = stepped.state;
                stepped.effects
            }
            Err(_) => Vec::new(),
        }
    }

    /// An internal send past the call's budget: queued on the internal
    /// queue unrun, and the driver halted, answering the spent-budget effect
    /// when it was not halted already.
    fn spend(&mut self, event: Event) -> Vec<Effect> {
        if !self.state.core.running {
            return Vec::new();
        }
        self.spent = true;
        self.state.core.enqueue(event);
        if self.state.halted {
            return Vec::new();
        }
        self.state.halted = true;
        vec![interpreter::budget_exhausted(
            &self.state.core,
            self.state.round_budget.spent(),
        )]
    }
}
