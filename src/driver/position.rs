//! A chart's position in the string-id vocabulary the reference migrates a
//! position in: [`State::export_position`] writes one and
//! [`Machine::import_position`] reads one back, onto the chart that wrote it
//! or onto another that holds every state it names.
//!
//! The reference is `Statifier.Position.export/1` and `import/2` in
//! statifier-ex. The export translates every state index to the id the
//! document wrote and drops the root, which has none; it refuses a position
//! whose internal queue holds an event and one that holds a state, the root
//! apart, with no written id. The import resolves every id against the
//! chart, refusing with every unknown id at once, sorted; it re-adds the
//! root, starts the internal queue empty and reads no identity.
//!
//! A position holds the interpreter's state and nothing of the driver's:
//! no session id beyond the datamodel's `_sessionid`, no virtual clock, no
//! pending timer, no queued external event and no budget halt. The import
//! starts each of those as a fresh session does: the clock at 0, nothing
//! pending, nothing queued, not halted, the session id read from
//! `_sessionid`, and the registered send types read from the entries of
//! `_ioprocessors` other than the SCXML Event I/O Processor's.

use super::{Done, RoundBudget, State, ids, is_built_in, leaf_count, leaf_ids};
use crate::interpreter::{self, Counters, MachineState, Routes, Sends};
use crate::machine::{Chart, Machine};
use predicator_rs::{Key, Map, Value};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

#[cfg(test)]
mod tests;

/// The datamodel root the session id is read from.
const SESSION_ID: &str = "_sessionid";

/// The datamodel root the registered send types are read from.
const IOPROCESSORS: &str = "_ioprocessors";

/// Whether a chart has finished stopping: the reference's `status`.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Status {
    /// The chart runs, or is stopping.
    Running,
    /// Every state has exited: the chart has stopped.
    Done,
}

/// A live invocation, as a position names it: the state whose `<invoke>`
/// started it, which of that state's `<invoke>` elements, in document
/// order from 0, and the invoke id.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ActiveInvocation {
    /// The id of the state holding the `<invoke>`.
    pub state: String,
    /// Which of the state's `<invoke>` elements.
    pub invoke_index: u64,
    /// The invoke id.
    pub invoke_id: String,
}

/// A chart's position in the string-id vocabulary: every field of the
/// reference's export, a state named by the id the document wrote and the
/// root never named.
///
/// [`State::export_position`] answers one; [`Machine::import_position`]
/// reads one, which a host may have kept anywhere and edited. With the
/// `serde` feature its serialized keys are the TypeScript sibling's, in
/// camel case, and each datamodel root is the predicator crate's tagged
/// text of its value, so a position one sibling writes reads in the other.
///
/// ```
/// use statifier::{Options, Value};
///
/// let machine = statifier::compile(
///     r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="a">
///          <state id="a"><transition event="e.go" target="b"/></state>
///          <state id="b"><transition event="e.go" target="c"/></state>
///          <final id="c"/>
///        </scxml>"#,
/// )
/// .map_err(|refusals| format!("{refusals:?}"))?;
/// let moved = machine.start(Options::new("s-1"))?.state.step("e.go", Value::Undefined)?;
/// let position = moved.state.export_position()?;
/// assert!(position.configuration.contains("b"));
///
/// let resumed = machine.import_position(&position)?;
/// assert_eq!(resumed.configuration(), ["b"]);
/// let stopped = resumed.step("e.go", Value::Undefined)?;
/// assert!(stopped.state.done().is_some());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub struct Position {
    /// Provenance only: [`Machine::import_position`] never reads it. This
    /// crate computes no chart identity, so an export writes
    /// [`Value::Null`]; a position another sibling wrote keeps the identity
    /// it carried.
    pub identity: Value,
    /// The active states' ids, the root left out.
    pub configuration: BTreeSet<String>,
    /// Every state ever entered, the root left out: a state's own `<data>`
    /// binds on its first entry under late binding.
    pub entered_states: BTreeSet<String>,
    /// The states whose `<invoke>` elements have yet to start.
    pub states_to_invoke: BTreeSet<String>,
    /// Each history state that has recorded a value, by id, to the ids of
    /// the states it restores.
    pub history_values: BTreeMap<String, BTreeSet<String>>,
    /// The live invocations.
    pub active_invocations: Vec<ActiveInvocation>,
    /// The last invoke id's number.
    pub invoke_counter: u64,
    /// The last generated send id's number.
    pub send_counter: u64,
    /// The last ordinal a delayed send, a cancel or a registered type's send
    /// minted.
    pub timer_counter: u64,
    /// The datamodel, the system variables included.
    pub datamodel: Map,
    /// The specification's `running`: false once a top-level final state is
    /// entered or the chart is cancelled.
    pub running: bool,
    /// Whether the chart has finished stopping.
    pub status: Status,
    /// The macrostep counter.
    pub macrostep: u64,
    /// The microstep counter.
    pub microstep: u64,
    /// The round counter.
    pub round: u64,
    /// The `trace` option the chart was started with.
    pub trace: bool,
    /// The round budget of each macrostep.
    pub max_macrostep_rounds: RoundBudget,
}

