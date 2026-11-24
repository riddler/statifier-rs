//! Tests of the apparatus itself: each check run over a copy of
//! `conformance/` that has been broken in one way, so a check that cannot
//! fail is noticed. The vendored files and the shipped registry are never
//! edited; copies live under the build's own temporary directory.

use crate::checks;
use crate::clock;
use crate::corpus::{self, CaseShape, Corpus, root};
use crate::ratchet;
use crate::refresh::{self, Mode, Scratch, copy_dir};
use crate::registry::{self, Entry, Registry};
use crate::runner::{self, Outcome, Results};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;

const HASH: &str = "sha256:7440f9a34a84b0657cb3bde11f96cf1707db08fb05e202c1e0249ab965517cc8";
const OTHER: &str = "sha256:0000000000000000000000000000000000000000000000000000000000000000";
const A: &str = "scion/actionSend/send1";
const B: &str = "scion/actionSend/send2";
const M: &str = "w3c/test144";

/// A fresh copy of `conformance/`, removed when dropped.
fn copy(name: &str) -> (Scratch, PathBuf) {
    let scratch = Scratch::new(name).unwrap();
    let dir = scratch.0.join("conformance");
    copy_dir(&root(), &dir).unwrap();
    (scratch, dir)
}

fn rewrite(dir: &Path, relative: &str, change: impl FnOnce(String) -> String) {
    let path = dir.join(relative);
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, change(text)).unwrap();
}

fn rewrite_json(dir: &Path, relative: &str, change: impl FnOnce(&mut Value)) {
    rewrite(dir, relative, |text| {
        let mut value: Value = serde_json::from_str(&text).unwrap();
        change(&mut value);
        value.to_string()
    });
}

fn corpus() -> Corpus {
    Corpus::load(&root()).unwrap()
}

/// A run in which the cases named pass and every other case fails.
fn run_passing(corpus: &Corpus, ids: &[&str]) -> Results {
    runner::run(corpus, |item| {
        if ids.contains(&corpus::id_of(item).unwrap_or_default()) {
            Outcome::Pass
        } else {
            Outcome::Fail("not this one".to_owned())
        }
    })
}

fn entry(id: &str, suite: &str) -> Entry {
    Entry::new(id, suite)
}

fn encoded(corpus: &Corpus, entries: Vec<Entry>, claims: Option<Vec<&str>>, hash: &str) -> String {
    let claims = claims.map_or_else(
        || registry::claims_of(&entries, &corpus.by_id()),
        |names| names.into_iter().map(str::to_owned).collect(),
    );
    registry::encode(&Registry {
        implementation: "statifier-rs".to_owned(),
        corpus_hash: hash.to_owned(),
        claims,
        entries,
    })
}

// ---- the corpus check ----

// Sabotage: append a space to conformance/statifier/corpus/statifier.json;
// the assert on the shipped files' problems fails.
#[test]
fn the_shipped_copy_hashes_to_all_four_fields() {
    assert_eq!(checks::problems(&root()), Vec::<String>::new());
    assert_eq!(corpus::corpus_hash(&root()).as_deref(), Some(HASH));
}

// Sabotage: hash the suite files in the order statifier, w3c, scion in
// corpus::corpus_hash; the assert_eq on the recomputed hash fails.
#[test]
fn the_hash_is_over_the_suite_files_in_suite_order() {
    let dir = root().join("statifier").join("corpus");
    let mut bytes = Vec::new();
    for suite in ["scion", "w3c", "statifier"] {
        bytes.extend(std::fs::read(dir.join(format!("{suite}.json"))).unwrap());
    }
    assert_eq!(corpus::sha256(&bytes), HASH);
    assert_eq!(
        corpus::corpus_hash(&root()).unwrap(),
        corpus::sha256(&bytes)
    );
}

// Sabotage: drop the provenance record's row from checks::fields; the
// assert_eq on the four named fields fails (three are named).
#[test]
fn a_changed_suite_byte_fails_every_field_naming_each() {
    let (_scratch, dir) = copy("changed-byte");
    rewrite(&dir, "statifier/corpus/w3c.json", |text| {
        text.replacen("w3c/test", "w3c/tesT", 1)
    });
    let problems = checks::problems(&dir);
    let labels: Vec<&str> = problems
        .iter()
        .map(|line| &line[..line.find("corpus_hash").unwrap() + "corpus_hash".len()])
        .collect();
    assert_eq!(
        labels,
        [
            "the vendored manifest's corpus_hash",
            "the provenance record's corpus_hash",
            "this crate's registry's corpus_hash",
            "the vendored reference registry's corpus_hash",
        ]
    );
    for line in &problems {
        assert!(
            line.contains(&format!("is \"{HASH}\"; the suite files hash to sha256:")),
            "{line}"
        );
    }
}

// Sabotage: compare each field with the manifest's corpus_hash rather than
// the recomputed one in checks::field_problem; the assert_eq naming the
// provenance record fails (the copy still agrees with the manifest).
#[test]
fn a_rewritten_provenance_hash_fails_naming_the_record() {
    let (_scratch, dir) = copy("rewritten-record");
    rewrite(&dir, "statifier.vendored.json", |text| {
        text.replace(HASH, OTHER)
    });
    assert_eq!(
        checks::problems(&dir),
        [format!(
            "the provenance record's corpus_hash is \"{OTHER}\"; the suite files hash to {HASH}"
        )]
    );
}

