//! The typed document: what lowering builds from the element tree.
//!
//! One node type per SCXML element the reference reads, each holding the
//! attributes the reference reads as it wrote them: expressions, conditions
//! and locations are the raw source text, uncompiled, and an attribute list
//! (`event`, `target`, `initial`, `namelist`) is split on whitespace. Every
//! node carries the span of its element, and every list keeps document order.
//!
//! The document is the pre-validation shape. It can hold what the
//! specification forbids (two `<initial>` transitions, `<donedata>` on a
//! state that is not final, `expr` beside a `<content>` body) because
//! reporting those is the validator's job, not lowering's.
//!
//! An attribute's value span is recorded in a node's `attribute_locations`
//! only when the attribute was written. A field with a default (`binding`,
//! a transition's `type`, a history's `type`, `autoforward`) and no entry
//! there took its default; the entry is how a later check tells a written
//! value from a defaulted one.

use crate::xml::Location;
use std::collections::BTreeMap;

/// The value spans of the attributes written on a node, keyed by attribute
/// name. Only attributes the node's element reads have an entry.
pub(crate) type AttributeLocations = BTreeMap<&'static str, Location>;

/// The `<scxml>` root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Document {
    /// The span of the `<scxml>` element.
    pub(crate) location: Location,
    /// The `name` attribute.
    pub(crate) name: Option<String>,
    /// The `version` attribute.
    pub(crate) version: Option<String>,
    /// The `xmlns` attribute as written; a prefixed root has none.
    pub(crate) xmlns: Option<String>,
    /// The namespace the root's name resolves to, if any.
    pub(crate) namespace: Option<String>,
    /// The `datamodel` attribute.
    pub(crate) datamodel: Option<String>,
    /// The `binding` attribute, `early` unless `late` is written.
    pub(crate) binding: Binding,
    /// The `initial` attribute, split on whitespace.
    pub(crate) initial: Vec<String>,
    /// The `<state>`, `<parallel>`, `<final>` and `<history>` children.
    pub(crate) states: Vec<State>,
    /// The last `<datamodel>` child, if any.
    pub(crate) datamodel_element: Option<Datamodel>,
    /// The `<script>` children.
    pub(crate) scripts: Vec<Script>,
    /// The spans of `initial`, `name`, `datamodel`, `binding`, `version` and
    /// `xmlns`, where written.
    pub(crate) attribute_locations: AttributeLocations,
}

/// When the datamodel binds its values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Binding {
    /// At initialization, every `<data>` of the document at once.
    Early,
    /// Each state's `<data>` when the state is first entered.
    Late,
}

/// The kind of a state node: the name of the element that made it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StateKind {
    /// `<state>`.
    State,
    /// `<parallel>`.
    Parallel,
    /// `<final>`.
    Final,
    /// `<history>`.
    History,
}

/// What a `<history>` records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HistoryType {
    /// The active direct children of its parent.
    Shallow,
    /// The active atomic descendants of its parent.
    Deep,
}

/// One `<state>`, `<parallel>`, `<final>` or `<history>`. Every field is
/// present on every kind; which kinds may hold which children is the
/// validator's to check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct State {
    /// Which element made the node.
    pub(crate) kind: StateKind,
    /// The span of the element.
    pub(crate) location: Location,
    /// The `id` attribute.
    pub(crate) id: Option<String>,
    /// The `initial` attribute, split on whitespace.
    pub(crate) initial: Vec<String>,
    /// The last `<initial>` child, if any.
    pub(crate) initial_element: Option<Initial>,
    /// The state children.
    pub(crate) states: Vec<State>,
    /// The `<transition>` children.
    pub(crate) transitions: Vec<Transition>,
    /// One block per `<onentry>` child.
    pub(crate) onentry: Vec<Block>,
    /// One block per `<onexit>` child.
    pub(crate) onexit: Vec<Block>,
    /// A history's `type`, `shallow` unless `deep` is written; `None` on
    /// every other kind.
    pub(crate) history_type: Option<HistoryType>,
    /// The last `<donedata>` child, if any.
    pub(crate) donedata: Option<Donedata>,
    /// The last `<datamodel>` child, if any.
    pub(crate) datamodel_element: Option<Datamodel>,
    /// The `<invoke>` children.
    pub(crate) invoke: Vec<Invoke>,
    /// The spans of `id`, `initial` and a history's `type`, where written.
    pub(crate) attribute_locations: AttributeLocations,
}

/// An `<initial>` child of a state: a slot on its parent, not a state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Initial {
    /// The span of the element.
    pub(crate) location: Location,
    /// The `<transition>` children, however many are written.
    pub(crate) transitions: Vec<Transition>,
}

/// Whether a transition exits its source state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransitionType {
    /// It does not exit a compound source whose descendants it targets.
    Internal,
    /// It exits its source.
    External,
}

