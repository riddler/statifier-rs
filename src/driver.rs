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
//! effects of the batch that sent it, as the reference defers it.
//! `#_parent` from an invoked child, and `#_` followed by the id of a live
//! invocation, deliver as external events (below). Another session's
//! address, a parent a session does not have and an invocation that is not
//! live are unreachable: the interpreter refuses an immediate send to one
//! with `error.communication`, and a delayed one raises the same event when
//! its timer fires.
//!
//! Invocations, as the reference's session and its built-in `scxml`
//! handler run them (`Statifier.Session`, `Statifier.Session.Invocations`
//! and `Statifier.Invoke.Source` in statifier-ex), with a tree of session
//! states in place of processes. An [`Effect::Invoke`] of the SCXML type
//! whose content is markup compiles it, a root with no namespace accepted
//! as inline content is read, and starts it as a child session. Its session
//! id is its parent's, a dot and the invoke id; its datamodel is seeded with
//! the params a root `<data>` of the child names, the rest dropped; it
//! inherits its parent's registered send types, and starts with the
//! default round budget and no trace, as the reference starts a child its
//! parent passes neither. A content that is not markup or does not compile, and an
//! invocation with no content (`src` is never fetched), raise
//! `error.communication`; any other type raises `error.execution`. A child
//! runs inside its parent's call on its parent's virtual clock: starting
//! it, an event sent to it and an event forwarded to it
//! ([`Effect::Autoforward`]) each run it to a stable configuration at once,
//! and its timers fire with its parent's, in due-then-scheduled order
//! across the tree. Every effect a child answers is among the call's
//! effects, in the order made, as an [`Effect::Child`] naming its session
//! id.
//!
//! What a child sends its parent (an event to `#_parent`, carrying the
//! invoke id; a notice that it reached a top-level final, ahead of the
//! batch that stops it; and its `done.invoke.` event with its donedata)
//! waits in the parent's mailbox, taken one entry at a time once the
//! parent's external queue is empty. An entry whose invocation is no longer
//! live is discarded, and the done event retires its invocation once taken.
//! An [`Effect::CancelInvoke`] retires the invocation and stops its child,
//! running its `<onexit>` handlers, unless the child already said it
//! completed; a cancelled child returns no done event.
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
//! as the reference's halted session drops a fired delayed send. A report
//! of a failed send ([`State::report_send_failed`]) is refused with
//! [`Error::Halted`].
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

mod invocations;
mod position;
#[cfg(test)]
mod tests;

pub use position::{ActiveInvocation, ExportError, ImportError, Position, Status};

use crate::datamodel::system_variables::{EventType, SCXML_EVENT_PROCESSOR, scxml_location};
use crate::effect::Effect;
use crate::interpreter::{self, Event, MachineState, Route, Routes, StartOptions};
use crate::machine::Machine;
use predicator_rs::{Map, Value};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
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
    /// or not; `_ioprocessors` holds an entry for each, the one
    /// [`Options::ioprocessors`] gives it or an empty map. A built-in
    /// spelling, `scxml` or the SCXML Event I/O Processor's URI, is refused.
    pub send_types: BTreeSet<String>,
    /// The `_ioprocessors` entry of a registered send type, by type: a type
    /// with none here has an empty map. An entry for a type
    /// [`Options::send_types`] does not register is refused. An invoked
    /// child inherits the entries with the types, each written for the
    /// child's own session.
    pub ioprocessors: BTreeMap<String, IoProcessorEntry>,
    /// The registered send types a `<send>` must name a target for. A send
    /// of one of them with no target is still answered as an effect, and
    /// the sending session then takes `error.communication`, carrying the
    /// send's id, onto its internal queue and runs to a stable
    /// configuration, as it takes an event a send delivers there; delayed or
    /// not, nothing waits. A name [`Options::send_types`] does not register
    /// has no effect. An invoked child inherits the set.
    pub target_required: BTreeSet<String>,
    /// The round budget of each macrostep, and of the two counts one call
    /// spends it on.
    pub round_budget: RoundBudget,
    /// Whether the chart answers trace effects. This version emits none,
    /// whatever the flag says; the state keeps it, and a position carries
    /// it.
    pub trace: bool,
}

