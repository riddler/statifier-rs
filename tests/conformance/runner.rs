//! The run: every case of every suite, each answered pass or fail, with no
//! skip list and no third result.

use crate::corpus::{Corpus, id_of};
use crate::registry::Entry;
use serde_json::Value;
use std::collections::BTreeSet;
use std::panic::{AssertUnwindSafe, catch_unwind};

/// The reason every case fails while nothing drives it.
pub const NOT_DRIVEN: &str = "not driven: this crate has no interpreter yet";

/// What a drive answers for one case, and what the run records for it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Outcome {
    /// The case agrees with the corpus.
    Pass,
    /// The case does not, and why.
    Fail(String),
}

/// THE SEAM: what the run does with one case. It is the one place the crate
/// is plugged into the run; the runner, the registry check and the ratchet
/// all reach the crate through a drive and nothing else.
///
/// This crate has no compiler, interpreter or driver yet, so [`drive`]
/// answers every case with [`NOT_DRIVEN`] and the run passes nothing. The
/// change that brings them replaces its body with the drive the copy's
/// `RATCHET.md` describes under "What a case asserts": compile the case's
/// source, start it and compare the active leaf states with
/// `initial_configuration`, then send each step's event and compare with the
/// step's `configuration` (and, for a case with a `host` object or a stated
/// position, what `RATCHET.md` says the runner honours there).
pub fn drive(_item: &Value) -> Outcome {
    Outcome::Fail(NOT_DRIVEN.to_owned())
}

/// The results of one run, in corpus order.
#[derive(Debug, Clone, Default)]
pub struct Results {
    /// Each case id with its outcome.
    pub outcomes: Vec<(String, Outcome)>,
}

impl Results {
    /// The outcome recorded for `id`, when the run reached it.
    pub fn get(&self, id: &str) -> Option<&Outcome> {
        self.outcomes
            .iter()
            .find(|(case, _)| case == id)
            .map(|(_, outcome)| outcome)
    }

    /// Whether the run recorded nothing.
    pub fn is_empty(&self) -> bool {
        self.outcomes.is_empty()
    }

    /// The ids the run recorded, sorted.
    pub fn ids(&self) -> BTreeSet<&str> {
        self.outcomes.iter().map(|(id, _)| id.as_str()).collect()
    }

    /// The ids that passed, in run order.
    pub fn passes(&self) -> Vec<&str> {
        self.outcomes
            .iter()
            .filter(|(_, outcome)| *outcome == Outcome::Pass)
            .map(|(id, _)| id.as_str())
            .collect()
    }
}

/// One run over `corpus`: every case handed to `with` in corpus order. A
/// drive that answers a fail with no reason is recorded as failing with one,
/// and a drive that panics is a fail naming the panic: a case is never
/// skipped and never ends the run.
pub fn run(corpus: &Corpus, with: impl Fn(&Value) -> Outcome) -> Results {
    let outcomes = corpus
        .cases()
        .filter_map(|item| id_of(item).map(|id| (id.to_owned(), outcome_of(item, &with))))
        .collect();
    Results { outcomes }
}

fn outcome_of(item: &Value, with: &impl Fn(&Value) -> Outcome) -> Outcome {
    match catch_unwind(AssertUnwindSafe(|| with(item))) {
        Ok(Outcome::Fail(reason)) if reason.is_empty() => {
            Outcome::Fail("failed, giving no reason".to_owned())
        }
        Ok(outcome) => outcome,
        Err(payload) => {
            let message = payload
                .downcast_ref::<&str>()
                .map(|text| (*text).to_owned())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "a value that is not a message".to_owned());
            Outcome::Fail(format!("panicked: {message}"))
        }
    }
}

/// One line per suite: how many of its cases this crate's registry
/// (`entries`) does not claim, and how many of those the reference's
/// registry (`reference`) claims.
pub fn unclaimed_lines(corpus: &Corpus, entries: &[Entry], reference: &[Entry]) -> Vec<String> {
    let ours: BTreeSet<&str> = entries.iter().map(|entry| entry.case_id.as_str()).collect();
    let theirs: BTreeSet<&str> = reference
        .iter()
        .map(|entry| entry.case_id.as_str())
        .collect();
    corpus
        .suites
        .iter()
        .map(|suite| {
            let open: Vec<&str> = suite
                .cases
                .iter()
                .filter_map(id_of)
                .filter(|id| !ours.contains(id))
                .collect();
            format!(
                "conformance: {}: {} of {} cases not yet claimed (the reference claims {} of them)",
                suite.suite,
                open.len(),
                suite.cases.len(),
                open.iter().filter(|id| theirs.contains(*id)).count()
            )
        })
        .collect()
}
