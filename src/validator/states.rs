//! The checks of the state tree: ids, targets, initials, default
//! transitions, histories, finals and default entry.

use super::context::{Context, is_compound, kind_name};
use super::finding::{DefaultTransitionOwner, Reason, ValidationError};
use crate::document::{Document, State, StateKind, Transition};
use crate::xml::Location;
use std::collections::HashSet;

/// A state's or a `<data>`'s id is unique across the document; the first
/// occurrence in document order reports nothing and every later one is a
/// duplicate. An id written empty is its own error and joins no set.
pub(super) fn ids(document: &Document, context: &Context<'_>) -> Vec<ValidationError> {
    let mut entries: Vec<(&str, Location)> = Vec::new();
    let data = document
        .datamodel_element
        .iter()
        .chain(
            context
                .states
                .iter()
                .filter_map(|state| state.datamodel_element.as_ref()),
        )
        .flat_map(|datamodel| &datamodel.data)
        .map(|data| {
            (
                data.id.as_str(),
                id_location(&data.attribute_locations, data.location),
            )
        });
    entries.extend(data);
    entries.extend(context.states.iter().filter_map(|state| {
        let id = state.id.as_deref()?;
        Some((id, id_location(&state.attribute_locations, state.location)))
    }));
    entries.sort_by_key(|(_, location)| location.start_offset);
    let mut empty = Vec::new();
    let mut duplicates = Vec::new();
    let mut seen = HashSet::new();
    for (id, location) in entries {
        if id.is_empty() {
            empty.push(ValidationError::new(Reason::EmptyId, location));
        } else if !seen.insert(id) {
            duplicates.push(ValidationError::new(
                Reason::DuplicateId { id: id.into() },
                location,
            ));
        }
    }
    empty.extend(duplicates);
    empty
}

fn id_location(locations: &crate::document::AttributeLocations, location: Location) -> Location {
    attribute_location(locations, "id", location)
}

fn attribute_location(
    locations: &crate::document::AttributeLocations,
    name: &str,
    fallback: Location,
) -> Location {
    locations.get(name).copied().unwrap_or(fallback)
}

fn target_location(transition: &Transition) -> Location {
    attribute_location(
        &transition.attribute_locations,
        "target",
        transition.location,
    )
}

/// Every transition target, a default transition's included, names a state.
pub(super) fn targets(_document: &Document, context: &Context<'_>) -> Vec<ValidationError> {
    context
        .transitions
        .iter()
        .flat_map(|transition| {
            transition
                .target
                .iter()
                .filter(|id| context.resolve(id).is_none())
                .map(|id| {
                    ValidationError::new(
                        Reason::UnresolvedTarget { id: id.clone() },
                        target_location(transition),
                    )
                })
        })
        .collect()
}

/// Every `initial`, attribute or element, lands inside its state, and only
/// a state with children to enter carries one; the root's `initial` need
/// only resolve. An element's target that does not resolve is the target
/// check's to report.
pub(super) fn initial_targets(document: &Document, context: &Context<'_>) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    for (index, state) in context.states.iter().enumerate() {
        if state.initial.is_empty() && state.initial_element.is_none() {
            continue;
        }
        if state.states.is_empty()
            || matches!(
                state.kind,
                StateKind::Parallel | StateKind::Final | StateKind::History
            )
        {
            let location = match &state.initial_element {
                Some(initial) if state.initial.is_empty() => initial.location,
                _ => attribute_location(&state.attribute_locations, "initial", state.location),
            };
            errors.push(ValidationError::new(
                Reason::InitialOnAtomicState {
                    id: state.id.clone(),
                },
                location,
            ));
            continue;
        }
        let location = attribute_location(&state.attribute_locations, "initial", state.location);
        for id in &state.initial {
            match context.resolve(id) {
                None => errors.push(ValidationError::new(
                    Reason::UnresolvedInitial { id: id.clone() },
                    location,
                )),
                Some(target) if !context.inside(target, index) => {
                    errors.push(not_descendant(id, state, location));
                }
                Some(_) => {}
            }
        }
        for transition in state
            .initial_element
            .iter()
            .flat_map(|initial| &initial.transitions)
        {
            for id in &transition.target {
                if context
                    .resolve(id)
                    .is_some_and(|target| !context.inside(target, index))
                {
                    errors.push(not_descendant(id, state, target_location(transition)));
                }
            }
        }
    }
    let location = attribute_location(&document.attribute_locations, "initial", document.location);
    errors.extend(
        document
            .initial
            .iter()
            .filter(|id| context.resolve(id).is_none())
            .map(|id| ValidationError::new(Reason::UnresolvedInitial { id: id.clone() }, location)),
    );
    errors
}

fn not_descendant(id: &str, parent: &State, location: Location) -> ValidationError {
    ValidationError::new(
        Reason::InitialNotDescendant {
            id: id.into(),
            parent_id: parent.id.clone(),
        },
        location,
    )
}