// Sabotage: drop this registry's row from checks::fields; the assert_eq on
// the problem naming it fails.
#[test]
fn a_registry_pinned_to_another_corpus_fails_naming_it() {
    let (_scratch, dir) = copy("registry-pin");
    rewrite(&dir, "registry.json", |text| text.replace(HASH, OTHER));
    assert_eq!(
        checks::problems(&dir),
        [format!(
            "this crate's registry's corpus_hash is \"{OTHER}\"; the suite files hash to {HASH}"
        )]
    );
}

// Sabotage: drop the vendored reference registry's row from checks::fields;
// the assert_eq on the problem naming it fails.
#[test]
fn a_reference_registry_out_of_step_fails_naming_it() {
    let (_scratch, dir) = copy("reference-pin");
    rewrite(&dir, "statifier/registry.json", |text| {
        text.replace(HASH, OTHER)
    });
    assert_eq!(
        checks::problems(&dir),
        [format!(
            "the vendored reference registry's corpus_hash is \"{OTHER}\"; the suite files hash to {HASH}"
        )]
    );
}

// Sabotage: make checks::field_problem answer None for a missing file; the
// assert on the missing record fails.
#[test]
fn a_missing_or_unreadable_record_is_named() {
    let (_scratch, dir) = copy("missing-record");
    std::fs::remove_file(dir.join("statifier.vendored.json")).unwrap();
    std::fs::write(dir.join("registry.json"), "{").unwrap();
    let problems = checks::problems(&dir);
    assert!(problems.contains(
        &"the provenance record's corpus_hash cannot be read: statifier.vendored.json is missing"
            .to_owned()
    ));
    assert!(problems.iter().any(|line| line.starts_with(
        "this crate's registry's corpus_hash cannot be read: registry.json is not JSON"
    )));
}

// Sabotage: make checks::suite_problems answer nothing; the assert on the
// case count fails.
#[test]
fn a_suite_file_whose_cases_disagree_with_its_case_count_is_named() {
    let (_scratch, dir) = copy("case-count");
    rewrite_json(&dir, "statifier/manifest.json", |manifest| {
        manifest["suites"][0]["case_count"] = json!(120);
    });
    let problems = checks::problems(&dir);
    assert!(
        problems.contains(&"corpus/scion.json holds 119 cases; the manifest says 120".to_owned()),
        "{problems:?}"
    );
}

// Sabotage: skip the is_file test in checks::suite_problems; the assert on
// the missing suite file fails (the check names a read error instead).
#[test]
fn a_listed_suite_file_the_copy_lacks_is_named() {
    let (_scratch, dir) = copy("missing-suite");
    std::fs::remove_file(dir.join("statifier/corpus/statifier.json")).unwrap();
    assert!(
        checks::problems(&dir)
            .contains(&"the manifest lists corpus/statifier.json, which the copy lacks".to_owned())
    );
}

// Sabotage: make corpus::corpus_hash hash an empty list of files; the
// assert_eq on the one problem saying there is nothing to hash fails.
#[test]
fn no_corpus_file_at_all_is_a_failure() {
    let (_scratch, dir) = copy("no-corpus");
    std::fs::remove_dir_all(dir.join("statifier/corpus")).unwrap();
    assert_eq!(
        checks::problems(&dir),
        ["there is no corpus file under statifier/corpus: nothing to hash"]
    );
}

// Sabotage: drop the record's sha from checks::summary; the assert on the
// tag and the commit fails.
#[test]
fn the_summary_names_the_tag_the_commit_and_the_counts() {
    let summary = checks::summary(&root()).unwrap();
    assert!(
        summary.contains("at v2.16.0 (76742af12138887c7f95125a80edf2b0a7dbaaf3)"),
        "{summary}"
    );
    assert!(
        summary.contains("scion 119, w3c 168, statifier 36 cases"),
        "{summary}"
    );
}

// ---- reading the corpus ----

fn load_error_after(name: &str, relative: &str, change: impl FnOnce(&mut Value)) -> String {
    let (_scratch, dir) = copy(name);
    rewrite_json(&dir, relative, change);
    Corpus::load(&dir).unwrap_err()
}

// Sabotage: make CaseShape::problems answer nothing; the assert_eq on the
// refusal of the unknown top-level key fails (Corpus::load succeeds).
#[test]
fn a_case_carrying_a_key_the_schema_does_not_name_is_refused() {
    let message = load_error_after("key-top", "statifier/corpus/scion.json", |file| {
        file["cases"][0]["skip"] = json!(true);
    });
    assert_eq!(
        message,
        "case \"scion/actionSend/send1\" carries the key skip, which schema/case.json does not name"
    );
}

// Sabotage: stop descending into array items in corpus::unknown; the
// assert_eq on the refusal of the key inside a step's event fails.
#[test]
fn a_key_the_schema_does_not_name_inside_a_step_is_refused() {
    let message = load_error_after("key-step", "statifier/corpus/scion.json", |file| {
        file["cases"][0]["steps"][0]["event"]["delay"] = json!(1);
    });
    assert_eq!(
        message,
        "case \"scion/actionSend/send1\" carries the key steps[0].event.delay, which schema/case.json does not name"
    );
}

