//! The index the checks share: built once per validation from the document
//! and dropped with it, never kept or handed to the compiler.

use crate::document::{Document, State, StateKind, Transition};
use std::collections::HashMap;

/// Every state in document order, a parent before its children, with where
/// each sits; the named states by id; and every transition.
pub(super) struct Context<'d> {
    /// The document the tree was read from.
    pub(super) source: &'d str,
    /// Every state, in document order.
    pub(super) states: Vec<&'d State>,
    /// Each state's parent, as an index into `states`; `None` for a state
    /// directly under the root.
    parents: Vec<Option<usize>>,
    /// Each named state's index; a repeated id keeps the last state written
    /// with it, as the reference's index does.
    by_id: HashMap<&'d str, usize>,
    /// Every transition in document order: a state's own, then its
    /// `<initial>` element's.
    pub(super) transitions: Vec<&'d Transition>,
    /// Whether a root that resolves to no namespace is accepted.
    pub(super) invoke_content_markup: bool,
}

impl<'d> Context<'d> {
    pub(super) fn build(
        document: &'d Document,
        source: &'d str,
        invoke_content_markup: bool,
    ) -> Self {
        let mut context = Context {
            source,
            states: Vec::new(),
            parents: Vec::new(),
            by_id: HashMap::new(),
            transitions: Vec::new(),
            invoke_content_markup,
        };
        // Lowering bounds the nesting, so the walk's recursion is bounded.
        context.walk(&document.states, None);
        for (index, state) in context.states.iter().enumerate() {
            if let Some(id) = &state.id {
                context.by_id.insert(id, index);
            }
            context.transitions.extend(&state.transitions);
            if state.kind != StateKind::History {
                if let Some(initial) = &state.initial_element {
                    context.transitions.extend(&initial.transitions);
                }
            }
        }
        context
    }

    fn walk(&mut self, children: &'d [State], parent: Option<usize>) {
        for state in children {
            let index = self.states.len();
            self.states.push(state);
            self.parents.push(parent);
            self.walk(&state.states, Some(index));
        }
    }

    /// The index of the state named `id`, if one is.
    pub(super) fn resolve(&self, id: &str) -> Option<usize> {
        self.by_id.get(id).copied()
    }

    /// The parent of the state at `index`, `None` for the root.
    pub(super) fn parent(&self, index: usize) -> Option<usize> {
        self.parents.get(index).copied().flatten()
    }

    /// Whether the state at `index` sits inside the state at `ancestor`, by
    /// the tree's structure: a state with no id, and two states that share
    /// one, are placed all the same.
    pub(super) fn inside(&self, index: usize, ancestor: usize) -> bool {
        let mut current = self.parent(index);
        while let Some(parent) = current {
            if parent == ancestor {
                return true;
            }
            current = self.parent(parent);
        }
        false
    }
}

/// Whether `state` is a `<state>` or `<parallel>` with children to enter.
pub(super) fn is_compound(state: &State) -> bool {
    matches!(state.kind, StateKind::State | StateKind::Parallel) && !state.states.is_empty()
}

/// The element name of a state kind.
pub(super) fn kind_name(kind: StateKind) -> &'static str {
    match kind {
        StateKind::State => "state",
        StateKind::Parallel => "parallel",
        StateKind::Final => "final",
        StateKind::History => "history",
    }
}