/// Why [`State::export_position`] refused.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportError {
    /// The internal queue holds an event, selected against this chart's
    /// transitions: the state is inside a macrostep, which a spent round
    /// budget leaves it in.
    InternalQueueNotEmpty,
    /// The position holds states, the root apart, that the document gave no
    /// id.
    #[non_exhaustive]
    UnnameableStates {
        /// Every such state's index in document order, ascending.
        indexes: Vec<usize>,
    },
}

impl fmt::Display for ExportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExportError::InternalQueueNotEmpty => {
                formatter.write_str("the internal queue holds an event")
            }
            ExportError::UnnameableStates { indexes } => {
                let indexes: Vec<String> = indexes.iter().map(usize::to_string).collect();
                write!(
                    formatter,
                    "states with no id are in the position: {}",
                    indexes.join(", ")
                )
            }
        }
    }
}

impl std::error::Error for ExportError {}

/// Why [`Machine::import_position`] refused.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportError {
    /// The datamodel's `_sessionid` is missing or not a string, so the state
    /// has no session to run as.
    NoSessionId,
    /// The position names states the chart does not hold.
    #[non_exhaustive]
    UnknownStateIds {
        /// Every such id, sorted.
        ids: Vec<String>,
    },
    /// A recorded history value the chart cannot restore: its key is not a
    /// `<history>`, or it holds a state that is a history or is not inside
    /// the history's parent.
    #[non_exhaustive]
    InvalidHistoryValue {
        /// The history's id, the first such in id order.
        history: String,
    },
    /// Two live invocations name the same `<invoke>` of the same state.
    #[non_exhaustive]
    DuplicateInvocation {
        /// The state's id.
        state: String,
        /// Which of its `<invoke>` elements.
        invoke_index: u64,
    },
    /// A live invocation names an `<invoke>` its state does not hold: its
    /// index is not below the state's count of `<invoke>` elements.
    #[non_exhaustive]
    UnknownInvoke {
        /// The state's id.
        state: String,
        /// The index named.
        invoke_index: u64,
    },
}

impl fmt::Display for ImportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImportError::NoSessionId => {
                formatter.write_str("the datamodel's _sessionid is not a string")
            }
            ImportError::UnknownStateIds { ids } => {
                write!(
                    formatter,
                    "the chart holds no state with these ids: {}",
                    ids.join(", ")
                )
            }
            ImportError::InvalidHistoryValue { history } => {
                write!(
                    formatter,
                    "the history value recorded for {history:?} cannot be restored"
                )
            }
            ImportError::DuplicateInvocation {
                state,
                invoke_index,
            } => write!(
                formatter,
                "two invocations name invoke {invoke_index} of {state:?}"
            ),
            ImportError::UnknownInvoke {
                state,
                invoke_index,
            } => write!(formatter, "{state:?} holds no invoke {invoke_index}"),
        }
    }
}

impl std::error::Error for ImportError {}