impl Options {
    /// The options for a session with this id, and every other field at its
    /// default: no starting values, no registered send type, no entry and
    /// no type that requires a target, the default round budget and no
    /// trace.
    pub fn new(session_id: impl Into<String>) -> Self {
        Options {
            session_id: session_id.into(),
            datamodel: Map::new(),
            send_types: BTreeSet::new(),
            ioprocessors: BTreeMap::new(),
            target_required: BTreeSet::new(),
            round_budget: RoundBudget::default(),
            trace: false,
        }
    }
}

/// The `_ioprocessors` entry a host supplies for a send type it registers
/// ([`Options::ioprocessors`]).
///
/// ```
/// use statifier::{IoProcessorEntry, Map, Options, Value};
///
/// let machine = statifier::compile(
///     r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="a">
///          <state id="a"/>
///        </scxml>"#,
/// )
/// .map_err(|refusals| format!("{refusals:?}"))?;
/// let mut options = Options::new("s-1");
/// options.send_types = ["x-fixed", "x-located"].into_iter().map(String::from).collect();
/// let mut fixed = Map::new();
/// fixed.insert("queue", Value::from("q1"));
/// options.ioprocessors.insert("x-fixed".to_owned(), IoProcessorEntry::Fixed(fixed));
/// options.ioprocessors.insert(
///     "x-located".to_owned(),
///     IoProcessorEntry::SessionLocation("http://localhost:8080/scxml".to_owned()),
/// );
/// let started = machine.start(options)?;
/// # let _ = started;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// The chart then reads `_ioprocessors['x-fixed']['queue']` as `q1` and
/// `_ioprocessors['x-located']['location']` as
/// `http://localhost:8080/scxml/s-1`.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub enum IoProcessorEntry {
    /// This map, written as it is for every session.
    Fixed(Map),
    /// A map whose one member, `location`, is this base address, a `/` and
    /// the id of the session it is written for, so an invoked child's entry
    /// names the child.
    SessionLocation(String),
}

impl IoProcessorEntry {
    /// The entry as the session `session_id` reads it; an owned copy.
    fn for_session(&self, session_id: &str) -> Value {
        match self {
            IoProcessorEntry::Fixed(map) => Value::Map(map.clone()),
            IoProcessorEntry::SessionLocation(base) => {
                let mut map = Map::new();
                map.insert("location", Value::from(format!("{base}/{session_id}")));
                Value::Map(map)
            }
        }
    }
}

/// Each registered type's `_ioprocessors` entry, written for the session
/// `session_id`: the one `entries` gives it, or an empty map.
fn ioprocessor_entries(
    send_types: &BTreeSet<String>,
    entries: &BTreeMap<String, IoProcessorEntry>,
    session_id: &str,
) -> Map {
    send_types
        .iter()
        .map(|send_type| {
            let entry = entries.get(send_type).map_or_else(
                || Value::Map(Map::new()),
                |entry| entry.for_session(session_id),
            );
            (send_type.as_str(), entry)
        })
        .collect()
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
    /// [`Options::ioprocessors`] gives an entry for a type
    /// [`Options::send_types`] does not register.
    #[non_exhaustive]
    UnregisteredEntries {
        /// The types named, sorted.
        types: Vec<String>,
    },
    /// A macrostep, or a call, spent its round budget and halted the chart:
    /// [`State::report_send_failed`] takes no report for it.
    Halted,
    /// [`State::report_send_failed`] was given an effect that is not a send
    /// the driver hands the host: an [`Effect::Send`] or an
    /// [`Effect::SendDelayed`] of a type the sending session registered,
    /// bare or, for an invoked child's, inside the [`Effect::Child`] that
    /// names the child.
    NotAHandedSend,
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
            Error::UnregisteredEntries { types } => write!(
                formatter,
                "an _ioprocessors entry names a send type not registered: {}",
                types.join(", ")
            ),
            Error::Halted => formatter.write_str("a spent round budget has halted the chart"),
            Error::NotAHandedSend => formatter
                .write_str("the effect is not a send of a registered type handed to the host"),
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
    /// The ids of the leaf states of the configuration the chart stopped
    /// in, the states in it with no child state, sorted; a state with no id
    /// is left out. A stopped chart has no active state, so
    /// [`State::active_leaf_states`] answers none; this is the set the
    /// reference's conformance harness compares once it restores the
    /// configuration its done effect carries.
    pub active_leaf_states: Vec<String>,
    /// How many leaf states the configuration the chart stopped in holds,
    /// a state with no id counted too: more than
    /// [`active_leaf_states`](Done::active_leaf_states) holds when a leaf it
    /// stopped in has no id. It is [`State::active_leaf_count`] for a
    /// stopped chart.
    pub active_leaf_count: usize,
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
    /// The host's `_ioprocessors` entries, by type, which an invoked child
    /// inherits; empty for a state imported from a position.
    ioprocessors: BTreeMap<String, IoProcessorEntry>,
    /// The registered types a send must name a target for, which an invoked
    /// child inherits; empty for a state imported from a position.
    target_required: BTreeSet<String>,
    round_budget: RoundBudget,
    trace: bool,
    now_ms: u64,
    timers: Vec<Timer>,
    sequence: u64,
    external_queue: VecDeque<Event>,
    halted: bool,
    done: Option<Done>,
    /// The live invocations' child sessions, by invoke id.
    invocations: BTreeMap<String, Invocation>,
    /// What the session's children sent it and it has not taken.
    mailbox: VecDeque<Mail>,
    /// The invoke id this session was started under, `None` for a session
    /// the host started.
    invoked_as: Option<String>,
    /// Whether the session's parent cancelled it.
    cancelled: bool,
}