// Sabotage: stop descending into a named key's schema in corpus::unknown;
// the assert on the refusal of the key inside a host's expected send fails.
#[test]
fn a_key_the_schema_does_not_name_inside_a_host_object_is_refused() {
    let message = load_error_after("key-host", "statifier/corpus/statifier.json", |file| {
        let cases = file["cases"].as_array_mut().unwrap();
        let item = cases
            .iter_mut()
            .find(|item| item["host"]["expect_sends"][0].is_object())
            .unwrap();
        item["host"]["expect_sends"][0]["extra"] = json!(1);
    });
    assert!(
        message.contains("carries the key host.expect_sends[0].extra, which schema/case.json"),
        "{message}"
    );
}

// Sabotage: treat every object the schema describes as closed in
// corpus::unknown (refuse when additionalProperties is absent); the first
// assert_eq fails (the open object's extra key is refused).
#[test]
fn an_object_the_schema_leaves_open_takes_any_key_and_a_member_schema_is_followed() {
    let shape = CaseShape::new(json!({
        "properties": {
            "id": {},
            "a": {"properties": {"b": {}}},
            "c": {"properties": {"d": {}}, "additionalProperties": false},
            "m": {"additionalProperties": {"properties": {"x": {}}, "additionalProperties": false}}
        },
        "additionalProperties": false
    }));
    assert_eq!(
        shape.problems(&json!({"id": "x", "a": {"b": 1, "e": 2}})),
        Vec::<String>::new()
    );
    assert_eq!(
        shape.problems(&json!({"id": "x", "c": {"d": 1, "e": 2}})),
        ["case \"x\" carries the key c.e, which schema/case.json does not name"]
    );
    assert_eq!(
        shape.problems(&json!({"id": "x", "m": {"k": {"x": 1, "y": 2}}})),
        ["case \"x\" carries the key m.k.y, which schema/case.json does not name"]
    );
}

// Sabotage: drop the duplicate test in Corpus::load; the assert_eq on the
// refusal fails.
#[test]
fn a_case_id_held_twice_is_refused() {
    let message = load_error_after("duplicate", "statifier/corpus/scion.json", |file| {
        let first = file["cases"][0].clone();
        file["cases"].as_array_mut().unwrap().push(first);
    });
    assert_eq!(
        message,
        "the corpus holds the case scion/actionSend/send1 more than once"
    );
}

// Sabotage: make corpus::claim_of answer the suite for every case; the
// assert_eq on the w3c claims fails.
#[test]
fn a_case_counts_toward_its_suite_with_w3c_split_by_class() {
    let corpus = corpus();
    let cases = corpus.by_id();
    assert_eq!(corpus::claim_of(cases[A]).as_deref(), Some("scion"));
    assert_eq!(corpus::claim_of(cases[M]).as_deref(), Some("w3c-mandatory"));
    let optional = corpus
        .cases_of("w3c")
        .iter()
        .find(|item| item["conformance"] == "optional")
        .unwrap();
    assert_eq!(corpus::claim_of(optional).as_deref(), Some("w3c-optional"));
    assert_eq!(
        corpus::claim_of(&json!({"suite": "w3c", "conformance": null})),
        None
    );
}

// ---- the run ----

// Sabotage: have runner::run record a pass without calling the drive; the
// assert_eq on the reason the drive gave fails.
#[test]
fn every_case_is_handed_to_the_drive() {
    let corpus = corpus();
    let seen = std::cell::RefCell::new(Vec::new());
    let results = runner::run(&corpus, |item| {
        let id = corpus::id_of(item).unwrap().to_owned();
        seen.borrow_mut().push(id.clone());
        Outcome::Fail(format!("drove {id}"))
    });
    assert_eq!(
        seen.into_inner(),
        corpus
            .ids()
            .iter()
            .map(|id| (*id).to_owned())
            .collect::<Vec<_>>()
    );
    assert_eq!(results.get(A), Some(&Outcome::Fail(format!("drove {A}"))));
}

// Sabotage: return the drive's answer unchanged in runner::outcome_of; the
// assert_eq on the empty reason fails.
#[test]
fn a_panic_is_a_fail_naming_it_and_an_empty_reason_is_named() {
    let corpus = corpus();
    let results = runner::run(&corpus, |item| match corpus::id_of(item) {
        Some(A) => Outcome::Pass,
        Some(B) => Outcome::Fail(String::new()),
        Some(M) => std::panic::panic_any("boom".to_owned()),
        _ => Outcome::Fail("no".to_owned()),
    });
    assert_eq!(results.get(A), Some(&Outcome::Pass));
    assert_eq!(
        results.get(B),
        Some(&Outcome::Fail("failed, giving no reason".to_owned()))
    );
    assert_eq!(
        results.get(M),
        Some(&Outcome::Fail("panicked: boom".to_owned()))
    );
    assert_eq!(results.outcomes.len(), corpus.ids().len());
}

// ---- the registry check ----

fn registry_problems(bytes: &str, results: &Results) -> Vec<String> {
    registry::check(bytes.as_bytes(), &corpus(), results)
}

