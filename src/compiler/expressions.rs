//! Every expression a chart writes compiles here, once, through the
//! predicator crate: a condition or a value through its expression compiler,
//! a `<script>` body through its program compiler, as the reference compiles
//! them.

use crate::machine::{Deferred, Expr, Program};
use crate::refusal::{CompileReason, Reason, Refusal};
use crate::xml::Location;
use predicator_rs::{Context, OnUnbound, Options, Value};

/// `source` compiled as an expression, or the refusal at `location` that
/// the predicator crate's refusal makes.
pub(super) fn compile(source: &str, location: Location) -> Result<Expr, Refusal> {
    match predicator_rs::compile(source) {
        Ok(instructions) => Ok(Expr::Compiled {
            instructions,
            source: source.to_owned(),
        }),
        Err(error) => Err(refusal(source, &error, location)),
    }
}

/// `compile`'s answer, a refusal deferred to when the value executes: the
/// reference's reading of an `<assign>` or `<data>` value and a `namelist`
/// entry.
pub(super) fn deferred(source: &str, location: Location) -> Deferred<Expr> {
    match compile(source, location) {
        Ok(expr) => Deferred::Ready(expr),
        Err(refusal) => Deferred::Invalid(Box::new(refusal)),
    }
}

/// `source` compiled as a statement program, a refusal deferred to when the
/// program executes.
pub(super) fn program(source: &str, location: Location) -> Deferred<Program> {
    match predicator_rs::compile_program(source) {
        Ok(instructions) => Deferred::Ready(Program {
            instructions,
            source: source.to_owned(),
        }),
        Err(error) => Deferred::Invalid(Box::new(refusal(source, &error, location))),
    }
}

/// A literal value: nothing to evaluate.
pub(super) fn literal(value: impl Into<Value>) -> Expr {
    Expr::Static(value.into())
}

/// An in-line value, a `<data>` or `<assign>` body, folded to a literal:
/// the trimmed text compiled and evaluated against an empty context that
/// refuses every unbound name, and the trimmed text itself when either step
/// fails. So `[1, 2]` is a list, `3` a number, and `x` the string `"x"`
/// rather than a read of a variable.
pub(super) fn inline_value(text: &str) -> Expr {
    let trimmed = text.trim();
    let options = Options::new().on_unbound(OnUnbound::Error);
    match predicator_rs::compile(trimmed)
        .and_then(|instructions| predicator_rs::evaluate(&instructions, &Context::new(), &options))
    {
        Ok(value) => Expr::Static(value),
        Err(_) => literal(trimmed),
    }
}

// The refusal for the predicator crate's `error` on `source`, with the
// reference's message.
fn refusal(source: &str, error: &predicator_rs::Error, location: Location) -> Refusal {
    let position = error
        .position()
        .map(|position| (position.line, position.column));
    let at = position.map_or_else(String::new, |(line, column)| {
        format!(" (predicator line {line}, column {column})")
    });
    Refusal::new(
        Reason::Compile(CompileReason::ExpressionCompileError {
            source: source.to_owned(),
            cause: error.reason().to_owned(),
            position,
        }),
        format!(
            "failed to compile expression {source:?}: {}{at}",
            error.message()
        ),
        location,
    )
}
