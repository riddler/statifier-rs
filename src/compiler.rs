//! The validated document in, the machine out.
//!
//! The fourth step of reading a chart, a port of the reference's compiler at
//! the vendored tag. One walk, depth first in document order, interns every
//! state: a state takes its index when the walk reaches it, then its own
//! `<onentry>` and `<onexit>` content, its transitions with their content (a
//! history's transitions are its default; any other kind's own come before
//! its `<initial>` element's), its `<donedata>`, its `<data>` and its
//! `<invoke>` elements with their `<finalize>` content are numbered, and then
//! the walk descends into its children. An executable-content container
//! takes its index before its children's. The root is index 0 and its own
//! `<data>` come first. A transition can target a state the walk has not
//! reached, so targets are resolved, and conditions compiled, once the walk
//! is done.
//!
//! Every expression compiles once, through the predicator crate:
//! conditions, values and `<content>` expressions through its expression
//! compiler, `<script>` bodies through its program compiler. An expression
//! it refuses is a refusal with the reason
//! [`CompileReason::ExpressionCompileError`](crate::CompileReason), except
//! for the classes the reference defers to when they execute: an
//! `<assign>`'s or a `<data>`'s value, a `namelist` entry and a `<script>`
//! body compile to a step that fails then. A `<send>` stops at its first
//! refused attribute; every other refusal is collected. The refusals are
//! answered sorted by where they start, and the machine only when there is
//! none. `<assign location>`, a `<foreach>`'s `item` and `index`, and an
//! `idlocation` stay as written: each is a location resolved when it
//! executes.
//!
//! The validator has resolved every id the walk looks up and checked every
//! shape it reads; a document that skipped it is not compiled, so a lookup
//! that finds nothing is skipped rather than refused.

mod expressions;

#[cfg(test)]
mod tests;

use crate::document::{
    self, AttributeLocations, Document, Executable, Initial, StateKind,
    Transition as DocumentTransition,
};
use crate::machine::{
    Assign, Block, Branch, Chart, Content, Data, DataValue, Deferred, Donedata, Expr, Foreach,
    Invoke, Kind, Param, ParamKind, Send, State, Transition,
};
use crate::refusal::{Refusal, Warning};
use crate::xml::Location;
use expressions::{compile as compile_expr, deferred, inline_value, literal, program};
use predicator_rs::Value;
use std::collections::BTreeMap;

/// Compiles `document`, which the validator answered `warnings` for, into
/// its machine; or answers every refusal, sorted by where it starts.
pub(crate) fn compile(document: &Document, warnings: Vec<Warning>) -> Result<Chart, Vec<Refusal>> {
    let mut walk = Walk::default();
    walk.states.push(None);
    walk.assign_data(document.datamodel_element.as_ref(), 0);
    let (children, after) = walk.siblings(&document.states, 1, 0);
    let initial = if document.initial.is_empty() {
        children.iter().take(1).copied().collect()
    } else {
        walk.resolve(&document.initial)
    };
    let root = State {
        index: 0,
        id: None,
        kind: Kind::Scxml,
        parent: None,
        last: after - 1,
        history_children: walk.histories(&children),
        children,
        initial,
        history_type: None,
        transitions: Vec::new(),
        onentry: Vec::new(),
        onexit: Vec::new(),
        initial_transition: None,
        history_default: None,
        donedata: None,
        data: walk.owned_data(0),
        invoke: Vec::new(),
        location: document.location,
        attribute_locations: document.attribute_locations.clone(),
    };
    if let Some(slot) = walk.states.first_mut() {
        *slot = Some(root);
    }
    let transitions = walk.transitions();
    let global_scripts = document
        .scripts
        .iter()
        .map(|script| program(&script.text, script.location))
        .collect();
    let errors = walk.errors.merged();
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(Chart {
        states: walk.states.into_iter().flatten().collect(),
        id_to_index: walk.id_to_index,
        transitions,
        contents: walk.contents.into_iter().flatten().collect(),
        data: walk.data,
        global_scripts,
        name: document.name.clone(),
        datamodel: document.datamodel.clone(),
        binding: document.binding,
        location: document.location,
        warnings,
    })
}

