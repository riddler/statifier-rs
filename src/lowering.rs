//! The element tree in, the typed document out.
//!
//! The second step of reading a chart, a port of the reference's lowering
//! at the vendored tag. One walk: every child element is dispatched by its
//! local name to one builder, each builder lowers its own children first and
//! then places each lowered child into a slot of its own node, and a child
//! the node has no slot for is reported as misplaced. No builder is told its
//! parent's name.
//!
//! Which elements are SCXML's own is decided by the namespace the reader
//! resolved, never by the name as written: an element whose name resolves to
//! the SCXML namespace, or to no namespace at all, is SCXML's vocabulary, so
//! a chart that declares no namespace lowers exactly as its declared twin
//! does, and a prefixed root lowers as `<scxml>` does.
//!
//! What is refused, each as a value with a reason and a location:
//!
//! - an element in SCXML's vocabulary that no builder reads, `<scxml>`
//!   below the root included;
//! - an element in any other namespace, except inside `<content>` and
//!   `<assign>`, whose element children are kept as opaque source text, and
//!   inside `<data>`, `<script>`, `<elseif>` and `<else>`, where any element
//!   child is misplaced;
//! - a root that is not `<scxml>`;
//! - a lowered child its parent has no slot for;
//! - text that is not blank where the content model has no room for it;
//!   `<content>`, `<data>`, `<assign>`, `<script>`, `<elseif>` and `<else>`
//!   read no stray text;
//! - a required attribute that is absent, any attribute on `<else>`, and a
//!   `<script>` with a `src`, which nothing fetches;
//! - an element nested deeper than [`MAX_DEPTH`] below the root, which the
//!   reference does not refuse.
//!
//! An attribute the reference does not read is ignored, and the first of
//! two attributes with one name is the one read.
//!
//! The walk does not stop at the first error: every error in the tree is
//! collected, and the document is answered only when there is none. The
//! errors are answered sorted by where they start; two that start at one
//! offset keep the order the reference's accumulation gives them.
//!
//! Blank is the reference's reading: text is blank when trimming Unicode
//! whitespace leaves nothing, and an attribute list splits on Unicode
//! whitespace other than the three no-break spaces.

mod error;

#[cfg(test)]
mod tests;

pub use error::Reason;
pub(crate) use error::{LoweringError, MAX_DEPTH};

use crate::document::{
    Assign, AttributeLocations, Binding, Block, Branch, Cancel, Content, Data, Datamodel, Document,
    Donedata, Executable, Foreach, HistoryType, If, Initial, Invoke, Log, Param, Raise, Script,
    Send, State, StateKind, Transition, TransitionType,
};
use crate::xml::{Element, Location, Node};

/// The SCXML namespace.
pub(crate) const SCXML_NAMESPACE: &str = "http://www.w3.org/2005/07/scxml";

/// Lowers the element tree `root`, read from `source`, into its typed
/// document. `<content>` and `<assign>` slice their markup out of `source`.
///
/// Answers the document only when the whole walk found no error; otherwise
/// every error found, sorted by where it starts.
pub(crate) fn lower(root: &Element, source: &str) -> Result<Document, Vec<LoweringError>> {
    if !is_scxml_vocabulary(root.namespace.as_deref()) {
        return Err(vec![foreign(root)]);
    }
    let name = local_name(&root.name);
    if name != "scxml" {
        return Err(vec![LoweringError::unexpected_root(name, root.location)]);
    }
    let ctx = Context { source, depth: 0 };
    let (document, mut errors) = build_scxml(root, &ctx);
    if errors.is_empty() {
        Ok(document)
    } else {
        // A stable sort, as the reference's is.
        errors.sort_by_key(|error| error.location.start_offset);
        Err(errors)
    }
}

// ---- the walk

struct Context<'s> {
    source: &'s str,
    // How far below the root the element being built is.
    depth: usize,
}

