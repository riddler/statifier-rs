//! `<send>` and `<cancel>`, as the reference's send and cancel nodes run
//! them (`Statifier.Machine.Content.Send` and `.Cancel`, with
//! `Statifier.Send.Target`, `.Types` and `.Routes`, in statifier-ex).
//!
//! The core delivers nothing and schedules nothing. A `<send>` answers one
//! effect describing the message, [`Effect::Send`] when it goes now and
//! [`Effect::SendDelayed`] when it waits, whatever its target, the
//! internal queue included: the driver delivers it. A `<cancel>` answers an
//! [`Effect::Cancel`].
//!
//! A `<send>` resolves its arguments in the reference's order: the event,
//! the target, the type, the delay, the `namelist` and `<param>` values in
//! document order, then the `<content>`. The first that fails discards the
//! send: no id is minted, nothing is written, and its failure is the
//! block's. Then the send id is minted (or the author's `id` read) and
//! `idlocation` written; a write that fails discards the send the same way.
//! Only then are the type and the target judged: a refused send keeps its
//! minted id and its `idlocation` write, answers no effect, and fails with
//! [`Failure::SendRejected`].

#[cfg(test)]
mod tests;

use super::{Run, Site, scope};
use crate::datamodel::delay;
use crate::datamodel::event_data;
use crate::datamodel::location::write_location;
use crate::datamodel::system_variables::SCXML_EVENT_PROCESSOR;
use crate::datamodel::{Failure, Rejection, RejectionKind, Scope, evaluate, evaluate_deferred};
use crate::effect::Effect;
use crate::machine::{Expr, Send};
use predicator_rs::Value;
use std::collections::BTreeSet;

/// The two counters a session carries for its sends from block to block:
/// `send_counter`, the sequence a generated send id is minted from (the
/// next id is `send_` and the counter plus one), and `timer_counter`, the
/// ordinal sequence every delayed send, every cancel and every immediate
/// send of a registered type advances. Each is advanced before it is read,
/// so the first id is `send_1` and the first ordinal 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Sends {
    /// The last generated send id's number.
    pub(crate) send_counter: u64,
    /// The last ordinal minted.
    pub(crate) timer_counter: u64,
}

/// What the driver declares a session can reach: the session ids a
/// `#_scxml_` target may name, whether the session has a parent, and the
/// invoke ids a `#_` target may name. The session itself and its internal
/// queue need no entry.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Routes {
    /// The reachable session ids.
    pub(crate) sessions: BTreeSet<String>,
    /// Whether the session has a parent.
    pub(crate) parent: bool,
    /// The reachable invoke ids.
    pub(crate) invokes: BTreeSet<String>,
}

impl Routes {
    /// Whether `route` is reachable.
    fn reachable(&self, route: &Route) -> bool {
        match route {
            Route::Own | Route::Internal => true,
            Route::Parent => self.parent,
            Route::Session(id) => self.sessions.contains(id),
            Route::Invoke(id) => self.invokes.contains(id),
            Route::Invalid => false,
        }
    }
}

/// What a resolved target names.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Route {
    /// No target: the session's own external queue.
    Own,
    /// The session's internal queue.
    Internal,
    /// The session's parent.
    Parent,
    /// Another session, by id.
    Session(String),
    /// An invoked session, by invoke id.
    Invoke(String),
    /// Nothing the SCXML Event I/O Processor carries.
    Invalid,
}

/// Parses a resolved target as the reference does. No target, or null,
/// is the session itself; both spellings of the internal and parent
/// targets, with and without the leading `#`, are accepted; `#_scxml_` and
/// a session id names a session, any other `#_` and a name an invoked
/// session; anything else, a value that is not text included, is invalid.
fn parse_target(target: Option<&Value>) -> Route {
    let text = match target {
        None | Some(Value::Null) => return Route::Own,
        Some(Value::String(text)) => text.as_str(),
        Some(_) => return Route::Invalid,
    };
    match text {
        "#_internal" | "_internal" => Route::Internal,
        "#_parent" | "_parent" => Route::Parent,
        _ => match (text.strip_prefix("#_scxml_"), text.strip_prefix("#_")) {
            (Some(session), _) if !session.is_empty() => Route::Session(session.to_owned()),
            (_, Some(invoke)) if !invoke.is_empty() => Route::Invoke(invoke.to_owned()),
            _ => Route::Invalid,
        },
    }
}

/// How a resolved type is carried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TypeClass {
    /// No type, null, the short form `scxml` or the SCXML Event I/O
    /// Processor's URI: the target is the specification's vocabulary.
    BuiltIn,
    /// A type the host registered: the target is its processor's own.
    Registered,
    /// Anything else.
    Unsupported,
}

/// Classifies a resolved type; a built-in spelling stays built in whatever
/// the host registered.
fn classify(send_types: &BTreeSet<String>, send_type: Option<&Value>) -> TypeClass {
    match send_type {
        None | Some(Value::Null) => TypeClass::BuiltIn,
        Some(Value::String(text)) if text == "scxml" || text == SCXML_EVENT_PROCESSOR => {
            TypeClass::BuiltIn
        }
        Some(Value::String(text)) if send_types.contains(text) => TypeClass::Registered,
        Some(_) => TypeClass::Unsupported,
    }
}