/// A `<transition>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Transition {
    /// The span of the element.
    pub(crate) location: Location,
    /// The `event` attribute, split on whitespace.
    pub(crate) event: Vec<String>,
    /// The `target` attribute, split on whitespace.
    pub(crate) target: Vec<String>,
    /// The `cond` attribute, raw.
    pub(crate) cond: Option<String>,
    /// The `type` attribute, `external` unless `internal` is written.
    pub(crate) transition_type: TransitionType,
    /// The executable content, unwrapped.
    pub(crate) content: Vec<Executable>,
    /// The spans of `event`, `target`, `cond` and `type`, where written.
    pub(crate) attribute_locations: AttributeLocations,
}

/// One `<onentry>`, `<onexit>` or `<finalize>` element's executable content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Block {
    /// The span of the element.
    pub(crate) location: Location,
    /// The executable content.
    pub(crate) content: Vec<Executable>,
}

/// One element of executable content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Executable {
    /// `<raise>`.
    Raise(Raise),
    /// `<log>`.
    Log(Log),
    /// `<assign>`.
    Assign(Assign),
    /// `<if>`, with its `<elseif>` and `<else>` partitions.
    If(If),
    /// `<foreach>`.
    Foreach(Foreach),
    /// `<script>`.
    Script(Script),
    /// `<send>`, boxed: it is the largest node by far.
    Send(Box<Send>),
    /// `<cancel>`.
    Cancel(Cancel),
}

impl Executable {
    /// The name of the element that made the node.
    pub(crate) fn element_name(&self) -> &'static str {
        match self {
            Executable::Raise(_) => "raise",
            Executable::Log(_) => "log",
            Executable::Assign(_) => "assign",
            Executable::If(_) => "if",
            Executable::Foreach(_) => "foreach",
            Executable::Script(_) => "script",
            Executable::Send(_) => "send",
            Executable::Cancel(_) => "cancel",
        }
    }

    /// The span of the element that made the node.
    pub(crate) fn location(&self) -> Location {
        match self {
            Executable::Raise(node) => node.location,
            Executable::Log(node) => node.location,
            Executable::Assign(node) => node.node_location,
            Executable::If(node) => node.location,
            Executable::Foreach(node) => node.location,
            Executable::Script(node) => node.location,
            Executable::Send(node) => node.location,
            Executable::Cancel(node) => node.location,
        }
    }
}

/// A `<raise>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Raise {
    /// The span of the element.
    pub(crate) location: Location,
    /// The `event` attribute, one name, never split.
    pub(crate) event: String,
    /// The span of `event`.
    pub(crate) attribute_locations: AttributeLocations,
}

/// A `<log>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Log {
    /// The span of the element.
    pub(crate) location: Location,
    /// The `label` attribute.
    pub(crate) label: Option<String>,
    /// The `expr` attribute, raw.
    pub(crate) expr: Option<String>,
    /// The spans of `label` and `expr`, where written.
    pub(crate) attribute_locations: AttributeLocations,
}

/// An `<assign>`. `location` is the attribute; the element's span is
/// `node_location`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Assign {
    /// The span of the element.
    pub(crate) node_location: Location,
    /// The `location` attribute, raw.
    pub(crate) location: String,
    /// The `expr` attribute, raw.
    pub(crate) expr: Option<String>,
    /// The direct text children joined, untrimmed.
    pub(crate) text: String,
    /// The source from the first to the last child that is not blank text,
    /// when one of them is an element; line breaks are not folded.
    pub(crate) markup: Option<String>,
    /// The span of `markup`.
    pub(crate) markup_location: Option<Location>,
    /// The spans of `location` and `expr`, where written.
    pub(crate) attribute_locations: AttributeLocations,
}

/// An `<if>`: one branch for the `<if>` itself, then one per `<elseif>` and
/// `<else>` child, each holding the content up to the next partition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct If {
    /// The span of the `<if>` element.
    pub(crate) location: Location,
    /// The branches, the `<if>`'s own first.
    pub(crate) branches: Vec<Branch>,
}

/// One branch of an `<if>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Branch {
    /// The span of the `<if>`, `<elseif>` or `<else>` element that opens it.
    pub(crate) location: Location,
    /// The `cond` attribute, raw; `None` for an `<else>`.
    pub(crate) cond: Option<String>,
    /// The executable content of the branch.
    pub(crate) content: Vec<Executable>,
    /// The span of `cond`, where written.
    pub(crate) attribute_locations: AttributeLocations,
}

/// A `<foreach>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Foreach {
    /// The span of the element.
    pub(crate) location: Location,
    /// The `array` attribute, raw.
    pub(crate) array: String,
    /// The `item` attribute, raw.
    pub(crate) item: String,
    /// The `index` attribute, raw.
    pub(crate) index: Option<String>,
    /// The executable content.
    pub(crate) content: Vec<Executable>,
    /// The spans of `array`, `item` and `index`, where written.
    pub(crate) attribute_locations: AttributeLocations,
}

/// A `<script>` with no `src`: lowering refuses one with a `src`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Script {
    /// The span of the element.
    pub(crate) location: Location,
    /// The direct text children joined, untrimmed.
    pub(crate) text: String,
}