// The refusals, in the four groups the reference merges in this order: the
// transitions', the content's, the donedata's and the invokes'.
#[derive(Default)]
struct Errors {
    transitions: Vec<Refusal>,
    content: Vec<Refusal>,
    donedata: Vec<Refusal>,
    invoke: Vec<Refusal>,
}

impl Errors {
    // Every refusal sorted by where it starts. The sort is stable, so
    // refusals at one offset keep the groups' order; the reference gathers
    // the invokes' group by prepending, so its own order is reversed first.
    fn merged(self) -> Vec<Refusal> {
        let mut errors = self.transitions;
        errors.extend(self.content);
        errors.extend(self.donedata);
        errors.extend(self.invoke.into_iter().rev());
        errors.sort_by_key(|error| error.span().start_offset);
        errors
    }
}

// A transition numbered by the walk, its targets and condition waiting for
// the walk to end.
struct Pending<'d> {
    transition: &'d DocumentTransition,
    source: usize,
    content: Vec<usize>,
}

// The numbering walk over one document. A slot is reserved when an index is
// taken and filled once its node is built.
#[derive(Default)]
struct Walk<'d> {
    id_to_index: BTreeMap<String, usize>,
    states: Vec<Option<State>>,
    pending: Vec<Pending<'d>>,
    contents: Vec<Option<Content>>,
    data: Vec<Data>,
    data_by_state: BTreeMap<usize, Vec<usize>>,
    errors: Errors,
}

impl<'d> Walk<'d> {
    // Numbers `states` and their subtrees from `index`; answers their
    // indexes and the first index after the last subtree.
    fn siblings(
        &mut self,
        states: &'d [document::State],
        mut index: usize,
        parent: usize,
    ) -> (Vec<usize>, usize) {
        let mut indexes = Vec::with_capacity(states.len());
        for state in states {
            indexes.push(index);
            index = self.state(state, index, parent);
        }
        (indexes, index)
    }

    // Numbers `state` at `index`, then its subtree; answers the first index
    // after the subtree.
    fn state(&mut self, state: &'d document::State, index: usize, parent: usize) -> usize {
        self.states.push(None);
        let onentry = self.blocks(&state.onentry);
        let onexit = self.blocks(&state.onexit);
        let own = self.number_transitions(&state.transitions, index);
        let (transitions, initial_transition, history_default) = if state.kind == StateKind::History
        {
            (Vec::new(), None, own.first().copied())
        } else {
            let written = state
                .initial_element
                .as_ref()
                .map_or(&[][..], |initial| &initial.transitions);
            let initial = self.number_transitions(written, index);
            (own, initial.first().copied(), None)
        };
        let donedata = state
            .donedata
            .as_ref()
            .map(|donedata| self.donedata(donedata));
        self.assign_data(state.datamodel_element.as_ref(), index);
        let invoke = state
            .invoke
            .iter()
            .enumerate()
            .map(|(position, invoke)| self.invoke(invoke, position))
            .collect();
        let (children, after) = self.siblings(&state.states, index + 1, index);
        let initial = self.default_entry(state, &children);
        let built = State {
            index,
            id: state.id.clone(),
            kind: kind(state.kind),
            parent: Some(parent),
            last: after - 1,
            history_children: self.histories(&children),
            children,
            initial,
            history_type: state.history_type,
            transitions,
            onentry,
            onexit,
            initial_transition,
            history_default,
            donedata,
            data: self.owned_data(index),
            invoke,
            location: state.location,
            attribute_locations: state.attribute_locations.clone(),
        };
        if let Some(slot) = self.states.get_mut(index) {
            *slot = Some(built);
        }
        if let Some(id) = state.id.as_ref().filter(|id| !id.is_empty()) {
            self.id_to_index.insert(id.clone(), index);
        }
        after
    }