// What a builder hands its parent: the node, tagged with the slot it goes in.
enum Lowered {
    State(State),
    Transition(Transition),
    Initial(Initial),
    Onentry(Block),
    Onexit(Block),
    Finalize(Block),
    Donedata(Donedata),
    Datamodel(Datamodel),
    Data(Data),
    Param(Param),
    Content(Content),
    Invoke(Invoke),
    Executable(Executable),
    Elseif(Branch),
    Else(Branch),
}

impl Lowered {
    // The name a misplaced child is reported under: its slot's, or for
    // executable content the node's own. The four state kinds share one
    // slot, so a misplaced one of any kind is reported as `state`.
    fn name(&self) -> &'static str {
        match self {
            Lowered::State(_) => "state",
            Lowered::Transition(_) => "transition",
            Lowered::Initial(_) => "initial",
            Lowered::Onentry(_) => "onentry",
            Lowered::Onexit(_) => "onexit",
            Lowered::Finalize(_) => "finalize",
            Lowered::Donedata(_) => "donedata",
            Lowered::Datamodel(_) => "datamodel",
            Lowered::Data(_) => "data",
            Lowered::Param(_) => "param",
            Lowered::Content(_) => "content",
            Lowered::Invoke(_) => "invoke",
            Lowered::Executable(node) => node.element_name(),
            Lowered::Elseif(_) => "elseif",
            Lowered::Else(_) => "else",
        }
    }

    fn location(&self) -> Location {
        match self {
            Lowered::State(node) => node.location,
            Lowered::Transition(node) => node.location,
            Lowered::Initial(node) => node.location,
            Lowered::Onentry(node) | Lowered::Onexit(node) | Lowered::Finalize(node) => {
                node.location
            }
            Lowered::Donedata(node) => node.location,
            Lowered::Datamodel(node) => node.location,
            Lowered::Data(node) => node.location,
            Lowered::Param(node) => node.location,
            Lowered::Content(node) => node.location,
            Lowered::Invoke(node) => node.location,
            Lowered::Executable(node) => node.location(),
            Lowered::Elseif(node) | Lowered::Else(node) => node.location,
        }
    }
}

// A builder's answer: the lowered node, or none when it cannot be built, and
// the errors found in the element and below it.
type Built = (Option<Lowered>, Vec<LoweringError>);

// The builder for a local name, run on `element`, or `None` when no builder
// reads the name. `<scxml>` has none: it is built only as the root.
fn dispatch(name: &str, element: &Element, ctx: &Context<'_>) -> Option<Built> {
    let built = match name {
        "state" => build_state(element, ctx, StateKind::State),
        "parallel" => build_state(element, ctx, StateKind::Parallel),
        "final" => build_state(element, ctx, StateKind::Final),
        "history" => build_state(element, ctx, StateKind::History),
        "initial" => build_initial(element, ctx),
        "transition" => build_transition(element, ctx),
        "onentry" => build_block(element, ctx, Lowered::Onentry),
        "onexit" => build_block(element, ctx, Lowered::Onexit),
        "finalize" => build_block(element, ctx, Lowered::Finalize),
        "raise" => build_raise(element, ctx),
        "log" => build_log(element, ctx),
        "donedata" => build_donedata(element, ctx),
        "content" => build_content(element, ctx),
        "param" => build_param(element, ctx),
        "datamodel" => build_datamodel(element, ctx),
        "data" => build_data(element),
        "assign" => build_assign(element, ctx),
        "if" => build_if(element, ctx),
        "elseif" => build_elseif(element),
        "else" => build_else(element),
        "foreach" => build_foreach(element, ctx),
        "script" => build_script(element),
        "invoke" => build_invoke(element, ctx),
        "send" => build_send(element, ctx),
        "cancel" => build_cancel(element, ctx),
        _ => return None,
    };
    Some(built)
}

