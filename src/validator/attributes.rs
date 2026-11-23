//! The check of the markup itself: an attribute name appears once on a tag
//! (XML 1.0 section 3.1, "Unique Att Spec"). The reader keeps every
//! repeated attribute and lowering reads the first, so the repeat is read
//! back out of the source: every tag, markup inside `<content>`, `<assign>`
//! and `<data>` included.

use super::context::Context;
use super::finding::{Reason, ValidationError};
use crate::document::Document;
use crate::xml::{Element, Node, parse};
use std::collections::HashSet;

/// Each later occurrence of a name on one tag, at that attribute's span;
/// names are compared as written, prefix included.
pub(super) fn duplicates(_document: &Document, context: &Context<'_>) -> Vec<ValidationError> {
    let Ok(root) = parse(context.source.as_bytes()) else {
        return Vec::new();
    };
    let mut errors = Vec::new();
    // A work list: markup kept as text is not bounded by lowering's depth.
    let mut pending: Vec<&Element> = vec![&root];
    while let Some(element) = pending.pop() {
        let mut seen = HashSet::new();
        for attribute in &element.attributes {
            if !seen.insert(attribute.name.as_str()) {
                errors.push(ValidationError::new(
                    Reason::DuplicateAttribute {
                        element: element.name.clone(),
                        attribute: attribute.name.clone(),
                    },
                    attribute.location,
                ));
            }
        }
        pending.extend(
            element
                .children
                .iter()
                .rev()
                .filter_map(|child| match child {
                    Node::Element(child) => Some(child),
                    Node::Text(_) => None,
                }),
        );
    }
    errors
}
