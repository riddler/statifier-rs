//! The conformance apparatus: the corpus vendored under
//! `conformance/statifier/`, the run over it, the corpus check, the registry
//! check, the ratchet that writes the registry and the refresh that moves the
//! copy.
//!
//! It lives under `tests/` because none of it is part of the crate: the
//! package ships `src/` alone, and nothing in `src/` knows the corpus exists.
//! The contract is the reference's, read in the copy at the tag the
//! provenance record names: `conformance/statifier/README.md` (what each file
//! is) and `conformance/statifier/RATCHET.md` (the registry, its checks and
//! the vendoring sequence). `conformance/README.md` says what is copied, from
//! where, and what is this repository's own.
//!
//! Four tests at this file's top level are tasks, each run on its own by a
//! `mise.toml` task so that what it prints is shown: `corpus_check`
//! (`mise run gate:corpus`) and `registry_check` (`mise run gate:registry`)
//! are stages of the full gate; `ratchet` (`mise run ratchet`), the one
//! writer of `conformance/registry.json`, and `refresh`
//! (`mise run corpus:refresh`) write tracked files, so the suite never runs
//! them. The other tests here are the conformance run itself; the tests of
//! the apparatus are in `apparatus.rs`.

mod checks;
mod clock;
mod corpus;
mod ratchet;
mod refresh;
mod registry;
mod runner;

#[cfg(test)]
mod apparatus;

use corpus::{Corpus, REGISTRY_FILE, VENDORED_DIR, root};
use std::path::PathBuf;

/// The environment variable naming the statifier-ex tag `refresh` moves the
/// copy to.
const TAG_VARIABLE: &str = "STATIFIER_CORPUS_TAG";

/// The environment variable naming the clone of statifier-ex `refresh`
/// reads the tag from.
const CLONE_VARIABLE: &str = "STATIFIER_CORPUS_CLONE";

/// The environment variable that, set to `1`, puts `refresh` in its check
/// mode, which writes nothing.
const CHECK_VARIABLE: &str = "STATIFIER_CORPUS_CHECK";

fn corpus() -> Corpus {
    Corpus::load(&root()).unwrap()
}

fn shipped_registry() -> registry::Registry {
    registry::parse(&std::fs::read(root().join(REGISTRY_FILE)).unwrap()).unwrap()
}

fn reference_registry() -> registry::Registry {
    let path = root().join(VENDORED_DIR).join("registry.json");
    registry::parse(&std::fs::read(path).unwrap()).unwrap()
}

// The corpus check, a stage of the full gate: the copy hashes to what the
// vendored manifest, the provenance record, this registry and the vendored
// reference registry say, and every suite file is there with its count.
// Sabotage: with one byte of conformance/statifier/corpus/statifier.json
// changed, this test fails naming all four fields (and the apparatus tests
// cover each field on a copy).
#[test]
fn corpus_check() {
    let dir = root();
    let problems = checks::problems(&dir);
    assert!(
        problems.is_empty(),
        "corpus check failed:\n  {}\nRestore the copied bytes or refresh the copy to a tag; never rewrite a hash.",
        problems.join("\n  ")
    );
    println!("{}", checks::summary(&dir).unwrap());
}

// The registry check, a stage of the full gate: every check the copy's
// RATCHET.md puts over a sibling's registry, against a run of the whole
// corpus made now. It prints, per suite, the cases not yet claimed.
// Sabotage: with an entry for a case no run passes added to registry.json,
// this test fails naming the entry.
#[test]
fn registry_check() {
    let dir = root();
    let corpus = corpus();
    let bytes = std::fs::read(dir.join(REGISTRY_FILE)).unwrap();
    let problems = registry::check(&bytes, &corpus, &runner::run(&corpus, runner::drive));
    assert!(
        problems.is_empty(),
        "registry check failed:\n  {}",
        problems.join("\n  ")
    );
    let entries = shipped_registry().entries;
    if entries.is_empty() {
        println!("registry: no claim is made");
    } else {
        println!("registry: {} entries pass today", entries.len());
    }
    for line in runner::unclaimed_lines(&corpus, &entries, &reference_registry().entries) {
        println!("{line}");
    }
}

// The one writer of conformance/registry.json: it runs the whole corpus now
// and adds every observed pass. It writes a tracked file, so the suite never
// runs it; `mise run ratchet` does.
// Sabotage: none - this is the writer, not a check; the apparatus tests
// cover its refusals.
#[test]
#[ignore = "writes conformance/registry.json; run it through `mise run ratchet`"]
fn ratchet() {
    let corpus = corpus();
    let path = root().join(REGISTRY_FILE);
    if let Err(problems) = ratchet::write(&path, &corpus, &runner::run(&corpus, runner::drive)) {
        panic!(
            "ratchet refused; nothing was written:\n  {}",
            problems.join("\n  ")
        );
    }
    println!("ratchet: wrote conformance/{REGISTRY_FILE}");
}

