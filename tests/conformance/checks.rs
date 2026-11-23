//! The corpus check, a stage of the full gate (`mise run gate:corpus`): it
//! recomputes the corpus hash from the copy as the reference's vendoring
//! sequence does and requires it to equal each of four fields:
//!
//! 1. the vendored manifest's `corpus_hash`;
//! 2. the provenance record's `corpus_hash`, beside the copy;
//! 3. this crate's registry's `corpus_hash`, outside the copy;
//! 4. the vendored reference registry's `corpus_hash`, inside the copy.
//!
//! It also holds the manifest's list of suites to the suite files it names,
//! each present with the case count the manifest gives.
//!
//! A mismatch names the field that disagrees. It is never repaired by
//! rewriting a hash: the hash is what a claim of conformance is pinned to,
//! and a hash rewritten to match whatever is on disk pins it to nothing. The
//! fix is to restore the copied bytes, or to refresh the copy to a tag
//! (`conformance/README.md`).

use crate::corpus::{PROVENANCE_FILE, REGISTRY_FILE, VENDORED_DIR, corpus_hash, read_json};
use serde_json::Value;
use std::path::Path;

/// The four fields, each as its label and its path under the conformance
/// directory.
pub fn fields() -> [(&'static str, String); 4] {
    [
        (
            "the vendored manifest's corpus_hash",
            format!("{VENDORED_DIR}/manifest.json"),
        ),
        (
            "the provenance record's corpus_hash",
            PROVENANCE_FILE.to_owned(),
        ),
        (
            "this crate's registry's corpus_hash",
            REGISTRY_FILE.to_owned(),
        ),
        (
            "the vendored reference registry's corpus_hash",
            format!("{VENDORED_DIR}/registry.json"),
        ),
    ]
}

/// Every disagreement under `dir` (a conformance directory), one sentence
/// each; empty when the copy hashes to what all four fields say and the
/// manifest's suites are there.
pub fn problems(dir: &Path) -> Vec<String> {
    let Some(computed) = corpus_hash(dir) else {
        return vec![format!(
            "there is no corpus file under {VENDORED_DIR}/corpus: nothing to hash"
        )];
    };
    let mut problems: Vec<String> = fields()
        .iter()
        .filter_map(|(label, path)| field_problem(dir, label, path, &computed))
        .collect();
    problems.extend(suite_problems(dir));
    problems
}

fn field_problem(dir: &Path, label: &str, path: &str, computed: &str) -> Option<String> {
    if !dir.join(path).is_file() {
        return Some(format!("{label} cannot be read: {path} is missing"));
    }
    let value = match read_json(dir, path) {
        Ok(value) => value,
        Err(error) => return Some(format!("{label} cannot be read: {error}")),
    };
    match value.get("corpus_hash") {
        Some(Value::String(hash)) if hash == computed => None,
        Some(other) => Some(format!(
            "{label} is {other}; the suite files hash to {computed}"
        )),
        None => Some(format!(
            "{label} is absent; the suite files hash to {computed}"
        )),
    }
}

/// The manifest's suites, as (suite, file, case count); none when the
/// manifest does not read, which the first field has already named.
fn manifest_suites(dir: &Path) -> Vec<(String, String, Option<u64>)> {
    let Ok(manifest) = read_json(&dir.join(VENDORED_DIR), "manifest.json") else {
        return Vec::new();
    };
    manifest
        .get("suites")
        .and_then(Value::as_array)
        .map(|suites| {
            suites
                .iter()
                .map(|entry| {
                    let text = |key: &str| {
                        entry
                            .get(key)
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned()
                    };
                    (
                        text("suite"),
                        text("file"),
                        entry.get("case_count").and_then(Value::as_u64),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn suite_problems(dir: &Path) -> Vec<String> {
    let copy = dir.join(VENDORED_DIR);
    manifest_suites(dir)
        .into_iter()
        .filter_map(|(_, file, wanted)| {
            if !copy.join(&file).is_file() {
                return Some(format!("the manifest lists {file}, which the copy lacks"));
            }
            let count = match read_json(&copy, &file) {
                Ok(value) => value.get("cases").and_then(Value::as_array).map(Vec::len),
                Err(error) => return Some(error),
            };
            let count = count.map(|count| count as u64);
            if count.is_some() && count == wanted {
                return None;
            }
            let show = |value: Option<u64>| value.map_or("no".to_owned(), |n| n.to_string());
            Some(format!(
                "{file} holds {} cases; the manifest says {}",
                show(count),
                show(wanted)
            ))
        })
        .collect()
}

/// The line the gate stage prints when there is nothing to report.
pub fn summary(dir: &Path) -> Result<String, String> {
    let record = read_json(dir, PROVENANCE_FILE)?;
    let text = |key: &str| {
        record
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let counts: Vec<String> = manifest_suites(dir)
        .into_iter()
        .map(|(suite, _, count)| format!("{suite} {}", count.unwrap_or_default()))
        .collect();
    Ok(format!(
        "corpus check: {} at {} ({}), {} cases; the copy hashes to {}, as all four fields say",
        text("repo"),
        text("tag"),
        text("sha"),
        counts.join(", "),
        text("corpus_hash")
    ))
}