fn passing_abm() -> Results {
    run_passing(&corpus(), &[A, B, M])
}

// Sabotage: append a blank line to conformance/registry.json; the assert on
// its problems fails (the encoding check names it).
#[test]
fn the_shipped_registry_passes_and_makes_no_claim() {
    let corpus = corpus();
    let bytes = std::fs::read(root().join("registry.json")).unwrap();
    assert_eq!(
        registry::check(&bytes, &corpus, &runner::run(&corpus, runner::drive)),
        Vec::<String>::new()
    );
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        value,
        json!({"implementation": "statifier-rs", "corpus_hash": HASH, "claims": [], "entries": []})
    );
}

// Sabotage: skip the outcome comparison in registry::check; the assert_eq on
// the entry that no longer passes fails.
#[test]
fn refuses_an_entry_that_no_longer_passes() {
    let corpus = corpus();
    let bytes = encoded(
        &corpus,
        vec![entry(A, "scion"), entry(B, "scion")],
        None,
        HASH,
    );
    assert_eq!(
        registry_problems(&bytes, &run_passing(&corpus, &[A])),
        [format!("the ratchet: {B} no longer passes: not this one")]
    );
}

// Sabotage: compare the registry's corpus_hash with itself in
// registry::check; the assert_eq on the pin problem fails.
#[test]
fn refuses_a_pin_that_is_not_the_manifests_hash() {
    let corpus = corpus();
    let bytes = encoded(&corpus, vec![entry(A, "scion")], None, OTHER);
    assert_eq!(
        registry_problems(&bytes, &passing_abm()),
        [format!(
            "the pin: the registry's corpus_hash is {OTHER}; the vendored manifest's is {HASH}"
        )]
    );
}

// Sabotage: compare only the suite in Entry::order; the second assert_eq
// (a case id out of order within one suite) fails.
#[test]
fn refuses_entries_out_of_order() {
    let corpus = corpus();
    let suites = encoded(
        &corpus,
        vec![entry(M, "w3c"), entry(A, "scion")],
        None,
        HASH,
    );
    assert_eq!(
        registry_problems(&suites, &passing_abm()),
        [format!(
            "the order: {A} comes after {M}; entries are sorted by suite, then case id"
        )]
    );
    let ids = encoded(
        &corpus,
        vec![entry(B, "scion"), entry(A, "scion")],
        None,
        HASH,
    );
    assert_eq!(
        registry_problems(&ids, &passing_abm()),
        [format!(
            "the order: {A} comes after {B}; entries are sorted by suite, then case id"
        )]
    );
}

// Sabotage: skip the membership loop in registry::check; the assert_eq on
// rule 1 fails.
#[test]
fn refuses_an_entry_the_corpus_does_not_hold_or_puts_in_another_suite() {
    let corpus = corpus();
    let bytes = encoded(
        &corpus,
        vec![entry("scion/no/such", "scion"), entry(M, "scion")],
        Some(vec!["scion"]),
        HASH,
    );
    assert_eq!(
        registry_problems(&bytes, &passing_abm()),
        [
            "rule 1: scion/no/such is in the registry but not in the corpus".to_owned(),
            format!("rule 1: {M}: the registry says suite scion, the corpus says w3c"),
            "the claims: claims is [\"scion\"]; the entries count toward [\"w3c-mandatory\"]"
                .to_owned(),
        ]
    );
}

// Sabotage: skip the claims comparison in registry::check; the assert_eq on
// the missing claim fails.
#[test]
fn refuses_claims_that_are_not_the_set_the_entries_count_toward() {
    let corpus = corpus();
    let bytes = encoded(
        &corpus,
        vec![entry(A, "scion"), entry(M, "w3c")],
        Some(vec!["scion"]),
        HASH,
    );
    assert_eq!(
        registry_problems(&bytes, &passing_abm()),
        [
            "the claims: claims is [\"scion\"]; the entries count toward [\"scion\",\"w3c-mandatory\"]"
        ]
    );
}

// Sabotage: skip the duplicate count in registry::check; the assert on the
// twice entered case fails.
#[test]
fn refuses_a_case_entered_twice() {
    let corpus = corpus();
    let bytes = encoded(
        &corpus,
        vec![entry(A, "scion"), entry(A, "scion")],
        None,
        HASH,
    );
    assert!(
        registry_problems(&bytes, &passing_abm())
            .contains(&format!("{A} is entered more than once"))
    );
}

// Sabotage: skip the encoding comparison in registry::check; the assert_eq
// on the reflowed file fails.
#[test]
fn refuses_bytes_the_ratchet_would_not_write() {
    let corpus = corpus();
    let value: Value =
        serde_json::from_str(&encoded(&corpus, vec![entry(A, "scion")], None, HASH)).unwrap();
    let reflowed = serde_json::to_string_pretty(&value).unwrap();
    assert_eq!(
        registry_problems(&reflowed, &passing_abm()),
        [
            "the registry's bytes are not what the ratchet writes; something other than the ratchet wrote it"
        ]
    );
}

