//! The README's Conformance section, read against the registry: one table
//! per suite, a row per group of the suite's cases, the counts under it and
//! the tag it names, so the prose cannot drift from
//! `conformance/registry.json`, from the corpus or from the provenance
//! record.

use crate::corpus::{Corpus, claim_of, id_of};
use crate::registry::Entry;
use std::collections::{BTreeMap, BTreeSet};

/// The heading the section starts at.
const HEADING: &str = "## Conformance";

/// The row of each table that holds the sums.
const TOTAL: &str = "Total";

/// What a table row counts for one group of a suite's cases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Counts {
    /// The cases the corpus holds in the group.
    pub cases: usize,
    /// The cases of the group the registry lists.
    pub claimed: usize,
}

/// The tables the README must carry: per suite, per group, the counts.
pub type Tables = BTreeMap<String, BTreeMap<String, Counts>>;

/// The group a case is listed under: the directory under its suite, or, for
/// a suite whose ids hold none (the w3c suite), the claim the case counts
/// toward, the corpus's `conformance` class read to split it.
fn group_of(id: &str, item: &serde_json::Value) -> String {
    let parts: Vec<&str> = id.split('/').collect();
    if parts.len() >= 3 {
        return parts[1].to_owned();
    }
    claim_of(item).unwrap_or_else(|| "unclassified".to_owned())
}

/// The tables the registry and the corpus give: every corpus case counted
/// in its suite's group, and counted as claimed when the registry lists it.
pub fn tables(entries: &[Entry], corpus: &Corpus) -> Tables {
    let listed: BTreeSet<&str> = entries.iter().map(|entry| entry.case_id.as_str()).collect();
    let mut tables = Tables::new();
    for suite in &corpus.suites {
        let groups = tables.entry(suite.suite.clone()).or_default();
        for item in &suite.cases {
            let Some(id) = id_of(item) else { continue };
            let counts = groups.entry(group_of(id, item)).or_default();
            counts.cases += 1;
            if listed.contains(id) {
                counts.claimed += 1;
            }
        }
    }
    tables
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

/// One table as the README writes it: its rows by the first cell, each the
/// other cells as written.
type Rows = BTreeMap<String, Vec<String>>;

/// The README's tables by the suite named in the `### ` heading above each,
/// and the problems met reading them.
fn read_tables(lines: &[&str]) -> (BTreeMap<String, Rows>, Vec<String>) {
    let mut found: BTreeMap<String, Rows> = BTreeMap::new();
    let mut problems = Vec::new();
    let mut suite: Option<String> = None;
    for line in lines {
        if let Some(name) = line.strip_prefix("### ") {
            let name = name.trim().to_owned();
            if found.insert(name.clone(), Rows::new()).is_some() {
                problems.push(format!("the section has two tables for the suite {name}"));
            }
            suite = Some(name);
        } else if line.starts_with('|') {
            let Some(rows) = suite.as_ref().and_then(|name| found.get_mut(name)) else {
                continue;
            };
            let mut cells = line
                .trim()
                .trim_matches('|')
                .split('|')
                .map(|cell| cell.trim().trim_matches('`').to_owned());
            let Some(first) = cells.next() else { continue };
            if first == "Group" || first.starts_with('-') {
                continue;
            }
            rows.insert(first, cells.collect());
        }
    }
    (found, problems)
}

fn number(cell: Option<&String>) -> Option<usize> {
    cell.and_then(|text| text.parse().ok())
}

/// Every problem with the README's Conformance section against the tables
/// the registry and the corpus give and the tag the provenance record names;
/// empty when it agrees. Each suite has a `### suite` heading above a table
/// whose rows are `| group | cases | claimed | unclaimed |`, and a row
/// `Total` holding the sums; the section also says how many entries the
/// registry holds, in the sentence `The registry holds N entries`.
pub fn problems(readme: &str, expected: &Tables, tag: &str) -> Vec<String> {
    let lines = section(readme);
    if lines.is_empty() {
        return vec![format!("the README has no {HEADING:?} section")];
    }
    let mut problems = Vec::new();
    if !lines.iter().any(|line| line.contains(tag)) {
        problems.push(format!("the section does not name the vendored tag {tag}"));
    }
    let (found, read_problems) = read_tables(&lines);
    problems.extend(read_problems);
    for suite in found.keys().filter(|suite| !expected.contains_key(*suite)) {
        problems.push(format!(
            "the section has a table for {suite}, which the corpus does not hold"
        ));
    }
    for (suite, groups) in expected {
        let Some(rows) = found.get(suite) else {
            problems.push(format!("the section has no table for the suite {suite}"));
            continue;
        };
        for (group, counts) in groups {
            check_row(&mut problems, suite, group, rows.get(group), *counts);
        }
        for group in rows.keys() {
            if group != TOTAL && !groups.contains_key(group) {
                problems.push(format!(
                    "the {suite} table has a row for {group}, which the suite does not hold"
                ));
            }
        }
        let total = Counts {
            cases: groups.values().map(|counts| counts.cases).sum(),
            claimed: groups.values().map(|counts| counts.claimed).sum(),
        };
        check_row(&mut problems, suite, TOTAL, rows.get(TOTAL), total);
    }
    let entries: usize = expected
        .values()
        .flat_map(|groups| groups.values())
        .map(|counts| counts.claimed)
        .sum();
    let sentence = format!("The registry holds {entries} entries");
    if !lines.iter().any(|line| line.contains(&sentence)) {
        problems.push(format!(
            "the section does not say {sentence:?}, the sum of the Claimed columns"
        ));
    }
    problems
}

/// Compares one table row with the counts it must hold.
fn check_row(
    problems: &mut Vec<String>,
    suite: &str,
    group: &str,
    row: Option<&Vec<String>>,
    counts: Counts,
) {
    let Some(row) = row else {
        problems.push(format!("the {suite} table has no row for {group}"));
        return;
    };
    let unclaimed = counts.cases - counts.claimed;
    for (column, got, want) in [
        ("Cases", number(row.first()), counts.cases),
        ("Claimed", number(row.get(1)), counts.claimed),
        ("Unclaimed", number(row.get(2)), unclaimed),
    ] {
        if got != Some(want) {
            let shown = got.map_or_else(|| "no number".to_owned(), |got| got.to_string());
            problems.push(format!(
                "the {suite} table says {group} has {shown} in {column}; the registry and corpus give {want}"
            ));
        }
    }
}