    // A `<state>`'s default entry, in the specification's precedence: its
    // `initial` attribute, its `<initial>` element's one transition, its
    // first child. Only a `<state>` has one: a parallel enters every child,
    // and a final or a history is never entered by default.
    fn default_entry(&self, state: &document::State, children: &[usize]) -> Vec<usize> {
        if state.kind != StateKind::State {
            return Vec::new();
        }
        if !state.initial.is_empty() {
            return self.resolve(&state.initial);
        }
        match state.initial_element.as_ref() {
            Some(Initial { transitions, .. }) if transitions.len() == 1 => transitions
                .iter()
                .flat_map(|t| self.resolve(&t.target))
                .collect(),
            _ => children.iter().take(1).copied().collect(),
        }
    }

    // The indexes of `ids`; the validator has resolved every one.
    fn resolve(&self, ids: &[String]) -> Vec<usize> {
        ids.iter()
            .filter_map(|id| self.id_to_index.get(id).copied())
            .collect()
    }

    // The `<history>` children among `children`, already built.
    fn histories(&self, children: &[usize]) -> Vec<usize> {
        children
            .iter()
            .copied()
            .filter(|&child| {
                matches!(
                    self.states.get(child),
                    Some(Some(State {
                        kind: Kind::History,
                        ..
                    }))
                )
            })
            .collect()
    }

    fn owned_data(&self, index: usize) -> Vec<usize> {
        self.data_by_state.get(&index).cloned().unwrap_or_default()
    }

    // ---- transitions

    fn number_transitions(
        &mut self,
        transitions: &'d [DocumentTransition],
        source: usize,
    ) -> Vec<usize> {
        transitions
            .iter()
            .map(|transition| {
                let content = self.content_list(&transition.content);
                self.pending.push(Pending {
                    transition,
                    source,
                    content,
                });
                self.pending.len() - 1
            })
            .collect()
    }

    // Every transition, its targets resolved and its condition compiled now
    // that every id has its index.
    fn transitions(&mut self) -> Vec<Transition> {
        let pending = std::mem::take(&mut self.pending);
        pending
            .into_iter()
            .map(|pending| {
                let transition = pending.transition;
                let cond_location = transition
                    .cond
                    .as_ref()
                    .map(|_| span(&transition.attribute_locations, "cond", transition.location));
                let cond = match (&transition.cond, cond_location) {
                    (Some(source), Some(location)) => match compile_expr(source, location) {
                        Ok(expr) => Some(expr),
                        Err(error) => {
                            self.errors.transitions.push(error);
                            None
                        }
                    },
                    _ => None,
                };
                Transition {
                    source: pending.source,
                    targets: self.resolve(&transition.target),
                    events: transition
                        .event
                        .iter()
                        .map(|descriptor| descriptor.split('.').map(str::to_owned).collect())
                        .collect(),
                    cond,
                    cond_location,
                    transition_type: transition.transition_type,
                    content: pending.content,
                    location: transition.location,
                    attribute_locations: transition.attribute_locations.clone(),
                }
            })
            .collect()
    }

    // ---- executable content

    fn blocks(&mut self, blocks: &'d [document::Block]) -> Vec<Block> {
        blocks.iter().map(|block| self.block(block)).collect()
    }

    fn block(&mut self, block: &'d document::Block) -> Block {
        Block {
            location: block.location,
            content: self.content_list(&block.content),
        }
    }

    fn content_list(&mut self, nodes: &'d [Executable]) -> Vec<usize> {
        nodes.iter().map(|node| self.content(node)).collect()
    }

    // Numbers `node`, then its children; answers its index. A refused node
    // leaves its slot empty: the machine is not built then.
    fn content(&mut self, node: &'d Executable) -> usize {
        let index = self.contents.len();
        self.contents.push(None);
        let built = match node {
            Executable::Raise(raise) => Some(Content::Raise {
                event: raise.event.clone(),
                location: raise.location,
            }),
            Executable::Log(log) => self.log(log),
            Executable::Assign(assign) => Some(Content::Assign(assign_node(assign))),
            Executable::If(node) => Some(self.if_node(node)),
            Executable::Foreach(foreach) => self.foreach(foreach),
            Executable::Script(script) => Some(Content::Script {
                program: program(&script.text, script.location),
                location: script.location,
            }),
            Executable::Send(send) => self.send(send).map(|send| Content::Send(Box::new(send))),
            Executable::Cancel(cancel) => self.cancel(cancel),
        };
        if let Some(slot) = self.contents.get_mut(index) {
            *slot = built;
        }
        index
    }