// Sabotage: skip the implementation comparison in registry::check; the
// assert_eq on the name fails.
#[test]
fn refuses_another_implementations_name() {
    let corpus = corpus();
    let bytes = encoded(&corpus, vec![entry(A, "scion")], None, HASH)
        .replace("statifier-rs", "statifier-ex");
    assert_eq!(
        registry_problems(&bytes, &passing_abm()),
        ["the registry names the implementation \"statifier-ex\", not \"statifier-rs\""]
    );
}

// Sabotage: drop the empty-results test in registry::check; the assert_eq on
// rule 3 fails.
#[test]
fn refuses_a_check_over_a_run_of_nothing() {
    let corpus = corpus();
    let bytes = encoded(&corpus, vec![entry(A, "scion")], None, HASH);
    assert_eq!(
        registry_problems(&bytes, &Results::default()),
        [
            format!("the ratchet: {A} was not run today"),
            "rule 3: no case was run".to_owned()
        ]
    );
}

// Sabotage: drop the keys test from registry::parse; the second assert_eq
// (a file without claims) fails.
#[test]
fn refuses_a_file_that_is_not_a_registry() {
    let results = passing_abm();
    let problems = registry_problems("{", &results);
    assert!(
        problems.len() == 1 && problems[0].starts_with("the registry is not JSON"),
        "{problems:?}"
    );
    let missing = json!({"implementation": "x", "corpus_hash": HASH, "entries": []}).to_string();
    assert_eq!(
        registry_problems(&missing, &results),
        [
            "the registry's keys are corpus_hash, entries, implementation; RATCHET.md's are implementation, corpus_hash, claims, entries"
        ]
    );
    let bad = json!({"implementation": "x", "corpus_hash": HASH, "claims": [], "entries": [{"case_id": A}]})
        .to_string();
    assert_eq!(
        registry_problems(&bad, &results),
        [format!(
            "the registry's entry 0 is not an object of a case_id and a suite: {{\"case_id\":\"{A}\"}}"
        )]
    );
}

// Sabotage: write the entries on one line in registry::encode; the
// assert_eq on the reference registry's bytes fails.
#[test]
fn encodes_in_the_references_shape_byte_for_byte() {
    let path = root().join("statifier").join("registry.json");
    let bytes = std::fs::read_to_string(path).unwrap();
    let parsed = registry::parse(bytes.as_bytes()).unwrap();
    assert!(parsed.entries.len() > 1);
    assert_eq!(registry::encode(&parsed), bytes);
    let corpus = corpus();
    assert_eq!(
        encoded(
            &corpus,
            vec![entry(A, "scion"), entry(M, "w3c")],
            None,
            HASH
        ),
        format!(
            "{{\n  \"implementation\": \"statifier-rs\",\n  \"corpus_hash\": \"{HASH}\",\n  \"claims\": [\"scion\",\"w3c-mandatory\"],\n  \"entries\": [\n    {{\"case_id\":\"{A}\",\"suite\":\"scion\"}},\n    {{\"case_id\":\"{M}\",\"suite\":\"w3c\"}}\n  ]\n}}\n"
        )
    );
}

// ---- the ratchet ----

/// Writes from `results` into a copy whose registry holds `entries` (the
/// shipped one when `None`); answers the outcome and the bytes left on disk.
fn ratchet_into(
    name: &str,
    results: &Results,
    entries: Option<Vec<Entry>>,
) -> (Result<(), Vec<String>>, String) {
    let (_scratch, dir) = copy(name);
    let path = dir.join("registry.json");
    if let Some(entries) = entries {
        let mut registry = registry::parse(&std::fs::read(&path).unwrap()).unwrap();
        registry.entries = entries;
        std::fs::write(&path, registry::encode(&registry)).unwrap();
    }
    let outcome = ratchet::write(&path, &corpus(), results);
    (outcome, std::fs::read_to_string(&path).unwrap())
}

// Sabotage: write serde_json::to_string_pretty in place of registry::encode
// in ratchet::write; the assert_eq on the written bytes fails.
#[test]
fn a_run_that_passes_nothing_writes_the_same_empty_registry() {
    let corpus = corpus();
    let (outcome, bytes) = ratchet_into("ratchet-none", &runner::run(&corpus, runner::drive), None);
    assert_eq!(outcome, Ok(()));
    assert_eq!(
        bytes,
        std::fs::read_to_string(root().join("registry.json")).unwrap()
    );
}

// Sabotage: drop the sort in ratchet::next_registry; the assert_eq on the
// entries' order fails (the recorded w3c entry is written first).
#[test]
fn adds_every_observed_pass_sorted_with_its_claims() {
    let corpus = corpus();
    let results = run_passing(&corpus, &[M, A]);
    let (outcome, bytes) = ratchet_into("ratchet-add", &results, Some(vec![entry(M, "w3c")]));
    assert_eq!(outcome, Ok(()));
    let written = registry::parse(bytes.as_bytes()).unwrap();
    assert_eq!(written.entries, [entry(A, "scion"), entry(M, "w3c")]);
    assert_eq!(written.claims, ["scion", "w3c-mandatory"]);
    assert_eq!(written.corpus_hash, HASH);
    assert_eq!(
        registry::check(bytes.as_bytes(), &corpus, &results),
        Vec::<String>::new()
    );
}

