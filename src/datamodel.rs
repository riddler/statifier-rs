//! The datamodel binding: how a chart's expressions reach the predicator
//! crate, the one expression language, as they reach it in the reference.
//!
//! The datamodel is a [`Map`] of root names to predicator values. Every
//! evaluation runs through [`predicator_rs::evaluate`] (a condition, a value)
//! or [`predicator_rs::execute`] (a `<script>` program) with the options one
//! evaluation site builds ([`Scope::options`]): `In()` as a host function over
//! the configuration the site was given, and the unbound policy that fails a
//! load of a root the datamodel does not hold. A site's options are built per
//! call and never kept: `In()` reads a configuration that moves at every
//! microstep, and `_event` is rewritten for every event.
//!
//! Nothing here panics or raises on a failed evaluation. A failure is a
//! [`Failure`] carrying its reason, and the interpreter turns it into the
//! `error.execution` platform event the reference raises
//! ([`Failure::event`]).

pub(crate) mod delay;
pub(crate) mod system_variables;

#[cfg(test)]
mod tests;

use system_variables::{EventFields, EventType};

use crate::machine::{Chart, Deferred, Expr, Program};
use crate::refusal::Refusal;
use predicator_rs::{Context, Error, Key, Map, OnUnbound, Options, Value};
use std::collections::BTreeSet;

/// The name of the platform event every failure here raises.
pub(crate) const ERROR_EXECUTION: &str = "error.execution";

/// The unbound policy every evaluation passes: a load of a root the
/// datamodel does not hold fails at the load, as the reference evaluates. A
/// declared root with no value holds [`Value::Undefined`] instead, and reads
/// as undefined.
pub(crate) const UNBOUND: OnUnbound = OnUnbound::Error;

/// One evaluation site's view: the datamodel, and the chart and the
/// configuration (the indexes of the active states) `In()` reads.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Scope<'a> {
    /// The datamodel.
    pub(crate) data: &'a Map,
    /// The chart whose state ids `In()` resolves.
    pub(crate) chart: &'a Chart,
    /// The indexes of the active states.
    pub(crate) configuration: &'a BTreeSet<usize>,
}

impl<'a> Scope<'a> {
    /// The options an evaluation at this site passes to the predicator
    /// crate: `In` as a host function and the unbound policy [`UNBOUND`].
    pub(crate) fn options(&self) -> Options<'a> {
        let chart = self.chart;
        let configuration = self.configuration;
        Options::new()
            .on_unbound(UNBOUND)
            .function("In", move |arguments| {
                in_state(chart, configuration, arguments)
            })
    }

    /// The context an evaluation at this site reads: the datamodel's roots.
    fn context(&self) -> Context {
        Context::from(self.data.clone())
    }
}

/// `In(stateId)`: true when the id names a state of `chart` that is in
/// `configuration`, false for an inactive state and for an id the chart
/// never declared, as the reference answers. An argument that is not a
/// string names no state, so it answers false too. It takes exactly one
/// argument, as the reference's arity does.
pub(crate) fn in_state(
    chart: &Chart,
    configuration: &BTreeSet<usize>,
    arguments: &[Value],
) -> Result<Value, String> {
    match arguments {
        [Value::String(id)] => Ok(Value::Bool(
            chart
                .id_to_index
                .get(id)
                .is_some_and(|index| configuration.contains(index)),
        )),
        [_] => Ok(Value::Bool(false)),
        _ => Err(format!(
            "In expects one argument, a state id; it was given {}",
            arguments.len()
        )),
    }
}

/// Why an evaluation, a program or a delay failed: the reason the
/// `error.execution` it raises carries as its data.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Failure {
    /// The predicator crate refused: `source` is the expression's text and
    /// `error` the crate's own error, unwrapped.
    Evaluation {
        /// The expression or program as written.
        source: String,
        /// The predicator crate's error.
        error: Error,
    },
    /// The expression did not compile; the reference defers that refusal to
    /// when the node runs.
    Compile(Box<Refusal>),
    /// A condition answered something other than true or false.
    NonBooleanCond(Value),
    /// A write reached `root`, a system variable's root (one spelled with a
    /// leading underscore).
    SystemVariable(String),
    /// A delay that is not one: text the duration reader refuses, or a value
    /// that is neither text nor a duration.
    InvalidDelay(Value),
}

impl Failure {
    /// The failure as the predicator value an `error.*` event carries as its
    /// data, so a chart reads `_event.data` with one more evaluation: a map
    /// whose `"kind"` names the failure, with its fields beside it.
    pub(crate) fn data(&self) -> Value {
        let mut map = Map::new();
        let kind = match self {
            Failure::Evaluation { source, error } => {
                map.insert("source", Value::from(source.as_str()));
                map.insert("type", Value::from(error.type_name()));
                map.insert("reason", Value::from(error.reason()));
                map.insert("message", Value::from(error.message()));
                "evaluator_error"
            }
            Failure::Compile(refusal) => {
                map.insert("message", Value::from(refusal.message()));
                "compile_error"
            }
            Failure::NonBooleanCond(value) => {
                map.insert("value", value.clone());
                "non_boolean_cond"
            }
            Failure::SystemVariable(root) => {
                map.insert("root", Value::from(root.as_str()));
                "system_variable"
            }
            Failure::InvalidDelay(value) => {
                map.insert("value", value.clone());
                "invalid_delay"
            }
        };
        map.insert("kind", Value::from(kind));
        Value::Map(map)
    }

