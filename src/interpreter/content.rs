//! The block runner: the specification's `executeContent` for one block of
//! executable content, ported from the reference's
//! (`Statifier.Interpreter.Content` and `Statifier.Machine.Content.*` in
//! statifier-ex).
//!
//! A block runs its nodes in document order against one datamodel: every
//! write a node makes (an `<assign>`, a `<foreach>`'s item and index, a
//! `<script>`'s program) is seen by every node after it. The first node
//! that fails stops the block. The nodes before it keep what they did; the
//! failing node answers no effect, and its failure becomes one
//! `error.execution` whose origin is that node, or, for a failure inside an
//! `<if>` or a `<foreach>`, the top-level node holding it, its failure then
//! naming the inner node ([`Failure::Nested`]). A failing leaf keeps
//! nothing; an `<if>` or a `<foreach>` keeps what its nodes wrote and
//! raised, though not their effects; a `<script>` keeps the writes its
//! program made before it stopped. Each block is independent: one that
//! fails leaves every other to run.
//!
//! An `<if>` condition that fails, or answers something other than true or
//! false, does not stop the block: its branch is passed over, and its
//! `error.execution` is raised once the top-level node holding it has run,
//! after any event its taken branch raised and before the node's own
//! failure, the order the reference takes.
//!
//! Nothing here moves a queue. The runner answers the events the block
//! raised, in the order they join the internal queue, beside its effects;
//! the step that called it queues them.

#[cfg(test)]
mod tests;

use crate::datamodel::location::write_location;
use crate::datamodel::{Failure, Scope, evaluate, evaluate_cond, evaluate_deferred, execute};
use crate::effect::{BlockOwner, Effect};
use crate::machine::{Assign, Branch, Chart, Content, Foreach};
use predicator_rs::{Key, Map, Value};
use std::collections::BTreeSet;

/// The counters as they stand while a block runs: no node moves them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Counters {
    /// The macrostep counter.
    pub(crate) macrostep: u64,
    /// The microstep counter.
    pub(crate) microstep: u64,
    /// The round counter.
    pub(crate) round: u64,
}

/// Where a block runs: the chart, the configuration `In()` reads, the
/// block's owner and the counters its effects carry.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Site<'a> {
    /// The chart whose content runs.
    pub(crate) chart: &'a Chart,
    /// The indexes of the active states.
    pub(crate) configuration: &'a BTreeSet<usize>,
    /// The block.
    pub(crate) owner: BlockOwner,
    /// The counters.
    pub(crate) counters: Counters,
}

/// An event a block raised onto the internal queue. Its origin is the
/// top-level node `content` of the block that raised it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Raised {
    /// A `<raise>`: an internal event with no data.
    Event {
        /// The event's name, as written.
        name: String,
        /// The index of the `<raise>`.
        content: usize,
    },
    /// A failure: the `error.execution` platform event carrying it
    /// ([`Failure::event`]).
    Error {
        /// Why.
        failure: Failure,
        /// The index of the top-level node it is raised against.
        content: usize,
    },
}

/// What a block answered.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Ran {
    /// The datamodel it leaves.
    pub(crate) data: Map,
    /// Its effects, in the order its nodes made them.
    pub(crate) effects: Vec<Effect>,
    /// The events it raised, in queue order.
    pub(crate) raised: Vec<Raised>,
    /// The index of every top-level node that ran, the failing one
    /// included.
    pub(crate) executed: Vec<usize>,
}

/// What a block threads from node to node.
struct Run {
    data: Map,
    effects: Vec<Effect>,
    raised: Vec<Raised>,
    pending: Vec<Failure>,
}

/// Runs the nodes `content` names, in document order, against `data`,
/// stopping at the first that fails.
pub(crate) fn execute_block(site: &Site<'_>, data: Map, content: &[usize]) -> Ran {
    let mut run = Run {
        data,
        effects: Vec::new(),
        raised: Vec::new(),
        pending: Vec::new(),
    };
    let mut executed = Vec::new();
    for &index in content {
        executed.push(index);
        let outcome = node(site, &mut run, index);
        for failure in std::mem::take(&mut run.pending) {
            run.raised.push(Raised::Error {
                failure,
                content: index,
            });
        }
        if let Err(failure) = outcome {
            run.raised.push(Raised::Error {
                failure,
                content: index,
            });
            break;
        }
    }
    Ran {
        data: run.data,
        effects: run.effects,
        raised: run.raised,
        executed,
    }
}

/// One node. On a failure the run keeps what the node wrote and raised,
/// never its effects.
fn node(site: &Site<'_>, run: &mut Run, index: usize) -> Result<(), Failure> {
    let effects = run.effects.len();
    let outcome = match site.chart.contents.get(index) {
        Some(Content::Raise { event, .. }) => {
            run.raised.push(Raised::Event {
                name: event.clone(),
                content: index,
            });
            Ok(())
        }
        Some(Content::Log { label, expr, .. }) => log(site, run, index, label, expr.as_ref()),
        Some(Content::Assign(assign)) => self::assign(site, run, index, assign),
        Some(Content::If { branches, .. }) => if_node(site, run, branches),
        Some(Content::Foreach(foreach)) => self::foreach(site, run, foreach),
        Some(Content::Script { program, .. }) => {
            let executed = execute(&scope(site, &run.data), program);
            run.data = executed.data;
            executed.failure.map_or(Ok(()), Err)
        }
        // `<send>` and `<cancel>` are not run here yet, and an index the
        // compiler never interned names no node: either runs as nothing.
        Some(Content::Send(_) | Content::Cancel { .. }) | None => Ok(()),
    };
    if outcome.is_err() {
        run.effects.truncate(effects);
    }
    outcome
}