// Sabotage: skip the pass test for a recorded entry in
// ratchet::next_registry; the assert_eq on the refusal fails, and the entry
// is dropped from the file.
#[test]
fn refuses_to_drop_a_recorded_entry_that_no_longer_passes() {
    let corpus = corpus();
    let recorded = vec![entry(A, "scion")];
    let (outcome, bytes) = ratchet_into(
        "ratchet-regress",
        &run_passing(&corpus, &[M]),
        Some(recorded.clone()),
    );
    assert_eq!(
        outcome,
        Err(vec![format!(
            "{A} is recorded and did not pass in this run"
        )])
    );
    assert_eq!(registry::parse(bytes.as_bytes()).unwrap().entries, recorded);
}

// Sabotage: make ratchet::run_problems answer nothing but for an empty run;
// the assert_eq on the refusal of a partial run fails.
#[test]
fn refuses_a_run_that_is_not_of_the_whole_corpus() {
    let corpus = corpus();
    let mut partial = run_passing(&corpus, &[A]);
    partial.outcomes.truncate(3);
    let (outcome, _) = ratchet_into("ratchet-partial", &partial, None);
    assert_eq!(
        outcome,
        Err(vec![format!(
            "the run is not of the whole corpus: 3 results for {} cases",
            corpus.ids().len()
        )])
    );
}

// Sabotage: drop the suite comparison in ratchet::next_registry; the
// assert_eq on the refusal of the entry under the wrong suite fails.
#[test]
fn refuses_a_recorded_entry_under_another_suite_or_absent_from_the_corpus() {
    let corpus = corpus();
    let (outcome, _) = ratchet_into(
        "ratchet-membership",
        &run_passing(&corpus, &[M]),
        Some(vec![entry("scion/no/such", "scion"), entry(M, "scion")]),
    );
    assert_eq!(
        outcome,
        Err(vec![
            "scion/no/such is recorded, and the corpus does not hold it".to_owned(),
            format!("{M} is recorded under suite scion; the corpus puts it in w3c"),
        ])
    );
}

// Sabotage: drop the empty-results test in ratchet::run_problems; the
// assert_eq on the refusal of a run of nothing fails.
#[test]
fn refuses_a_run_of_nothing() {
    let (outcome, _) = ratchet_into("ratchet-nothing", &Results::default(), None);
    assert_eq!(
        outcome,
        Err(vec![
            "no case was run: there is no run to ratchet from".to_owned()
        ])
    );
}

// ---- the vendored copy and the package ----

// Sabotage: reorder the keys in refresh::record (tag first); the assert_eq
// on the committed record's line fails.
#[test]
fn the_record_is_the_one_line_the_sequence_writes() {
    let line = std::fs::read_to_string(root().join("statifier.vendored.json")).unwrap();
    let record: Value = serde_json::from_str(&line).unwrap();
    let text = |key: &str| record[key].as_str().unwrap().to_owned();
    assert_eq!(
        line,
        refresh::record(&text("tag"), &text("sha"), &text("corpus_hash"))
    );
    assert_eq!(text("repo"), "https://github.com/riddler/statifier-ex");
    assert_eq!(text("tag"), "v2.16.0");
    assert_eq!(text("sha"), "76742af12138887c7f95125a80edf2b0a7dbaaf3");
    assert_eq!(text("corpus_hash"), HASH);
}

// Sabotage: delete conformance/statifier/LICENSES/Apache-2.0.txt; the
// assert that each named file exists fails.
#[test]
fn the_copy_carries_the_references_files_and_the_licences() {
    let copy = root().join("statifier");
    for path in [
        "README.md",
        "RATCHET.md",
        "registry.json",
        "exclusions.json",
        "manifest.json",
        "schema/case.json",
        "schema/registry.json",
        "LICENSES/Apache-2.0.txt",
        "LICENSES/BSD-3-Clause-W3C.txt",
    ] {
        assert!(copy.join(path).is_file(), "the copy lacks {path}");
    }
    let files = refresh::files(&copy.join("cases")).unwrap();
    assert!(files.keys().any(|path| path.ends_with(".json")));
}

// Sabotage: add "/conformance/" to Cargo.toml's include; the assert that no
// packaged file is under conformance/ fails.
#[test]
fn the_package_ships_none_of_conformance() {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let output = Command::new(cargo)
        .args(["package", "--list", "--allow-dirty", "--offline"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let listed = String::from_utf8(output.stdout).unwrap();
    assert!(listed.lines().any(|path| path == "src/lib.rs"), "{listed}");
    assert!(
        !listed
            .lines()
            .any(|path| path.starts_with("conformance/") || path.starts_with("tests/")),
        "{listed}"
    );
}

// ---- the refresh, against throwaway repositories made here ----

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args([
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "tag.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "init.defaultBranch=main",
            "-C",
        ])
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn write(dir: &Path, relative: &str, text: &str) {
    let path = dir.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// A reference repository whose conformance/ at tag v1.0.0 holds two suite
/// files, a manifest naming their hash (or `hash`, when given) and a readme;
/// answers the tagged commit and the manifest's hash.
fn reference(dir: &Path, hash: Option<&str>) -> (String, String) {
    let suites = [
        ("scion", "{\"cases\":[1]}\n"),
        ("statifier", "{\"cases\":[2]}\n"),
    ];
    for (suite, text) in suites {
        write(dir, &format!("conformance/corpus/{suite}.json"), text);
    }
    let hash = hash.map_or_else(
        || corpus::sha256(suites.map(|(_, text)| text).concat().as_bytes()),
        str::to_owned,
    );
    write(
        dir,
        "conformance/manifest.json",
        &format!("{{\"corpus_hash\": \"{hash}\"}}\n"),
    );
    write(dir, "conformance/README.md", "a\n");
    git(dir, &["init", "-q"]);
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "a"]);
    git(dir, &["tag", "v1.0.0"]);
    (git(dir, &["rev-parse", "HEAD"]).trim().to_owned(), hash)
}

/// Two throwaway repositories: a stand-in for the reference (empty) and a
/// stand-in for this repository, holding a committed conformance/README.md.
fn repositories(name: &str) -> (Scratch, PathBuf, PathBuf) {
    let scratch = Scratch::new(name).unwrap();
    let from = scratch.0.join("reference");
    let repo = scratch.0.join("repository");
    std::fs::create_dir_all(&from).unwrap();
    write(&repo, "conformance/README.md", "b\n");
    git(&repo, &["init", "-q"]);
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "b"]);
    (scratch, from, repo)
}