/// Why a send is refused, or `None`. The type is judged first; a
/// registered type's target is never parsed; with routes declared, an
/// immediate send's target must be reachable, while a delayed one is
/// routed when its delay has passed.
fn reject_reason(
    site: &Site<'_>,
    target: Option<&Value>,
    send_type: Option<&Value>,
    delayed: bool,
) -> Option<(RejectionKind, Rejection)> {
    let resolved = |value: Option<&Value>| value.cloned().unwrap_or(Value::Null);
    match classify(site.send_types, send_type) {
        TypeClass::Unsupported => Some((
            RejectionKind::Execution,
            Rejection::UnsupportedType(resolved(send_type)),
        )),
        TypeClass::Registered => None,
        TypeClass::BuiltIn => {
            let route = parse_target(target);
            if route == Route::Invalid {
                return Some((
                    RejectionKind::Execution,
                    Rejection::InvalidTarget(resolved(target)),
                ));
            }
            match site.routes {
                Some(routes) if !delayed && !routes.reachable(&route) => Some((
                    RejectionKind::Communication,
                    Rejection::UnreachableTarget(resolved(target)),
                )),
                _ => None,
            }
        }
    }
}

/// An optional attribute's value: `None` when it was not written.
fn optional(scope: &Scope<'_>, expr: Option<&Expr>) -> Result<Option<Value>, Failure> {
    expr.map(|expr| evaluate(scope, expr)).transpose()
}

/// The event's data: the `<content>`'s, when the `<send>` has one (text
/// read as [`event_data::text`] reads it, an expression's value as it is),
/// else the `namelist` and `<param>` values, the first failure stopping.
fn payload(scope: &Scope<'_>, node: &Send) -> Result<Value, Failure> {
    let mut pairs = Vec::with_capacity(node.namelist.len() + node.params.len());
    for param in node.namelist.iter().chain(&node.params) {
        pairs.push((param.name.as_str(), evaluate_deferred(scope, &param.expr)?));
    }
    match &node.content {
        None => Ok(event_data::params(pairs)),
        Some(Expr::Static(Value::String(text))) => Ok(event_data::text(text)),
        Some(expr) => evaluate(scope, expr),
    }
}

/// `<send>`: the arguments resolved, the id minted and `idlocation`
/// written, the type and target judged, then the `idlocation` write's
/// change and the send's own effect.
pub(super) fn send(
    site: &Site<'_>,
    run: &mut Run,
    index: usize,
    node: &Send,
) -> Result<(), Failure> {
    let scope = scope(site, &run.data);
    let event = optional(&scope, node.event.as_ref())?;
    let target = optional(&scope, node.target.as_ref())?;
    let send_type = optional(&scope, node.send_type.as_ref())?;
    let delay_ms = match &node.delay {
        Some(expr) => Some(delay::milliseconds(&evaluate(&scope, expr)?)?),
        None => None,
    };
    let data = payload(&scope, node)?;
    let (send_id, send_counter) = match &node.id {
        Some(id) => (id.clone(), run.sends.send_counter),
        None => {
            let next = run.sends.send_counter.saturating_add(1);
            (format!("send_{next}"), next)
        }
    };
    let written = match &node.idlocation {
        Some(location) => Some(write_location(
            &run.data,
            location,
            Value::from(send_id.as_str()),
        )?),
        None => None,
    };
    run.sends.send_counter = send_counter;
    let change = written.map(|written| {
        run.data = written.data;
        Effect::DatamodelChange {
            location_path: written.path,
            location_source: node.idlocation.clone().unwrap_or_default(),
            new_value: written.new_value,
            prior_value: written.prior_value,
            data_index: None,
            content_index: Some(index),
            owner: Some(site.owner),
            macrostep: site.counters.macrostep,
            microstep: site.counters.microstep,
            round: site.counters.round,
        }
    });
    if let Some((kind, rejection)) = reject_reason(
        site,
        target.as_ref(),
        send_type.as_ref(),
        delay_ms.is_some(),
    ) {
        return Err(Failure::SendRejected {
            send_id,
            kind,
            rejection,
        });
    }
    let registered = classify(site.send_types, send_type.as_ref()) == TypeClass::Registered;
    let ordinal = (delay_ms.is_some() || registered).then(|| {
        run.sends.timer_counter = run.sends.timer_counter.saturating_add(1);
        run.sends.timer_counter
    });
    run.effects.extend(change);
    let id_from_author = node.id.is_some() || node.idlocation.is_some();
    let counters = site.counters;
    run.effects.push(match (delay_ms, ordinal) {
        (Some(delay_ms), Some(ordinal)) => Effect::SendDelayed {
            event,
            target,
            send_type,
            data,
            send_id,
            id_from_author,
            delay_ms,
            ordinal,
            content_index: index,
            owner: site.owner,
            macrostep: counters.macrostep,
            microstep: counters.microstep,
            round: counters.round,
        },
        _ => Effect::Send {
            event,
            target,
            send_type,
            data,
            send_id,
            id_from_author,
            ordinal,
            content_index: index,
            owner: site.owner,
            macrostep: counters.macrostep,
            microstep: counters.microstep,
            round: counters.round,
        },
    });
    Ok(())
}

/// `<cancel>`: the send id resolved, the timer counter advanced, and the
/// effect naming the send. An id that names nothing pending is the
/// driver's no-op, not a failure. A `<cancel>` with neither attribute,
/// which the validator refuses, names [`Value::Undefined`].
pub(super) fn cancel(
    site: &Site<'_>,
    run: &mut Run,
    index: usize,
    sendid: Option<&Expr>,
) -> Result<(), Failure> {
    let send_id = optional(&scope(site, &run.data), sendid)?.unwrap_or(Value::Undefined);
    run.sends.timer_counter = run.sends.timer_counter.saturating_add(1);
    run.effects.push(Effect::Cancel {
        send_id,
        ordinal: run.sends.timer_counter,
        content_index: index,
        owner: site.owner,
        macrostep: site.counters.macrostep,
        microstep: site.counters.microstep,
        round: site.counters.round,
    });
    Ok(())
}
