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
//! Five tests at this file's top level are tasks, each run on its own by a
//! `mise.toml` task so that what it prints is shown: `corpus_check`
//! (`mise run gate:corpus`), `registry_check` (`mise run gate:registry`)
//! and `position_check` (`mise run gate:position`) are stages of the full
//! gate; `ratchet` (`mise run ratchet`), the one
//! writer of `conformance/registry.json`, and `refresh`
//! (`mise run corpus:refresh`) write tracked files, so the suite never runs
//! them. The other tests here are the conformance run itself; the tests of
//! the apparatus are in `apparatus.rs`.

mod checks;
mod clock;
mod corpus;
mod position;
mod ratchet;
mod readme;
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
// corpus made now. It prints, per driven suite, how many cases pass, then
// the gap list (every unclaimed case of every suite in the reference's
// order, with the features it needs) and, per suite, the cases not yet
// claimed.
// Sabotage: with an entry for a case no run passes added to registry.json,
// this test fails naming the entry.
#[test]
fn registry_check() {
    let dir = root();
    let corpus = corpus();
    let bytes = std::fs::read(dir.join(REGISTRY_FILE)).unwrap();
    let results = runner::run(&corpus, runner::drive);
    let problems = registry::check(&bytes, &corpus, &results);
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
    for line in runner::pass_lines(&corpus, &results) {
        println!("{line}");
    }
    for line in runner::gap_lines(&corpus, &entries, &results) {
        println!("{line}");
    }
    for line in runner::unclaimed_lines(&corpus, &entries, &reference_registry().entries) {
        println!("{line}");
    }
}

