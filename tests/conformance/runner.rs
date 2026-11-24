//! The run: every case of every suite, each answered pass or fail, with no
//! skip list and no third result.

use crate::clock;
use crate::corpus::{Corpus, id_of};
use crate::registry::Entry;
use serde_json::Value;
use statifier::{Options, State};
use std::collections::BTreeSet;
use std::panic::{AssertUnwindSafe, catch_unwind};

/// The suites the drive runs. Every case of another suite fails with
/// [`NOT_DRIVEN`], so the ratchet can claim none of it.
pub const DRIVEN_SUITES: [&str; 2] = ["scion", "w3c"];

/// The reason every case of a suite the drive does not run fails with.
pub const NOT_DRIVEN: &str = "not driven: this runner drives the scion and w3c suites only";

/// The one `host` key the drive honours: the send types a case registers,
/// each handed to [`Options::send_types`]. A case carrying any other key
/// fails naming it, as `RATCHET.md` says of a host object a runner cannot
/// honour.
pub const SEND_TYPES: &str = "send_types";

/// The `host` key naming the Event I/O Processors a case needs run; this
/// runner runs none, so a case naming one fails naming each.
pub const EVENT_IO_PROCESSORS: &str = "event_io_processors";

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
/// The drive is the one the copy's `RATCHET.md` describes under "What a case
/// asserts", made through the crate's public surface alone
/// ([`statifier::compile`], [`statifier::Machine::start`],
/// [`State::step`], [`State::advance`] through [`clock`],
/// [`State::active_leaf_states`] and [`State::done`]): compile the case's
/// source, start it and
/// compare the active leaf states with `initial_configuration`, then for
/// each step settle the clock, send the step's event and compare with the
/// step's `configuration`. Each comparison is made once the configuration
/// deadline has passed or the expectation is reached ([`clock`]), as the
/// reference's harness compares (`assert_configuration/3` and
/// `assert_configuration_eventually/3` in `lib/statifier/testing/case.ex`
/// at the vendored tag). A configuration is compared as a set of ids; a
/// stopped chart is compared at the leaves of the configuration it stopped
/// in ([`statifier::Done::active_leaf_states`]), as the reference's harness
/// restores that configuration from the done effect before it reads the
/// leaves (`observed_state_chart/2` in the same file). Only the event's name
/// is sent, as the reference's harness sends only the name.
///
/// The reference's harness also fails a case whose active leaf has no id
/// (`assert_every_leaf_named/2`): the leaf read leaves such a state out, so
/// an anonymous leaf could pass unseen while the named ones match. Each
/// comparison makes the same check first, by count: the leaves counted
/// ([`State::active_leaf_count`], or [`statifier::Done::active_leaf_count`]
/// once the chart has stopped) against the ids read.
///
/// A case's `host` object is honoured as `RATCHET.md` says: its
/// [`SEND_TYPES`] are registered in the options the chart starts with, and
/// a case carrying any other key, the Event I/O Processors of
/// [`EVENT_IO_PROCESSORS`] among them, fails naming what it needs.
///
/// It drives the suites in [`DRIVEN_SUITES`]; every other case fails with
/// [`NOT_DRIVEN`]. A case that does not compile, that the driver refuses,
/// or whose configuration disagrees fails with the reason.
pub fn drive(item: &Value) -> Outcome {
    let suite = item.get("suite").and_then(Value::as_str).unwrap_or("");
    if !DRIVEN_SUITES.contains(&suite) {
        return Outcome::Fail(NOT_DRIVEN.to_owned());
    }
    match drive_case(item) {
        Ok(()) => Outcome::Pass,
        Err(reason) => Outcome::Fail(reason),
    }
}

