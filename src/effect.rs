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
        /// The block the write ran in; `None` for a write made outside one.
        owner: Option<BlockOwner>,
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