/// A `<send>`. Attributes the specification makes exclusive are all kept;
/// the validator reports the pairs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Send {
    /// The span of the element.
    pub(crate) location: Location,
    /// The `event` attribute.
    pub(crate) event: Option<String>,
    /// The `eventexpr` attribute, raw.
    pub(crate) eventexpr: Option<String>,
    /// The `target` attribute.
    pub(crate) target: Option<String>,
    /// The `targetexpr` attribute, raw.
    pub(crate) targetexpr: Option<String>,
    /// The `type` attribute.
    pub(crate) send_type: Option<String>,
    /// The `typeexpr` attribute, raw.
    pub(crate) typeexpr: Option<String>,
    /// The `id` attribute.
    pub(crate) id: Option<String>,
    /// The `idlocation` attribute, raw.
    pub(crate) idlocation: Option<String>,
    /// The `delay` attribute.
    pub(crate) delay: Option<String>,
    /// The `delayexpr` attribute, raw.
    pub(crate) delayexpr: Option<String>,
    /// The `namelist` attribute, split on whitespace.
    pub(crate) namelist: Vec<String>,
    /// The `<param>` children.
    pub(crate) params: Vec<Param>,
    /// The last `<content>` child, if any.
    pub(crate) content: Option<Content>,
    /// The spans of the eleven attributes, where written.
    pub(crate) attribute_locations: AttributeLocations,
}

/// A `<cancel>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Cancel {
    /// The span of the element.
    pub(crate) location: Location,
    /// The `sendid` attribute.
    pub(crate) sendid: Option<String>,
    /// The `sendidexpr` attribute, raw.
    pub(crate) sendidexpr: Option<String>,
    /// The spans of `sendid` and `sendidexpr`, where written.
    pub(crate) attribute_locations: AttributeLocations,
}

/// A `<datamodel>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Datamodel {
    /// The span of the element.
    pub(crate) location: Location,
    /// The `<data>` children.
    pub(crate) data: Vec<Data>,
}

/// A `<data>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Data {
    /// The span of the element.
    pub(crate) location: Location,
    /// The `id` attribute.
    pub(crate) id: String,
    /// The `expr` attribute, raw.
    pub(crate) expr: Option<String>,
    /// The `src` attribute.
    pub(crate) src: Option<String>,
    /// The direct text children joined, untrimmed.
    pub(crate) text: String,
    /// The spans of `id`, `expr` and `src`, where written.
    pub(crate) attribute_locations: AttributeLocations,
}

/// A `<param>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Param {
    /// The span of the element.
    pub(crate) location: Location,
    /// The `name` attribute.
    pub(crate) name: String,
    /// The `expr` attribute, raw.
    pub(crate) expr: Option<String>,
    /// The `location` attribute, raw.
    pub(crate) param_location: Option<String>,
    /// The spans of `name`, `expr` and `location`, where written.
    pub(crate) attribute_locations: AttributeLocations,
}

/// A `<content>`. `expr`, text and markup are all kept; the validator
/// reports a document that gives more than one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Content {
    /// The span of the element.
    pub(crate) location: Location,
    /// The `expr` attribute, raw.
    pub(crate) expr: Option<String>,
    /// The direct text children joined, untrimmed.
    pub(crate) text: String,
    /// The source from the first to the last child that is not blank text,
    /// when one of them is an element; line breaks are not folded.
    pub(crate) markup: Option<String>,
    /// The span of `markup`.
    pub(crate) markup_location: Option<Location>,
    /// The span of `expr`, where written.
    pub(crate) attribute_locations: AttributeLocations,
}

/// A `<donedata>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Donedata {
    /// The span of the element.
    pub(crate) location: Location,
    /// The last `<content>` child, if any.
    pub(crate) content: Option<Content>,
    /// The `<param>` children.
    pub(crate) params: Vec<Param>,
}

/// An `<invoke>`. Attributes the specification makes exclusive are all kept;
/// the validator reports the pairs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Invoke {
    /// The span of the element.
    pub(crate) location: Location,
    /// The `type` attribute.
    pub(crate) invoke_type: Option<String>,
    /// The `typeexpr` attribute, raw.
    pub(crate) typeexpr: Option<String>,
    /// The `src` attribute.
    pub(crate) src: Option<String>,
    /// The `srcexpr` attribute, raw.
    pub(crate) srcexpr: Option<String>,
    /// The `id` attribute.
    pub(crate) id: Option<String>,
    /// The `idlocation` attribute, raw.
    pub(crate) idlocation: Option<String>,
    /// The `namelist` attribute, split on whitespace.
    pub(crate) namelist: Vec<String>,
    /// The `autoforward` attribute, false unless `true` is written.
    pub(crate) autoforward: bool,
    /// The `<param>` children.
    pub(crate) params: Vec<Param>,
    /// The last `<content>` child, if any.
    pub(crate) content: Option<Content>,
    /// The last `<finalize>` child, if any; an empty `<finalize/>` is an
    /// empty block, not `None`.
    pub(crate) finalize: Option<Block>,
    /// The spans of the eight attributes, where written.
    pub(crate) attribute_locations: AttributeLocations,
}