/// The start and every step, compared in turn; the reason at the first
/// that disagrees.
fn drive_case(item: &Value) -> Result<(), String> {
    let source = item
        .get("source")
        .and_then(Value::as_str)
        .ok_or("the case has no source")?;
    let machine = statifier::compile(source).map_err(|refusals| {
        let messages: Vec<&str> = refusals.iter().map(|refusal| refusal.message()).collect();
        format!("the source does not compile: {}", messages.join("; "))
    })?;
    let options = options(item)?;
    let expected = configuration(item.get("initial_configuration"))?;
    let started = machine
        .start(options)
        .map_err(|error| format!("the driver refused the start: {error}"))?;
    let mut state = reach(&started.state, 0, &expected)?;
    compare(&state, &expected, "the initial configuration")?;
    let steps = item
        .get("steps")
        .and_then(Value::as_array)
        .ok_or("the case has no steps")?;
    for (index, step) in steps.iter().enumerate() {
        let name = step
            .get("event")
            .and_then(|event| event.get("name"))
            .and_then(Value::as_str)
            .ok_or_else(|| format!("step {} has no event name", index + 1))?;
        let expected = configuration(step.get("configuration"))?;
        let settled =
            clock::settle(&state).map_err(|error| format!("the settle window: {error}"))?;
        let since = settled.now_ms();
        let moved = settled
            .step(name, statifier::Value::Undefined)
            .map_err(|error| format!("step {} (event {name:?}): {error}", index + 1))?;
        state = reach(&moved.state, since, &expected)?;
        compare(
            &state,
            &expected,
            &format!("step {} (event {name:?})", index + 1),
        )?;
    }
    Ok(())
}

/// The options a case starts with: its id as the session id, and the send
/// types its `host` object registers. A `host` key the drive does not
/// honour is the reason the case fails.
fn options(item: &Value) -> Result<Options, String> {
    let mut options = Options::new(id_of(item).unwrap_or(""));
    let Some(host) = item.get("host") else {
        return Ok(options);
    };
    let host = host.as_object().ok_or("the case's host is not an object")?;
    for (key, value) in host {
        match key.as_str() {
            SEND_TYPES => {
                options.send_types = names(value)
                    .ok_or("the case's host send_types is not a list of names")?
                    .into_iter()
                    .collect();
            }
            EVENT_IO_PROCESSORS => {
                let processors = names(value).unwrap_or_default();
                return Err(format!(
                    "the case needs the Event I/O Processor {}, which this runner does not run",
                    processors.join(", ")
                ));
            }
            other => {
                return Err(format!(
                    "the case needs the host key {other}, which this runner does not honour"
                ));
            }
        }
    }
    Ok(options)
}

/// The strings of a list, `None` when it is not a list of strings.
fn names(value: &Value) -> Option<Vec<String>> {
    value
        .as_array()?
        .iter()
        .map(|item| item.as_str().map(str::to_owned))
        .collect()
}

/// The state once `expected` is reached or the deadline after `since` has
/// passed.
fn reach(state: &State, since: u64, expected: &[String]) -> Result<State, String> {
    clock::await_until(state, since, |current| leaves(current) == expected)
        .map_err(|error| format!("the configuration deadline: {error}"))
}

