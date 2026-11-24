//! The README's Conformance section, read against the registry: its table
//! of claims and the case counts under them, and the tag it names, so the
//! prose cannot drift from `conformance/registry.json` or from the
//! provenance record.

use crate::corpus::{Corpus, claim_of};
use crate::registry::Entry;
use std::collections::BTreeMap;

/// The heading the section starts at.
const HEADING: &str = "## Conformance";

/// The row of the table that holds the sum.
const TOTAL: &str = "Total";

/// The count of entries per claim, the corpus's `conformance` class read to
/// split the w3c suite.
pub fn counts(entries: &[Entry], corpus: &Corpus) -> BTreeMap<String, usize> {
    let cases = corpus.by_id();
    let mut counts = BTreeMap::new();
    for entry in entries {
        if let Some(claim) = cases
            .get(entry.case_id.as_str())
            .and_then(|item| claim_of(item))
        {
            *counts.entry(claim).or_default() += 1;
        }
    }
    counts
}

/// The Conformance section's lines: from its heading to the next `## `.
fn section(readme: &str) -> Vec<&str> {
    readme
        .lines()
        .skip_while(|line| line.trim_end() != HEADING)
        .skip(1)
        .take_while(|line| !line.starts_with("## "))
        .collect()
}

/// Every problem with the README's Conformance section against the counts
/// the registry gives and the tag the provenance record names; empty when it
/// agrees. The table's rows are `| claim | count |`, and the row `Total`
/// holds the sum.
pub fn problems(readme: &str, counts: &BTreeMap<String, usize>, tag: &str) -> Vec<String> {
    let lines = section(readme);
    if lines.is_empty() {
        return vec![format!("the README has no {HEADING:?} section")];
    }
    let mut problems = Vec::new();
    if !lines.iter().any(|line| line.contains(tag)) {
        problems.push(format!("the section does not name the vendored tag {tag}"));
    }
    let mut rows: BTreeMap<String, String> = BTreeMap::new();
    for line in lines.iter().filter(|line| line.starts_with('|')) {
        let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
        if let [claim, count] = cells.as_slice() {
            rows.insert((*claim).to_owned(), (*count).to_owned());
        }
    }
    for (claim, count) in counts {
        match rows.get(claim) {
            None => problems.push(format!("the table has no row for the claim {claim}")),
            Some(text) if *text != count.to_string() => problems.push(format!(
                "the table says {claim} has {text} cases; the registry has {count}"
            )),
            Some(_) => {}
        }
    }
    for claim in rows.keys() {
        if claim != TOTAL
            && claim != "Claim"
            && !claim.starts_with('-')
            && !counts.contains_key(claim)
        {
            problems.push(format!(
                "the table has a row for {claim}, which the registry does not claim"
            ));
        }
    }
    let sum: usize = counts.values().sum();
    match rows.get(TOTAL) {
        None => problems.push("the table has no Total row".to_owned()),
        Some(text) if *text != sum.to_string() => problems.push(format!(
            "the table's Total is {text}; the registry has {sum} entries"
        )),
        Some(_) => {}
    }
    problems
}