// Lowers `element`'s children: each child element through its builder, each
// run of text by the stray-text rule. Answers the lowered children in
// document order, and the errors in the reverse of the order they were found
// in, as the reference's accumulation answers them.
fn walk_children(element: &Element, ctx: &Context<'_>) -> (Vec<Lowered>, Vec<LoweringError>) {
    let child_ctx = Context {
        source: ctx.source,
        depth: ctx.depth + 1,
    };
    let mut lowered = Vec::new();
    let mut found = Vec::new();
    for child in &element.children {
        match child {
            Node::Text(text) => {
                if !is_blank(&text.value) {
                    found.push(LoweringError::stray_text(text.value.trim(), text.location));
                }
            }
            Node::Element(child) => {
                if child_ctx.depth > MAX_DEPTH {
                    found.push(LoweringError::nesting_too_deep(&child.name, child.location));
                } else if !is_scxml_vocabulary(child.namespace.as_deref()) {
                    found.push(foreign(child));
                } else if let Some((node, errors)) =
                    dispatch(local_name(&child.name), child, &child_ctx)
                {
                    found.extend(errors);
                    lowered.extend(node);
                } else {
                    found.push(LoweringError::unsupported(&child.name, child.location));
                }
            }
        }
    }
    found.reverse();
    (lowered, found)
}

// Places each lowered child through `place`, which hands back a child its
// node has no slot for; each of those is reported as misplaced inside
// `parent`. Answers the errors in the reverse of placement order, as the
// reference's accumulation answers them.
fn place_children(
    lowered: Vec<Lowered>,
    parent: &str,
    mut place: impl FnMut(Lowered) -> Option<Lowered>,
) -> Vec<LoweringError> {
    let mut errors: Vec<LoweringError> = lowered
        .into_iter()
        .filter_map(&mut place)
        .map(|child| LoweringError::misplaced(child.name(), parent, child.location()))
        .collect();
    errors.reverse();
    errors
}

// Reports each element child of an element that holds no lowered children,
// in document order.
fn refuse_element_children(element: &Element, parent: &str) -> Vec<LoweringError> {
    element
        .children
        .iter()
        .filter_map(|child| match child {
            Node::Element(child) => Some(LoweringError::misplaced(
                &child.name,
                parent,
                child.location,
            )),
            Node::Text(_) => None,
        })
        .collect()
}

// ---- the builders

fn build_scxml(element: &Element, ctx: &Context<'_>) -> (Document, Vec<LoweringError>) {
    let (lowered, mut errors) = walk_children(element, ctx);
    let mut states = Vec::new();
    let mut scripts = Vec::new();
    let mut datamodel_element = None;
    errors.extend(place_children(
        lowered,
        &element.name,
        |child| match child {
            Lowered::State(node) => {
                states.push(node);
                None
            }
            Lowered::Datamodel(node) => {
                datamodel_element = Some(node);
                None
            }
            Lowered::Executable(Executable::Script(node)) => {
                scripts.push(node);
                None
            }
            other => Some(other),
        },
    ));
    let document = Document {
        location: element.location,
        name: value(element, "name"),
        version: value(element, "version"),
        xmlns: value(element, "xmlns"),
        namespace: element.namespace.clone(),
        datamodel: value(element, "datamodel"),
        binding: match value(element, "binding").as_deref() {
            Some("late") => Binding::Late,
            _ => Binding::Early,
        },
        initial: list(element, "initial"),
        states,
        datamodel_element,
        scripts,
        attribute_locations: locations(
            element,
            &[
                "initial",
                "name",
                "datamodel",
                "binding",
                "version",
                "xmlns",
            ],
        ),
    };
    (document, errors)
}

