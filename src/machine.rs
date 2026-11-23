//! The compiled chart: what the compiler builds from a validated document,
//! and the one thing the interpreter reads.
//!
//! Every state, transition, executable-content node and `<data>` is interned
//! to an index in document order, in its own index space: a state's index,
//! a transition's, a content node's and a `<data>`'s. A state carries its
//! parent, its children and `last`, the index of its last descendant, so its
//! descendants are exactly the indexes after its own up to `last`; the root
//! `<scxml>` is index 0. Every target and every `initial` is resolved to
//! state indexes, and every expression is compiled once, through the
//! predicator crate. Nothing in a machine changes after it is built.

use crate::document::{AttributeLocations, Binding, HistoryType, TransitionType};
use crate::refusal::{Refusal, Warning};
use crate::xml::Location;
use predicator_rs::{Instructions, Value};
use std::collections::BTreeMap;
use std::sync::Arc;

/// A compiled chart: an immutable value, cheap to clone, that any number of
/// threads may share.
///
/// [`compile`](crate::compile) answers one. Cloning a machine shares it
/// rather than copying it.
///
/// ```
/// let machine = statifier::compile(
///     r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0"><state id="a"/></scxml>"#,
/// )?;
/// let shared = machine.clone();
/// std::thread::spawn(move || assert!(shared.warnings().is_empty())).join().unwrap();
/// # Ok::<(), Vec<statifier::Refusal>>(())
/// ```
#[derive(Debug, Clone)]
pub struct Machine {
    chart: Arc<Chart>,
}

impl Machine {
    pub(crate) fn new(chart: Chart) -> Self {
        Machine {
            chart: Arc::new(chart),
        }
    }

    /// The warnings the chart drew, in document order.
    pub fn warnings(&self) -> &[Warning] {
        &self.chart.warnings
    }

    #[cfg(test)]
    pub(crate) fn chart(&self) -> &Chart {
        &self.chart
    }
}

/// What a machine holds.
#[derive(Debug, PartialEq)]
pub(crate) struct Chart {
    /// Every state, the root at index 0, in document order.
    pub(crate) states: Vec<State>,
    /// The index of every state with an id.
    pub(crate) id_to_index: BTreeMap<String, usize>,
    /// Every transition, in document order: a state's own, then its
    /// `<initial>` element's, a history's being its default.
    pub(crate) transitions: Vec<Transition>,
    /// Every executable-content node, a container before its children.
    pub(crate) contents: Vec<Content>,
    /// Every `<data>`, the root's first.
    pub(crate) data: Vec<Data>,
    /// The root's `<script>` children, each compiled as a program.
    pub(crate) global_scripts: Vec<Deferred<Program>>,
    /// The root's `name`.
    pub(crate) name: Option<String>,
    /// The root's `datamodel`.
    pub(crate) datamodel: Option<String>,
    /// The root's `binding`.
    pub(crate) binding: Binding,
    /// The span of the root.
    pub(crate) location: Location,
    /// The validator's warnings.
    pub(crate) warnings: Vec<Warning>,
}

/// Which element made a state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// The `<scxml>` root.
    Scxml,
    /// `<state>`.
    State,
    /// `<parallel>`.
    Parallel,
    /// `<final>`.
    Final,
    /// `<history>`.
    History,
}

/// One interned state.
#[derive(Debug, PartialEq)]
pub(crate) struct State {
    /// Its index.
    pub(crate) index: usize,
    /// Its id, if it has one.
    pub(crate) id: Option<String>,
    /// Which element made it.
    pub(crate) kind: Kind,
    /// Its parent's index; `None` for the root.
    pub(crate) parent: Option<usize>,
    /// The index of its last descendant, its own when it has none.
    pub(crate) last: usize,
    /// Its child states' indexes, in document order.
    pub(crate) children: Vec<usize>,
    /// The states it enters by default: its `initial` attribute's, else its
    /// `<initial>` element's target, else its first child; empty for any
    /// kind but `<state>` and the root.
    pub(crate) initial: Vec<usize>,
    /// A history's type.
    pub(crate) history_type: Option<HistoryType>,
    /// Its `<history>` children's indexes.
    pub(crate) history_children: Vec<usize>,
    /// Its selectable transitions' indexes.
    pub(crate) transitions: Vec<usize>,
    /// Its `<onentry>` blocks.
    pub(crate) onentry: Vec<Block>,
    /// Its `<onexit>` blocks.
    pub(crate) onexit: Vec<Block>,
    /// Its `<initial>` element's transition.
    pub(crate) initial_transition: Option<usize>,
    /// A history's default transition.
    pub(crate) history_default: Option<usize>,
    /// A final state's `<donedata>`.
    pub(crate) donedata: Option<Donedata>,
    /// The indexes of the `<data>` its own `<datamodel>` declares.
    pub(crate) data: Vec<usize>,
    /// Its `<invoke>` elements.
    pub(crate) invoke: Vec<Invoke>,
    /// The span of its element.
    pub(crate) location: Location,
    /// The spans of its written attributes.
    pub(crate) attribute_locations: AttributeLocations,
}

