//! What [`compile`](crate::compile) answers besides a machine: the refusals
//! of the stage that refused a chart, and the warnings a chart that compiles
//! carries.
//!
//! Every refusal and every warning is a reason, a message for a person and
//! the span of the source it names. The reason is normative: each variant is
//! one of the reference's reasons, under its name, with the detail it
//! carries, apart from the owner [`CompileReason::ExpressionCompileError`]
//! leaves out. The message is not.
//!
//! Every variant that carries detail is `#[non_exhaustive]`, so a later
//! version may add a field: a host matches its fields with `..` and never
//! builds one. A field naming an element, an attribute or a kind of state
//! from SCXML's fixed vocabulary is a `&'static str`.

use crate::xml::Location;

pub use crate::lowering::Reason as LoweringReason;
pub use crate::validator::{DefaultTransitionOwner, Reason as ValidationReason, WarningReason};
pub use crate::xml::Reason as ReadReason;

/// A span of the source: 1-based lines and columns, 0-based byte offsets and
/// an exclusive end. Columns count code points, and only a line feed starts
/// a new line.
///
/// ```
/// let refusals = statifier::compile("<scxml").unwrap_err();
/// let span = refusals[0].span();
/// assert_eq!((span.start_line, span.start_column), (1, 7));
/// ```
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    /// The line the span starts on.
    pub start_line: usize,
    /// The column, in code points, the span starts at.
    pub start_column: usize,
    /// The byte offset the span starts at.
    pub start_offset: usize,
    /// The line the span ends on.
    pub end_line: usize,
    /// The column the span ends at.
    pub end_column: usize,
    /// The byte offset just past the span.
    pub end_offset: usize,
}

impl From<Location> for Span {
    fn from(location: Location) -> Self {
        Span {
            start_line: location.start_line,
            start_column: location.start_column,
            start_offset: location.start_offset,
            end_line: location.end_line,
            end_column: location.end_column,
            end_offset: location.end_offset,
        }
    }
}

/// Why a chart was refused: the stage that refused it, with that stage's own
/// reason.
///
/// ```
/// use statifier::{ReadReason, Reason};
///
/// let refusals = statifier::compile("<scxml").unwrap_err();
/// assert_eq!(refusals[0].reason(), &Reason::Read(ReadReason::UnexpectedEnd));
/// ```
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// The source is not XML the reader reads.
    Read(ReadReason),
    /// The element tree is not an SCXML document.
    Lowering(LoweringReason),
    /// The document breaks a rule of the specification.
    Validation(ValidationReason),
    /// An expression of the document does not compile.
    Compile(CompileReason),
}

/// Why a valid document does not compile.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompileReason {
    /// The predicator crate refused an expression the chart writes: a
    /// condition, a value or a `<content>` expression. A refused `<assign>`
    /// or `<data>` value, `namelist` entry or `<script>` body is not
    /// refused here: it compiles to a step that fails when it executes.
    ///
    /// The reference also names the expression's owner (a transition, a
    /// node of executable content, a `<donedata>`, a `<data>`, a global
    /// script or an `<invoke>`); this version does not, and the refusal's
    /// span locates the expression instead.
    #[non_exhaustive]
    ExpressionCompileError {
        /// The expression as written.
        source: String,
        /// The predicator crate's reason for the refusal.
        cause: String,
        /// Where in the expression the predicator crate stopped, as a line
        /// and a column, both counted from 1, when it says.
        position: Option<(usize, usize)>,
    },
}

/// One refusal: the reason, a message for a person, and the span refused.
///
/// ```
/// let refusals = statifier::compile("<scxml").unwrap_err();
/// assert!(refusals[0].message().contains("line 1"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    // Boxed: the reasons carry their detail, and a refusal travels in a
    // `Result`.
    reason: Box<Reason>,
    message: String,
    span: Span,
}

impl Refusal {
    pub(crate) fn new(reason: Reason, message: String, location: Location) -> Self {
        Refusal {
            reason: Box::new(reason),
            message,
            span: location.into(),
        }
    }

    /// Why the chart was refused.
    pub fn reason(&self) -> &Reason {
        &self.reason
    }

    /// What is wrong, for a person.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// The span refused.
    pub fn span(&self) -> Span {
        self.span
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for Refusal {}

/// A warning on a chart that compiles: a finding the engine has a behavior
/// for either way, which never stops a chart compiling.
///
/// ```
/// let source = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0">
///   <state id="a"><invoke type="scxml"><finalize><raise event="e"/></finalize>
///   <content><scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0"/></content>
///   </invoke></state></scxml>"#;
/// let machine = statifier::compile(source)?;
/// assert_eq!(machine.warnings().len(), 1);
/// assert_eq!(machine.warnings()[0].message(), "<raise> must not occur inside <finalize>");
/// # Ok::<(), Vec<statifier::Refusal>>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    reason: WarningReason,
    message: String,
    span: Span,
}

impl Warning {
    /// What was found.
    pub fn reason(&self) -> &WarningReason {
        &self.reason
    }

    /// What was found, for a person.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// The span reported.
    pub fn span(&self) -> Span {
        self.span
    }
}

impl From<crate::xml::ParseError> for Refusal {
    fn from(error: crate::xml::ParseError) -> Self {
        Refusal::new(Reason::Read(error.reason), error.message, error.location)
    }
}

impl From<crate::lowering::LoweringError> for Refusal {
    fn from(error: crate::lowering::LoweringError) -> Self {
        Refusal::new(
            Reason::Lowering(error.reason),
            error.message,
            error.location,
        )
    }
}

impl From<crate::validator::ValidationError> for Refusal {
    fn from(error: crate::validator::ValidationError) -> Self {
        Refusal::new(
            Reason::Validation(error.reason),
            error.message,
            error.location,
        )
    }
}

impl From<crate::validator::Warning> for Warning {
    fn from(warning: crate::validator::Warning) -> Self {
        Warning {
            reason: warning.reason,
            message: warning.message,
            span: warning.location.into(),
        }
    }
}