/// The evaluation site a node reads: the block's datamodel as it stands.
fn scope<'a>(site: &Site<'a>, data: &'a Map) -> Scope<'a> {
    Scope {
        data,
        chart: site.chart,
        configuration: site.configuration,
    }
}

/// `<log>`: the label and the evaluated `expr` as an effect.
fn log(
    site: &Site<'_>,
    run: &mut Run,
    index: usize,
    label: &Option<String>,
    expr: Option<&crate::machine::Expr>,
) -> Result<(), Failure> {
    let value = match expr {
        Some(expr) => Some(evaluate(&scope(site, &run.data), expr)?),
        None => None,
    };
    run.effects.push(Effect::Log {
        label: label.clone(),
        value,
        content_index: index,
        owner: site.owner,
        macrostep: site.counters.macrostep,
        microstep: site.counters.microstep,
        round: site.counters.round,
    });
    Ok(())
}

/// `<assign>`: the value evaluated, then written at the location, which
/// never declares a root; a write that lands answers its change.
fn assign(site: &Site<'_>, run: &mut Run, index: usize, node: &Assign) -> Result<(), Failure> {
    let value = evaluate_deferred(&scope(site, &run.data), &node.value)?;
    let written = write_location(&run.data, &node.location, value)?;
    run.data = written.data;
    run.effects.push(Effect::DatamodelChange {
        location_path: written.path,
        location_source: node.location.clone(),
        new_value: written.new_value,
        prior_value: written.prior_value,
        data_index: None,
        content_index: Some(index),
        owner: Some(site.owner),
        macrostep: site.counters.macrostep,
        microstep: site.counters.microstep,
        round: site.counters.round,
    });
    Ok(())
}

/// `<if>`: the first branch whose condition is true runs, an `<else>`
/// always; a condition that fails or is not a boolean is recorded and its
/// branch passed over. Every condition is read against the datamodel as it
/// stood when the `<if>` began.
fn if_node(site: &Site<'_>, run: &mut Run, branches: &[Branch]) -> Result<(), Failure> {
    for branch in branches {
        match evaluate_cond(&scope(site, &run.data), branch.cond.as_ref()) {
            Ok(true) => return nested(site, run, &branch.content),
            Ok(false) => {}
            Err(failure) => run.pending.push(failure),
        }
    }
    Ok(())
}

/// A partition or a loop body: each node in order, the first failure
/// wrapped to name the inner node.
fn nested(site: &Site<'_>, run: &mut Run, content: &[usize]) -> Result<(), Failure> {
    for &index in content {
        node(site, run, index).map_err(|failure| Failure::Nested {
            content: index,
            failure: Box::new(failure),
        })?;
    }
    Ok(())
}

/// Whether `name` is a bare name, the one shape a `<foreach>` `item` and
/// `index` take.
fn bare_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

/// Why `name` cannot be bound, or `None`: a system variable's root, or not
/// a bare name.
fn illegal_name(name: &str, illegal: fn(String) -> Failure) -> Option<Failure> {
    if name.starts_with('_') {
        Some(Failure::SystemVariable(name.to_owned()))
    } else if bare_name(name) {
        None
    } else {
        Some(illegal(name.to_owned()))
    }
}

/// `<foreach>`: the names checked, `array` evaluated once (the list is a
/// value, so the body cannot change what is iterated), the item and index
/// declared undefined unless the datamodel holds them, even for an empty
/// list, and the body run once per element with the item and the zero-based
/// index bound. Unlike an `<assign>` it may bind a name the datamodel does
/// not yet hold. A body failure keeps what earlier iterations wrote.
fn foreach(site: &Site<'_>, run: &mut Run, node: &Foreach) -> Result<(), Failure> {
    if let Some(failure) = illegal_name(&node.item, Failure::IllegalItemName) {
        return Err(failure);
    }
    if let Some(failure) = node
        .index
        .as_deref()
        .and_then(|index| illegal_name(index, Failure::IllegalIndexName))
    {
        return Err(failure);
    }
    let items = match evaluate(&scope(site, &run.data), &node.array)? {
        Value::List(ref items) => items.clone(),
        other => return Err(Failure::NotIterable(other)),
    };
    let names: Vec<&str> = std::iter::once(node.item.as_str())
        .chain(node.index.as_deref())
        .collect();
    for name in names {
        let key = Key::from(name);
        if !run.data.contains_key(&key) {
            run.data.insert(key, Value::Undefined);
        }
    }
    for (position, item) in items.into_iter().enumerate() {
        run.data.insert(node.item.as_str(), item);
        if let Some(index) = node.index.as_deref() {
            run.data.insert(
                index,
                Value::Integer(i64::try_from(position).unwrap_or(i64::MAX)),
            );
        }
        nested(site, run, &node.content)?;
    }
    Ok(())
}