// The position round-trip stage, a stage of the full gate: every scion
// case driven as the run drives it, its position exported after the start
// and after every step, imported into a fresh machine and driven on, and
// compared at every later step (`position.rs`). It prints the counts and
// names every point not carried.
// Sabotage: with the import starting `entered_states` empty, the late
// binding cases disagree and this test fails naming them.
#[test]
fn position_check() {
    let corpus = corpus();
    let report = position::run(corpus.cases_of("scion"), position::export).unwrap();
    for line in report.lines() {
        println!("{line}");
    }
    let failures = report.failures();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// The stage catches a lost field the configuration never shows: an export
// that loses the macrostep counter disagrees at every point it carries.
// Sabotage: with `disagreement` comparing only whether each export was
// refused, the planted loss agrees and this goes red.
#[test]
fn the_position_stage_fails_an_export_that_loses_a_field() {
    fn lossy(state: &statifier::State) -> Result<statifier::Position, statifier::ExportError> {
        let mut position = state.export_position()?;
        position.macrostep = 0;
        Ok(position)
    }
    let corpus = corpus();
    let report = position::run(corpus.cases_of("scion"), lossy).unwrap();
    assert!(!report.disagreements().is_empty());
    assert!(
        report
            .failures()
            .iter()
            .any(|line| line.contains("disagrees"))
    );
}

// The README check: the Conformance section's table of claims, the counts
// under them and the tag it names agree with the shipped registry and the
// provenance record, so the README cannot drift from either.
// Sabotage: with one count in the README's table changed, this test fails
// naming the claim.
#[test]
fn readme_conformance_matches_the_registry() {
    let corpus = corpus();
    let counts = readme::counts(&shipped_registry().entries, &corpus);
    let readme = std::fs::read_to_string(root().join("..").join("README.md")).unwrap();
    let provenance = corpus::read_json(&root(), corpus::PROVENANCE_FILE).unwrap();
    let tag = provenance["tag"].as_str().unwrap();
    let problems = readme::problems(&readme, &counts, tag);
    assert!(
        problems.is_empty(),
        "README check failed:\n  {}",
        problems.join("\n  ")
    );
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

// Every suite of the corpus is driven, and a case of any other suite fails
// as not driven.
// Sabotage: drop "statifier" from runner::DRIVEN_SUITES; the assert on the
// statifier suite fails (its cases fail as not driven).
#[test]
fn the_drive_runs_every_suite_of_the_corpus_and_no_other() {
    let corpus = corpus();
    let results = runner::run(&corpus, runner::drive);
    let not_driven = runner::Outcome::Fail(runner::NOT_DRIVEN.to_owned());
    for suite in corpus::SUITES {
        let outcomes: std::collections::BTreeSet<&runner::Outcome> = corpus
            .cases_of(suite)
            .iter()
            .filter_map(corpus::id_of)
            .filter_map(|id| results.get(id))
            .collect();
        assert!(!outcomes.contains(&not_driven), "{suite}");
    }
    let mut other = scion_case(CHART, &["a", "c"], &[]);
    other["suite"] = serde_json::json!("other");
    assert_eq!(runner::drive(&other), not_driven);
}

/// A scion case of `source` with the initial configuration and steps given.
fn scion_case(source: &str, initial: &[&str], steps: &[(&str, &[&str])]) -> serde_json::Value {
    let steps: Vec<serde_json::Value> = steps
        .iter()
        .map(|(name, configuration)| {
            serde_json::json!({"event": {"name": name}, "configuration": configuration})
        })
        .collect();
    serde_json::json!({
        "id": "scion/x/y",
        "suite": "scion",
        "source": source,
        "initial_configuration": initial,
        "steps": steps,
    })
}

const CHART: &str = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="p">
  <parallel id="p">
    <state id="a"><transition event="e.go" target="b"/></state>
    <state id="c"/>
  </parallel>
  <state id="b"><transition event="e.end" target="f"/></state>
  <final id="f"/>
</scxml>"#;

// The drive compares active leaf states, a stopped chart's read from its
// done answer, as the reference's harness reads the done effect.
// Sabotage: read State::active_leaf_states alone in runner::leaves; the
// assert_eq fails at step 2 (a stopped chart has no active leaf).
#[test]
fn the_drive_compares_the_active_leaves_through_to_a_stopped_chart() {
    let case = scion_case(CHART, &["c", "a"], &[("e.go", &["b"]), ("e.end", &["f"])]);
    assert_eq!(runner::drive(&case), runner::Outcome::Pass);
}

// Sabotage: make runner::compare answer Ok(()); every assert_eq below fails.
#[test]
fn the_drive_fails_a_case_whose_configuration_disagrees_naming_where() {
    let wrong_start = scion_case(CHART, &["a"], &[]);
    assert_eq!(
        runner::drive(&wrong_start),
        runner::Outcome::Fail(
            "the initial configuration: expected active leaf states [\"a\"], got [\"a\", \"c\"]"
                .to_owned()
        )
    );
    let wrong_step = scion_case(CHART, &["a", "c"], &[("e.go", &["f"])]);
    assert_eq!(
        runner::drive(&wrong_step),
        runner::Outcome::Fail(
            "step 1 (event \"e.go\"): expected active leaf states [\"f\"], got [\"b\"]".to_owned()
        )
    );
    let after_stop = scion_case(
        CHART,
        &["a", "c"],
        &[("e.go", &["b"]), ("e.end", &["f"]), ("e.go", &["f"])],
    );
    assert_eq!(
        runner::drive(&after_stop),
        runner::Outcome::Fail("step 3 (event \"e.go\"): the chart has stopped".to_owned())
    );
    let broken = scion_case("<scxml", &[], &[]);
    let runner::Outcome::Fail(reason) = runner::drive(&broken) else {
        panic!("a source that does not compile passed");
    };
    assert!(
        reason.starts_with("the source does not compile: "),
        "{reason}"
    );
}

// A delayed send within the settle window fires before the next event; one
// due after the deadline never fires.
// Sabotage: drop the clock::settle call in runner::drive_case; the
// assert_eq fails at step 1 (the 50 ms timer has not fired).
#[test]
fn the_drive_settles_the_clock_before_each_event_and_waits_to_the_deadline() {
    let source = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="a">
  <state id="a">
    <onentry><send event="e.tick" delay="50ms"/><send event="e.late" delay="9s"/></onentry>
    <transition event="e.tick" target="b"/>
  </state>
  <state id="b">
    <transition event="e.go" target="c"/>
    <transition event="e.late" target="z"/>
  </state>
  <state id="c"/>
  <state id="z"/>
</scxml>"#;
    let case = scion_case(source, &["a"], &[("e.go", &["c"])]);
    assert_eq!(runner::drive(&case), runner::Outcome::Pass);
    let held = scion_case(source, &["z"], &[]);
    assert_eq!(
        runner::drive(&held),
        runner::Outcome::Fail(
            "the initial configuration: expected active leaf states [\"z\"], got [\"b\"]"
                .to_owned()
        )
    );
}

/// `case` moved to the w3c suite, with `host` as its host object.
fn w3c_case(mut case: serde_json::Value, host: serde_json::Value) -> serde_json::Value {
    case["suite"] = serde_json::json!("w3c");
    case["id"] = serde_json::json!("w3c/x");
    case["host"] = host;
    case
}

// An active leaf with no id fails the case even when the named leaves
// agree, running or stopped, as the reference's assert_every_leaf_named/2
// fails it; a w3c case is held to it as a scion case is.
// The message names the expectation, not the leaves read.
// Sabotage: make runner::compare skip the count; the first two assert_eq go
// red (the named leaves agree, so the cases pass). Print the leaves read in
// the message instead of the expectation; the third goes red.
#[test]
fn the_drive_fails_a_case_whose_active_leaf_has_no_id() {
    let running = scion_case(
        r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="p">
  <parallel id="p"><state id="a"/><state/></parallel>
</scxml>"#,
        &["a"],
        &[],
    );
    assert_eq!(
        runner::drive(&w3c_case(running, serde_json::json!({}))),
        runner::Outcome::Fail(
            "the initial configuration: 1 active leaf state(s) have no id; the expectation [\"a\"] cannot name them"
                .to_owned()
        )
    );
    let stopped = scion_case(
        r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0"><final/></scxml>"#,
        &[],
        &[],
    );
    assert_eq!(
        runner::drive(&stopped),
        runner::Outcome::Fail(
            "the initial configuration: 1 active leaf state(s) have no id; the expectation [] cannot name them"
                .to_owned()
        )
    );
    let differing = scion_case(
        r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="p">
  <parallel id="p"><state id="a"/><state/></parallel>
</scxml>"#,
        &["b"],
        &[],
    );
    assert_eq!(
        runner::drive(&differing),
        runner::Outcome::Fail(
            "the initial configuration: 1 active leaf state(s) have no id; the expectation [\"b\"] cannot name them"
                .to_owned()
        )
    );
}

const SENDS: &str = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="a">
  <state id="a">
    <onentry><send type="x-test" event="e.out"/></onentry>
    <transition event="error.*" target="z"/>
  </state>
  <state id="z"/>
</scxml>"#;

// The send types a case's host object declares are registered, so a send
// of one is handed to the host and raises no error; undeclared, the send's
// type is unknown and the chart takes the error.
// Sabotage: make runner::options skip the send_types key; the first
// assert_eq goes red (the chart moves to z).
#[test]
fn the_drive_registers_the_send_types_a_case_declares() {
    let declared = w3c_case(
        scion_case(SENDS, &["a"], &[]),
        serde_json::json!({"send_types": ["x-test"], "expect_sends": [
            {"type": "x-test", "target": null, "event": {"name": "e.out"}},
        ]}),
    );
    assert_eq!(runner::drive(&declared), runner::Outcome::Pass);
    let undeclared = w3c_case(scion_case(SENDS, &["a"], &[]), serde_json::json!({}));
    assert_eq!(
        runner::drive(&undeclared),
        runner::Outcome::Fail(
            "the initial configuration: expected active leaf states [\"a\"], got [\"z\"]"
                .to_owned()
        )
    );
}

// A case naming an Event I/O Processor fails naming it, since this runner
// runs none, and any other host key it cannot honour fails naming the key;
// neither is skipped.
// Sabotage: make runner::options ignore event_io_processors; the first
// assert_eq goes red (the chart reaches its expected configuration).
#[test]
fn the_drive_fails_a_case_whose_host_it_cannot_honour_naming_what_it_needs() {
    let processor = w3c_case(
        scion_case(CHART, &["a", "c"], &[]),
        serde_json::json!({"event_io_processors": ["http://www.w3.org/TR/scxml/#BasicHTTPEventProcessor"]}),
    );
    assert_eq!(
        runner::drive(&processor),
        runner::Outcome::Fail(
            "the case needs the Event I/O Processor http://www.w3.org/TR/scxml/#BasicHTTPEventProcessor, which this runner does not run"
                .to_owned()
        )
    );
    let other = w3c_case(
        scion_case(CHART, &["a", "c"], &[]),
        serde_json::json!({"declared_events": []}),
    );
    assert_eq!(
        runner::drive(&other),
        runner::Outcome::Fail(
            "the case needs the host key declared_events, which this runner does not honour"
                .to_owned()
        )
    );
    let several = w3c_case(
        scion_case(CHART, &["a", "c"], &[]),
        serde_json::json!({"to_source": "", "expect_diff": {}, "send_types": []}),
    );
    assert_eq!(
        runner::drive(&several),
        runner::Outcome::Fail(
            "the case needs the host keys expect_diff, to_source, which this runner does not honour"
                .to_owned()
        )
    );
}

/// A statifier case of `source` with the initial configuration, the steps
/// and the host object given.
fn statifier_case(
    source: &str,
    initial: &[&str],
    steps: serde_json::Value,
    host: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "id": "statifier/x/y",
        "suite": "statifier",
        "source": source,
        "initial_configuration": initial,
        "steps": steps,
        "host": host,
    })
}

