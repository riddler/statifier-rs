//! An SCXML statechart interpreter core.
//!
//! [`compile`] reads an SCXML document into a [`Machine`]: an immutable
//! value, cheap to clone and safe to share between threads, that holds every
//! state interned in document order and every expression compiled through
//! the predicator crate. A document it cannot compile is answered with the
//! [`Refusal`]s of the first stage that refused it.
//!
//! [`Machine::start`] starts a chart in execution: a [`State`], an
//! immutable value that [`State::step`] (an external event) and
//! [`State::advance`] (the virtual clock) move, each answering the new
//! state and the [`Effect`]s the call produced. The driver takes no
//! runtime, thread, clock or I/O of its own: a send of a type the host
//! registered is answered as an effect for the host to deliver.
//!
//! [`State::export_position`] writes a chart's position in the string-id
//! vocabulary, and [`Machine::import_position`] reads one back onto the same
//! chart or another that holds every state it names.
//!
//! No function panics on a document or a position a host passes: every
//! refusal is a value, and nothing is configured globally.
//!
//! The `serde` feature, off by default, implements `Serialize` and
//! `Deserialize` for a [`Position`], an [`Effect`] and a [`Done`], with the
//! types they reach, and turns on the predicator crate's own for a
//! [`Value`].

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod compiler;
mod datamodel;
mod document;
mod driver;
mod effect;
mod interpreter;
mod lowering;
mod machine;
mod refusal;
#[cfg(feature = "serde")]
mod serde_support;
mod validator;
mod xml;

#[cfg(test)]
mod testing;

// The README's Rust examples, run as doctests.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeExamples;

pub use driver::{
    ActiveInvocation, Done, Error, ExportError, ImportError, Moved, Options, Position, RoundBudget,
    State, Status,
};
pub use effect::{BlockOwner, Effect, PathSegment};
pub use machine::Machine;
/// The value type of a chart's datamodel: the predicator crate's own
/// value, re-exported rather than mirrored, so a value a host passes in and
/// a value an expression answers are one type.
pub use predicator_rs::Value;
/// The map type of a chart's datamodel and of a map value: the predicator
/// crate's own, re-exported beside [`Value`], with its key type.
pub use predicator_rs::{Key, Map};
pub use refusal::{
    CompileReason, DefaultTransitionOwner, LoweringReason, ReadReason, Reason, Refusal, Span,
    ValidationReason, Warning, WarningReason,
};

/// Compiles the SCXML document `source` into its machine.
///
/// Reading a chart is four stages, each a port of the reference's: the
/// reader (the XML), lowering (the SCXML elements), the validator (the
/// specification's rules) and the compiler (the expressions, through the
/// predicator crate). The first stage that refuses the document answers
/// every refusal it found, sorted by where each starts, and no later stage
/// runs. A document that compiles carries the validator's warnings on the
/// machine ([`Machine::warnings`]).
///
/// ```
/// let machine = statifier::compile(
///     r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="a">
///          <state id="a"><transition event="e.go" cond="x > 1" target="b"/></state>
///          <final id="b"/>
///        </scxml>"#,
/// )?;
/// assert!(machine.warnings().is_empty());
///
/// let refusals = statifier::compile(
///     r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0">
///          <state id="a"><transition cond="x >" target="a"/></state>
///        </scxml>"#,
/// )
/// .unwrap_err();
/// assert!(matches!(
///     refusals[0].reason(),
///     statifier::Reason::Compile(statifier::CompileReason::ExpressionCompileError { .. })
/// ));
/// # Ok::<(), Vec<statifier::Refusal>>(())
/// ```
pub fn compile(source: &str) -> Result<Machine, Vec<Refusal>> {
    compile_with(source, validator::Options::default())
}

/// Compiles an invocation's inline `<content>` markup as the reference's
/// `Statifier.Invoke.Source` compiles it, a root with no namespace accepted;
/// `None` when any stage refuses it.
pub(crate) fn compile_inline(source: &str) -> Option<Machine> {
    compile_with(
        source,
        validator::Options {
            invoke_content_markup: true,
        },
    )
    .ok()
}

fn compile_with(source: &str, options: validator::Options) -> Result<Machine, Vec<Refusal>> {
    let root = xml::parse(source.as_bytes()).map_err(|error| vec![Refusal::from(error)])?;
    let document = lowering::lower(&root, source)
        .map_err(|errors| errors.into_iter().map(Refusal::from).collect::<Vec<_>>())?;
    drop(root);
    let warnings = validator::validate(&document, source, options).map_err(|refused| {
        refused
            .errors
            .into_iter()
            .map(Refusal::from)
            .collect::<Vec<_>>()
    })?;
    let warnings = warnings.into_iter().map(Warning::from).collect();
    compiler::compile(&document, warnings).map(Machine::new)
}

#[cfg(test)]
mod tests {
    // Sabotage: rename the package in Cargo.toml and this assertion goes red.
    #[test]
    fn the_crate_is_named_statifier() {
        assert_eq!(env!("CARGO_PKG_NAME"), "statifier");
    }
}