fn record_line(sha: &str, hash: &str) -> String {
    format!(
        "{{\"repo\":\"https://github.com/riddler/statifier-ex\",\"tag\":\"v1.0.0\",\"sha\":\"{sha}\",\"corpus_hash\":\"{hash}\"}}\n"
    )
}

// Sabotage: drop the trailing newline in refresh::record; the assert_eq on
// the record fails.
#[test]
fn a_refresh_copies_every_file_at_the_tag_and_writes_the_record() {
    let (_scratch, from, repo) = repositories("refresh-copy");
    let (sha, hash) = reference(&from, None);
    let line = refresh::run("v1.0.0", &from, &repo, Mode::Write).unwrap();
    assert_eq!(
        line,
        format!("corpus refresh: 4 files from v1.0.0 ({sha}) into conformance/statifier, {hash}")
    );
    let copied: Vec<String> = refresh::files(&repo.join("conformance/statifier"))
        .unwrap()
        .into_keys()
        .collect();
    assert_eq!(
        copied,
        [
            "README.md",
            "corpus/scion.json",
            "corpus/statifier.json",
            "manifest.json"
        ]
    );
    assert_eq!(
        std::fs::read_to_string(repo.join("conformance/statifier.vendored.json")).unwrap(),
        record_line(&sha, &hash)
    );
}

// Sabotage: run the Write arm for Mode::Check in refresh::run; the
// assert_eq that nothing was written fails.
#[test]
fn the_check_mode_compares_and_writes_nothing() {
    let (_scratch, from, repo) = repositories("refresh-check");
    let (sha, _) = reference(&from, None);
    let refusal = refresh::run("v1.0.0", &from, &repo, Mode::Check).unwrap_err();
    assert!(
        refusal.starts_with(&format!(
            "the copy is not v1.0.0 ({sha}):\nmissing: statifier/README.md"
        )),
        "{refusal}"
    );
    assert!(!repo.join("conformance/statifier").exists());
    assert!(!repo.join("conformance/statifier.vendored.json").exists());

    refresh::run("v1.0.0", &from, &repo, Mode::Write).unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "c"]);
    let agrees = refresh::run("v1.0.0", &from, &repo, Mode::Check).unwrap();
    assert_eq!(
        agrees,
        format!(
            "corpus refresh check: conformance/statifier is v1.0.0 ({sha}) byte for byte, 4 files, and the record agrees"
        )
    );
}

// Sabotage: make refresh::compare skip the byte comparison; the assert_eq
// on the changed file fails (the check then agrees).
#[test]
fn the_check_mode_names_a_changed_an_extra_file_and_a_changed_record() {
    let (_scratch, from, repo) = repositories("refresh-check-diff");
    let (sha, _) = reference(&from, None);
    refresh::run("v1.0.0", &from, &repo, Mode::Write).unwrap();
    write(&repo, "conformance/statifier/README.md", "edited\n");
    write(&repo, "conformance/statifier/extra.json", "{}\n");
    write(&repo, "conformance/statifier.vendored.json", "{}\n");
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "c"]);
    assert_eq!(
        refresh::run("v1.0.0", &from, &repo, Mode::Check).unwrap_err(),
        format!(
            "the copy is not v1.0.0 ({sha}):\nchanged: statifier/README.md\nnot at the tag: statifier/extra.json\nchanged: statifier.vendored.json is not the record the refresh writes"
        )
    );
}

// Sabotage: make refresh::dirty_paths answer nothing; the assert_eq on the
// refusal fails, and the copy is written.
#[test]
fn a_refresh_refuses_while_the_record_has_uncommitted_changes() {
    let (_scratch, from, repo) = repositories("refresh-dirty-record");
    reference(&from, None);
    write(&repo, "conformance/statifier.vendored.json", "{}\n");
    for mode in [Mode::Write, Mode::Check] {
        assert_eq!(
            refresh::run("v1.0.0", &from, &repo, mode).unwrap_err(),
            "the copy or its record has uncommitted changes; commit or discard them first:\n?? conformance/statifier.vendored.json"
        );
    }
    assert!(!repo.join("conformance/statifier").exists());
}

