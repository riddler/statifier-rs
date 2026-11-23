//! The file `conformance/registry.json`: the corpus cases this crate claims
//! to pass, against the pinned corpus. Its contract is the copy's
//! `RATCHET.md`: the four fields, claims sorted, entries sorted by suite then
//! case id, one entry per line, written only by a run that observed the pass
//! (the ratchet), and the checks a sibling's registry check runs
//! ([`check`]).
//!
//! Before its first entry the registry carries the pin, no claims and no
//! entries. The copy's `schema/registry.json` refuses that file (an empty
//! registry is a defect, not a claim); here it passes the check as the
//! registry that makes no claim, which the gate stage says rather than
//! reporting a pass of anything, and it stays this registry's shape until a
//! run passes a case.

use crate::corpus::{Corpus, IMPLEMENTATION, claim_of};
use crate::runner::{Outcome, Results};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

/// The top-level keys, in the order `RATCHET.md`'s table gives them and the
/// encoding writes them.
pub const KEYS: [&str; 4] = ["implementation", "corpus_hash", "claims", "entries"];

/// One registry entry.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Entry {
    /// The corpus case.
    pub case_id: String,
    /// The case's suite, copied from the corpus.
    pub suite: String,
}

impl Entry {
    /// An entry for `case_id` in `suite`.
    pub fn new(case_id: &str, suite: &str) -> Entry {
        Entry {
            case_id: case_id.to_owned(),
            suite: suite.to_owned(),
        }
    }

    /// The order `RATCHET.md` gives entries: by suite, then by case id
    /// (byte order, which for UTF-8 is codepoint order).
    pub fn order(&self) -> (&str, &str) {
        (&self.suite, &self.case_id)
    }
}

/// A parsed registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registry {
    /// The implementation's name.
    pub implementation: String,
    /// The pin.
    pub corpus_hash: String,
    /// The claim names.
    pub claims: Vec<String>,
    /// The entries, in file order.
    pub entries: Vec<Entry>,
}

fn json(text: &str) -> String {
    Value::String(text.to_owned()).to_string()
}

/// The registry's bytes, in the reference's encoding: the four keys in
/// order, claims on one line, each entry one line of compact JSON. Entries
/// are written in the order given; a registry with none writes `[]`.
pub fn encode(registry: &Registry) -> String {
    let claims: Vec<String> = registry.claims.iter().map(|claim| json(claim)).collect();
    let entries: Vec<String> = registry
        .entries
        .iter()
        .map(|entry| {
            format!(
                "    {{\"case_id\":{},\"suite\":{}}}",
                json(&entry.case_id),
                json(&entry.suite)
            )
        })
        .collect();
    let list = if entries.is_empty() {
        "[]".to_owned()
    } else {
        format!("[\n{}\n  ]", entries.join(",\n"))
    };
    format!(
        "{{\n  \"implementation\": {},\n  \"corpus_hash\": {},\n  \"claims\": [{}],\n  \"entries\": {list}\n}}\n",
        json(&registry.implementation),
        json(&registry.corpus_hash),
        claims.join(",")
    )
}

fn text(object: &Map<String, Value>, key: &str) -> Result<String, String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("the registry's {key} is not a string"))
}

fn entry_of(index: usize, value: &Value) -> Result<Entry, String> {
    let fields = value.as_object().filter(|object| {
        object.len() == 2 && object.contains_key("case_id") && object.contains_key("suite")
    });
    match fields.map(|object| (text(object, "case_id"), text(object, "suite"))) {
        Some((Ok(case_id), Ok(suite))) => Ok(Entry { case_id, suite }),
        _ => Err(format!(
            "the registry's entry {index} is not an object of a case_id and a suite: {value}"
        )),
    }
}

/// The parsed registry, or a sentence saying why `bytes` are not one.
pub fn parse(bytes: &[u8]) -> Result<Registry, String> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("the registry is not JSON: {error}"))?;
    let object = value
        .as_object()
        .ok_or("the registry is not a JSON object")?;
    let keys: BTreeSet<&str> = object.keys().map(String::as_str).collect();
    if keys != KEYS.into_iter().collect() {
        return Err(format!(
            "the registry's keys are {}; RATCHET.md's are {}",
            keys.into_iter().collect::<Vec<_>>().join(", "),
            KEYS.join(", ")
        ));
    }
    let claims = object
        .get("claims")
        .and_then(Value::as_array)
        .and_then(|items| {
            items
                .iter()
                .map(|item| item.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()
        })
        .ok_or("the registry's claims must be an array of claim names")?;
    let entries = object
        .get("entries")
        .and_then(Value::as_array)
        .ok_or("the registry's entries must be an array")?
        .iter()
        .enumerate()
        .map(|(index, value)| entry_of(index, value))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Registry {
        implementation: text(object, "implementation")?,
        corpus_hash: text(object, "corpus_hash")?,
        claims,
        entries,
    })
}