// `<state>`, `<parallel>`, `<final>` and `<history>`: one builder, the kind
// its own. A history's `type` is `shallow` unless `deep` is written, and an
// out-of-range value keeps its location so a check can point at it.
fn build_state(element: &Element, ctx: &Context<'_>, kind: StateKind) -> Built {
    let (lowered, mut errors) = walk_children(element, ctx);
    let mut states = Vec::new();
    let mut transitions = Vec::new();
    let mut onentry = Vec::new();
    let mut onexit = Vec::new();
    let mut invoke = Vec::new();
    let mut initial_element = None;
    let mut donedata = None;
    let mut datamodel_element = None;
    errors.extend(place_children(lowered, &element.name, |child| {
        match child {
            Lowered::State(node) => states.push(node),
            Lowered::Transition(node) => transitions.push(node),
            Lowered::Initial(node) => initial_element = Some(node),
            Lowered::Onentry(node) => onentry.push(node),
            Lowered::Onexit(node) => onexit.push(node),
            Lowered::Donedata(node) => donedata = Some(node),
            Lowered::Datamodel(node) => datamodel_element = Some(node),
            Lowered::Invoke(node) => invoke.push(node),
            other => return Some(other),
        }
        None
    }));
    let (history_type, attribute_locations) = if kind == StateKind::History {
        let history_type = match value(element, "type").as_deref() {
            Some("deep") => HistoryType::Deep,
            _ => HistoryType::Shallow,
        };
        (
            Some(history_type),
            locations(element, &["id", "initial", "type"]),
        )
    } else {
        (None, locations(element, &["id", "initial"]))
    };
    let state = State {
        kind,
        location: element.location,
        id: value(element, "id"),
        initial: list(element, "initial"),
        initial_element,
        states,
        transitions,
        onentry,
        onexit,
        history_type,
        donedata,
        datamodel_element,
        invoke,
        attribute_locations,
    };
    (Some(Lowered::State(state)), errors)
}

fn build_initial(element: &Element, ctx: &Context<'_>) -> Built {
    let (lowered, mut errors) = walk_children(element, ctx);
    let mut transitions = Vec::new();
    errors.extend(place_children(
        lowered,
        &element.name,
        |child| match child {
            Lowered::Transition(node) => {
                transitions.push(node);
                None
            }
            other => Some(other),
        },
    ));
    let initial = Initial {
        location: element.location,
        transitions,
    };
    (Some(Lowered::Initial(initial)), errors)
}

// A transition's executable content goes into it unwrapped. An out-of-range
// `type` lowers to `external` and keeps its location.
fn build_transition(element: &Element, ctx: &Context<'_>) -> Built {
    let (lowered, mut errors) = walk_children(element, ctx);
    let mut content = Vec::new();
    errors.extend(place_children(
        lowered,
        &element.name,
        place_into(&mut content),
    ));
    let transition = Transition {
        location: element.location,
        event: list(element, "event"),
        target: list(element, "target"),
        cond: value(element, "cond"),
        transition_type: match value(element, "type").as_deref() {
            Some("internal") => TransitionType::Internal,
            _ => TransitionType::External,
        },
        content,
        attribute_locations: locations(element, &["event", "target", "cond", "type"]),
    };
    (Some(Lowered::Transition(transition)), errors)
}

// `<onentry>`, `<onexit>` and `<finalize>`: one block per element, each with
// its own span.
fn build_block(element: &Element, ctx: &Context<'_>, tag: fn(Block) -> Lowered) -> Built {
    let (lowered, mut errors) = walk_children(element, ctx);
    let mut content = Vec::new();
    errors.extend(place_children(
        lowered,
        &element.name,
        place_into(&mut content),
    ));
    let block = Block {
        location: element.location,
        content,
    };
    (Some(tag(block)), errors)
}

// The placement of a node whose one slot is a list of executable content.
fn place_into(content: &mut Vec<Executable>) -> impl FnMut(Lowered) -> Option<Lowered> + '_ {
    |child| match child {
        Lowered::Executable(node) => {
            content.push(node);
            None
        }
        other => Some(other),
    }
}

// A node with no slot: every lowered child is misplaced.
fn place_none(child: Lowered) -> Option<Lowered> {
    Some(child)
}