// Sabotage: list only the record in refresh::dirty_paths; the assert_eq on
// the edited copy's file fails.
#[test]
fn a_refresh_refuses_while_the_copy_has_uncommitted_changes() {
    let (_scratch, from, repo) = repositories("refresh-dirty-copy");
    reference(&from, None);
    refresh::run("v1.0.0", &from, &repo, Mode::Write).unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "c"]);
    write(&repo, "conformance/statifier/README.md", "edited\n");
    assert_eq!(
        refresh::dirty_paths(&repo).unwrap(),
        [" M conformance/statifier/README.md"]
    );
    assert!(
        refresh::run("v1.0.0", &from, &repo, Mode::Write)
            .unwrap_err()
            .starts_with("the copy or its record has uncommitted changes")
    );
    assert_eq!(
        std::fs::read_to_string(repo.join("conformance/statifier/README.md")).unwrap(),
        "edited\n"
    );
}

// Sabotage: drop the refs/tags/ test in refresh::resolve_tag; the assert on
// the branch name fails (the branch resolves to a commit).
#[test]
fn a_refresh_refuses_a_name_that_is_not_a_tag() {
    let (_scratch, from, repo) = repositories("refresh-branch");
    reference(&from, None);
    let refusal = refresh::run("main", &from, &repo, Mode::Write).unwrap_err();
    assert!(refusal.starts_with("main is not a tag in "), "{refusal}");
}

// Sabotage: drop the hash comparison in refresh::stage; the assert on the
// copy whose manifest names another hash fails, and it is written.
#[test]
fn a_refresh_refuses_a_copy_that_does_not_hash_to_its_manifest_and_writes_nothing() {
    let (_scratch, from, repo) = repositories("refresh-hash");
    reference(&from, Some(OTHER));
    let refusal = refresh::run("v1.0.0", &from, &repo, Mode::Write).unwrap_err();
    assert!(
        refusal.starts_with("the copy at v1.0.0 hashes to sha256:")
            && refusal.ends_with(&format!("; its manifest says {OTHER}")),
        "{refusal}"
    );
    assert!(!repo.join("conformance/statifier").exists());
    assert!(!repo.join("conformance/statifier.vendored.json").exists());
}

const TIMED: &str = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="a">
  <state id="a">
    <onentry><send event="e.soon" delay="60ms"/><send event="e.later" delay="150ms"/>
      <send event="e.past" delay="5000ms"/></onentry>
    <transition event="e.soon" target="b"/>
  </state>
  <state id="b">
    <transition event="e.later" target="c"/>
    <transition event="e.past" target="d"/>
  </state>
  <state id="c"><transition event="e.past" target="d"/></state>
  <state id="d"/>
</scxml>"#;

fn timed() -> statifier::State {
    statifier::compile(TIMED)
        .unwrap()
        .start(statifier::Options::new("s-1"))
        .unwrap()
        .state
}

// The knobs are the reference harness's numbers.
// Sabotage: with SETTLE_WINDOW_MS at 200, the settle test below reaches `c`.
#[test]
fn the_clock_knobs_are_the_reference_harness_constants() {
    assert_eq!(clock::SETTLE_WINDOW_MS, 100);
    assert_eq!(clock::CONFIGURATION_DEADLINE_MS, 4_000);
}

// Sabotage: with `settle` advancing to the next due time whatever it is,
// the 150 ms timer fires and the chart reaches `c`.
#[test]
fn the_settle_window_fires_only_the_timers_due_within_it() {
    let settled = clock::settle(&timed()).unwrap();
    assert_eq!(settled.now_ms(), 100);
    assert_eq!(settled.configuration(), ["b"]);
    let idle = statifier::compile(
        r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0"><state id="a"/></scxml>"#,
    )
    .unwrap()
    .start(statifier::Options::new("s-1"))
    .unwrap()
    .state;
    assert_eq!(clock::settle(&idle).unwrap().now_ms(), 0);
}

// Sabotage: with the deadline ignored, the 5000 ms timer fires and the
// chart reaches `d`.
#[test]
fn the_deadline_stops_waiting_four_seconds_after_the_event() {
    let reached = clock::await_until(&timed(), 0, |state| state.configuration() == ["z"]).unwrap();
    assert_eq!(reached.configuration(), ["c"]);
    assert_eq!(reached.now_ms(), 150);
    let early = clock::await_until(&timed(), 0, |state| state.configuration() == ["b"]).unwrap();
    assert_eq!(early.now_ms(), 60);
}

// A chart that reschedules a timer due at once under the default budget:
// the wait still ends.
// Sabotage: with the driver's fired-timer count removed, this never returns.
#[test]
fn the_deadline_wait_ends_on_a_timer_rescheduled_at_once() {
    let state = statifier::compile(
        r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="a">
             <state id="a"><onentry><send event="tick" delay="0ms"/></onentry>
               <transition event="tick"><send event="tick" delay="0ms"/></transition></state>
           </scxml>"#,
    )
    .unwrap()
    .start(statifier::Options::new("s-1"))
    .unwrap()
    .state;
    let waited = clock::await_until(&state, 0, |_| false).unwrap();
    assert_eq!(waited.next_due_ms(), None);
}