/// An `<initial>` element is not written beside an `initial` attribute, and
/// holds one default transition.
pub(super) fn initial_elements(
    _document: &Document,
    context: &Context<'_>,
) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    for state in &context.states {
        let Some(initial) = &state.initial_element else {
            continue;
        };
        if !state.initial.is_empty() {
            errors.push(ValidationError::new(
                Reason::InitialAttributeAndElement {
                    id: state.id.clone(),
                },
                initial.location,
            ));
        }
        errors.extend(default_transition(
            &DefaultTransitionOwner::Initial(state.id.clone()),
            &initial.transitions,
            initial.location,
        ));
    }
    errors
}

// The content model an `<initial>` and a `<history>` share: exactly one
// transition, with a target, and no `event` or `cond`. A written `event` is
// told from an absent one by its location, since both lower to no events.
fn default_transition(
    owner: &DefaultTransitionOwner,
    transitions: &[Transition],
    owner_location: Location,
) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    if transitions.len() != 1 {
        errors.push(ValidationError::new(
            Reason::TransitionCount {
                owner: owner.clone(),
                count: transitions.len(),
            },
            owner_location,
        ));
    }
    for transition in transitions {
        if transition.target.is_empty() {
            errors.push(ValidationError::new(
                Reason::TransitionMissingTarget {
                    owner: owner.clone(),
                },
                transition.location,
            ));
        }
        let written = [
            (
                "event",
                transition.attribute_locations.contains_key("event"),
            ),
            ("cond", transition.cond.is_some()),
        ];
        for (attribute, _) in written.into_iter().filter(|(_, written)| *written) {
            errors.push(ValidationError::new(
                Reason::TransitionForbiddenAttribute {
                    owner: owner.clone(),
                    attribute,
                },
                attribute_location(
                    &transition.attribute_locations,
                    attribute,
                    transition.location,
                ),
            ));
        }
    }
    errors
}

/// A `<history>` sits in a compound state, holds one default transition
/// whose resolved targets sit inside that parent, and has a `type` of
/// `shallow` or `deep` as written.
pub(super) fn histories(_document: &Document, context: &Context<'_>) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    for (index, state) in context.states.iter().enumerate() {
        if state.kind != StateKind::History {
            continue;
        }
        let parent = context
            .parent(index)
            .and_then(|parent| Some((parent, *context.states.get(parent)?)));
        let compound_parent = parent.filter(|(_, parent)| is_compound(parent));
        if compound_parent.is_none() {
            errors.push(ValidationError::new(
                Reason::HistoryBadParent {
                    id: state.id.clone(),
                    parent_kind: parent.map_or("scxml", |(_, parent)| kind_name(parent.kind)),
                },
                state.location,
            ));
        }
        errors.extend(default_transition(
            &DefaultTransitionOwner::History(state.id.clone()),
            &state.transitions,
            state.location,
        ));
        if let Some((parent_index, parent)) = compound_parent {
            for transition in &state.transitions {
                for id in &transition.target {
                    if context
                        .resolve(id)
                        .is_some_and(|target| !context.inside(target, parent_index))
                    {
                        errors.push(not_descendant(id, parent, target_location(transition)));
                    }
                }
            }
        }
        if let Some(location) = state.attribute_locations.get("type") {
            let raw = location.slice(context.source).unwrap_or_default();
            if !matches!(raw, "shallow" | "deep") {
                errors.push(ValidationError::new(
                    Reason::HistoryBadType { raw: raw.into() },
                    *location,
                ));
            }
        }
    }
    errors
}

/// A `<final>` holds no state and no transition, each reported at the
/// child.
pub(super) fn finals(_document: &Document, context: &Context<'_>) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    for state in context
        .states
        .iter()
        .filter(|state| state.kind == StateKind::Final)
    {
        errors.extend(state.states.iter().map(|child| {
            ValidationError::new(
                Reason::FinalHasStates {
                    id: child.id.clone(),
                },
                child.location,
            )
        }));
        errors.extend(state.transitions.iter().map(|transition| {
            ValidationError::new(
                Reason::FinalHasTransitions {
                    id: state.id.clone(),
                },
                transition.location,
            )
        }));
    }
    errors
}

/// A state that holds a `<final>` has an id, since its completion event is
/// named after it: reported once per such state.
pub(super) fn final_parents(_document: &Document, context: &Context<'_>) -> Vec<ValidationError> {
    context
        .states
        .iter()
        .filter(|state| state.id.is_none())
        .filter_map(|state| {
            let first = state
                .states
                .iter()
                .find(|child| child.kind == StateKind::Final)?;
            Some(ValidationError::new(
                Reason::FinalParentMissingId {
                    final_id: first.id.clone(),
                },
                state.location,
            ))
        })
        .collect()
}

/// A `<state>` with no initial is entered at its first child, which must
/// not be a `<history>`. A `<parallel>` enters every child, so it is not
/// held to this.
pub(super) fn default_entries(_document: &Document, context: &Context<'_>) -> Vec<ValidationError> {
    context
        .states
        .iter()
        .filter(|state| {
            state.kind == StateKind::State
                && state.initial.is_empty()
                && state.initial_element.is_none()
        })
        .filter_map(|state| {
            let first = state.states.first()?;
            (first.kind == StateKind::History).then(|| {
                ValidationError::new(
                    Reason::DefaultEntryNotEnterable {
                        id: state.id.clone(),
                        child_kind: "history",
                    },
                    first.location,
                )
            })
        })
        .collect()
}