    fn log(&mut self, log: &document::Log) -> Option<Content> {
        let (expr, expr_location) = match &log.expr {
            None => (None, None),
            Some(source) => {
                let location = span(&log.attribute_locations, "expr", log.location);
                (Some(self.content_expr(source, location)?), Some(location))
            }
        };
        Some(Content::Log {
            label: log.label.clone(),
            expr,
            location: log.location,
            expr_location,
        })
    }

    // Every branch's condition compiles, each refusal its own, before the
    // branches' content is numbered.
    fn if_node(&mut self, node: &'d document::If) -> Content {
        let conds: Vec<(Option<Expr>, Option<Location>)> = node
            .branches
            .iter()
            .map(|branch| match &branch.cond {
                None => (None, None),
                Some(source) => {
                    let location = span(&branch.attribute_locations, "cond", branch.location);
                    (self.content_expr(source, location), Some(location))
                }
            })
            .collect();
        let branches = node
            .branches
            .iter()
            .zip(conds)
            .map(|(branch, (cond, cond_location))| Branch {
                cond,
                cond_location,
                content: self.content_list(&branch.content),
            })
            .collect();
        Content::If {
            location: node.location,
            branches,
        }
    }

    fn foreach(&mut self, foreach: &'d document::Foreach) -> Option<Content> {
        let locations = &foreach.attribute_locations;
        let array_location = span(locations, "array", foreach.location);
        let array = self.content_expr(&foreach.array, array_location);
        let content = self.content_list(&foreach.content);
        Some(Content::Foreach(Foreach {
            location: foreach.location,
            array: array?,
            array_location,
            item: foreach.item.clone(),
            item_location: span(locations, "item", foreach.location),
            index: foreach.index.clone(),
            index_location: foreach
                .index
                .as_ref()
                .map(|_| span(locations, "index", foreach.location)),
            content,
        }))
    }

    // A `<send>` stops at its first refused attribute, in the reference's
    // order: the event, the target, the type, the delay, the `<content>`,
    // then every `<param>`.
    fn send(&mut self, send: &document::Send) -> Option<Send> {
        let locations = &send.attribute_locations;
        let at = |name| span(locations, name, send.location);
        let namelist_location = at("namelist");
        let namelist = send
            .namelist
            .iter()
            .map(|entry| namelist_param(entry, send.location, namelist_location))
            .collect();
        let pairs = [
            (&send.event, &send.eventexpr, "eventexpr"),
            (&send.target, &send.targetexpr, "targetexpr"),
            (&send.send_type, &send.typeexpr, "typeexpr"),
            (&send.delay, &send.delayexpr, "delayexpr"),
        ];
        let mut compiled = Vec::with_capacity(pairs.len());
        for (literal_value, source, name) in pairs {
            match pair(literal_value.as_ref(), source.as_ref(), at(name)) {
                Ok(expr) => compiled.push(expr),
                Err(error) => {
                    self.errors.content.push(error);
                    return None;
                }
            }
        }
        let content = match send.content.as_ref().map(content_value).transpose() {
            Ok(content) => content,
            Err(error) => {
                self.errors.content.push(error);
                return None;
            }
        };
        let params = params(&send.params);
        let params = match params {
            Ok(params) => params,
            Err(errors) => {
                self.errors.content.extend(errors);
                return None;
            }
        };
        let mut compiled = compiled.into_iter();
        Some(Send {
            location: send.location,
            event: compiled.next().flatten(),
            target: compiled.next().flatten(),
            send_type: compiled.next().flatten(),
            id: send.id.clone(),
            idlocation: send.idlocation.clone(),
            delay: compiled.next().flatten(),
            namelist,
            params,
            content,
            attribute_locations: send.attribute_locations.clone(),
        })
    }

    fn cancel(&mut self, cancel: &document::Cancel) -> Option<Content> {
        let location = span(&cancel.attribute_locations, "sendidexpr", cancel.location);
        match pair(cancel.sendid.as_ref(), cancel.sendidexpr.as_ref(), location) {
            Ok(sendid) => Some(Content::Cancel {
                sendid,
                location: cancel.location,
                attribute_locations: cancel.attribute_locations.clone(),
            }),
            Err(error) => {
                self.errors.content.push(error);
                None
            }
        }
    }

