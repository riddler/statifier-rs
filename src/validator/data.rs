//! The checks of data and the root: `<donedata>`, `<content>`, `<param>`,
//! the root's boilerplate, the enumerated attributes, `<datamodel>` and
//! `<data>`.

use super::context::{Context, kind_name};
use super::executable::all_sends;
use super::finding::{Reason, ValidationError, content_message};
use crate::document::{AttributeLocations, Content, Document, Param, StateKind};
use crate::lowering::{SCXML_NAMESPACE, is_blank};
use crate::xml::Location;

/// `<donedata>` sits only on a `<final>`, and holds a `<content>` or
/// `<param>` children, not both.
pub(super) fn donedata(_document: &Document, context: &Context<'_>) -> Vec<ValidationError> {
    let placed = context.states.iter().filter_map(|state| {
        let donedata = state.donedata.as_ref()?;
        (state.kind != StateKind::Final).then(|| {
            ValidationError::new(
                Reason::DonedataNotOnFinal {
                    id: state.id.clone(),
                },
                donedata.location,
            )
        })
    });
    let shaped = context.states.iter().filter_map(|state| {
        let donedata = state.donedata.as_ref()?;
        (donedata.content.is_some() && !donedata.params.is_empty()).then(|| {
            ValidationError::new(
                Reason::DonedataContentAndParams {
                    id: state.id.clone(),
                },
                donedata.location,
            )
        })
    });
    placed.chain(shaped).collect()
}

/// A `<content>`, under `<donedata>` or `<invoke>`, has an `expr` or inline
/// content, not both; blank text is formatting, not content.
pub(super) fn contents(_document: &Document, context: &Context<'_>) -> Vec<ValidationError> {
    context
        .states
        .iter()
        .flat_map(|state| {
            let donedata = state
                .donedata
                .iter()
                .filter_map(|donedata| donedata.content.as_ref());
            donedata.chain(
                state
                    .invoke
                    .iter()
                    .filter_map(|invoke| invoke.content.as_ref()),
            )
        })
        .filter_map(content_error)
        .collect()
}

fn content_error(content: &Content) -> Option<ValidationError> {
    let expr = content.expr.as_ref()?;
    let form = if content.markup.is_some() {
        "markup"
    } else if !is_blank(&content.text) {
        "text"
    } else {
        return None;
    };
    Some(ValidationError {
        reason: Reason::ContentExprAndText { expr: expr.clone() },
        message: content_message(expr, form),
        location: content.location,
    })
}

/// A `<param>`, under `<donedata>`, `<invoke>` or `<send>`, has exactly one
/// of `expr` and `location`.
pub(super) fn params(document: &Document, context: &Context<'_>) -> Vec<ValidationError> {
    let state_params = context.states.iter().flat_map(|state| {
        let donedata = state.donedata.iter().flat_map(|donedata| &donedata.params);
        donedata.chain(state.invoke.iter().flat_map(|invoke| &invoke.params))
    });
    let sends = all_sends(document, context);
    let send_params = sends.iter().flat_map(|send| &send.params);
    state_params
        .chain(send_params)
        .filter_map(param_error)
        .collect()
}

fn param_error(param: &Param) -> Option<ValidationError> {
    let reason = match (&param.expr, &param.param_location) {
        (None, None) => Reason::ParamNoValue {
            name: param.name.clone(),
        },
        (Some(_), Some(_)) => Reason::ParamExprAndLocation {
            name: param.name.clone(),
        },
        _ => return None,
    };
    Some(ValidationError::new(reason, param.location))
}

/// The root resolves to the SCXML namespace (or, with invoke content
/// markup, to none) and its `version` is `1.0`.
pub(super) fn boilerplate(document: &Document, context: &Context<'_>) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    let namespace = document.namespace.as_deref();
    let relaxed = context.invoke_content_markup && namespace.is_none();
    if namespace != Some(SCXML_NAMESPACE) && !relaxed {
        errors.push(ValidationError::new(
            Reason::BadNamespace {
                uri: document.namespace.clone(),
            },
            root_span(&document.attribute_locations, "xmlns", document.location),
        ));
    }
    if document.version.as_deref() != Some("1.0") {
        errors.push(ValidationError::new(
            Reason::BadVersion {
                version: document.version.clone(),
            },
            root_span(&document.attribute_locations, "version", document.location),
        ));
    }
    errors
}

fn root_span(locations: &AttributeLocations, name: &str, fallback: Location) -> Location {
    locations.get(name).copied().unwrap_or(fallback)
}

/// The enumerated attributes lowering maps onto a default, read back as
/// written: every transition's `type`, the root's `binding` and
/// `datamodel`, and every `<invoke>`'s `autoforward`. An attribute not
/// written is its default and always in range.
pub(super) fn enums(document: &Document, context: &Context<'_>) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    for transition in &context.transitions {
        errors.extend(out_of_range(
            &transition.attribute_locations,
            "type",
            &["internal", "external"],
            context,
            |raw| Reason::TransitionBadType { raw },
        ));
    }
    errors.extend(out_of_range(
        &document.attribute_locations,
        "binding",
        &["early", "late"],
        context,
        |raw| Reason::ScxmlBadBinding { raw },
    ));
    errors.extend(out_of_range(
        &document.attribute_locations,
        "datamodel",
        &["predicator", "elixir", "null", "ecmascript", "xpath"],
        context,
        |raw| Reason::ScxmlBadDatamodel { raw },
    ));
    for invoke in context.states.iter().flat_map(|state| &state.invoke) {
        errors.extend(out_of_range(
            &invoke.attribute_locations,
            "autoforward",
            &["true", "false"],
            context,
            |raw| Reason::InvokeBadAutoforward { raw },
        ));
    }
    errors
}

fn out_of_range(
    locations: &AttributeLocations,
    name: &str,
    allowed: &[&str],
    context: &Context<'_>,
    reason: impl Fn(String) -> Reason,
) -> Option<ValidationError> {
    let location = locations.get(name)?;
    let raw = location.slice(context.source).unwrap_or_default();
    (!allowed.contains(&raw)).then(|| ValidationError::new(reason(raw.into()), *location))
}

/// A `<data>` has `expr` or `src`, not both, and then no child text; its id
/// does not start with `_`. A `<datamodel>` is not on a `<final>` or a
/// `<history>`.
pub(super) fn data(document: &Document, context: &Context<'_>) -> Vec<ValidationError> {
    let datamodels = document.datamodel_element.iter().chain(
        context
            .states
            .iter()
            .filter_map(|state| state.datamodel_element.as_ref()),
    );
    let mut errors = Vec::new();
    for data in datamodels.flat_map(|datamodel| &datamodel.data) {
        let location = root_span(&data.attribute_locations, "id", data.location);
        let mut report = |reason| errors.push(ValidationError::new(reason, location));
        if data.expr.is_some() && data.src.is_some() {
            report(Reason::DataExprAndSrc {
                id: data.id.clone(),
            });
        }
        if (data.expr.is_some() || data.src.is_some()) && !is_blank(&data.text) {
            report(Reason::DataValueAndChildren {
                id: data.id.clone(),
            });
        }
        if data.id.starts_with('_') {
            report(Reason::DataReservedId {
                id: data.id.clone(),
            });
        }
    }
    for state in &context.states {
        if let Some(datamodel) = &state.datamodel_element {
            if matches!(state.kind, StateKind::Final | StateKind::History) {
                errors.push(ValidationError::new(
                    Reason::DatamodelBadParent {
                        kind: kind_name(state.kind),
                    },
                    datamodel.location,
                ));
            }
        }
    }
    errors
}