/// A live invocation of a child session, as its parent holds it.
#[derive(Debug, Clone)]
struct Invocation {
    /// Whether the child said it reached a top-level final.
    completed: bool,
    /// The child's state; its clock and timer sequence are its parent's as
    /// they stood when it last ran.
    child: State,
}

/// What a child sent its parent and the parent has not taken.
#[derive(Debug, Clone)]
enum Mail {
    /// The child reached a top-level final.
    Completed { invoke_id: String },
    /// An event the child sent to `#_parent`.
    Event { invoke_id: String, event: Event },
    /// The child stopped on its own, with this donedata.
    Done { invoke_id: String, donedata: Value },
}

impl Mail {
    /// The invocation it came from.
    fn invoke_id(&self) -> &str {
        match self {
            Mail::Completed { invoke_id }
            | Mail::Event { invoke_id, .. }
            | Mail::Done { invoke_id, .. } => invoke_id,
        }
    }
}

impl State {
    /// A session over a core that has just started, at virtual time
    /// `now_ms` with the timer sequence at `sequence`.
    fn fresh(
        core: MachineState,
        session_id: String,
        ioprocessors: BTreeMap<String, IoProcessorEntry>,
        target_required: BTreeSet<String>,
        round_budget: RoundBudget,
        trace: bool,
    ) -> Self {
        State {
            core,
            session_id,
            ioprocessors,
            target_required,
            round_budget,
            trace,
            now_ms: 0,
            timers: Vec::new(),
            sequence: 0,
            external_queue: VecDeque::new(),
            halted: false,
            done: None,
            invocations: BTreeMap::new(),
            mailbox: VecDeque::new(),
            invoked_as: None,
            cancelled: false,
        }
    }
}

impl Machine {
    /// Starts the chart: its datamodel bound, its `<scxml>` scripts run, its
    /// initial states entered, and the whole run to a stable configuration,
    /// with every event it sent itself taken.
    ///
    /// Refused with [`Error::BuiltInSendTypes`] when `options` registers a
    /// built-in send type, then with [`Error::UnregisteredEntries`] when it
    /// gives an `_ioprocessors` entry for a type it does not register.
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
        let unregistered: Vec<String> = options
            .ioprocessors
            .keys()
            .filter(|send_type| !options.send_types.contains(*send_type))
            .cloned()
            .collect();
        if !unregistered.is_empty() {
            return Err(Error::UnregisteredEntries {
                types: unregistered,
            });
        }
        let ioprocessors = ioprocessor_entries(
            &options.send_types,
            &options.ioprocessors,
            &options.session_id,
        );
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
        let mut call = Call::open(State::fresh(
            started.state,
            options.session_id,
            options.ioprocessors,
            options.target_required,
            options.round_budget,
            options.trace,
        ));
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
    /// next. A timer an invoked child scheduled fires the same way, in the
    /// child. A stopped chart has no timer, so only its clock moves.
    ///
    /// Refused with [`Error::InvalidDuration`] when the clock would pass its
    /// largest value.
    pub fn advance(&self, ms: u64) -> Result<Moved, Error> {
        let until = self
            .now_ms
            .checked_add(ms)
            .ok_or(Error::InvalidDuration { ms })?;
        let mut call = Call::open(self.clone());
        while let Some((path, due_ms, sequence)) = call.next_due(until) {
            if due_ms > call.state.now_ms {
                call.next_instant();
            }
            call.state.now_ms = due_ms;
            call.fire_at(&path, sequence);
        }
        call.state.now_ms = until;
        Ok(call.close())
    }

