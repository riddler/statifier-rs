//! The ratchet, the only writer of `conformance/registry.json`
//! (`mise run ratchet`; rule 2 of the copy's `RATCHET.md`: the ratchet only
//! moves forward).
//!
//! Its only input is the results of a run made now over the whole corpus;
//! there is no way to add a case by id, so every entry the file holds was
//! added by a run that observed the case pass. It never removes an entry: an
//! entry the run did not observe passing is a regression, and it refuses to
//! write, naming each one. Otherwise it writes the union of the recorded
//! entries and the passes, pinned to the vendored manifest's `corpus_hash`,
//! with exactly the claims those entries count toward. A run that passes
//! nothing new writes the same bytes again.

use crate::corpus::{Corpus, IMPLEMENTATION};
use crate::registry::{self, Entry, Registry};
use crate::runner::{Outcome, Results};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::Path;

/// Writes the registry at `path` from `results` (a run over `corpus`).
/// Answers the problems that stopped it, having written nothing.
pub fn write(path: &Path, corpus: &Corpus, results: &Results) -> Result<(), Vec<String>> {
    let name = path
        .file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    let bytes = std::fs::read(path).map_err(|error| vec![format!("{name}: {error}")])?;
    let existing = registry::parse(&bytes).map_err(|problem| vec![format!("{name}: {problem}")])?;
    let next = next_registry(&existing, corpus, results)?;
    std::fs::write(path, registry::encode(&next)).map_err(|error| vec![format!("{name}: {error}")])
}

/// The registry `results` earn from `existing`, or the problems that refuse
/// it.
pub fn next_registry(
    existing: &Registry,
    corpus: &Corpus,
    results: &Results,
) -> Result<Registry, Vec<String>> {
    let cases = corpus.by_id();
    let mut problems = run_problems(corpus, results);
    for entry in &existing.entries {
        let id = entry.case_id.as_str();
        match cases.get(id) {
            None => problems.push(format!("{id} is recorded, and the corpus does not hold it")),
            Some(item) => {
                let suite = item.get("suite").and_then(Value::as_str).unwrap_or("none");
                if suite != entry.suite {
                    problems.push(format!(
                        "{id} is recorded under suite {}; the corpus puts it in {suite}",
                        entry.suite
                    ));
                } else if results.get(id) != Some(&Outcome::Pass) {
                    problems.push(format!("{id} is recorded and did not pass in this run"));
                }
            }
        }
    }
    if !problems.is_empty() {
        return Err(problems);
    }
    let recorded: BTreeSet<&str> = existing
        .entries
        .iter()
        .map(|entry| entry.case_id.as_str())
        .collect();
    let mut entries = existing.entries.clone();
    for id in results.passes() {
        if !recorded.contains(id) {
            let suite = cases
                .get(id)
                .and_then(|item| item.get("suite"))
                .and_then(Value::as_str)
                .unwrap_or("none");
            entries.push(Entry::new(id, suite));
        }
    }
    entries.sort_by(|left, right| left.order().cmp(&right.order()));
    Ok(Registry {
        implementation: IMPLEMENTATION.to_owned(),
        corpus_hash: corpus.corpus_hash.clone(),
        claims: registry::claims_of(&entries, &cases),
        entries,
    })
}

/// A run that is not of the whole corpus is no run to ratchet from.
fn run_problems(corpus: &Corpus, results: &Results) -> Vec<String> {
    if results.is_empty() {
        return vec!["no case was run: there is no run to ratchet from".to_owned()];
    }
    let ids: BTreeSet<&str> = corpus.ids().into_iter().collect();
    if results.ids() == ids && results.outcomes.len() == ids.len() {
        return Vec::new();
    }
    vec![format!(
        "the run is not of the whole corpus: {} results for {} cases",
        results.outcomes.len(),
        ids.len()
    )]
}