// `event` is one name, never split. Without it nothing is built, and the
// children are not placed.
fn build_raise(element: &Element, ctx: &Context<'_>) -> Built {
    let (lowered, mut errors) = walk_children(element, ctx);
    let Some(event) = value(element, "event") else {
        errors.push(LoweringError::missing_attribute(
            "raise",
            "event",
            element.location,
        ));
        return (None, errors);
    };
    errors.extend(place_children(lowered, &element.name, place_none));
    let raise = Raise {
        location: element.location,
        event,
        attribute_locations: locations(element, &["event"]),
    };
    (Some(Lowered::Executable(Executable::Raise(raise))), errors)
}

fn build_log(element: &Element, ctx: &Context<'_>) -> Built {
    let (lowered, mut errors) = walk_children(element, ctx);
    errors.extend(place_children(lowered, &element.name, place_none));
    let log = Log {
        location: element.location,
        label: value(element, "label"),
        expr: value(element, "expr"),
        attribute_locations: locations(element, &["label", "expr"]),
    };
    (Some(Lowered::Executable(Executable::Log(log))), errors)
}

fn build_donedata(element: &Element, ctx: &Context<'_>) -> Built {
    let (lowered, mut errors) = walk_children(element, ctx);
    let mut content = None;
    let mut params = Vec::new();
    errors.extend(place_children(lowered, &element.name, |child| {
        match child {
            Lowered::Content(node) => content = Some(node),
            Lowered::Param(node) => params.push(node),
            other => return Some(other),
        }
        None
    }));
    let donedata = Donedata {
        location: element.location,
        content,
        params,
    };
    (Some(Lowered::Donedata(donedata)), errors)
}

// `name` is required: without it nothing is built, and the children are not
// placed.
fn build_param(element: &Element, ctx: &Context<'_>) -> Built {
    let (lowered, mut errors) = walk_children(element, ctx);
    let Some(name) = value(element, "name") else {
        errors.push(LoweringError::missing_attribute(
            "param",
            "name",
            element.location,
        ));
        return (None, errors);
    };
    errors.extend(place_children(lowered, &element.name, place_none));
    let param = Param {
        location: element.location,
        name,
        expr: value(element, "expr"),
        param_location: value(element, "location"),
        attribute_locations: locations(element, &["name", "expr", "location"]),
    };
    (Some(Lowered::Param(param)), errors)
}

// Its children are its payload: the text is kept untrimmed and element
// children as a source slice, never lowered, so nothing below it is refused.
fn build_content(element: &Element, ctx: &Context<'_>) -> Built {
    let (markup, markup_location) = slice_markup(element, ctx.source);
    let content = Content {
        location: element.location,
        expr: value(element, "expr"),
        text: text(element),
        markup,
        markup_location,
        attribute_locations: locations(element, &["expr"]),
    };
    (Some(Lowered::Content(content)), Vec::new())
}

fn build_datamodel(element: &Element, ctx: &Context<'_>) -> Built {
    let (lowered, mut errors) = walk_children(element, ctx);
    let mut data = Vec::new();
    errors.extend(place_children(
        lowered,
        &element.name,
        |child| match child {
            Lowered::Data(node) => {
                data.push(node);
                None
            }
            other => Some(other),
        },
    ));
    let datamodel = Datamodel {
        location: element.location,
        data,
    };
    (Some(Lowered::Datamodel(datamodel)), errors)
}

// Its text is its payload, kept untrimmed; an element child is misplaced.
// `id` is required: without it nothing is built.
fn build_data(element: &Element) -> Built {
    let mut errors = refuse_element_children(element, "data");
    let Some(id) = value(element, "id") else {
        errors.push(LoweringError::missing_attribute(
            "data",
            "id",
            element.location,
        ));
        return (None, errors);
    };
    let data = Data {
        location: element.location,
        id,
        expr: value(element, "expr"),
        src: value(element, "src"),
        text: text(element),
        attribute_locations: locations(element, &["id", "expr", "src"]),
    };
    (Some(Lowered::Data(data)), errors)
}