impl State {
    /// The chart's position, in the string-id vocabulary.
    ///
    /// Only the interpreter's state travels: the virtual clock, the pending
    /// timers, the queued external events and a budget halt are the
    /// driver's, and a position carries none of them.
    ///
    /// Refused with [`ExportError::InternalQueueNotEmpty`] when the internal
    /// queue holds an event, and with [`ExportError::UnnameableStates`],
    /// naming every one, when the position holds a state other than the
    /// root that the document gave no id.
    pub fn export_position(&self) -> Result<Position, ExportError> {
        let core = &self.core;
        if !core.internal_queue.is_empty() {
            return Err(ExportError::InternalQueueNotEmpty);
        }
        let chart = core.chart();
        let unnameable: Vec<usize> = referenced(core)
            .into_iter()
            .filter(|&index| index != 0 && id_of(chart, index).is_none())
            .collect();
        if !unnameable.is_empty() {
            return Err(ExportError::UnnameableStates {
                indexes: unnameable,
            });
        }
        let names = |indexes: &BTreeSet<usize>| -> BTreeSet<String> {
            indexes
                .iter()
                .filter_map(|&index| id_of(chart, index))
                .collect()
        };
        let history_values = core
            .history
            .iter()
            .filter_map(|(&history, recorded)| {
                id_of(chart, history).map(|history| (history, names(recorded)))
            })
            .collect();
        let active_invocations = core
            .active_invocations
            .iter()
            .filter_map(|(&(state, invoke_index), invoke_id)| {
                id_of(chart, state).map(|state| ActiveInvocation {
                    state,
                    invoke_index,
                    invoke_id: invoke_id.clone(),
                })
            })
            .collect();
        Ok(Position {
            identity: Value::Null,
            configuration: names(&core.configuration),
            entered_states: names(&core.entered_states),
            states_to_invoke: names(&core.states_to_invoke),
            history_values,
            active_invocations,
            invoke_counter: core.invoke_counter,
            send_counter: core.sends.send_counter,
            timer_counter: core.sends.timer_counter,
            datamodel: core.datamodel.clone(),
            running: core.running,
            status: match core.status {
                interpreter::Status::Running => Status::Running,
                interpreter::Status::Done => Status::Done,
            },
            macrostep: core.counters.macrostep,
            microstep: core.counters.microstep,
            round: core.counters.round,
            trace: self.trace,
            max_macrostep_rounds: self.round_budget,
        })
    }
}

impl Machine {
    /// A chart in execution over this machine, at `position`: the reverse
    /// of [`State::export_position`]. The position's identity is not read,
    /// so a position may be imported into any chart that holds every state
    /// it names.
    ///
    /// The root is re-added to the configuration and to the entered states,
    /// and the internal queue starts empty. The driver's own fields start
    /// as a fresh session's do: the virtual clock at 0, no pending timer,
    /// no queued external event, not halted. The session id is the
    /// datamodel's `_sessionid`, and the registered send types are the
    /// entries of `_ioprocessors` other than the SCXML Event I/O
    /// Processor's, as [`Machine::start`] writes them. A position carries
    /// no child session, so a live invocation it names has none: it is
    /// sent nothing, forwarded nothing and returns nothing, and its state's
    /// exit cancels it.
    ///
    /// A stopped position imports stopped: [`State::done`] answers, and
    /// [`State::step`] refuses. A position carries no donedata and a stopped
    /// chart's configuration is empty, so [`Done::donedata`] is
    /// [`Value::Undefined`] and [`Done::configuration`] the position's own.
    ///
    /// Refused, in this order, with [`ImportError::NoSessionId`] when
    /// `_sessionid` is not a string; with [`ImportError::UnknownStateIds`],
    /// naming every one, when the position names a state the chart does not
    /// hold; with [`ImportError::InvalidHistoryValue`] when a recorded
    /// history value is not one the chart can restore; and, live invocation
    /// by live invocation, with [`ImportError::UnknownInvoke`] when one names
    /// an `<invoke>` its state does not hold and with
    /// [`ImportError::DuplicateInvocation`] when two name one `<invoke>`. Every state the position names is checked against
    /// the chart before any of it is read.
    pub fn import_position(&self, position: &Position) -> Result<State, ImportError> {
        let session_id = match position.datamodel.get(&Key::from(SESSION_ID)) {
            Some(Value::String(session_id)) => session_id.clone(),
            _ => return Err(ImportError::NoSessionId),
        };
        let chart = self.chart();
        let unknown: BTreeSet<&str> = named(position)
            .filter(|id| !chart.id_to_index.contains_key(*id))
            .collect();
        if !unknown.is_empty() {
            return Err(ImportError::UnknownStateIds {
                ids: unknown.into_iter().map(str::to_owned).collect(),
            });
        }
        let index = |id: &str| chart.id_to_index.get(id).copied().unwrap_or(0);
        let indexes = |ids: &BTreeSet<String>| -> BTreeSet<usize> {
            ids.iter().map(|id| index(id)).collect()
        };
        let mut history = BTreeMap::new();
        for (key, recorded) in &position.history_values {
            let restored = indexes(recorded);
            if !restorable(chart, index(key), &restored) {
                return Err(ImportError::InvalidHistoryValue {
                    history: key.clone(),
                });
            }
            history.insert(index(key), restored);
        }
        let mut active_invocations = BTreeMap::new();
        for invocation in &position.active_invocations {
            let key = (index(&invocation.state), invocation.invoke_index);
            let held = chart
                .states
                .get(key.0)
                .map_or(0, |state| state.invoke.len());
            if usize::try_from(invocation.invoke_index).map_or(true, |at| at >= held) {
                return Err(ImportError::UnknownInvoke {
                    state: invocation.state.clone(),
                    invoke_index: invocation.invoke_index,
                });
            }
            if active_invocations
                .insert(key, invocation.invoke_id.clone())
                .is_some()
            {
                return Err(ImportError::DuplicateInvocation {
                    state: invocation.state.clone(),
                    invoke_index: invocation.invoke_index,
                });
            }
        }
        let mut configuration = indexes(&position.configuration);
        configuration.insert(0);
        let mut entered_states = indexes(&position.entered_states);
        entered_states.insert(0);
        let core = MachineState {
            machine: self.clone(),
            configuration,
            history,
            datamodel: position.datamodel.clone(),
            internal_queue: VecDeque::new(),
            entered_states,
            sends: Sends {
                send_counter: position.send_counter,
                timer_counter: position.timer_counter,
            },
            states_to_invoke: indexes(&position.states_to_invoke),
            active_invocations,
            invoke_counter: position.invoke_counter,
            send_types: send_types(&position.datamodel),
            routes: Some(Routes {
                sessions: BTreeSet::from([session_id.clone()]),
                ..Routes::default()
            }),
            last_selection: None,
            running: position.running,
            status: match position.status {
                Status::Running => interpreter::Status::Running,
                Status::Done => interpreter::Status::Done,
            },
            counters: Counters {
                macrostep: position.macrostep,
                microstep: position.microstep,
                round: position.round,
            },
            max_macrostep_rounds: position.max_macrostep_rounds.interpreter(),
        };
        let done = (position.status == Status::Done).then(|| Done {
            donedata: Value::Undefined,
            configuration: ids(&core, core.configuration.iter().copied()),
            active_leaf_states: leaf_ids(&core, core.configuration.iter().copied()),
            active_leaf_count: leaf_count(&core, core.configuration.iter().copied()),
        });
        let mut state = State::fresh(
            core,
            session_id,
            BTreeMap::new(),
            position.max_macrostep_rounds,
            position.trace,
        );
        state.done = done;
        Ok(state)
    }
}

