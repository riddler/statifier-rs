//! An SCXML statechart interpreter core.
//!
//! [`compile`] reads an SCXML document into a [`Machine`]: an immutable
//! value, cheap to clone and safe to share between threads, that holds every
//! state interned in document order and every expression compiled through
//! the predicator crate. A document it cannot compile is answered with the
//! [`Refusal`]s of the first stage that refused it. The interpreter that
//! executes a machine arrives in a later version.
//!
//! No function panics on a document a host passes: every refusal is a
//! value, and nothing is configured globally.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod compiler;
// Nothing in the library evaluates through the binding until the
// interpreter does; the expectation fails the build once something does, so
// it comes off then.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "no caller until the interpreter evaluates through the binding"
    )
)]
mod datamodel;
mod document;
// Nothing in the library steps a machine until the interpreter loop does;
// the expectation fails the build once something does, so it comes off then.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "no caller until the interpreter loop selects, exits and enters"
    )
)]
mod interpreter;
mod lowering;
// Nothing in the library reads the machine until the interpreter steps it;
// the expectation fails the build once something does, so it comes off then.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "no caller until the interpreter reads the machine"
    )
)]
mod machine;
mod refusal;
mod validator;
mod xml;

#[cfg(test)]
mod testing;

pub use machine::Machine;
/// The value type of a chart's datamodel: the predicator crate's own
/// value, re-exported rather than mirrored, so a value a host passes in and
/// a value an expression answers are one type.
pub use predicator_rs::Value;
pub use refusal::{
    CompileReason, LoweringReason, Owner, ReadReason, Reason, Refusal, Span, ValidationReason,
    Warning, WarningReason,
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
    let root = xml::parse(source.as_bytes()).map_err(|error| vec![Refusal::from(error)])?;
    let document = lowering::lower(&root, source)
        .map_err(|errors| errors.into_iter().map(Refusal::from).collect::<Vec<_>>())?;
    drop(root);
    let warnings = validator::validate(&document, source, validator::Options::default()).map_err(
        |refused| {
            refused
                .errors
                .into_iter()
                .map(Refusal::from)
                .collect::<Vec<_>>()
        },
    )?;
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