const HANDED: &str = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="a">
  <state id="a">
    <onentry>
      <send type="x-test" target="t1" event="e.one" id="one"><param name="n" expr="1"/></send>
      <send type="http://www.w3.org/TR/scxml/#SCXMLEventProcessor" event="e.self"/>
      <send type="x-test" target="t2" event="e.later" delay="5s"/>
      <send event="e.tick" delay="50ms"/>
    </onentry>
    <transition event="e.tick" target="b"/>
  </state>
  <state id="b">
    <onentry><send type="x-test" target="t3" event="e.ticked"/></onentry>
    <transition event="e.go" target="c">
      <send type="x-test" target="t4" event="e.went"><param name="v" expr="_event.data.n"/></send>
    </transition>
  </state>
  <state id="c"/>
</scxml>"#;

/// The sends [`HANDED`] hands the host, in order, when its step carries
/// the data `{"n": 7}`.
fn handed_sends() -> serde_json::Value {
    serde_json::json!([
        {"type": "x-test", "target": "t1", "event": {"name": "e.one", "data": {"n": 1}}, "send_id": "one"},
        {"type": "x-test", "target": "t2", "event": {"name": "e.later"}, "delay_ms": 5000},
        {"type": "x-test", "target": "t3", "event": {"name": "e.ticked"}},
        {"type": "x-test", "target": "t4", "event": {"name": "e.went", "data": {"v": 7}}},
    ])
}

