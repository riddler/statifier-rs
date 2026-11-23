//! The typed document in, its errors and warnings out.
//!
//! The third step of reading a chart, a port of the reference's validator at
//! the vendored tag, and the one gate in front of the compiler: lowering
//! keeps what was written, shapes the specification forbids included, and
//! the validator holds the document to the rules lowering leaves to it. A
//! document with an error is not compiled; a warning is a finding the engine
//! has a behavior for either way, and never stops anything.
//!
//! The checks, in the order they run, each the reference's:
//!
//! - state ids are unique and none is empty, a `<data>` id sharing the set;
//! - every transition target resolves;
//! - every `initial` resolves, sits inside its state, and is not on a state
//!   with nothing to enter;
//! - an `<initial>` element is not beside an `initial` attribute, and holds
//!   one transition with a target and no `event` or `cond`;
//! - a `<history>` sits in a compound state, holds such a transition to a
//!   target inside its parent, and has a `type` of `shallow` or `deep`;
//! - a `<final>` holds no state and no transition, and its parent has an id;
//! - a `<state>` entered by default does not start with a `<history>`;
//! - `<donedata>`, `<content>`, `<param>`, `<datamodel>` and `<data>` keep
//!   their content models;
//! - the root's namespace and version, and the enumerated attributes
//!   (`type`, `binding`, `datamodel`, `autoforward`) as written;
//! - `<assign>`, `<if>`, `<script>`, `<invoke>`, `<send>` and `<cancel>`
//!   keep theirs;
//! - no attribute is repeated on one tag;
//!
//! and one warning: a `<raise>` or `<send>` inside `<finalize>`.
//!
//! Four contracts, the reference's own: every check runs over the whole
//! document on every call; the errors, and separately the warnings, are
//! sorted by where they start, findings at one offset keeping the order the
//! checks ran in; the document is never rewritten; and the warnings are
//! answered whether or not an error is. No check here reads an expression:
//! a `cond` or an `expr` is checked when the document is compiled.
//!
//! The compiler calls [`validate`] on the document lowering answered, with
//! the source it was read from, and stops on an `Err`.

mod attributes;
mod context;
mod data;
mod executable;
mod finding;
mod states;

#[cfg(test)]
mod tests;

pub(crate) use finding::{ValidationError, Warning};

use crate::document::Document;
use context::Context;

/// How a document is validated.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Options {
    /// Accept a root that resolves to no namespace, as inline invoke content
    /// is read; a root in another namespace is still refused.
    pub(crate) invoke_content_markup: bool,
}

/// A document that does not validate: every error, and every warning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Refused {
    /// The errors, in document order.
    pub(crate) errors: Vec<ValidationError>,
    /// The warnings, in document order.
    pub(crate) warnings: Vec<Warning>,
}

/// Holds `document`, lowered from `source`, to every rule that gates
/// compilation. Answers its warnings when no check found an error, and
/// every error with the warnings otherwise.
///
/// `source` must be the document the tree was read from: the enumerated
/// attributes and the attributes of each tag are read back out of it.
pub(crate) fn validate(
    document: &Document,
    source: &str,
    options: Options,
) -> Result<Vec<Warning>, Refused> {
    let context = Context::build(document, source, options.invoke_content_markup);
    let checks: [fn(&Document, &Context<'_>) -> Vec<ValidationError>; 21] = [
        states::ids,
        states::targets,
        states::initial_targets,
        states::initial_elements,
        states::histories,
        states::finals,
        states::final_parents,
        states::default_entries,
        data::donedata,
        data::contents,
        data::params,
        data::boilerplate,
        data::enums,
        data::data,
        executable::assigns,
        executable::ifs,
        executable::scripts,
        executable::invokes,
        executable::sends,
        executable::cancels,
        attributes::duplicates,
    ];
    let mut errors: Vec<ValidationError> = checks
        .iter()
        .flat_map(|check| check(document, &context))
        .collect();
    let mut warnings = executable::finalize_warnings(document, &context);
    // Stable sorts: findings at one offset keep the checks' order.
    errors.sort_by_key(|error| error.location.start_offset);
    warnings.sort_by_key(|warning| warning.location.start_offset);
    if errors.is_empty() {
        Ok(warnings)
    } else {
        Err(Refused { errors, warnings })
    }
}
