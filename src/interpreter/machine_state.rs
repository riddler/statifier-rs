//! The interpreter's position, as the reference keeps it
//! (`Statifier.MachineState` in statifier-ex): every global and loop
//! variable of the specification's pseudocode in one value, so a position
//! between two calls is data and never a stack frame.
//!
//! The loop's entry points take a position by reference and answer a new
//! one, leaving the one they were given as it was. Inside one call the
//! position is threaded as an owned value the call alone holds.
//!
//! Three counters stamp every effect and every raised event. `macrostep`
//! advances once when the chart starts and once per external event, and
//! starts the other two over; `microstep` advances once per microstep that
//! exits and enters, the initial entry being microstep 1; `round` advances
//! once per pass of the macrostep's loop, a pass that moves nothing
//! included. Each has one writer: [`MachineState::begin_macrostep`],
//! [`MachineState::begin_microstep`] and [`MachineState::begin_round`].

use super::content::{Counters, Raised, Routes, Sends, Site, execute_block};
use super::{History, Snapshot};
use crate::datamodel::Failure;
use crate::datamodel::system_variables::{self, EventFields, EventType};
use crate::effect::{BlockOwner, Effect};
use crate::machine::{Chart, Machine};
use predicator_rs::{Map, Value};
use std::collections::{BTreeSet, VecDeque};

/// The round budget when the host sets none, the reference's
/// (`max_macrostep_rounds` in `Statifier.MachineState`).
pub(crate) const MAX_MACROSTEP_ROUNDS: u64 = 10_000;

/// The most rounds one macrostep may take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RoundBudget {
    /// At most this many rounds.
    Rounds(u64),
    /// No bound: a macrostep that never reaches a stable configuration
    /// never returns, as the specification's loop never does.
    Unbounded,
}

impl Default for RoundBudget {
    fn default() -> Self {
        RoundBudget::Rounds(MAX_MACROSTEP_ROUNDS)
    }
}

/// An event as the loop queues and takes it: the fields `_event` reads
/// (spec 5.10.1).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Event {
    /// The event's name.
    pub(crate) name: String,
    /// Its type.
    pub(crate) event_type: EventType,
    /// The send id, when a `<send>` the document named sent it.
    pub(crate) sendid: Option<String>,
    /// Where it came from, when known.
    pub(crate) origin: Option<String>,
    /// The type of `origin`, when known.
    pub(crate) origintype: Option<String>,
    /// The invoke id, when an invoked session sent it.
    pub(crate) invokeid: Option<String>,
    /// Its data, [`Value::Undefined`] for none.
    pub(crate) data: Value,
}

impl Event {
    /// An event raised by the chart itself (`<raise>`): no data.
    pub(crate) fn internal(name: &str) -> Self {
        Event::bare(name, EventType::Internal, Value::Undefined)
    }

    /// A platform event (a `done.state.*`, an `error.*`) carrying `data`.
    pub(crate) fn platform(name: &str, data: Value) -> Self {
        Event::bare(name, EventType::Platform, data)
    }

    /// The platform event a failure raises: its name, its send id when it
    /// is a refused `<send>`'s, and the failure as its data.
    pub(crate) fn from_failure(failure: &Failure) -> Self {
        let mut event = Event::platform(failure.event_name(), failure.data());
        if let Failure::SendRejected { send_id, .. } = failure {
            event.sendid = Some(send_id.clone());
        }
        event
    }

    fn bare(name: &str, event_type: EventType, data: Value) -> Self {
        Event {
            name: name.to_owned(),
            event_type,
            sendid: None,
            origin: None,
            origintype: None,
            invokeid: None,
            data,
        }
    }

    /// The event as `_event` reads it.
    pub(crate) fn value(&self) -> Value {
        system_variables::event(&EventFields {
            name: &self.name,
            event_type: self.event_type,
            sendid: self.sendid.as_deref(),
            origin: self.origin.as_deref(),
            origintype: self.origintype.as_deref(),
            invokeid: self.invokeid.as_deref(),
            data: self.data.clone(),
        })
    }
}

/// Whether the chart has finished stopping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Status {
    /// Not yet: the chart runs, or is stopping.
    Running,
    /// `exitInterpreter` has finished: every state has exited.
    Done,
}

/// What starting a chart takes.
#[derive(Debug, Clone, Default)]
pub(crate) struct StartOptions {
    /// The session's id, minted by the host: `_sessionid` reads it.
    pub(crate) session_id: String,
    /// The host's values, under the system variables. A root `<data>` whose
    /// id the host supplied keeps the host's value.
    pub(crate) datamodel: Map,
    /// The send types the host registered a processor for.
    pub(crate) send_types: BTreeSet<String>,
    /// The `_ioprocessors` entry of each registered send type, by type.
    pub(crate) ioprocessors: Map,
    /// The routes the driver declares reachable, `None` for none declared.
    pub(crate) routes: Option<Routes>,
    /// The round budget of each macrostep.
    pub(crate) max_macrostep_rounds: RoundBudget,
}