// Its children are an in-line value, read as `<content>`'s are. `location`
// is required: without it nothing is built.
fn build_assign(element: &Element, ctx: &Context<'_>) -> Built {
    let Some(location) = value(element, "location") else {
        let error = LoweringError::missing_attribute("assign", "location", element.location);
        return (None, vec![error]);
    };
    let (markup, markup_location) = slice_markup(element, ctx.source);
    let assign = Assign {
        node_location: element.location,
        location,
        expr: value(element, "expr"),
        text: text(element),
        markup,
        markup_location,
        attribute_locations: locations(element, &["location", "expr"]),
    };
    (
        Some(Lowered::Executable(Executable::Assign(assign))),
        Vec::new(),
    )
}

// One branch opens with the `<if>` itself; each `<elseif>` and `<else>`
// child opens the next, and executable content goes into the open branch.
// `cond` is required: without it nothing is built.
fn build_if(element: &Element, ctx: &Context<'_>) -> Built {
    let (lowered, mut errors) = walk_children(element, ctx);
    let Some(cond) = value(element, "cond") else {
        errors.push(LoweringError::missing_attribute(
            "if",
            "cond",
            element.location,
        ));
        return (None, errors);
    };
    let mut branches = vec![Branch {
        location: element.location,
        cond: Some(cond),
        content: Vec::new(),
        attribute_locations: locations(element, &["cond"]),
    }];
    errors.extend(place_children(lowered, &element.name, |child| {
        match child {
            Lowered::Executable(node) => {
                if let Some(open) = branches.last_mut() {
                    open.content.push(node);
                }
            }
            Lowered::Elseif(branch) | Lowered::Else(branch) => branches.push(branch),
            other => return Some(other),
        }
        None
    }));
    let node = If {
        location: element.location,
        branches,
    };
    (Some(Lowered::Executable(Executable::If(node))), errors)
}

// A partition of an `<if>`, placed nowhere else. It takes no children: an
// element child is misplaced. `cond` is required: without it nothing is
// built.
fn build_elseif(element: &Element) -> Built {
    let mut errors = refuse_element_children(element, "elseif");
    let Some(cond) = value(element, "cond") else {
        errors.push(LoweringError::missing_attribute(
            "elseif",
            "cond",
            element.location,
        ));
        return (None, errors);
    };
    let branch = Branch {
        location: element.location,
        cond: Some(cond),
        content: Vec::new(),
        attribute_locations: locations(element, &["cond"]),
    };
    (Some(Lowered::Elseif(branch)), errors)
}

// A partition of an `<if>`, placed nowhere else. It takes no children and no
// attributes: an element child is misplaced and the first attribute written
// is refused, and the branch is built either way.
fn build_else(element: &Element) -> Built {
    let mut errors = refuse_element_children(element, "else");
    if let Some(attribute) = element.attributes.first() {
        errors.push(LoweringError::unexpected_attribute(
            "else",
            &attribute.name,
            attribute.location,
        ));
    }
    let branch = Branch {
        location: element.location,
        cond: None,
        content: Vec::new(),
        attribute_locations: AttributeLocations::new(),
    };
    (Some(Lowered::Else(branch)), errors)
}

// `array` and `item` are required, and both are reported when both are
// absent; without either nothing is built.
fn build_foreach(element: &Element, ctx: &Context<'_>) -> Built {
    let (lowered, mut errors) = walk_children(element, ctx);
    let array = value(element, "array");
    let item = value(element, "item");
    let (Some(array), Some(item)) = (array.clone(), item.clone()) else {
        // `item` first, as the reference accumulates them.
        if item.is_none() {
            errors.push(LoweringError::missing_attribute(
                "foreach",
                "item",
                element.location,
            ));
        }
        if array.is_none() {
            errors.push(LoweringError::missing_attribute(
                "foreach",
                "array",
                element.location,
            ));
        }
        return (None, errors);
    };
    let mut content = Vec::new();
    errors.extend(place_children(
        lowered,
        &element.name,
        place_into(&mut content),
    ));
    let foreach = Foreach {
        location: element.location,
        array,
        item,
        index: value(element, "index"),
        content,
        attribute_locations: locations(element, &["array", "item", "index"]),
    };
    (
        Some(Lowered::Executable(Executable::Foreach(foreach))),
        errors,
    )
}

