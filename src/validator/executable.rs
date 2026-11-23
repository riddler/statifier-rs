//! The checks of executable content: `<assign>`, `<if>`, `<script>`,
//! `<invoke>`, `<send>` and `<cancel>`, and the warning for `<finalize>`.
//!
//! Each walk reaches what the reference's reaches: a state's `<onentry>`
//! and `<onexit>` blocks, its transitions and its `<initial>` element's,
//! descending into `<if>` branches and `<foreach>` bodies; the `<send>` and
//! `<cancel>` walks reach every `<invoke>`'s `<finalize>` too.

use super::context::Context;
use super::finding::{Reason, ValidationError, Warning, WarningReason};
use crate::document::{Document, Executable, Invoke, Send, State};
use crate::lowering::is_blank;

// The content lists of a state the walks start from, in the reference's
// order; with `finalize`, every `<invoke>`'s `<finalize>` last.
fn state_content(state: &State, finalize: bool) -> Vec<&[Executable]> {
    let mut lists: Vec<&[Executable]> = Vec::new();
    lists.extend(state.onentry.iter().map(|block| block.content.as_slice()));
    lists.extend(state.onexit.iter().map(|block| block.content.as_slice()));
    lists.extend(
        state
            .transitions
            .iter()
            .map(|transition| transition.content.as_slice()),
    );
    if let Some(initial) = &state.initial_element {
        lists.extend(
            initial
                .transitions
                .iter()
                .map(|transition| transition.content.as_slice()),
        );
    }
    if finalize {
        lists.extend(
            state
                .invoke
                .iter()
                .filter_map(|invoke| invoke.finalize.as_ref())
                .map(|block| block.content.as_slice()),
        );
    }
    lists
}

// The nodes of `content` that are not `<if>` or `<foreach>`, descending into
// both, in document order. Lowering bounds the nesting.
fn leaves<'d>(content: &'d [Executable], found: &mut Vec<&'d Executable>) {
    for node in content {
        match node {
            Executable::If(node) => {
                for branch in &node.branches {
                    leaves(&branch.content, found);
                }
            }
            Executable::Foreach(node) => leaves(&node.content, found),
            other => found.push(other),
        }
    }
}

fn state_leaves<'d>(context: &Context<'d>, finalize: bool) -> Vec<&'d Executable> {
    let mut found = Vec::new();
    for state in &context.states {
        for content in state_content(state, finalize) {
            leaves(content, &mut found);
        }
    }
    found
}

/// Every `<send>` the walk reaches, in walk order.
pub(super) fn all_sends<'d>(_document: &Document, context: &Context<'d>) -> Vec<&'d Send> {
    state_leaves(context, true)
        .into_iter()
        .filter_map(|node| match node {
            Executable::Send(send) => Some(send.as_ref()),
            _ => None,
        })
        .collect()
}

/// An `<assign>` has an `expr` or child content, not both.
pub(super) fn assigns(_document: &Document, context: &Context<'_>) -> Vec<ValidationError> {
    state_leaves(context, false)
        .into_iter()
        .filter_map(|node| match node {
            Executable::Assign(assign) => {
                let expr = assign.expr.as_ref()?;
                (assign.markup.is_some() || !is_blank(&assign.text)).then(|| {
                    ValidationError::new(
                        Reason::AssignExprAndText { expr: expr.clone() },
                        assign.node_location,
                    )
                })
            }
            _ => None,
        })
        .collect()
}

/// An `<if>`'s `<else>` comes last and once: every branch after the first
/// `<else>` is reported, a second `<else>` as a duplicate and an `<elseif>`
/// as out of order. Nested `<if>`s are checked too.
pub(super) fn ifs(_document: &Document, context: &Context<'_>) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    for state in &context.states {
        for content in state_content(state, false) {
            check_ifs(content, &mut errors);
        }
    }
    errors
}

fn check_ifs(content: &[Executable], errors: &mut Vec<ValidationError>) {
    for node in content {
        match node {
            Executable::If(node) => {
                let mut seen_else = false;
                for branch in &node.branches {
                    if seen_else {
                        let reason = if branch.cond.is_none() {
                            Reason::IfDuplicateElse
                        } else {
                            Reason::IfElseifAfterElse
                        };
                        errors.push(ValidationError::new(reason, branch.location));
                    }
                    seen_else |= branch.cond.is_none();
                }
                for branch in &node.branches {
                    check_ifs(&branch.content, errors);
                }
            }
            Executable::Foreach(node) => check_ifs(&node.content, errors),
            _ => {}
        }
    }
}