// Every send of a registered type the driver hands the host, from the
// start, a fired timer and a step, is compared in order with the case's
// expect_sends, in the item shape: the payload only when there is one,
// delay_ms only for a delayed send, send_id only when the document named
// it. A send of the SCXML type is the driver's, never the host's. A step's
// event carries the step's data.
// Sabotage: make runner::Sends::take read nothing; the first assert_eq goes
// red. Drop sends.take(&fired) in runner::reach; the first goes red (the
// third send is missing). Step with Value::Undefined in place of the step's
// data in runner::drive_case; the first goes red (the fourth send has no
// data). Make runner::Sends::hand record a send of any type; the first goes
// red (the SCXML send is handed).
#[test]
fn the_drive_compares_the_sends_handed_to_the_host_in_order() {
    let steps =
        serde_json::json!([{"event": {"name": "e.go", "data": {"n": 7}}, "configuration": ["c"]}]);
    let host = |expected: serde_json::Value| serde_json::json!({"send_types": ["x-test"], "expect_sends": expected});
    let agreeing = statifier_case(HANDED, &["b"], steps.clone(), host(handed_sends()));
    assert_eq!(runner::drive(&agreeing), runner::Outcome::Pass);
    let mut reordered = handed_sends();
    reordered.as_array_mut().unwrap().swap(0, 1);
    let runner::Outcome::Fail(reason) = runner::drive(&statifier_case(
        HANDED,
        &["b"],
        steps.clone(),
        host(reordered),
    )) else {
        panic!("sends handed out of order passed");
    };
    assert!(
        reason.starts_with("expected the sends handed to the host [{"),
        "{reason}"
    );
    let none = statifier_case(HANDED, &["b"], steps, host(serde_json::json!([])));
    assert_eq!(
        runner::drive(&none),
        runner::Outcome::Fail(format!(
            "expected the sends handed to the host [], got {}",
            handed_sends()
        ))
    );
}