/// The id the document gave the state at `index`, `None` for none.
fn id_of(chart: &Chart, index: usize) -> Option<String> {
    chart.states.get(index).and_then(|state| state.id.clone())
}

/// Every state index a position's fields hold, ascending.
fn referenced(core: &MachineState) -> BTreeSet<usize> {
    let mut indexes: BTreeSet<usize> = core
        .configuration
        .iter()
        .chain(&core.entered_states)
        .chain(&core.states_to_invoke)
        .copied()
        .collect();
    for (&history, recorded) in &core.history {
        indexes.insert(history);
        indexes.extend(recorded);
    }
    indexes.extend(core.active_invocations.keys().map(|&(state, _)| state));
    indexes
}

/// Every state id a position names, in no particular order.
fn named(position: &Position) -> impl Iterator<Item = &str> {
    position
        .configuration
        .iter()
        .chain(&position.entered_states)
        .chain(&position.states_to_invoke)
        .chain(
            position
                .history_values
                .iter()
                .flat_map(|(history, recorded)| std::iter::once(history).chain(recorded)),
        )
        .chain(position.active_invocations.iter().map(|item| &item.state))
        .map(String::as_str)
}

/// Whether the history state at `history` can restore `recorded`: it is a
/// `<history>`, and each recorded state is a state other than a history
/// inside the history's parent. Every restore then enters states deeper in
/// the chart than the history's parent, so entering one always ends.
fn restorable(chart: &Chart, history: usize, recorded: &BTreeSet<usize>) -> bool {
    let Some(parent) = chart
        .states
        .get(history)
        .filter(|_| chart.is_history(history))
        .and_then(|state| state.parent)
    else {
        return false;
    };
    recorded
        .iter()
        .all(|&state| !chart.is_history(state) && chart.is_descendant(state, parent))
}

/// The send types a position's chart registered: the keys of
/// `_ioprocessors` other than a built-in spelling of the SCXML Event I/O
/// Processor.
fn send_types(datamodel: &Map) -> BTreeSet<String> {
    let Some(Value::Map(entries)) = datamodel.get(&Key::from(IOPROCESSORS)) else {
        return BTreeSet::new();
    };
    entries
        .keys()
        .filter_map(|key| match key {
            Key::String(send_type) if !is_built_in(send_type) => Some(send_type.clone()),
            _ => None,
        })
        .collect()
}