// The refresh: moves conformance/statifier/ to the tag STATIFIER_CORPUS_TAG
// names, read from the clone STATIFIER_CORPUS_CLONE names; with
// STATIFIER_CORPUS_CHECK=1 it compares and writes nothing. It writes tracked
// files, so the suite never runs it; `mise run corpus:refresh` does.
// Sabotage: none - this is the task, not a check; the apparatus tests cover
// its copy, its check mode and its refusals.
#[test]
#[ignore = "writes conformance/statifier/; run it through `mise run corpus:refresh`"]
fn refresh() {
    let read = |name: &str| std::env::var(name).unwrap_or_default();
    let (tag, clone) = (read(TAG_VARIABLE), read(CLONE_VARIABLE));
    assert!(
        !tag.is_empty() && !clone.is_empty(),
        "refresh: name the tag in {TAG_VARIABLE} and a clone of statifier-ex in {CLONE_VARIABLE}"
    );
    let mode = if read(CHECK_VARIABLE) == "1" {
        refresh::Mode::Check
    } else {
        refresh::Mode::Write
    };
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    match refresh::run(&tag, &PathBuf::from(clone), &repo_root, mode) {
        Ok(line) => println!("{line}"),
        Err(reason) if mode == refresh::Mode::Check => {
            panic!("refresh check failed; nothing was written: {reason}")
        }
        Err(reason) => panic!("refresh refused; nothing was written: {reason}"),
    }
}

// Sabotage: make Corpus::load read only the first suite the manifest lists;
// the assert_eq on the manifest's total case count fails.
#[test]
fn reads_every_case_of_every_suite() {
    let corpus = corpus();
    let names: Vec<&str> = corpus
        .suites
        .iter()
        .map(|suite| suite.suite.as_str())
        .collect();
    assert_eq!(names, corpus::SUITES);
    let manifest = corpus::read_json(&root().join(VENDORED_DIR), "manifest.json").unwrap();
    let total: u64 = manifest["suites"]
        .as_array()
        .unwrap()
        .iter()
        .map(|suite| suite["case_count"].as_u64().unwrap())
        .sum();
    assert_eq!(corpus.cases().count() as u64, total);
    assert_eq!(corpus.by_id().len() as u64, total);
}

// Sabotage: make runner::run drop every failing result; the assert_eq on the
// ids answered fails.
#[test]
fn every_case_is_answered_pass_or_fail_with_no_skip() {
    let corpus = corpus();
    let results = runner::run(&corpus, runner::drive);
    let answered: Vec<&str> = results.outcomes.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(answered, corpus.ids());
    for (id, outcome) in &results.outcomes {
        if let runner::Outcome::Fail(reason) = outcome {
            assert!(!reason.is_empty(), "{id} failed with no reason");
        }
    }
}

// Sabotage: make runner::drive answer Outcome::Pass; the assert_eq on the
// one reason every case fails with fails (and the registry would then claim
// cases no interpreter drives).
#[test]
fn the_drive_passes_nothing_while_no_interpreter_exists() {
    let corpus = corpus();
    let reasons: std::collections::BTreeSet<runner::Outcome> = runner::run(&corpus, runner::drive)
        .outcomes
        .into_iter()
        .map(|(_, outcome)| outcome)
        .collect();
    assert_eq!(
        reasons.into_iter().collect::<Vec<_>>(),
        vec![runner::Outcome::Fail(runner::NOT_DRIVEN.to_owned())]
    );
}

// Sabotage: make runner::run answer a fail for every case; the assertion
// that each claimed case passes fails once the registry claims one. With no
// entry it loops over nothing: the registry check carries the same rule
// over a run made now.
#[test]
fn every_claimed_case_passes() {
    let corpus = corpus();
    let results = runner::run(&corpus, runner::drive);
    for entry in shipped_registry().entries {
        assert_eq!(
            results.get(&entry.case_id),
            Some(&runner::Outcome::Pass),
            "{} is claimed and fails",
            entry.case_id
        );
    }
}

// Sabotage: count the reference's entries in place of this registry's in
// runner::unclaimed_lines; the assert_eq on the printed lines fails (no case
// is then unclaimed).
#[test]
fn reports_per_suite_the_cases_not_yet_claimed() {
    let corpus = corpus();
    let ours = shipped_registry().entries;
    let reference = reference_registry().entries;
    let lines = runner::unclaimed_lines(&corpus, &ours, &reference);
    for line in &lines {
        println!("{line}");
    }
    let expected: Vec<String> = corpus::SUITES
        .iter()
        .map(|suite| {
            let open: Vec<&str> = corpus
                .cases_of(suite)
                .iter()
                .filter_map(corpus::id_of)
                .filter(|id| !ours.iter().any(|entry| entry.case_id == *id))
                .collect();
            let theirs = open
                .iter()
                .filter(|id| reference.iter().any(|entry| entry.case_id == **id))
                .count();
            format!(
                "conformance: {suite}: {} of {} cases not yet claimed (the reference claims {theirs} of them)",
                open.len(),
                corpus.cases_of(suite).len()
            )
        })
        .collect();
    assert_eq!(lines, expected);
}
