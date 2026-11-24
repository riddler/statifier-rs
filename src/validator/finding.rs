//! What the validator reports: an error, which gates compilation, and a
//! warning, which does not. Both carry a reason with the detail it names, a
//! message for a person, and the span reported.

use crate::xml::Location;

/// The slot a default transition came from: a state's `<initial>` element or
/// a `<history>`'s own transitions, with the owning state's id.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefaultTransitionOwner {
    /// The `<initial>` element of the state with this id.
    Initial(Option<String>),
    /// The `<history>` with this id.
    History(Option<String>),
}

/// Why a document does not validate, with the detail each reason carries.
/// Each variant is one of the reference's reasons, under its name.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// A state or `<data>` id written after its first occurrence.
    #[non_exhaustive]
    DuplicateId {
        /// The repeated id.
        id: String,
    },
    /// A state or `<data>` written with `id=""`.
    EmptyId,
    /// A transition target that names no state.
    #[non_exhaustive]
    UnresolvedTarget {
        /// The id that does not resolve.
        id: String,
    },
    /// An `initial` attribute, the state's or the root's, that names no
    /// state.
    #[non_exhaustive]
    UnresolvedInitial {
        /// The id that does not resolve.
        id: String,
    },
    /// An initial or default target outside the state it initializes.
    #[non_exhaustive]
    InitialNotDescendant {
        /// The target.
        id: String,
        /// The state the target should be inside.
        parent_id: Option<String>,
    },
    /// An initial on a state with no child states to enter.
    #[non_exhaustive]
    InitialOnAtomicState {
        /// The state's id.
        id: Option<String>,
    },
    /// A state with both an `initial` attribute and an `<initial>` element.
    #[non_exhaustive]
    InitialAttributeAndElement {
        /// The state's id.
        id: Option<String>,
    },
    /// An `<initial>` or `<history>` that holds other than one transition.
    #[non_exhaustive]
    TransitionCount {
        /// Whose default transition.
        owner: DefaultTransitionOwner,
        /// How many it holds.
        count: usize,
    },
    /// A default transition with no target.
    #[non_exhaustive]
    TransitionMissingTarget {
        /// Whose default transition.
        owner: DefaultTransitionOwner,
    },
    /// A default transition written with `event` or `cond`.
    #[non_exhaustive]
    TransitionForbiddenAttribute {
        /// Whose default transition.
        owner: DefaultTransitionOwner,
        /// `event` or `cond`.
        attribute: &'static str,
    },
    /// A `<history>` whose parent is not a compound state.
    #[non_exhaustive]
    HistoryBadParent {
        /// The history's id.
        id: Option<String>,
        /// The parent's element name.
        parent_kind: &'static str,
    },
    /// A `<history>` `type` other than `shallow` or `deep`.
    #[non_exhaustive]
    HistoryBadType {
        /// The value as written.
        raw: String,
    },
    /// A `<transition>` `type` other than `internal` or `external`.
    #[non_exhaustive]
    TransitionBadType {
        /// The value as written.
        raw: String,
    },
    /// A root `binding` other than `early` or `late`.
    #[non_exhaustive]
    ScxmlBadBinding {
        /// The value as written.
        raw: String,
    },
    /// A state child of a `<final>`.
    #[non_exhaustive]
    FinalHasStates {
        /// The child's id.
        id: Option<String>,
    },
    /// A `<transition>` child of a `<final>`.
    #[non_exhaustive]
    FinalHasTransitions {
        /// The `<final>`'s id.
        id: Option<String>,
    },
    /// A state with no id that holds a `<final>`.
    #[non_exhaustive]
    FinalParentMissingId {
        /// The first `<final>` child's id.
        final_id: Option<String>,
    },
    /// A `<state>` entered by default whose first child is a `<history>`.
    #[non_exhaustive]
    DefaultEntryNotEnterable {
        /// The state's id.
        id: Option<String>,
        /// The first child's element name.
        child_kind: &'static str,
    },
    /// A `<donedata>` on a state that is not a `<final>`.
    #[non_exhaustive]
    DonedataNotOnFinal {
        /// The state's id.
        id: Option<String>,
    },
    /// A `<donedata>` with both `<content>` and `<param>` children.
    #[non_exhaustive]
    DonedataContentAndParams {
        /// The state's id.
        id: Option<String>,
    },
    /// A `<content>` with both `expr` and inline content.
    #[non_exhaustive]
    ContentExprAndText {
        /// The `expr` as written.
        expr: String,
    },
    /// A `<param>` with both `expr` and `location`.
    #[non_exhaustive]
    ParamExprAndLocation {
        /// The param's name.
        name: String,
    },
    /// A `<param>` with neither `expr` nor `location`.
    #[non_exhaustive]
    ParamNoValue {
        /// The param's name.
        name: String,
    },
    /// A root whose namespace is not SCXML's.
    #[non_exhaustive]
    BadNamespace {
        /// The namespace the root resolves to, if any.
        uri: Option<String>,
    },
    /// A root whose `version` is not `1.0`.
    #[non_exhaustive]
    BadVersion {
        /// The version as written, if any.
        version: Option<String>,
    },
    /// A root `datamodel` outside the values the reference accepts.
    #[non_exhaustive]
    ScxmlBadDatamodel {
        /// The value as written.
        raw: String,
    },
    /// A `<data>` with both `expr` and `src`.
    #[non_exhaustive]
    DataExprAndSrc {
        /// The data's id.
        id: String,
    },
    /// A `<data>` with `expr` or `src` and child text.
    #[non_exhaustive]
    DataValueAndChildren {
        /// The data's id.
        id: String,
    },
    /// A `<data>` id that starts with `_`.
    #[non_exhaustive]
    DataReservedId {
        /// The data's id.
        id: String,
    },
    /// A `<datamodel>` on a `<final>` or a `<history>`.
    #[non_exhaustive]
    DatamodelBadParent {
        /// The parent's element name.
        kind: &'static str,
    },
    /// An `<assign>` with both `expr` and child content.
    #[non_exhaustive]
    AssignExprAndText {
        /// The `expr` as written.
        expr: String,
    },
    /// An `<elseif>` after the `<else>` of its `<if>`.
    IfElseifAfterElse,
    /// A second `<else>` in one `<if>`.
    IfDuplicateElse,
    /// A `<script>` with neither `src` nor text.
    ScriptNoSrcOrText,
    /// An `<invoke>` with both `type` and `typeexpr`.
    InvokeTypeAndTypeexpr,
    /// An `<invoke>` with both `src` and `srcexpr`.
    InvokeSrcAndSrcexpr,
    /// An `<invoke>` with `src` or `srcexpr` and a `<content>` child.
    InvokeSrcAndContent,
    /// An `<invoke>` with both `id` and `idlocation`.
    InvokeIdAndIdlocation,
    /// An `<invoke>` with `namelist` and a `<param>` child.
    InvokeNamelistAndParam,
    /// An `<invoke>` `autoforward` other than `true` or `false`.
    #[non_exhaustive]
    InvokeBadAutoforward {
        /// The value as written.
        raw: String,
    },
    /// A `<send>` with both `event` and `eventexpr`.
    SendEventAndEventexpr,
    /// A `<send>` with both `target` and `targetexpr`.
    SendTargetAndTargetexpr,
    /// A `<send>` with both `type` and `typeexpr`.
    SendTypeAndTypeexpr,
    /// A `<send>` with both `id` and `idlocation`.
    SendIdAndIdlocation,
    /// A `<send>` with both `delay` and `delayexpr`.
    SendDelayAndDelayexpr,
    /// A `<send>` with a delay and the internal target.
    SendDelayAndInternalTarget,
    /// A `<send>` with `namelist` and a `<content>` child.
    SendNamelistAndContent,
    /// A `<send>` with `<param>` and `<content>` children.
    SendParamAndContent,
    /// A `<cancel>` with both `sendid` and `sendidexpr`.
    CancelSendidAndSendidexpr,
    /// A `<cancel>` with neither `sendid` nor `sendidexpr`.
    CancelNoSendid,
    /// An attribute name repeated on one tag.
    #[non_exhaustive]
    DuplicateAttribute {
        /// The tag's name as written.
        element: String,
        /// The attribute's name as written.
        attribute: String,
    },
}