// Its text is the program, kept untrimmed; an element child is misplaced. A
// written `src` is refused and nothing is built.
fn build_script(element: &Element) -> Built {
    let mut errors = refuse_element_children(element, "script");
    if attribute(element, "src").is_some() {
        errors.push(LoweringError::unsupported_attribute(
            "script",
            "src",
            element.location,
        ));
        return (None, errors);
    }
    let script = Script {
        location: element.location,
        text: text(element),
    };
    (
        Some(Lowered::Executable(Executable::Script(script))),
        errors,
    )
}

// Every attribute is kept, the exclusive pairs included; `autoforward` is
// false unless `true` is written.
fn build_invoke(element: &Element, ctx: &Context<'_>) -> Built {
    let (lowered, mut errors) = walk_children(element, ctx);
    let mut params = Vec::new();
    let mut content = None;
    let mut finalize = None;
    errors.extend(place_children(lowered, &element.name, |child| {
        match child {
            Lowered::Param(node) => params.push(node),
            Lowered::Content(node) => content = Some(node),
            Lowered::Finalize(node) => finalize = Some(node),
            other => return Some(other),
        }
        None
    }));
    let invoke = Invoke {
        location: element.location,
        invoke_type: value(element, "type"),
        typeexpr: value(element, "typeexpr"),
        src: value(element, "src"),
        srcexpr: value(element, "srcexpr"),
        id: value(element, "id"),
        idlocation: value(element, "idlocation"),
        namelist: list(element, "namelist"),
        autoforward: value(element, "autoforward").as_deref() == Some("true"),
        params,
        content,
        finalize,
        attribute_locations: locations(
            element,
            &[
                "type",
                "typeexpr",
                "src",
                "srcexpr",
                "id",
                "idlocation",
                "namelist",
                "autoforward",
            ],
        ),
    };
    (Some(Lowered::Invoke(invoke)), errors)
}

// Every attribute is kept, the exclusive pairs included.
fn build_send(element: &Element, ctx: &Context<'_>) -> Built {
    let (lowered, mut errors) = walk_children(element, ctx);
    let mut params = Vec::new();
    let mut content = None;
    errors.extend(place_children(lowered, &element.name, |child| {
        match child {
            Lowered::Param(node) => params.push(node),
            Lowered::Content(node) => content = Some(node),
            other => return Some(other),
        }
        None
    }));
    let send = Send {
        location: element.location,
        event: value(element, "event"),
        eventexpr: value(element, "eventexpr"),
        target: value(element, "target"),
        targetexpr: value(element, "targetexpr"),
        send_type: value(element, "type"),
        typeexpr: value(element, "typeexpr"),
        id: value(element, "id"),
        idlocation: value(element, "idlocation"),
        delay: value(element, "delay"),
        delayexpr: value(element, "delayexpr"),
        namelist: list(element, "namelist"),
        params,
        content,
        attribute_locations: locations(
            element,
            &[
                "event",
                "eventexpr",
                "target",
                "targetexpr",
                "type",
                "typeexpr",
                "id",
                "idlocation",
                "delay",
                "delayexpr",
                "namelist",
            ],
        ),
    };
    (
        Some(Lowered::Executable(Executable::Send(Box::new(send)))),
        errors,
    )
}