/// The sorted set of claim names `entries` count toward, their cases read in
/// `cases`. An entry the corpus lacks counts toward nothing here; rule 1 is
/// where it fails.
pub fn claims_of(entries: &[Entry], cases: &BTreeMap<&str, &Value>) -> Vec<String> {
    let claims: BTreeSet<String> = entries
        .iter()
        .filter_map(|entry| cases.get(entry.case_id.as_str()))
        .filter_map(|item| claim_of(item))
        .collect();
    claims.into_iter().collect()
}

/// Every check the copy's `RATCHET.md` puts over a sibling's registry ("The
/// check"), run over the file's `bytes` against `corpus` and the `results`
/// of a run made now: the pin, rule 1, the claims, every entry passing today
/// and something to check; and besides them the implementation name, the
/// order, one entry per case and the encoding, so the file is the one the
/// ratchet would write. One sentence per problem; empty when it passes.
pub fn check(bytes: &[u8], corpus: &Corpus, results: &Results) -> Vec<String> {
    let registry = match parse(bytes) {
        Ok(registry) => registry,
        Err(problem) => return vec![problem],
    };
    let cases = corpus.by_id();
    let entries = &registry.entries;
    let mut problems = Vec::new();

    if registry.implementation != IMPLEMENTATION {
        problems.push(format!(
            "the registry names the implementation {}, not {}",
            json(&registry.implementation),
            json(IMPLEMENTATION)
        ));
    }
    if registry.corpus_hash != corpus.corpus_hash {
        problems.push(format!(
            "the pin: the registry's corpus_hash is {}; the vendored manifest's is {}",
            registry.corpus_hash, corpus.corpus_hash
        ));
    }
    for pair in entries.windows(2) {
        if let [left, right] = pair {
            if left.order() > right.order() {
                problems.push(format!(
                    "the order: {} comes after {}; entries are sorted by suite, then case id",
                    right.case_id, left.case_id
                ));
            }
        }
    }
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for entry in entries {
        *counts.entry(entry.case_id.as_str()).or_default() += 1;
    }
    for (id, count) in counts {
        if count > 1 {
            problems.push(format!("{id} is entered more than once"));
        }
    }
    for entry in entries {
        match cases.get(entry.case_id.as_str()) {
            None => problems.push(format!(
                "rule 1: {} is in the registry but not in the corpus",
                entry.case_id
            )),
            Some(item) => {
                let suite = item.get("suite").and_then(Value::as_str).unwrap_or("none");
                if suite != entry.suite {
                    problems.push(format!(
                        "rule 1: {}: the registry says suite {}, the corpus says {suite}",
                        entry.case_id, entry.suite
                    ));
                }
            }
        }
    }
    let wanted = claims_of(entries, &cases);
    if registry.claims != wanted {
        problems.push(format!(
            "the claims: claims is {}; the entries count toward {}",
            Value::from(registry.claims.clone()),
            Value::from(wanted)
        ));
    }
    if encode(&registry).as_bytes() != bytes {
        problems.push(
            "the registry's bytes are not what the ratchet writes; something other than the ratchet wrote it"
                .to_owned(),
        );
    }
    for entry in entries {
        match results.get(&entry.case_id) {
            None if !cases.contains_key(entry.case_id.as_str()) => {}
            None => problems.push(format!("the ratchet: {} was not run today", entry.case_id)),
            Some(Outcome::Pass) => {}
            Some(Outcome::Fail(reason)) => problems.push(format!(
                "the ratchet: {} no longer passes: {reason}",
                entry.case_id
            )),
        }
    }
    if cases.is_empty() {
        problems.push("rule 3: there is no corpus case to read".to_owned());
    }
    if results.is_empty() {
        problems.push("rule 3: no case was run".to_owned());
    }
    problems
}