/// A `<script>`, in a state or under the root, has text; lowering refuses
/// one with a `src`.
pub(super) fn scripts(document: &Document, context: &Context<'_>) -> Vec<ValidationError> {
    let nested = state_leaves(context, false)
        .into_iter()
        .filter_map(|node| match node {
            Executable::Script(script) => Some(script),
            _ => None,
        });
    nested
        .chain(&document.scripts)
        .filter(|script| is_blank(&script.text))
        .map(|script| ValidationError::new(Reason::ScriptNoSrcOrText, script.location))
        .collect()
}

/// An `<invoke>` writes no exclusive pair, each reported at the element.
pub(super) fn invokes(_document: &Document, context: &Context<'_>) -> Vec<ValidationError> {
    context
        .states
        .iter()
        .flat_map(|state| &state.invoke)
        .flat_map(invoke_errors)
        .collect()
}

fn invoke_errors(invoke: &Invoke) -> Vec<ValidationError> {
    let src = invoke.src.is_some() || invoke.srcexpr.is_some();
    [
        (
            invoke.invoke_type.is_some() && invoke.typeexpr.is_some(),
            Reason::InvokeTypeAndTypeexpr,
        ),
        (
            invoke.src.is_some() && invoke.srcexpr.is_some(),
            Reason::InvokeSrcAndSrcexpr,
        ),
        (invoke.content.is_some() && src, Reason::InvokeSrcAndContent),
        (
            invoke.id.is_some() && invoke.idlocation.is_some(),
            Reason::InvokeIdAndIdlocation,
        ),
        (
            !invoke.namelist.is_empty() && !invoke.params.is_empty(),
            Reason::InvokeNamelistAndParam,
        ),
    ]
    .into_iter()
    .filter(|(broken, _)| *broken)
    .map(|(_, reason)| ValidationError::new(reason, invoke.location))
    .collect()
}

/// A `<send>` writes no exclusive pair, no delay to the internal target
/// (`_internal` or `#_internal`, as written), and no `namelist` or `<param>`
/// beside `<content>`; each reported at the element.
pub(super) fn sends(document: &Document, context: &Context<'_>) -> Vec<ValidationError> {
    all_sends(document, context)
        .into_iter()
        .flat_map(send_errors)
        .collect()
}

fn send_errors(send: &Send) -> Vec<ValidationError> {
    let delay = send.delay.is_some() || send.delayexpr.is_some();
    let internal = matches!(send.target.as_deref(), Some("_internal" | "#_internal"));
    [
        (
            send.event.is_some() && send.eventexpr.is_some(),
            Reason::SendEventAndEventexpr,
        ),
        (
            send.target.is_some() && send.targetexpr.is_some(),
            Reason::SendTargetAndTargetexpr,
        ),
        (
            send.send_type.is_some() && send.typeexpr.is_some(),
            Reason::SendTypeAndTypeexpr,
        ),
        (
            send.id.is_some() && send.idlocation.is_some(),
            Reason::SendIdAndIdlocation,
        ),
        (
            send.delay.is_some() && send.delayexpr.is_some(),
            Reason::SendDelayAndDelayexpr,
        ),
        (delay && internal, Reason::SendDelayAndInternalTarget),
        (
            !send.namelist.is_empty() && send.content.is_some(),
            Reason::SendNamelistAndContent,
        ),
        (
            !send.params.is_empty() && send.content.is_some(),
            Reason::SendParamAndContent,
        ),
    ]
    .into_iter()
    .filter(|(broken, _)| *broken)
    .map(|(_, reason)| ValidationError::new(reason, send.location))
    .collect()
}

/// A `<cancel>` writes exactly one of `sendid` and `sendidexpr`.
pub(super) fn cancels(_document: &Document, context: &Context<'_>) -> Vec<ValidationError> {
    state_leaves(context, true)
        .into_iter()
        .filter_map(|node| match node {
            Executable::Cancel(cancel) => {
                let reason = match (&cancel.sendid, &cancel.sendidexpr) {
                    (None, None) => Reason::CancelNoSendid,
                    (Some(_), Some(_)) => Reason::CancelSendidAndSendidexpr,
                    _ => return None,
                };
                Some(ValidationError::new(reason, cancel.location))
            }
            _ => None,
        })
        .collect()
}

/// The one warning: a `<raise>` or `<send>` inside an `<invoke>`'s
/// `<finalize>`, at the element's own span.
pub(super) fn finalize_warnings(_document: &Document, context: &Context<'_>) -> Vec<Warning> {
    let mut found = Vec::new();
    for block in context
        .states
        .iter()
        .flat_map(|state| &state.invoke)
        .filter_map(|invoke| invoke.finalize.as_ref())
    {
        leaves(&block.content, &mut found);
    }
    found
        .into_iter()
        .filter_map(|node| match node {
            Executable::Raise(_) | Executable::Send(_) => Some(Warning::new(
                WarningReason::FinalizeForbiddenContent {
                    element: node.element_name(),
                },
                node.location(),
            )),
            _ => None,
        })
        .collect()
}