/// An expression compiled through the predicator crate, or a literal value.
#[derive(Debug, PartialEq)]
pub(crate) enum Expr {
    /// An expression, compiled, with its source.
    Compiled {
        /// The instructions it runs as.
        instructions: Instructions,
        /// The source as written.
        source: String,
    },
    /// A value with nothing to evaluate.
    Static(Value),
}

/// A statement program compiled through the predicator crate.
#[derive(Debug, PartialEq)]
pub(crate) struct Program {
    /// The instructions it runs as.
    pub(crate) instructions: Instructions,
    /// The source as written.
    pub(crate) source: String,
}

/// A compiled value whose compile failure the reference defers to when it
/// executes.
#[derive(Debug, PartialEq)]
pub(crate) enum Deferred<T> {
    /// It compiled.
    Ready(T),
    /// It did not; executing it fails with this refusal.
    Invalid(Box<Refusal>),
}

/// One interned transition.
#[derive(Debug, PartialEq)]
pub(crate) struct Transition {
    /// The index of the state it belongs to.
    pub(crate) source: usize,
    /// Its targets' indexes.
    pub(crate) targets: Vec<usize>,
    /// Each event descriptor split on its dots.
    pub(crate) events: Vec<Vec<String>>,
    /// Its condition.
    pub(crate) cond: Option<Expr>,
    /// The span of its condition.
    pub(crate) cond_location: Option<Location>,
    /// Its type.
    pub(crate) transition_type: TransitionType,
    /// Its executable content's indexes.
    pub(crate) content: Vec<usize>,
    /// The span of its element.
    pub(crate) location: Location,
    /// The spans of its written attributes.
    pub(crate) attribute_locations: AttributeLocations,
}

/// An `<onentry>`, `<onexit>` or `<finalize>` block.
#[derive(Debug, PartialEq)]
pub(crate) struct Block {
    /// The span of its element.
    pub(crate) location: Location,
    /// Its executable content's indexes.
    pub(crate) content: Vec<usize>,
}

/// One interned executable-content node.
#[derive(Debug, PartialEq)]
pub(crate) enum Content {
    /// `<raise>`.
    Raise {
        /// The event.
        event: String,
        /// The span of the element.
        location: Location,
    },
    /// `<log>`.
    Log {
        /// The label.
        label: Option<String>,
        /// The expression.
        expr: Option<Expr>,
        /// The span of the element.
        location: Location,
        /// The span of the expression.
        expr_location: Option<Location>,
    },
    /// `<assign>`.
    Assign(Assign),
    /// `<if>`.
    If {
        /// The span of the element.
        location: Location,
        /// The branches, the `<if>`'s own first.
        branches: Vec<Branch>,
    },
    /// `<foreach>`.
    Foreach(Foreach),
    /// `<script>`.
    Script {
        /// The body.
        program: Deferred<Program>,
        /// The span of the element.
        location: Location,
    },
    /// `<send>`.
    Send(Box<Send>),
    /// `<cancel>`.
    Cancel {
        /// The send id, a literal or an expression.
        sendid: Option<Expr>,
        /// The span of the element.
        location: Location,
        /// The spans of its written attributes.
        attribute_locations: AttributeLocations,
    },
}

/// A compiled `<assign>`.
#[derive(Debug, PartialEq)]
pub(crate) struct Assign {
    /// The `location` attribute, raw: it is resolved when it executes.
    pub(crate) location: String,
    /// The span of the element.
    pub(crate) node_location: Location,
    /// The value: the expression, else the markup or text as a value.
    pub(crate) value: Deferred<Expr>,
    /// The span of the `location` attribute.
    pub(crate) location_location: Location,
    /// The span of the expression.
    pub(crate) expr_location: Option<Location>,
}

/// One branch of an `<if>`.
#[derive(Debug, PartialEq)]
pub(crate) struct Branch {
    /// The condition; `None` for an `<else>`.
    pub(crate) cond: Option<Expr>,
    /// The span of the condition.
    pub(crate) cond_location: Option<Location>,
    /// Its executable content's indexes.
    pub(crate) content: Vec<usize>,
}