const CANCELS: &str = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="a">
  <state id="a">
    <onentry>
      <send type="x-test" target="t" event="e.late" id="late" delay="10ms"/>
      <send type="x-test" target="t" event="e.kept" id="kept" delay="10ms"/>
    </onentry>
    <transition event="e.go" target="b"/>
  </state>
  <state id="b"><onentry><cancel sendid="late"/><cancel sendid="kept"/></onentry></state>
</scxml>"#;

// An expected cancelled outcome agrees only when the driver hands the host
// a cancel for that delayed send, and an item with no outcome makes no
// claim about a cancel, as the reference's runner marks them.
// Sabotage: make runner::Sends::cancel mark nothing; the first assert_eq
// goes red. Drop the expected item's claim from it (mark every cancelled
// send); the first goes red (the second item gains an outcome).
#[test]
fn the_drive_compares_a_cancelled_outcome_by_the_cancel_handed_to_the_host() {
    let expected = serde_json::json!({"send_types": ["x-test"], "expect_sends": [
        {"type": "x-test", "target": "t", "event": {"name": "e.late"}, "delay_ms": 10, "send_id": "late", "outcome": "cancelled"},
        {"type": "x-test", "target": "t", "event": {"name": "e.kept"}, "delay_ms": 10, "send_id": "kept"},
    ]});
    let go = serde_json::json!([{"event": {"name": "e.go"}, "configuration": ["b"]}]);
    assert_eq!(
        runner::drive(&statifier_case(CANCELS, &["a"], go, expected.clone())),
        runner::Outcome::Pass
    );
    let runner::Outcome::Fail(reason) = runner::drive(&statifier_case(
        CANCELS,
        &["a"],
        serde_json::json!([]),
        expected,
    )) else {
        panic!("a cancelled outcome no cancel reached passed");
    };
    assert!(
        reason.starts_with("expected the sends handed to the host "),
        "{reason}"
    );
}