    /// Reports that the host could not deliver `send`, a send of a
    /// registered type this driver handed it: the sending session takes
    /// `error.communication` as a platform event onto its internal queue
    /// and runs to a stable configuration within the call, then takes its
    /// external queue as [`State::step`] does.
    ///
    /// `send` is the [`Effect::Send`] or [`Effect::SendDelayed`] as the
    /// host was handed it, or, for an invoked child's send, the
    /// [`Effect::Child`] carrying it, which reports to that child. The
    /// event's `sendid` is the send's id, whether the document named the
    /// send or the id was generated; the event carries no data, and its
    /// `origin` and `origintype` are unset. The reference also records the
    /// send's content as the event's cause, which only its traces read;
    /// this crate answers no trace. `reason` is accepted and written into
    /// no field of the event, as the reference reads no failure it is
    /// given.
    ///
    /// Refused with [`Error::NotAHandedSend`] for any other effect, or a
    /// send of a type the sending session did not register; with
    /// [`Error::NotRunning`] once the sending session has stopped, or when
    /// the child a report names is not a live invocation; and with
    /// [`Error::Halted`] while a spent round budget has halted the sending
    /// session. A refused report queues nothing.
    ///
    /// ```
    /// use statifier::{Effect, Options, Value};
    ///
    /// let machine = statifier::compile(
    ///     r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="a">
    ///          <state id="a">
    ///            <onentry><send type="x-host" event="e.out" id="k"/></onentry>
    ///            <transition event="error.communication" cond="_event.sendid == 'k'" target="b"/>
    ///          </state>
    ///          <state id="b"/>
    ///        </scxml>"#,
    /// )
    /// .map_err(|refusals| format!("{refusals:?}"))?;
    /// let mut options = Options::new("s-1");
    /// options.send_types = ["x-host"].into_iter().map(String::from).collect();
    /// let started = machine.start(options)?;
    /// let send = started
    ///     .effects
    ///     .iter()
    ///     .find(|effect| matches!(effect, Effect::Send { .. }))
    ///     .ok_or("the send is handed to the host")?;
    ///
    /// let reported = started.state.report_send_failed(send, Some("no route"))?;
    /// assert_eq!(reported.state.configuration(), ["b"]);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn report_send_failed(&self, send: &Effect, reason: Option<&str>) -> Result<Moved, Error> {
        // The reason is the host's own record; no field of the event holds it.
        let _ = reason;
        let (path, sender, send) = self.handed(send)?;
        let send_id = match send {
            Effect::Send { send_id, .. } | Effect::SendDelayed { send_id, .. } => send_id,
            _ => return Err(Error::NotAHandedSend),
        };
        if !sender.core.running {
            return Err(Error::NotRunning);
        }
        if sender.halted {
            return Err(Error::Halted);
        }
        let mut event = Event::platform("error.communication", Value::Undefined);
        event.sendid = Some(send_id.clone());
        let mut call = Call::open(self.clone());
        call.fail_at(&path, event);
        Ok(call.close())
    }

    /// A send handed to the host, as [`State::report_send_failed`] takes
    /// it: the path to the sending session, that session and the bare send.
    /// Refused with [`Error::NotRunning`] when a child it names is not a
    /// live invocation, and with [`Error::NotAHandedSend`] for an effect
    /// that is not a send, or a send of a type the sender did not register.
    fn handed<'a>(&self, send: &'a Effect) -> Result<(Vec<String>, &State, &'a Effect), Error> {
        let (path, send) = match send {
            Effect::Child { session_id, effect } => (
                self.path_to(session_id).ok_or(Error::NotRunning)?,
                effect.as_ref(),
            ),
            send => (Vec::new(), send),
        };
        let send_type = match send {
            Effect::Send { send_type, .. } | Effect::SendDelayed { send_type, .. } => send_type,
            _ => return Err(Error::NotAHandedSend),
        };
        let sender = self.session_at(&path).ok_or(Error::NotRunning)?;
        if !matches!(send_type, Some(Value::String(send_type)) if sender.core.send_types.contains(send_type))
        {
            return Err(Error::NotAHandedSend);
        }
        Ok((path, sender, send))
    }

    /// The session that made a send handed to the host, and the bare send,
    /// refused as [`State::report_send_failed`] refuses an effect that is
    /// not one.
    #[cfg(feature = "basichttp")]
    pub(crate) fn handed_send<'a>(&self, send: &'a Effect) -> Result<(&State, &'a Effect), Error> {
        let (_, sender, send) = self.handed(send)?;
        Ok((sender, send))
    }

    /// The session's id.
    #[cfg(feature = "basichttp")]
    pub(crate) fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Whether a send of `send_type` this session makes must name a target
    /// ([`Options::target_required`]).
    pub(crate) fn requires_target(&self, send_type: Option<&Value>) -> bool {
        matches!(send_type, Some(Value::String(send_type))
            if self.core.send_types.contains(send_type) && self.target_required.contains(send_type))
    }

    /// The invoke ids from this session down to the live session named
    /// `session_id`, `None` when no session of the tree is named so.
    fn path_to(&self, session_id: &str) -> Option<Vec<String>> {
        if self.session_id == session_id {
            return Some(Vec::new());
        }
        self.invocations.iter().find_map(|(invoke_id, invocation)| {
            let mut path = invocation.child.path_to(session_id)?;
            path.insert(0, invoke_id.clone());
            Some(path)
        })
    }

    /// The session `path` names, from this one down.
    fn session_at(&self, path: &[String]) -> Option<&State> {
        match path.split_first() {
            None => Some(self),
            Some((invoke_id, rest)) => self
                .invocations
                .get(invoke_id)
                .and_then(|invocation| invocation.child.session_at(rest)),
        }
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
    /// A stopped chart has no active state, so it answers none, as the
    /// reference does; the leaves it stopped in are
    /// [`Done::active_leaf_states`].
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
    /// assert!(stopped.state.active_leaf_states().is_empty());
    /// let done = stopped.state.done().ok_or("the chart has stopped")?;
    /// assert_eq!(done.active_leaf_states, ["f"]);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn active_leaf_states(&self) -> Vec<String> {
        leaf_ids(&self.core, self.core.configuration.iter().copied())
    }

    /// How many active leaf states there are, a state with no id counted
    /// too: more than [`active_leaf_states`](State::active_leaf_states)
    /// holds when an active leaf has no id, which no expected configuration
    /// can name. The reference's conformance harness compares the two
    /// counts so that an anonymous active leaf fails a case rather than
    /// passing unseen. A stopped chart has none; the leaves it stopped in
    /// are counted in [`Done::active_leaf_count`].
    ///
    /// ```
    /// use statifier::Options;
    ///
    /// let machine = statifier::compile(
    ///     r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="p">
    ///          <parallel id="p">
    ///            <state id="a"/>
    ///            <state/>
    ///          </parallel>
    ///        </scxml>"#,
    /// )
    /// .map_err(|refusals| format!("{refusals:?}"))?;
    /// let started = machine.start(Options::new("s-1"))?;
    /// assert_eq!(started.state.active_leaf_states(), ["a"]);
    /// assert_eq!(started.state.active_leaf_count(), 2);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn active_leaf_count(&self) -> usize {
        leaf_count(&self.core, self.core.configuration.iter().copied())
    }

    /// `None` while the chart runs; once it has stopped, its donedata, the
    /// configuration it stopped in and that configuration's leaf states.
    pub fn done(&self) -> Option<&Done> {
        self.done.as_ref()
    }

    /// The virtual clock, in milliseconds since the chart started.
    pub fn now_ms(&self) -> u64 {
        self.now_ms
    }

    /// The virtual time the earliest pending timer fires at, an invoked
    /// child's included, `None` when no timer is pending.
    pub fn next_due_ms(&self) -> Option<u64> {
        self.timers
            .iter()
            .map(|timer| timer.due_ms)
            .chain(
                self.invocations
                    .values()
                    .filter_map(|invocation| invocation.child.next_due_ms()),
            )
            .min()
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

/// How many of `indexes` are atomic states, a state with no id counted too.
fn leaf_count(core: &MachineState, indexes: impl Iterator<Item = usize>) -> usize {
    let chart = core.chart();
    indexes
        .filter(|&index| index < chart.states.len() && chart.is_atomic(index))
        .count()
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
    /// The mailbox, each entry with whether this call counts it when it
    /// takes it.
    mailbox: VecDeque<(Mail, bool)>,
    /// What this session sent its parent in this call, each with whether
    /// the parent counts it.
    outbox: Vec<(Mail, bool)>,
    /// The external event the call is taking, which an autoforward
    /// delivers.
    taking: Option<Event>,
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
        let mailbox = std::mem::take(&mut state.mailbox)
            .into_iter()
            .map(|mail| (mail, false))
            .collect();
        Call {
            state,
            external,
            mailbox,
            outbox: Vec::new(),
            taking: None,
            effects: Vec::new(),
            taken: 0,
            sent_internal: 0,
            spent: false,
            first_sequence,
            fired: 0,
        }
    }

    /// The state the call leaves, and its effects.
    fn close(self) -> Moved {
        let (state, effects, _) = self.finish();
        Moved { state, effects }
    }

    /// The state the call leaves, its effects, and what it sent its parent.
    fn finish(mut self) -> (State, Vec<Effect>, Vec<(Mail, bool)>) {
        self.state.external_queue = self.external.into_iter().map(|q| q.event).collect();
        self.state.mailbox = self.mailbox.into_iter().map(|(mail, _)| mail).collect();
        (self.state, self.effects, self.outbox)
    }

    /// The earliest timer due by `until` anywhere in the tree of sessions,
    /// the first scheduled among equals: the invoke ids from this session
    /// down to the one holding it, its due time and its sequence.
    fn next_due(&self, until: u64) -> Option<(Vec<String>, u64, u64)> {
        let mut pending = Vec::new();
        pending_timers(&self.state, &mut Vec::new(), &mut pending);
        pending
            .into_iter()
            .filter(|(_, due_ms, _)| *due_ms <= until)
            .min_by_key(|(_, due_ms, sequence)| (*due_ms, *sequence))
    }

    /// Fires the timer scheduled at `sequence` in the session `path` names,
    /// each session on the way drained after it.
    fn fire_at(&mut self, path: &[String], sequence: u64) {
        let Some((invoke_id, rest)) = path.split_first() else {
            let Some(position) = self
                .state
                .timers
                .iter()
                .position(|timer| timer.sequence == sequence)
            else {
                return;
            };
            let timer = self.state.timers.remove(position);
            self.count_fire(&timer);
            let effects = self.route(timer.send, false);
            self.perform(effects);
            self.drain();
            return;
        };
        self.within(invoke_id, |child| child.fire_at(rest, sequence));
        self.drain();
    }

    /// Delivers a failed send's `error.communication` onto the internal
    /// queue of the session `path` names, the reference's
    /// `Statifier.Session.failed_send/3`, and drains each session on the
    /// way after it.
    fn fail_at(&mut self, path: &[String], event: Event) {
        let Some((invoke_id, rest)) = path.split_first() else {
            let effects = self.deliver_internal(event);
            self.perform(effects);
            self.drain();
            return;
        };
        self.within(invoke_id, |child| child.fail_at(rest, event));
        self.drain();
    }

    /// Takes the external queue one event per macrostep while the chart
    /// runs and the driver is not halted, and, once it is empty, the
    /// mailbox one entry at a time. A counted entry the budget has no room
    /// for is left where it waits, and the driver halts.
    fn drain(&mut self) {
        while self.state.core.running && !self.state.halted {
            if let Some(queued) = self.external.pop_front() {
                if queued.counted && !self.count_taken() {
                    self.external.push_front(queued);
                    self.halt();
                    return;
                }
                self.take(queued.event);
                continue;
            }
            let Some((mail, counted)) = self.mailbox.pop_front() else {
                return;
            };
            if counted && !self.count_taken() {
                self.mailbox.push_front((mail, counted));
                self.halt();
                return;
            }
            self.take_mail(mail);
        }
    }

    /// Counts one more counted entry taken, answering whether the budget
    /// had room for it.
    fn count_taken(&mut self) -> bool {
        let taken = self.taken.saturating_add(1);
        if !self.state.round_budget.admits(taken) {
            return false;
        }
        self.taken = taken;
        true
    }

    /// One external event, one macrostep, with the routes the session can
    /// reach declared to the interpreter as the event arrives.
    fn take(&mut self, event: Event) {
        self.stamp();
        if let Ok(stepped) = interpreter::handle_event(&self.state.core, &event) {
            self.state.core = stepped.state;
            self.taking = Some(event);
            self.perform(stepped.effects);
            self.taking = None;
        }
    }

    /// The routes the session can reach now, declared to the interpreter:
    /// itself, its parent when it has one, and its live invocations.
    fn stamp(&mut self) {
        self.state.core.routes = Some(Routes {
            sessions: BTreeSet::from([self.state.session_id.clone()]),
            parent: self.state.invoked_as.is_some(),
            invokes: self.state.invocations.keys().cloned().collect(),
        });
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
            self.announce(&batch);
            let mut deferred = Vec::new();
            for effect in batch {
                self.effects.push(self.report(effect.clone()));
                deferred.extend(self.perform_one(&effect));
            }
            batch = deferred;
        }
    }

    /// `effect` as the call answers it: an invoked child's wrapped in an
    /// [`Effect::Child`] naming the child.
    fn report(&self, effect: Effect) -> Effect {
        match self.state.invoked_as {
            Some(_) => Effect::Child {
                session_id: self.state.session_id.clone(),
                effect: Box::new(effect),
            },
            None => effect,
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
            Effect::Send {
                target: None,
                send_type,
                send_id,
                ..
            }
            | Effect::SendDelayed {
                target: None,
                send_type,
                send_id,
                ..
            } if self.state.requires_target(send_type.as_ref()) => {
                let mut event = Event::platform("error.communication", Value::Undefined);
                event.sendid = Some(send_id.clone());
                self.deliver_internal(event)
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
                    active_leaf_states: leaf_ids(&self.state.core, configuration.iter().copied()),
                    active_leaf_count: leaf_count(&self.state.core, configuration.iter().copied()),
                });
                self.state.timers.clear();
                self.external.clear();
                self.mailbox.clear();
                self.state.invocations.clear();
                self.return_done(donedata.clone().unwrap_or(Value::Undefined));
                Vec::new()
            }
            Effect::Invoke {
                invoke_id,
                invoke_type,
                params,
                content,
                ..
            } => self.start_child(invoke_id, invoke_type.as_ref(), params, content.as_ref()),
            Effect::CancelInvoke { invoke_id, .. } => {
                self.cancel_child(invoke_id);
                Vec::new()
            }
            Effect::Autoforward { invoke_id, .. } => {
                if let Some(event) = self.taking.clone() {
                    self.deliver(invoke_id, event);
                }
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
            Route::Parent if self.state.invoked_as.is_some() => {
                if !self.state.halted || immediate {
                    self.post_to_parent(send.event, immediate);
                }
                Vec::new()
            }
            Route::Invoke(id) if self.state.invocations.contains_key(&id) => {
                if !self.state.halted || immediate {
                    self.deliver(&id, send.event);
                }
                Vec::new()
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
        self.stamp();
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

/// Every pending timer of the tree under `state`, each with the path to its
/// session, its due time and its sequence.
fn pending_timers(state: &State, path: &mut Vec<String>, out: &mut Vec<(Vec<String>, u64, u64)>) {
    out.extend(
        state
            .timers
            .iter()
            .map(|timer| (path.clone(), timer.due_ms, timer.sequence)),
    );
    for (invoke_id, invocation) in &state.invocations {
        path.push(invoke_id.clone());
        pending_timers(&invocation.child, path, out);
        path.pop();
    }
}