/// The interpreter's position.
#[derive(Debug, Clone)]
pub(crate) struct MachineState {
    /// The chart.
    pub(crate) machine: Machine,
    /// The indexes of the active states, ancestors included.
    pub(crate) configuration: BTreeSet<usize>,
    /// The recorded history.
    pub(crate) history: History,
    /// The datamodel, the system variables included.
    pub(crate) datamodel: Map,
    /// The internal queue, oldest first.
    pub(crate) internal_queue: VecDeque<Event>,
    /// Every state ever entered: a state's own `<data>` binds on its first
    /// entry under late binding, the specification's `isFirstEntry` held
    /// here since the chart is immutable.
    pub(crate) entered_states: BTreeSet<usize>,
    /// The send and timer counters.
    pub(crate) sends: Sends,
    /// The send types the host registered a processor for.
    pub(crate) send_types: BTreeSet<String>,
    /// The routes the driver declares reachable.
    pub(crate) routes: Option<Routes>,
    /// Whether the last external event selected a transition, `None` before
    /// one.
    pub(crate) last_selection: Option<bool>,
    /// The specification's `running`: false once a top-level final is
    /// entered or the chart is cancelled.
    pub(crate) running: bool,
    /// [`Status::Done`] once `exitInterpreter` has finished.
    pub(crate) status: Status,
    /// The counters as they stand.
    pub(crate) counters: Counters,
    /// The round budget of each macrostep.
    pub(crate) max_macrostep_rounds: RoundBudget,
}

impl MachineState {
    /// A fresh position over `machine`: nothing entered, the counters at
    /// zero, and the host's datamodel with the system variables over it.
    pub(crate) fn new(machine: &Machine, options: StartOptions) -> Self {
        let chart = machine.chart();
        let variables = system_variables::initial(
            &options.session_id,
            chart.name.as_deref(),
            options.ioprocessors,
        );
        MachineState {
            machine: machine.clone(),
            configuration: BTreeSet::new(),
            history: History::new(),
            datamodel: system_variables::seed(options.datamodel, variables),
            internal_queue: VecDeque::new(),
            entered_states: BTreeSet::new(),
            sends: Sends::default(),
            send_types: options.send_types,
            routes: options.routes,
            last_selection: None,
            running: true,
            status: Status::Running,
            counters: Counters::default(),
            max_macrostep_rounds: options.max_macrostep_rounds,
        }
    }

    /// The chart.
    pub(crate) fn chart(&self) -> &Chart {
        self.machine.chart()
    }

    /// What selection, exit and entry read of the position.
    pub(crate) fn snapshot(&self) -> Snapshot<'_> {
        Snapshot {
            chart: self.machine.chart(),
            configuration: &self.configuration,
            history: &self.history,
        }
    }

    /// A new macrostep: the other two counters start over.
    pub(crate) fn begin_macrostep(&mut self) {
        self.counters = Counters {
            macrostep: self.counters.macrostep.saturating_add(1),
            microstep: 0,
            round: 0,
        };
    }

    /// The next microstep.
    pub(crate) fn begin_microstep(&mut self) {
        self.counters.microstep = self.counters.microstep.saturating_add(1);
    }

    /// The next round.
    pub(crate) fn begin_round(&mut self) {
        self.counters.round = self.counters.round.saturating_add(1);
    }

    /// `event` queued last on the internal queue.
    pub(crate) fn enqueue(&mut self, event: Event) {
        self.internal_queue.push_back(event);
    }

    /// The `error.*` event `failure` raises, queued.
    pub(crate) fn raise_failure(&mut self, failure: &Failure) {
        self.enqueue(Event::from_failure(failure));
    }

    /// `_event` set to `event`'s value.
    pub(crate) fn put_event(&mut self, event: &Event) {
        self.datamodel = system_variables::put_event(&self.datamodel, event.value());
    }

    /// One block of executable content, owned by `owner`, run against the
    /// position: the datamodel and the send counters it leaves kept, its
    /// effects appended to `effects`, and the events it raised queued in
    /// the order it raised them.
    pub(crate) fn run_block(
        &mut self,
        owner: BlockOwner,
        content: &[usize],
        effects: &mut Vec<Effect>,
    ) {
        if content.is_empty() {
            return;
        }
        let machine = self.machine.clone();
        let site = Site {
            chart: machine.chart(),
            configuration: &self.configuration,
            owner,
            counters: self.counters,
            send_types: &self.send_types,
            routes: self.routes.as_ref(),
        };
        let ran = execute_block(
            &site,
            std::mem::take(&mut self.datamodel),
            self.sends,
            content,
        );
        self.datamodel = ran.data;
        self.sends = ran.sends;
        effects.extend(ran.effects);
        for raised in ran.raised {
            self.enqueue(match raised {
                Raised::Event { name, .. } => Event::internal(&name),
                Raised::Error { failure, .. } => Event::from_failure(&failure),
            });
        }
    }

    /// Each queued event as `_event` would read it, oldest first.
    pub(crate) fn pending_internal_events(&self) -> Vec<Value> {
        self.internal_queue.iter().map(Event::value).collect()
    }
}