fn build_cancel(element: &Element, ctx: &Context<'_>) -> Built {
    let (lowered, mut errors) = walk_children(element, ctx);
    errors.extend(place_children(lowered, &element.name, place_none));
    let cancel = Cancel {
        location: element.location,
        sendid: value(element, "sendid"),
        sendidexpr: value(element, "sendidexpr"),
        attribute_locations: locations(element, &["sendid", "sendidexpr"]),
    };
    (
        Some(Lowered::Executable(Executable::Cancel(cancel))),
        errors,
    )
}

// ---- reading elements

// The first attribute of `element` named `name`.
fn attribute<'e>(element: &'e Element, name: &str) -> Option<&'e crate::xml::Attribute> {
    element
        .attributes
        .iter()
        .find(|attribute| attribute.name == name)
}

// The value of the first attribute named `name`, as the reader expanded it.
fn value(element: &Element, name: &str) -> Option<String> {
    attribute(element, name).map(|attribute| attribute.value.clone())
}

// The value of the first attribute named `name`, split on whitespace; empty
// when it is absent or blank.
fn list(element: &Element, name: &str) -> Vec<String> {
    attribute(element, name).map_or_else(Vec::new, |attribute| {
        attribute
            .value
            .split(is_list_separator)
            .filter(|part| !part.is_empty())
            .map(str::to_owned)
            .collect()
    })
}

// The value spans of the attributes in `names` that were written.
fn locations(element: &Element, names: &[&'static str]) -> AttributeLocations {
    names
        .iter()
        .filter_map(|name| attribute(element, name).map(|found| (*name, found.value_location)))
        .collect()
}

// The direct text children joined, untrimmed.
fn text(element: &Element) -> String {
    element
        .children
        .iter()
        .filter_map(|child| match child {
            Node::Text(text) => Some(text.value.as_str()),
            Node::Element(_) => None,
        })
        .collect()
}

// When `element` has an element child: the source from the start of its
// first child that is not blank text to the end of its last one, line
// breaks unfolded, and that span. Otherwise neither.
fn slice_markup(element: &Element, source: &str) -> (Option<String>, Option<Location>) {
    let span = markup_span(element);
    (
        span.and_then(|span| span.slice(source).map(str::to_owned)),
        span,
    )
}

fn markup_span(element: &Element) -> Option<Location> {
    let significant: Vec<&Node> = element
        .children
        .iter()
        .filter(|child| match child {
            Node::Text(text) => !is_blank(&text.value),
            Node::Element(_) => true,
        })
        .collect();
    if !significant
        .iter()
        .any(|child| matches!(child, Node::Element(_)))
    {
        return None;
    }
    let first = node_location(significant.first()?);
    let last = node_location(significant.last()?);
    Some(Location {
        end_line: last.end_line,
        end_column: last.end_column,
        end_offset: last.end_offset,
        ..first
    })
}

fn node_location(node: &Node) -> Location {
    match node {
        Node::Element(element) => element.location,
        Node::Text(text) => text.location,
    }
}

// The name after the first colon, or the whole name when there is none.
fn local_name(name: &str) -> &str {
    name.split_once(':').map_or(name, |(_, local)| local)
}

// SCXML's vocabulary: its namespace, or no namespace at all.
fn is_scxml_vocabulary(namespace: Option<&str>) -> bool {
    namespace.is_none_or(|uri| uri == SCXML_NAMESPACE)
}

fn foreign(element: &Element) -> LoweringError {
    let uri = element.namespace.as_deref().unwrap_or_default();
    LoweringError::foreign_element(&element.name, uri, element.location)
}

// Whether trimming Unicode whitespace from `text` leaves nothing. Rust's
// whitespace is the Unicode White_Space property, the set the reference
// trims.
pub(crate) fn is_blank(text: &str) -> bool {
    text.chars().all(char::is_whitespace)
}

// Unicode whitespace less the three no-break spaces, the set the reference
// splits an attribute list on.
fn is_list_separator(c: char) -> bool {
    c.is_whitespace() && !matches!(c, '\u{A0}' | '\u{2007}' | '\u{202F}')
}