/// A compiled `<foreach>`.
#[derive(Debug, PartialEq)]
pub(crate) struct Foreach {
    /// The span of the element.
    pub(crate) location: Location,
    /// The array expression.
    pub(crate) array: Expr,
    /// The span of the array expression.
    pub(crate) array_location: Location,
    /// The `item` location, raw.
    pub(crate) item: String,
    /// The span of `item`.
    pub(crate) item_location: Location,
    /// The `index` location, raw.
    pub(crate) index: Option<String>,
    /// The span of `index`.
    pub(crate) index_location: Option<Location>,
    /// Its executable content's indexes.
    pub(crate) content: Vec<usize>,
}

/// A compiled `<send>`.
#[derive(Debug, PartialEq)]
pub(crate) struct Send {
    /// The span of the element.
    pub(crate) location: Location,
    /// The event, a literal or an expression.
    pub(crate) event: Option<Expr>,
    /// The target, a literal or an expression.
    pub(crate) target: Option<Expr>,
    /// The type, a literal or an expression.
    pub(crate) send_type: Option<Expr>,
    /// The `id` attribute.
    pub(crate) id: Option<String>,
    /// The `idlocation` attribute, raw.
    pub(crate) idlocation: Option<String>,
    /// The delay, a literal or an expression.
    pub(crate) delay: Option<Expr>,
    /// The `namelist` entries.
    pub(crate) namelist: Vec<Param>,
    /// The `<param>` children.
    pub(crate) params: Vec<Param>,
    /// The `<content>` value.
    pub(crate) content: Option<Expr>,
    /// The spans of its written attributes.
    pub(crate) attribute_locations: AttributeLocations,
}

/// Which attribute a param's value came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ParamKind {
    /// `expr`.
    Expr,
    /// `location`, or a `namelist` entry.
    Location,
}

/// A compiled `<param>` or `namelist` entry.
#[derive(Debug, PartialEq)]
pub(crate) struct Param {
    /// The name.
    pub(crate) name: String,
    /// Which attribute the value came from.
    pub(crate) kind: ParamKind,
    /// The value; only a `namelist` entry's is ever deferred.
    pub(crate) expr: Deferred<Expr>,
    /// The span of the value's attribute.
    pub(crate) expr_location: Location,
    /// The span of the element.
    pub(crate) location: Location,
}

/// A compiled `<data>`.
#[derive(Debug, PartialEq)]
pub(crate) struct Data {
    /// The id.
    pub(crate) id: String,
    /// The value.
    pub(crate) value: DataValue,
    /// The span of the element.
    pub(crate) location: Location,
    /// The span of the attribute the value came from, else the element's.
    pub(crate) value_location: Location,
}

/// Where a `<data>`'s value comes from.
#[derive(Debug, PartialEq)]
pub(crate) enum DataValue {
    /// An expression or a literal.
    Expr(Expr),
    /// A `src` to fetch.
    Src(String),
    /// An expression that did not compile.
    Invalid(Box<Refusal>),
}

/// A compiled `<donedata>`.
#[derive(Debug, PartialEq)]
pub(crate) struct Donedata {
    /// The span of the element.
    pub(crate) location: Location,
    /// The `<content>` value.
    pub(crate) expr: Option<Expr>,
    /// The span of the `<content>` value.
    pub(crate) expr_location: Option<Location>,
    /// The `<param>` children.
    pub(crate) params: Vec<Param>,
}

/// A compiled `<invoke>`.
#[derive(Debug, PartialEq)]
pub(crate) struct Invoke {
    /// Its position among its state's `<invoke>` elements.
    pub(crate) index: usize,
    /// The span of the element.
    pub(crate) location: Location,
    /// The type, a literal or an expression.
    pub(crate) invoke_type: Option<Expr>,
    /// The source, a literal or an expression.
    pub(crate) src: Option<Expr>,
    /// The `id` attribute.
    pub(crate) id: Option<String>,
    /// The `idlocation` attribute, raw.
    pub(crate) idlocation: Option<String>,
    /// The `namelist` entries.
    pub(crate) namelist: Vec<Param>,
    /// The `<param>` children.
    pub(crate) params: Vec<Param>,
    /// The `<content>` value.
    pub(crate) content: Option<Expr>,
    /// The `autoforward` attribute.
    pub(crate) autoforward: bool,
    /// The `<finalize>` block.
    pub(crate) finalize: Option<Block>,
    /// The spans of its written attributes.
    pub(crate) attribute_locations: AttributeLocations,
}