// A case this runner cannot compare yet fails naming what it waits on,
// before the chart starts: a send expected to fail needs the failed-send
// report, an outcome it does not know is named, and a step carrying
// expect_position needs the position rendering.
// Sabotage: drop the positions call in runner::drive_case; the
// expect_position assert_eq goes red (the case passes). Drop the fail arm
// in runner::expect_sends; the fail assert_eq goes red (the generic reason).
#[test]
fn the_drive_fails_a_case_it_cannot_compare_yet_naming_what_it_waits_on() {
    let go = serde_json::json!([{"event": {"name": "e.go"}, "configuration": ["b"]}]);
    let outcome = |outcome: &str| {
        serde_json::json!({"send_types": ["x-test"], "expect_sends": [
            {"type": "x-test", "target": "t", "event": {"name": "e.out"}, "outcome": outcome},
        ]})
    };
    assert_eq!(
        runner::drive(&statifier_case(CHART, &["a", "c"], go.clone(), outcome("fail"))),
        runner::Outcome::Fail(
            "expect_sends item 1 has the outcome fail, which needs the failed-send report this runner does not make yet"
                .to_owned()
        )
    );
    assert_eq!(
        runner::drive(&statifier_case(
            CHART,
            &["a", "c"],
            go.clone(),
            outcome("lost")
        )),
        runner::Outcome::Fail(
            "expect_sends item 1 has the outcome \"lost\", which this runner does not observe"
                .to_owned()
        )
    );
    let positioned = serde_json::json!([
        {"event": {"name": "e.go"}, "configuration": ["b"]},
        {"event": {"name": "e.end"}, "configuration": ["f"], "expect_position": {}},
    ]);
    assert_eq!(
        runner::drive(&statifier_case(
            CHART,
            &["a", "c"],
            positioned,
            serde_json::json!({})
        )),
        runner::Outcome::Fail(
            "step 2 carries expect_position, which needs the position rendering this runner does not compare yet"
                .to_owned()
        )
    );
}

// A processor the host provides lets a case naming it through to the drive;
// one it does not provide fails the case naming that processor alone, and
// the runner's own host provides none.
// Sabotage: make runner::options name no processor as missing; the
// unprovided assert_eq goes red (the case passes). Make it name every
// processor, provided or not; the first assert_eq goes red (the case
// fails). Make runner::drive_case ignore the host it is given; the first
// assert_eq goes red (the case fails).
#[test]
fn the_drive_routes_a_cases_processors_through_the_host_it_is_given() {
    let stand_in = "x-test-processor";
    let other = "x-other-processor";
    let case = w3c_case(
        scion_case(CHART, &["a", "c"], &[]),
        serde_json::json!({"event_io_processors": [stand_in]}),
    );
    assert_eq!(
        runner::drive_with(&case, &runner::Host::providing(&[stand_in])),
        runner::Outcome::Pass
    );
    let missing = |name: &str| {
        runner::Outcome::Fail(format!(
            "the case needs the Event I/O Processor {name}, which this runner does not run"
        ))
    };
    assert_eq!(
        runner::drive_with(&case, &runner::Host::providing(&[other])),
        missing(stand_in)
    );
    assert_eq!(runner::drive(&case), missing(stand_in));
    let both = w3c_case(
        scion_case(CHART, &["a", "c"], &[]),
        serde_json::json!({"event_io_processors": [stand_in, other]}),
    );
    assert_eq!(
        runner::drive_with(&both, &runner::Host::providing(&[stand_in])),
        missing(other)
    );
    let malformed = w3c_case(
        scion_case(CHART, &["a", "c"], &[]),
        serde_json::json!({"event_io_processors": "x-test-processor"}),
    );
    assert_eq!(
        runner::drive_with(&malformed, &runner::Host::providing(&[stand_in])),
        runner::Outcome::Fail(
            "the case's host event_io_processors is not a list of names".to_owned()
        )
    );
}