    // `source` compiled, a refusal recorded in the content's group.
    fn content_expr(&mut self, source: &str, location: Location) -> Option<Expr> {
        compile_expr(source, location)
            .map_err(|error| self.errors.content.push(error))
            .ok()
    }

    // ---- donedata, data and invoke

    // A final state's `<donedata>`: its `<content>`, else every `<param>`.
    fn donedata(&mut self, donedata: &document::Donedata) -> Donedata {
        let (expr, expr_location, params) = match &donedata.content {
            Some(content) => {
                let expr = content_value(content)
                    .map_err(|error| self.errors.donedata.push(error))
                    .ok();
                (expr, Some(content_location(content)), Vec::new())
            }
            None => {
                let params = params(&donedata.params)
                    .map_err(|errors| self.errors.donedata.extend(errors))
                    .unwrap_or_default();
                (None, None, params)
            }
        };
        Donedata {
            location: donedata.location,
            expr,
            expr_location,
            params,
        }
    }

    fn assign_data(&mut self, datamodel: Option<&document::Datamodel>, state: usize) {
        for node in datamodel.map_or(&[][..], |datamodel| &datamodel.data) {
            let index = self.data.len();
            self.data.push(data(node));
            self.data_by_state.entry(state).or_default().push(index);
        }
    }

    // One `<invoke>`, whole: every refusal recorded in the invokes' group,
    // its `<finalize>` content numbered with every other block's.
    fn invoke(&mut self, invoke: &'d document::Invoke, position: usize) -> Invoke {
        let locations = &invoke.attribute_locations;
        let at = |name| span(locations, name, invoke.location);
        let invoke_type = pair(
            invoke.invoke_type.as_ref(),
            invoke.typeexpr.as_ref(),
            at("typeexpr"),
        )
        .map_err(|error| self.errors.invoke.push(error))
        .ok()
        .flatten();
        let src = pair(invoke.src.as_ref(), invoke.srcexpr.as_ref(), at("srcexpr"))
            .map_err(|error| self.errors.invoke.push(error))
            .ok()
            .flatten();
        let content = invoke.content.as_ref().and_then(|content| {
            content_value(content)
                .map_err(|error| self.errors.invoke.push(error))
                .ok()
        });
        let params = invoke
            .params
            .iter()
            .filter_map(|node| {
                param(node)?
                    .map_err(|error| self.errors.invoke.push(error))
                    .ok()
            })
            .collect();
        let namelist_location = at("namelist");
        let namelist = invoke
            .namelist
            .iter()
            .map(|entry| namelist_param(entry, invoke.location, namelist_location))
            .collect();
        let finalize = invoke.finalize.as_ref().map(|block| self.block(block));
        Invoke {
            index: position,
            location: invoke.location,
            invoke_type,
            src,
            id: invoke.id.clone(),
            idlocation: invoke.idlocation.clone(),
            namelist,
            params,
            content,
            autoforward: invoke.autoforward,
            finalize,
            attribute_locations: invoke.attribute_locations.clone(),
        }
    }
}

fn kind(kind: StateKind) -> Kind {
    match kind {
        StateKind::State => Kind::State,
        StateKind::Parallel => Kind::Parallel,
        StateKind::Final => Kind::Final,
        StateKind::History => Kind::History,
    }
}

// The span of the attribute `name` where it was written, else `fallback`.
fn span(locations: &AttributeLocations, name: &str, fallback: Location) -> Location {
    locations.get(name).copied().unwrap_or(fallback)
}

// An attribute that may be written as a literal or as an expression: the
// literal wins, else the expression compiles; `None` when neither is written.
fn pair(
    literal_value: Option<&String>,
    source: Option<&String>,
    location: Location,
) -> Result<Option<Expr>, Refusal> {
    match (literal_value, source) {
        (Some(value), _) => Ok(Some(literal(value.as_str()))),
        (None, Some(source)) => compile_expr(source, location).map(Some),
        (None, None) => Ok(None),
    }
}