/// One error: the reason, a message for a person, and the span reported.
/// A document with an error is not compiled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ValidationError {
    /// Why the document does not validate.
    pub(crate) reason: Reason,
    /// What is wrong, for a person.
    pub(crate) message: String,
    /// The span reported.
    pub(crate) location: Location,
}

impl ValidationError {
    pub(super) fn new(reason: Reason, location: Location) -> Self {
        ValidationError {
            message: message(&reason),
            reason,
            location,
        }
    }
}

/// Why a document draws a warning: a finding the engine has a behavior for
/// either way.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WarningReason {
    /// A `<raise>` or `<send>` inside `<finalize>`.
    #[non_exhaustive]
    FinalizeForbiddenContent {
        /// The element's name.
        element: &'static str,
    },
}

/// One warning: the reason, a message for a person, and the span reported.
/// A warning never stops compilation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Warning {
    /// What was found.
    pub(crate) reason: WarningReason,
    /// What was found, for a person.
    pub(crate) message: String,
    /// The span reported.
    pub(crate) location: Location,
}

impl Warning {
    pub(super) fn new(reason: WarningReason, location: Location) -> Self {
        let WarningReason::FinalizeForbiddenContent { element } = reason;
        Warning {
            message: format!("<{element}> must not occur inside <finalize>"),
            reason,
            location,
        }
    }
}