// A chart holding an <invoke> is driven: the child starts, stops, and its
// done.invoke event moves the parent on.
// Sabotage: make Call::start_child return before the child starts; the
// assert_eq goes red (the parent stays in a).
#[test]
fn the_drive_runs_a_chart_holding_an_invoke() {
    let source = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="a">
  <state id="a">
    <invoke type="scxml"><content><scxml version="1.0"><final id="c"/></scxml></content></invoke>
    <transition event="done.invoke" target="b"/>
  </state>
  <final id="b"/>
</scxml>"#;
    assert_eq!(
        runner::drive(&scion_case(source, &["b"], &[])),
        runner::Outcome::Pass
    );
}

// Sabotage: list every case in runner::gap_lines, claimed or not; the
// assert_eq on the lines fails (a claimed case appears). Keep the gap list
// to the driven suites; the assert_eq on the ids fails (no statifier case).
#[test]
fn the_gap_list_names_each_unclaimed_case_of_every_suite_in_order_with_its_features() {
    let corpus = corpus();
    let results = runner::run(&corpus, |item| {
        if corpus::id_of(item) == Some("scion/actionSend/send2") {
            runner::Outcome::Fail("why".to_owned())
        } else {
            runner::Outcome::Pass
        }
    });
    let claimed: Vec<registry::Entry> = corpus
        .cases_of("scion")
        .iter()
        .filter_map(corpus::id_of)
        .skip(2)
        .map(|id| registry::Entry::new(id, "scion"))
        .collect();
    let lines = runner::gap_lines(&corpus, &claimed, &results);
    assert_eq!(
        lines[..2],
        [
            "conformance: gap: scion/actionSend/send1 (needs basic_states, compound_states, event_transitions, raise_elements): passes, not yet claimed",
            "conformance: gap: scion/actionSend/send2 (needs basic_states, compound_states, event_transitions, onexit_actions, raise_elements): fails: why",
        ]
    );
    let unclaimed: Vec<&str> = corpus
        .ids()
        .into_iter()
        .filter(|id| !claimed.iter().any(|entry| entry.case_id == *id))
        .collect();
    let named: Vec<&str> = lines
        .iter()
        .map(|line| {
            let rest = line.strip_prefix("conformance: gap: ").unwrap();
            &rest[..rest.find(' ').unwrap()]
        })
        .collect();
    assert_eq!(named, unclaimed);
    let w3c = corpus.cases_of("w3c")[0].clone();
    let statifier = corpus.cases_of("statifier")[0].clone();
    let line_of = |item: &serde_json::Value| {
        let id = corpus::id_of(item).unwrap();
        lines
            .iter()
            .find(|line| line.starts_with(&format!("conformance: gap: {id} ")))
            .cloned()
            .unwrap()
    };
    assert_eq!(
        line_of(&w3c),
        "conformance: gap: w3c/test144 (mandatory; needs basic_states, compound_states, event_transitions, final_states, log_elements, onentry_actions, raise_elements, wildcard_events): passes, not yet claimed"
    );
    assert!(
        line_of(&statifier).starts_with(&format!(
            "conformance: gap: {} (needs ",
            corpus::id_of(&statifier).unwrap()
        )),
        "{}",
        line_of(&statifier)
    );
    assert!(
        line_of(&statifier).contains("; host declared_events, expect_accepts, expect_sends, send_types): passes, not yet claimed"),
        "{}",
        line_of(&statifier)
    );
    let lines = runner::pass_lines(&corpus, &results);
    assert_eq!(
        lines,
        [
            format!(
                "conformance: scion: {} of {} cases pass (the first in the reference's order: scion/actionSend/send1)",
                corpus.cases_of("scion").len() - 1,
                corpus.cases_of("scion").len()
            ),
            format!(
                "conformance: w3c: {0} of {0} cases pass (the first in the reference's order: w3c/test144)",
                corpus.cases_of("w3c").len()
            ),
            format!(
                "conformance: statifier: {0} of {0} cases pass (the first in the reference's order: {1})",
                corpus.cases_of("statifier").len(),
                corpus::id_of(&statifier).unwrap()
            ),
        ]
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