/// The expected configuration, sorted.
fn configuration(value: Option<&Value>) -> Result<Vec<String>, String> {
    let mut ids = value
        .and_then(Value::as_array)
        .and_then(|items| {
            items
                .iter()
                .map(|item| item.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()
        })
        .ok_or("a configuration is not a list of state ids")?;
    ids.sort();
    ids.dedup();
    Ok(ids)
}

/// The active leaf states the drive compares: a stopped chart's are the
/// leaves it stopped in, read from its done answer, as the reference's
/// harness reads the done effect; a running chart's are read live.
fn leaves(state: &State) -> Vec<String> {
    state.done().map_or_else(
        || state.active_leaf_states(),
        |done| done.active_leaf_states.clone(),
    )
}

/// How many active leaf states there are, a leaf with no id counted too: a
/// stopped chart's are the leaves it stopped in, as [`leaves`] reads them.
fn leaf_count(state: &State) -> usize {
    state
        .done()
        .map_or_else(|| state.active_leaf_count(), |done| done.active_leaf_count)
}

/// Nothing when every active leaf has an id and the active leaf states are
/// `expected`; else the reason. The count is checked first, as the
/// reference's `assert_every_leaf_named/2` is.
fn compare(state: &State, expected: &[String], at: &str) -> Result<(), String> {
    let found = leaves(state);
    let unnamed = leaf_count(state).saturating_sub(found.len());
    if unnamed > 0 {
        return Err(format!(
            "{at}: {unnamed} active leaf state(s) have no id; the expectation {expected:?} cannot name them"
        ));
    }
    if found == expected {
        return Ok(());
    }
    Err(format!(
        "{at}: expected active leaf states {expected:?}, got {found:?}"
    ))
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

/// One line per driven suite ([`DRIVEN_SUITES`]): how many of its cases
/// pass in `results`, and the first that does, in the reference's order.
pub fn pass_lines(corpus: &Corpus, results: &Results) -> Vec<String> {
    corpus
        .suites
        .iter()
        .filter(|suite| DRIVEN_SUITES.contains(&suite.suite.as_str()))
        .map(|suite| {
            let passing: Vec<&str> = suite
                .cases
                .iter()
                .filter_map(id_of)
                .filter(|id| results.get(id) == Some(&Outcome::Pass))
                .collect();
            let first = passing
                .first()
                .map_or_else(|| "none".to_owned(), |id| (*id).to_owned());
            format!(
                "conformance: {}: {} of {} cases pass (the first in the reference's order: {first})",
                suite.suite,
                passing.len(),
                suite.cases.len()
            )
        })
        .collect()
}

/// The gap list: one line per case of every suite this crate's registry
/// (`entries`) does not claim, in the reference's order (the manifest's
/// suites, each suite's cases as its file lists them): the case, its
/// conformance class when the corpus gives one, the features it needs (its
/// `required_features`), the host keys it carries when it carries any, and
/// where it stands in `results`, passing or failing with why (a case of a
/// suite the drive does not run fails with [`NOT_DRIVEN`]). A case that
/// passes stays on the list until the ratchet claims it.
pub fn gap_lines(corpus: &Corpus, entries: &[Entry], results: &Results) -> Vec<String> {
    let ours: BTreeSet<&str> = entries.iter().map(|entry| entry.case_id.as_str()).collect();
    corpus
        .suites
        .iter()
        .flat_map(|suite| suite.cases.iter())
        .filter_map(|item| id_of(item).map(|id| (id, item)))
        .filter(|(id, _)| !ours.contains(id))
        .map(|(id, item)| {
            let standing = match results.get(id) {
                Some(Outcome::Pass) => "passes, not yet claimed".to_owned(),
                Some(Outcome::Fail(reason)) => format!("fails: {reason}"),
                None => "not run".to_owned(),
            };
            format!("conformance: gap: {id} ({}): {standing}", needs(item))
        })
        .collect()
}

/// What a case needs, for its gap line: its conformance class when the
/// corpus gives one, the features its `required_features` lists, and the
/// keys of its `host` object, sorted, when it has one.
fn needs(item: &Value) -> String {
    let features: Vec<&str> = item
        .get("required_features")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let mut parts = Vec::new();
    if let Some(class) = item.get("conformance").and_then(Value::as_str) {
        parts.push(class.to_owned());
    }
    if features.is_empty() {
        parts.push("needs no feature".to_owned());
    } else {
        parts.push(format!("needs {}", features.join(", ")));
    }
    if let Some(host) = item.get("host").and_then(Value::as_object) {
        let keys: Vec<&str> = host.keys().map(String::as_str).collect();
        parts.push(format!("host {}", keys.join(", ")));
    }
    parts.join("; ")
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
