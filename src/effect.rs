//! What interpreting answers besides the next state: the effects, plain data
//! a host reads and acts on.
//!
//! The interpreter never logs, never writes anywhere and never delivers an
//! event itself. Each of those is an [`Effect`], an owned value the host
//! receives in the order the chart made them, ported field for field from
//! the reference's effect vocabulary (`Statifier.Effect` in statifier-ex).
//! Every effect carries the three counters as they stood when it was made:
//! the macrostep, the microstep within it and the round.

use crate::Value;
use predicator_rs::Map;

/// One effect: something the chart did that a host may observe.
///
/// The enum and every variant are `#[non_exhaustive]`: later versions add
/// variants and fields, so a host matches with a wildcard arm and binds
/// fields with `..`.
///
/// ```
/// use statifier::Effect;
///
/// fn describe(effect: &Effect) -> String {
///     match effect {
///         Effect::Log { label, value, .. } => format!("log {label:?}: {value:?}"),
///         Effect::DatamodelChange { location_source, new_value, .. } => {
///             format!("{location_source} = {new_value:?}")
///         }
///         _ => String::from("another effect"),
///     }
/// }
/// # let _ = describe;
/// ```
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// A `<log>` ran: its label and the value of its expression.
    #[non_exhaustive]
    Log {
        /// The `label` attribute as written, `None` when it was not.
        label: Option<String>,
        /// The value `expr` evaluated to, `None` for a `<log>` that wrote
        /// no `expr`. The value is not coerced: it is displayed, not sent.
        value: Option<Value>,
        /// The index of the `<log>` among the chart's executable-content
        /// nodes, in document order.
        content_index: usize,
        /// The block the `<log>` ran in.
        owner: BlockOwner,
        /// The macrostep counter when it ran.
        macrostep: u64,
        /// The microstep counter when it ran.
        microstep: u64,
        /// The round counter when it ran.
        round: u64,
    },
    /// A write to the datamodel landed: enough to apply the same write to a
    /// copy of the datamodel without reading the chart's.
    #[non_exhaustive]
    DatamodelChange {
        /// The location written, resolved: the root first, then one segment
        /// per property or index.
        location_path: Vec<PathSegment>,
        /// The location as the document wrote it.
        location_source: String,
        /// The value written.
        new_value: Value,
        /// The value that stood at the full path before the write,
        /// [`Value::Undefined`] when nothing did.
        prior_value: Value,
        /// The index of the `<data>` the write bound, among the chart's
        /// `<data>` elements in document order; `None` for any other write.
        data_index: Option<usize>,
        /// The index of the executable-content node that made the write;
        /// `None` for a write made outside executable content.
        content_index: Option<usize>,
        /// The block the write belongs to; `None` only for a `<data>`
        /// binding. The value an empty `<finalize>` assigns carries
        /// [`BlockOwner::Finalize`], and an `<invoke>`'s `idlocation` write
        /// will carry an owner of its own once invoke arrives.
        owner: Option<BlockOwner>,
        /// The macrostep counter when it ran.
        macrostep: u64,
        /// The microstep counter when it ran.
        microstep: u64,
        /// The round counter when it ran.
        round: u64,
    },
    /// A `<send>` with no delay ran: a message for the host to deliver now.
    ///
    /// Every target is an effect, the session's own internal queue
    /// (`#_internal`) included: the host routes the message, so an internal
    /// send comes back to the chart as the host delivers it. A send the
    /// chart refused for its type or its target answers no effect: it
    /// raises `error.execution` or `error.communication` instead.
    #[non_exhaustive]
    Send {
        /// The event's name, as `event` or `eventexpr` gave it; `None` when
        /// neither was written.
        event: Option<Value>,
        /// The target, as `target` or `targetexpr` gave it; `None` when
        /// neither was written, which is the session itself. For a type the
        /// host registered, it is the host processor's own route and is
        /// never read here.
        target: Option<Value>,
        /// The type, as `type` or `typeexpr` gave it; `None` when neither
        /// was written, which is the SCXML Event I/O Processor.
        send_type: Option<Value>,
        /// The event's data: the `<content>`'s value when the `<send>` has
        /// one, else the `namelist` and `<param>` values as a map of name
        /// to value (the last of a repeated name kept), else
        /// [`Value::Undefined`].
        data: Value,
        /// The send id: the `id` attribute as written, else one generated,
        /// `send_` and the session's send counter.
        send_id: String,
        /// Whether the document named the send (an `id` or an
        /// `idlocation`), rather than the id being generated: an event the
        /// send delivers carries its send id only when it did.
        id_from_author: bool,
        /// The session's timer counter after this send advanced it, which
        /// it does only for a type the host registered; `None` for the
        /// SCXML Event I/O Processor's types. It tells apart two sends that
        /// share every other field, as two iterations of a `<foreach>` do.
        ordinal: Option<u64>,
        /// The index of the `<send>` among the chart's executable-content
        /// nodes, in document order.
        content_index: usize,
        /// The block the `<send>` ran in.
        owner: BlockOwner,
        /// The macrostep counter when it ran.
        macrostep: u64,
        /// The microstep counter when it ran.
        microstep: u64,
        /// The round counter when it ran.
        round: u64,
    },
    /// A `<send>` with a delay ran: a message for the host to deliver once
    /// the delay has passed, unless a `<cancel>` names its send id first.
    /// It carries what [`Effect::Send`] does, and the delay.
    #[non_exhaustive]
    SendDelayed {
        /// The event's name, as `event` or `eventexpr` gave it; `None` when
        /// neither was written.
        event: Option<Value>,
        /// The target, as `target` or `targetexpr` gave it; `None` when
        /// neither was written. It is judged when the delay has passed,
        /// not here, apart from a target that names nothing the SCXML Event
        /// I/O Processor carries, which is refused now.
        target: Option<Value>,
        /// The type, as `type` or `typeexpr` gave it; `None` when neither
        /// was written.
        send_type: Option<Value>,
        /// The event's data, as [`Effect::Send`]'s `data`.
        data: Value,
        /// The send id a `<cancel>` names it by: the `id` attribute as
        /// written, else one generated.
        send_id: String,
        /// Whether the document named the send (an `id` or an
        /// `idlocation`).
        id_from_author: bool,
        /// The delay in whole milliseconds, as `delay` or `delayexpr` gave
        /// it.
        delay_ms: i64,
        /// The session's timer counter after this send advanced it: every
        /// delayed send advances it.
        ordinal: u64,
        /// The index of the `<send>` among the chart's executable-content
        /// nodes, in document order.
        content_index: usize,
        /// The block the `<send>` ran in.
        owner: BlockOwner,
        /// The macrostep counter when it ran, not when the delay passes.
        macrostep: u64,
        /// The microstep counter when it ran.
        microstep: u64,
        /// The round counter when it ran.
        round: u64,
    },
    /// The datamodel as it stood when the chart started, every `<data>` id
    /// declared and none of them bound yet: the base a host applies each
    /// [`Effect::DatamodelChange`] to. It is the first effect a chart
    /// answers when it starts.
    #[non_exhaustive]
    DatamodelInit {
        /// The datamodel: the host's values, the system variables over them,
        /// and every `<data>` id the host did not supply, as
        /// [`Value::Undefined`].
        datamodel: Map,
        /// The macrostep counter when it was made.
        macrostep: u64,
        /// The microstep counter when it was made.
        microstep: u64,
        /// The round counter when it was made.
        round: u64,
    },
    /// A macrostep spent its round budget without reaching a stable
    /// configuration. The chart is where the last round left it and still
    /// running: nothing exited and nothing stopped. It comes last among the
    /// effects of the call that spent the budget.
    #[non_exhaustive]
    BudgetExhausted {
        /// The indexes of the active states as the last round left them,
        /// ascending.
        configuration: Vec<usize>,
        /// The budget spent: the most rounds one macrostep may take.
        budget: u64,
        /// The events still on the internal queue, oldest first, each as
        /// `_event` would read it: a map of its name, type, send id, origin,
        /// origin type, invoke id and data.
        pending_internal_events: Vec<Value>,
        /// The macrostep counter when it was spent.
        macrostep: u64,
        /// The microstep counter when it was spent.
        microstep: u64,
        /// The round counter when it was spent: the rounds the macrostep took.
        round: u64,
    },
    /// The chart stopped: every active state has exited, and nothing more
    /// happens to it. It comes last among the effects of the call that
    /// stopped it.
    #[non_exhaustive]
    Done {
        /// The top-level final state's `<donedata>`: [`Value::Undefined`]
        /// when it has none, or when evaluating it failed; `None` when no
        /// top-level final state exited, as when the chart was cancelled.
        donedata: Option<Value>,
        /// The data of the first `error.execution` evaluating the donedata
        /// raised, `None` when it raised none. Nothing can take that event
        /// from the internal queue once the chart has stopped, so it is
        /// carried here.
        donedata_error: Option<Value>,
        /// The indexes of the states that were active when the chart
        /// stopped, ascending.
        configuration: Vec<usize>,
        /// The macrostep counter when it stopped.
        macrostep: u64,
        /// The microstep counter when it stopped.
        microstep: u64,
        /// The round counter when it stopped.
        round: u64,
    },
    /// A `<cancel>` ran: the host drops the delayed send it names, if one
    /// is still pending. A send id that names nothing pending is not an
    /// error.
    #[non_exhaustive]
    Cancel {
        /// The send id, as `sendid` or `sendidexpr` gave it.
        send_id: Value,
        /// The session's timer counter after this cancel advanced it: every
        /// cancel advances it.
        ordinal: u64,
        /// The index of the `<cancel>` among the chart's
        /// executable-content nodes, in document order.
        content_index: usize,
        /// The block the `<cancel>` ran in.
        owner: BlockOwner,
        /// The macrostep counter when it ran.
        macrostep: u64,
        /// The microstep counter when it ran.
        microstep: u64,
        /// The round counter when it ran.
        round: u64,
    },
}

/// The block of executable content an effect came from. State and
/// transition indexes count in document order, the `<scxml>` root being
/// state 0, and `ordinal` counts a state's blocks of one kind from 0.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlockOwner {
    /// One of a state's `<onentry>` blocks.
    #[non_exhaustive]
    OnEntry {
        /// The state's index.
        state: usize,
        /// Which of its `<onentry>` blocks.
        ordinal: usize,
    },
    /// One of a state's `<onexit>` blocks.
    #[non_exhaustive]
    OnExit {
        /// The state's index.
        state: usize,
        /// Which of its `<onexit>` blocks.
        ordinal: usize,
    },
    /// A transition's own content.
    #[non_exhaustive]
    Transition {
        /// The transition's index.
        transition: usize,
    },
    /// An `<invoke>`'s `<finalize>` block.
    #[non_exhaustive]
    Finalize {
        /// The index of the state holding the `<invoke>`.
        state: usize,
        /// Which of the state's `<invoke>` elements.
        invoke: usize,
    },
}

/// One segment of a resolved datamodel location.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PathSegment {
    /// A property, or the root's name: `b` in `a.b`, `'b'` in `a['b']`.
    Key(String),
    /// An integer index or key: `0` in `a[0]`.
    Index(i64),
}