// A `<content>`'s value: its expression compiled, else its markup, else its
// text, as a literal string.
fn content_value(content: &document::Content) -> Result<Expr, Refusal> {
    match (&content.expr, &content.markup) {
        (Some(source), _) => compile_expr(source, content_location(content)),
        (None, Some(markup)) => Ok(literal(markup.as_str())),
        (None, None) => Ok(literal(content.text.as_str())),
    }
}

// The span a `<content>`'s value is reported at: its `expr`, else its
// markup, else the element.
fn content_location(content: &document::Content) -> Location {
    match (&content.expr, content.markup_location) {
        (Some(_), _) => span(&content.attribute_locations, "expr", content.location),
        (None, Some(location)) => location,
        (None, None) => content.location,
    }
}

// Every `<param>`, or every refusal among them.
fn params(nodes: &[document::Param]) -> Result<Vec<Param>, Vec<Refusal>> {
    let mut built = Vec::with_capacity(nodes.len());
    let mut errors = Vec::new();
    for result in nodes.iter().filter_map(param) {
        match result {
            Ok(param) => built.push(param),
            Err(error) => errors.push(error),
        }
    }
    if errors.is_empty() {
        Ok(built)
    } else {
        Err(errors)
    }
}

// A `<param>`'s `expr`, else its `location`, compiled as a value; `None`
// for one with neither, which the validator refuses.
fn param(node: &document::Param) -> Option<Result<Param, Refusal>> {
    let (kind, source, name) = match (&node.expr, &node.param_location) {
        (Some(source), None) => (ParamKind::Expr, source, "expr"),
        (_, Some(source)) => (ParamKind::Location, source, "location"),
        (None, None) => return None,
    };
    let expr_location = span(&node.attribute_locations, name, node.location);
    Some(compile_expr(source, expr_location).map(|expr| Param {
        name: node.name.clone(),
        kind,
        expr: Deferred::Ready(expr),
        expr_location,
        location: node.location,
    }))
}

// A `namelist` entry: a location named by itself, its refusal deferred.
fn namelist_param(entry: &str, location: Location, expr_location: Location) -> Param {
    Param {
        name: entry.to_owned(),
        kind: ParamKind::Location,
        expr: deferred(entry, expr_location),
        expr_location,
        location,
    }
}

// An `<assign>`'s value: its expression, a refusal deferred; else its markup
// as a string; else its text folded as an in-line value; else null.
fn assign_node(assign: &document::Assign) -> Assign {
    let locations = &assign.attribute_locations;
    let expr_location = assign
        .expr
        .as_ref()
        .map(|_| span(locations, "expr", assign.node_location));
    let value = match (&assign.expr, &assign.markup, expr_location) {
        (Some(source), _, Some(location)) => deferred(source, location),
        (_, Some(markup), _) => Deferred::Ready(literal(markup.as_str())),
        _ => Deferred::Ready(text_value(&assign.text)),
    };
    Assign {
        location: assign.location.clone(),
        node_location: assign.node_location,
        value,
        location_location: span(locations, "location", assign.node_location),
        expr_location,
    }
}

// Text folded as an in-line value; null when it is blank.
fn text_value(text: &str) -> Expr {
    if crate::lowering::is_blank(text) {
        literal(Value::Null)
    } else {
        inline_value(text)
    }
}

// A `<data>`'s value, in the specification's precedence: its `expr`, a
// refusal deferred; its `src`; its text folded as an in-line value; null.
fn data(node: &document::Data) -> Data {
    let locations = &node.attribute_locations;
    let (value, value_location) = match (&node.expr, &node.src) {
        (Some(source), _) => {
            let location = span(locations, "expr", node.location);
            let value = match compile_expr(source, location) {
                Ok(expr) => DataValue::Expr(expr),
                Err(error) => DataValue::Invalid(Box::new(error)),
            };
            (value, location)
        }
        (None, Some(src)) => (
            DataValue::Src(src.clone()),
            span(locations, "src", node.location),
        ),
        (None, None) => (DataValue::Expr(text_value(&node.text)), node.location),
    };
    Data {
        id: node.id.clone(),
        value,
        location: node.location,
        value_location,
    }
}
