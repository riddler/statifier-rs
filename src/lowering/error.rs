//! Why an element tree could not be lowered.

use crate::xml::Location;

/// The most elements the walk descends into, counting from the root, before
/// it refuses the next one. The reference has no bound; this one keeps every
/// later walk of the document off the edge of the call stack.
pub(crate) const MAX_DEPTH: usize = 128;

/// Why lowering refused something, with the detail each reason carries.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// An element in SCXML's vocabulary that no builder reads, named as
    /// written.
    UnsupportedElement {
        /// The qualified name as written.
        name: String,
    },
    /// A lowered element its parent has no slot for.
    MisplacedElement {
        /// The element's name: its slot's, or its own for executable content.
        name: String,
        /// The parent's qualified name as written.
        parent: String,
    },
    /// Text that is not blank where the content model has no room for it.
    StrayText {
        /// The run, trimmed.
        text: String,
    },
    /// A root element other than `<scxml>`.
    UnexpectedRoot {
        /// The root's local name.
        name: String,
    },
    /// A required attribute that is absent.
    MissingAttribute {
        /// The element's name.
        element: &'static str,
        /// The attribute's name.
        attribute: &'static str,
    },
    /// An element whose name resolves to a namespace that is not SCXML's.
    ForeignElement {
        /// The qualified name as written.
        name: String,
        /// The namespace it resolves to.
        uri: String,
    },
    /// An attribute on an element that takes none.
    UnexpectedAttribute {
        /// The element's name.
        element: &'static str,
        /// The attribute's name as written.
        attribute: String,
    },
    /// An attribute the specification defines and this engine does not act
    /// on.
    UnsupportedAttribute {
        /// The element's name.
        element: &'static str,
        /// The attribute's name.
        attribute: &'static str,
    },
    /// An element nested more than 128 elements below the root.
    NestingTooDeep {
        /// The qualified name as written.
        name: String,
    },
}

/// One refusal: the reason, a message for a person, and the span refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LoweringError {
    /// Why it was refused.
    pub(crate) reason: Reason,
    /// What is wrong, for a person.
    pub(crate) message: String,
    /// The span refused.
    pub(crate) location: Location,
}

impl LoweringError {
    pub(super) fn unsupported(name: &str, location: Location) -> Self {
        LoweringError {
            message: format!("unsupported element {name:?}"),
            reason: Reason::UnsupportedElement { name: name.into() },
            location,
        }
    }

    pub(super) fn misplaced(name: &str, parent: &str, location: Location) -> Self {
        LoweringError {
            message: format!("element {name:?} is not allowed inside {parent:?}"),
            reason: Reason::MisplacedElement {
                name: name.into(),
                parent: parent.into(),
            },
            location,
        }
    }

    // The message previews the trimmed run at forty code points.
    pub(super) fn stray_text(trimmed: &str, location: Location) -> Self {
        let preview: String = trimmed.chars().take(40).collect();
        let preview = if trimmed.chars().count() > 40 {
            format!("{preview}...")
        } else {
            preview
        };
        LoweringError {
            message: format!("stray text {preview:?} is not allowed here"),
            reason: Reason::StrayText {
                text: trimmed.into(),
            },
            location,
        }
    }

    pub(super) fn unexpected_root(name: &str, location: Location) -> Self {
        LoweringError {
            message: format!("expected the document root to be <scxml>, got {name:?}"),
            reason: Reason::UnexpectedRoot { name: name.into() },
            location,
        }
    }

    pub(super) fn missing_attribute(
        element: &'static str,
        attribute: &'static str,
        location: Location,
    ) -> Self {
        LoweringError {
            message: format!("element {element:?} is missing required attribute {attribute:?}"),
            reason: Reason::MissingAttribute { element, attribute },
            location,
        }
    }

    pub(super) fn foreign_element(name: &str, uri: &str, location: Location) -> Self {
        LoweringError {
            message: format!("element {name:?} is bound to foreign namespace {uri:?}, not SCXML"),
            reason: Reason::ForeignElement {
                name: name.into(),
                uri: uri.into(),
            },
            location,
        }
    }

    pub(super) fn unexpected_attribute(
        element: &'static str,
        attribute: &str,
        location: Location,
    ) -> Self {
        LoweringError {
            message: format!("element {element:?} does not accept attribute {attribute:?}"),
            reason: Reason::UnexpectedAttribute {
                element,
                attribute: attribute.into(),
            },
            location,
        }
    }

    pub(super) fn unsupported_attribute(
        element: &'static str,
        attribute: &'static str,
        location: Location,
    ) -> Self {
        LoweringError {
            message: format!(
                "element {element:?}'s {attribute:?} attribute is defined by the spec but not \
                 implemented by this engine"
            ),
            reason: Reason::UnsupportedAttribute { element, attribute },
            location,
        }
    }

    pub(super) fn nesting_too_deep(name: &str, location: Location) -> Self {
        LoweringError {
            message: format!(
                "element {name:?} is nested more than {MAX_DEPTH} elements below the root"
            ),
            reason: Reason::NestingTooDeep { name: name.into() },
            location,
        }
    }
}