// A value as the reference prints one: quoted, or `nil` when absent.
fn quote(value: Option<&String>) -> String {
    value.map_or_else(|| "nil".to_owned(), |value| format!("{value:?}"))
}

fn owner_description(owner: &DefaultTransitionOwner) -> String {
    match owner {
        DefaultTransitionOwner::Initial(None) => "<initial>".to_owned(),
        DefaultTransitionOwner::Initial(Some(id)) => format!("<initial> on {id:?}"),
        DefaultTransitionOwner::History(None) => "<history>".to_owned(),
        DefaultTransitionOwner::History(Some(id)) => format!("<history> {id:?}"),
    }
}

// The reference's message for each reason.
fn message(reason: &Reason) -> String {
    const FINAL_CONTENT: &str = "only onentry, onexit, and donedata are legal there";
    match reason {
        Reason::DuplicateId { id } => format!("duplicate state id {id:?}"),
        Reason::EmptyId => {
            "state id must not be empty - an absent id is legal, an empty one is not".to_owned()
        }
        Reason::UnresolvedTarget { id } => {
            format!("transition target {id:?} does not resolve to a state in the document")
        }
        Reason::UnresolvedInitial { id } => {
            format!("initial state {id:?} does not resolve to a state in the document")
        }
        Reason::InitialNotDescendant { id, parent_id } => {
            let parent = parent_id.as_ref().map_or_else(
                || "its parent state, which has no id".to_owned(),
                |parent| format!("{parent:?}"),
            );
            format!("initial state {id:?} is not a descendant of {parent}")
        }
        Reason::InitialOnAtomicState { id } => {
            let state = id.as_ref().map_or_else(
                || "a state with no id".to_owned(),
                |id| format!("state {id:?}"),
            );
            format!("{state} has no child states to default into")
        }
        Reason::InitialAttributeAndElement { id } => format!(
            "state {} has both an initial attribute and an <initial> element - at most one is legal",
            quote(id.as_ref())
        ),
        Reason::TransitionCount { owner, count } => format!(
            "{} must have exactly one transition, found {count}",
            owner_description(owner)
        ),
        Reason::TransitionMissingTarget { owner } => format!(
            "{} transition must specify a target",
            owner_description(owner)
        ),
        Reason::TransitionForbiddenAttribute { owner, attribute } => format!(
            "{} transition must not specify {attribute}",
            owner_description(owner)
        ),
        Reason::HistoryBadParent { id, parent_kind } => format!(
            "history {} must be a child of a compound <state> or <parallel>, not {parent_kind}",
            quote(id.as_ref())
        ),
        Reason::HistoryBadType { raw } => {
            format!("history type {raw:?} must be \"shallow\" or \"deep\"")
        }
        Reason::TransitionBadType { raw } => {
            format!("transition type {raw:?} must be \"internal\" or \"external\"")
        }
        Reason::ScxmlBadBinding { raw } => format!("binding {raw:?} must be \"early\" or \"late\""),
        Reason::FinalHasStates { id } => format!(
            "state {} must not be a child of <final> - {FINAL_CONTENT}",
            quote(id.as_ref())
        ),
        Reason::FinalHasTransitions { id } => format!(
            "state {} is a <final> and must not carry a <transition> - {FINAL_CONTENT}",
            quote(id.as_ref())
        ),
        Reason::FinalParentMissingId { .. } => "a state containing a <final> child must have an \
             id - its completion event is named done.state.<id>"
            .to_owned(),
        Reason::DefaultEntryNotEnterable { id, child_kind } => format!(
            "state {} has no initial attribute or <initial> element, and its first child is a \
             {child_kind} pseudo-state, which cannot be entered by default",
            quote(id.as_ref())
        ),
        Reason::DonedataNotOnFinal { id } => format!(
            "state {} carries <donedata>, which is only legal on <final>",
            quote(id.as_ref())
        ),
        Reason::DonedataContentAndParams { id } => format!(
            "<donedata> on {} must not specify both a <content> child and one or more <param> \
             children",
            quote(id.as_ref())
        ),
        Reason::ContentExprAndText { expr } => content_message(expr, "text"),
        Reason::ParamExprAndLocation { name } => format!(
            "<param> {name:?} must not specify both an expr attribute and a location attribute"
        ),
        Reason::ParamNoValue { name } => {
            format!(
                "<param> {name:?} must specify either an expr attribute or a location attribute"
            )
        }
        Reason::BadNamespace { uri: None } => format!(
            "the root element declares no namespace - expected xmlns={:?}",
            crate::lowering::SCXML_NAMESPACE
        ),
        Reason::BadNamespace { uri: Some(uri) } => format!(
            "the root element's namespace {uri:?} is not the SCXML namespace {:?}",
            crate::lowering::SCXML_NAMESPACE
        ),
        Reason::BadVersion { version: None } => {
            "the root element has no version attribute, expected \"1.0\"".to_owned()
        }
        Reason::BadVersion {
            version: Some(version),
        } => format!("the root element's version {version:?} must be \"1.0\""),
        Reason::ScxmlBadDatamodel { raw } => format!(
            "datamodel {raw:?} must be one of \"predicator\", \"elixir\", \"null\", \
             \"ecmascript\", or \"xpath\""
        ),
        Reason::DataExprAndSrc { id } => {
            format!("<data> {id:?} must not specify both expr and src")
        }
        Reason::DataValueAndChildren { id } => {
            format!("<data> {id:?} must not specify expr or src together with child content")
        }
        Reason::DataReservedId { id } => {
            format!("<data> id {id:?} must not begin with \"_\" - that prefix is reserved")
        }
        Reason::DatamodelBadParent { kind } => {
            format!("<datamodel> must not be a child of a {kind} state")
        }
        Reason::AssignExprAndText { expr } => {
            format!("<assign> must not specify both an expr attribute ({expr:?}) and child content")
        }
        Reason::IfElseifAfterElse => {
            "<elseif> must not follow <else> - <else> must occur after all <elseif> tags".to_owned()
        }
        Reason::IfDuplicateElse => "<if> must not carry more than one <else> branch".to_owned(),
        Reason::ScriptNoSrcOrText => {
            "<script> must specify either the src attribute or child content".to_owned()
        }
        Reason::InvokeTypeAndTypeexpr => both("invoke", "type", "typeexpr"),
        Reason::InvokeSrcAndSrcexpr => both("invoke", "src", "srcexpr"),
        Reason::InvokeSrcAndContent => {
            "<invoke> must not specify src or srcexpr together with a <content> child".to_owned()
        }
        Reason::InvokeIdAndIdlocation => both("invoke", "id", "idlocation"),
        Reason::InvokeNamelistAndParam => {
            "<invoke> must not specify namelist together with a <param> child".to_owned()
        }
        Reason::InvokeBadAutoforward { raw } => {
            format!("invoke autoforward {raw:?} must be \"true\" or \"false\"")
        }
        Reason::SendEventAndEventexpr => both("send", "event", "eventexpr"),
        Reason::SendTargetAndTargetexpr => both("send", "target", "targetexpr"),
        Reason::SendTypeAndTypeexpr => both("send", "type", "typeexpr"),
        Reason::SendIdAndIdlocation => both("send", "id", "idlocation"),
        Reason::SendDelayAndDelayexpr => both("send", "delay", "delayexpr"),
        Reason::SendDelayAndInternalTarget => {
            "<send> must not specify delay or delayexpr when target is \"_internal\"".to_owned()
        }
        Reason::SendNamelistAndContent => {
            "<send> must not specify namelist together with a <content> child".to_owned()
        }
        Reason::SendParamAndContent => {
            "<send> must not specify a <param> child together with a <content> child".to_owned()
        }
        Reason::CancelSendidAndSendidexpr => both("cancel", "sendid", "sendidexpr"),
        Reason::CancelNoSendid => {
            "<cancel> must specify exactly one of sendid or sendidexpr".to_owned()
        }
        Reason::DuplicateAttribute { element, attribute } => {
            format!("duplicate attribute {attribute:?} on <{element}>")
        }
    }
}

// The `<content>` message names the form its inline content takes, `text`
// or `markup`.
pub(super) fn content_message(expr: &str, form: &str) -> String {
    format!("<content> must not specify both an expr attribute ({expr:?}) and inline {form}")
}

fn both(element: &str, first: &str, second: &str) -> String {
    format!("<{element}> must not specify both {first} and {second}")
}