    /// `_event`'s value for the `error.execution` platform event this
    /// failure raises: its name, the type `platform`, no send id, origin or
    /// invoke id, and the failure ([`Failure::data`]) as its data.
    pub(crate) fn event(&self) -> Value {
        system_variables::event(&EventFields {
            name: ERROR_EXECUTION,
            event_type: EventType::Platform,
            sendid: None,
            origin: None,
            origintype: None,
            invokeid: None,
            data: self.data(),
        })
    }
}

/// Evaluates `expr` at `scope`. A literal comes back untouched; a compiled
/// expression runs through the predicator crate, and a refusal is a
/// [`Failure::Evaluation`] carrying the expression's text.
pub(crate) fn evaluate(scope: &Scope<'_>, expr: &Expr) -> Result<Value, Failure> {
    match expr {
        Expr::Static(value) => Ok(value.clone()),
        Expr::Compiled {
            instructions,
            source,
        } => predicator_rs::evaluate(instructions, &scope.context(), &scope.options()).map_err(
            |error| Failure::Evaluation {
                source: source.clone(),
                error,
            },
        ),
    }
}

/// Evaluates a value whose compile refusal is deferred: a value that did
/// not compile answers that refusal, as the reference raises it when the
/// node runs.
pub(crate) fn evaluate_deferred(
    scope: &Scope<'_>,
    expr: &Deferred<Expr>,
) -> Result<Value, Failure> {
    match expr {
        Deferred::Ready(expr) => evaluate(scope, expr),
        Deferred::Invalid(refusal) => Err(Failure::Compile(refusal.clone())),
    }
}

/// Evaluates a transition's or a branch's condition: none written is true.
/// A condition that fails, or answers anything but true or false, is a
/// failure; the specification treats both as false and raises
/// `error.execution` for both, as the reference does.
pub(crate) fn evaluate_cond(scope: &Scope<'_>, cond: Option<&Expr>) -> Result<bool, Failure> {
    let Some(cond) = cond else {
        return Ok(true);
    };
    match evaluate(scope, cond)? {
        Value::Bool(answer) => Ok(answer),
        other => Err(Failure::NonBooleanCond(other)),
    }
}

/// What a program's run left: the datamodel, every write made before a
/// failure included, and the failure, if there was one.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Executed {
    /// The datamodel the run leaves.
    pub(crate) data: Map,
    /// Why the run failed, if it did.
    pub(crate) failure: Option<Failure>,
}

/// Runs a `<script>` program at `scope`, as the reference runs one.
///
/// Every root the datamodel already holds whose name starts with an
/// underscore is protected through the predicator crate's own option: a
/// store into one fails at the attempt, so no later statement runs and none
/// reads the value, and the failure is a [`Failure::SystemVariable`] naming
/// it. The four system variables are always among those roots, since a
/// session seeds them ([`system_variables::seed`]). A system root the program creates fresh
/// is not on that list: its write lands, the run goes on, and the change
/// found after the run is never kept and fails the run, naming the first
/// such root in sort order. The writes to other roots made before any
/// failure are kept. A program that did not compile answers its refusal and
/// writes nothing.
pub(crate) fn execute(scope: &Scope<'_>, program: &Deferred<Program>) -> Executed {
    let program = match program {
        Deferred::Ready(program) => program,
        Deferred::Invalid(refusal) => {
            return Executed {
                data: scope.data.clone(),
                failure: Some(Failure::Compile(refusal.clone())),
            };
        }
    };
    let before = scope.data;
    let options = scope.options().protected_roots(protected_roots(before));
    let (context, stored, run_failure) =
        match predicator_rs::execute(&program.instructions, &scope.context(), &options) {
            Ok(execution) => {
                let stored = execution.stored_roots().to_vec();
                (execution.into_context(), stored, None)
            }
            Err(stopped) => {
                let stored = stopped.stored_roots().to_vec();
                let failure = run_failure(&program.source, stopped.error());
                (stopped.into_context(), stored, Some(failure))
            }
        };
    let mut data = before.clone();
    let mut system_changed = Vec::new();
    for root in stored {
        let after = context.get(&root).cloned().unwrap_or(Value::Undefined);
        if before.get(&root) == Some(&after) {
            continue;
        }
        match &root {
            Key::String(name) if system_root(name) => system_changed.push(name.clone()),
            _ => {
                data.insert(root, after);
            }
        }
    }
    system_changed.sort();
    let failure = run_failure.or_else(|| {
        system_changed
            .into_iter()
            .next()
            .map(Failure::SystemVariable)
    });
    Executed { data, failure }
}

/// The failure a stopped run answers: the crate's refusal of a protected
/// root names the root; any other is the crate's own error.
fn run_failure(source: &str, error: &Error) -> Failure {
    match error {
        Error::Evaluation(refusal) if refusal.reason() == "protected_root" => {
            match refusal.root() {
                Some(root) => Failure::SystemVariable(root.to_owned()),
                None => Failure::Evaluation {
                    source: source.to_owned(),
                    error: error.clone(),
                },
            }
        }
        _ => Failure::Evaluation {
            source: source.to_owned(),
            error: error.clone(),
        },
    }
}

/// The roots a program may not store into, derived per run as the reference
/// derives them: every root `data` holds that is a system root. The crate's
/// option is a membership set and the rule is a prefix, so the set is
/// built from the roots already there.
pub(crate) fn protected_roots(data: &Map) -> Vec<String> {
    data.keys()
        .filter_map(|key| match key {
            Key::String(name) if system_root(name) => Some(name.clone()),
            _ => None,
        })
        .collect()
}

/// Whether `root` is a system variable's: the rule is the underscore
/// prefix, as the reference's, so it covers the four seeded variables and
/// any other root spelled that way.
pub(crate) fn system_root(root: &str) -> bool {
    root.starts_with('_')
}
