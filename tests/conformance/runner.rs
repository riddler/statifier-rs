//! The run: every case of every suite, each answered pass or fail, with no
//! skip list and no third result.

use crate::clock;
use crate::corpus::{Corpus, id_of};
use crate::registry::Entry;
use serde_json::{Map, Value};
use statifier::{Effect, Options, State};
use std::collections::BTreeSet;
use std::panic::{AssertUnwindSafe, catch_unwind};

/// The suites the drive runs. Every case of another suite fails with
/// [`NOT_DRIVEN`], so the ratchet can claim none of it.
pub const DRIVEN_SUITES: [&str; 3] = ["scion", "w3c", "statifier"];

/// The reason every case of a suite the drive does not run fails with.
pub const NOT_DRIVEN: &str =
    "not driven: this runner drives the scion, w3c and statifier suites only";

/// The `host` key naming the send types a case registers, each handed to
/// [`Options::send_types`]. A case carrying a key other than this one,
/// [`EXPECT_SENDS`], [`EVENT_IO_PROCESSORS`], [`DECLARED_EVENTS`] and
/// [`EXPECT_ACCEPTS`] fails naming it, as `RATCHET.md` says of a host
/// object a runner cannot honour.
pub const SEND_TYPES: &str = "send_types";

/// The `host` key listing the sends a case expects its registered send
/// types to be handed over the whole run, in order ([`Sends`]).
pub const EXPECT_SENDS: &str = "expect_sends";

/// The `outcome` an `expect_sends` item carries for a delayed send the
/// drive observes cancelled: a cancel the driver hands the host for it.
pub const CANCELLED: &str = "cancelled";

/// The `outcome` an `expect_sends` item carries for a send the host
/// reports it could not deliver: the drive reports the send handed at that
/// item's position through [`State::report_send_failed`] as soon as it
/// reads it, and the item records the outcome.
pub const FAILED: &str = "fail";

/// The step key holding the position a case expects once that step's
/// configuration agrees. The drive does not render a position, so a case
/// carrying one fails naming it.
pub const EXPECT_POSITION: &str = "expect_position";

/// The `host` key holding the event names a case's chart is declared to
/// accept, checked with [`statifier::Machine::check_accepts`] before the
/// chart starts ([`accepts`]).
pub const DECLARED_EVENTS: &str = "declared_events";

/// The `host` key holding what the accepts check answers for
/// [`DECLARED_EVENTS`]: its `unreachable` and `undeclared` lists, compared
/// exactly, order included.
pub const EXPECT_ACCEPTS: &str = "expect_accepts";

/// The `host` key naming the Event I/O Processors a case needs. Each is
/// looked up in the [`Host`] the drive was given: a case whose processors
/// are all provided runs, and one naming a processor the host does not
/// provide fails naming it.
pub const EVENT_IO_PROCESSORS: &str = "event_io_processors";

/// The host object a drive builds a case's run against: the table of Event
/// I/O Processors this runner provides, by the location the corpus names
/// them with. The runner provides none in this round; a later round
/// registers the Basic HTTP processor here. The crate has no processor
/// hook yet, so a provided processor is a name the case is allowed to
/// carry, and its behaviour is the provider's to wire when it registers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Host {
    provided: BTreeSet<String>,
}

impl Host {
    /// The host the runner drives every case with: no processor provided.
    pub fn runner() -> Self {
        Self::default()
    }

    /// A host providing `processors`, for a stand-in registered by a test.
    pub fn providing(processors: &[&str]) -> Self {
        Self {
            provided: processors.iter().map(|name| (*name).to_owned()).collect(),
        }
    }

    /// Whether `processor` is provided.
    pub fn provides(&self, processor: &str) -> bool {
        self.provided.contains(processor)
    }
}

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
/// A step's event carries the step's `data`, when it gives one, as the
/// reference's host case runner sends it (`event/1` in
/// `lib/mix/statifier/corpus/host_case.ex` at the vendored tag).
///
/// A case's `host` object is honoured as `RATCHET.md` says and as that
/// runner honours it: its [`SEND_TYPES`] are registered in the options the
/// chart starts with, every send of one the driver hands the host over the
/// whole run is recorded ([`Sends`]), and once every configuration agrees
/// the sends recorded must be exactly its [`EXPECT_SENDS`], in order. Its
/// [`EVENT_IO_PROCESSORS`] are routed through the [`Host`] (a processor the
/// host does not provide fails the case naming it). Its [`DECLARED_EVENTS`]
/// are checked against the compiled chart before anything else, and the
/// answer must be exactly its [`EXPECT_ACCEPTS`] ([`accepts`]). A case carrying any
/// other host key, an `expect_sends` item whose outcome is neither
/// [`CANCELLED`] nor [`FAILED`], or a step carrying an [`EXPECT_POSITION`]
/// fails naming what it needs, before the chart starts. A send whose
/// expected item says [`FAILED`] is reported failed through
/// [`State::report_send_failed`] as soon as the drive reads it, before the
/// configuration is read again, as the reference's runner reports it
/// (`pump/3` in `lib/mix/statifier/corpus/host_case.ex` at the vendored
/// tag).
///
/// It drives the suites in [`DRIVEN_SUITES`]; every other case fails with
/// [`NOT_DRIVEN`]. A case that does not compile, that the driver refuses,
/// whose configuration disagrees, or whose handed sends disagree fails with
/// the reason.
pub fn drive(item: &Value) -> Outcome {
    drive_with(item, &Host::runner())
}

/// [`drive`] against `host`: the table of Event I/O Processors the case's
/// `host` object is routed through.
pub fn drive_with(item: &Value, host: &Host) -> Outcome {
    let suite = item.get("suite").and_then(Value::as_str).unwrap_or("");
    if !DRIVEN_SUITES.contains(&suite) {
        return Outcome::Fail(NOT_DRIVEN.to_owned());
    }
    match drive_case(item, host) {
        Ok(()) => Outcome::Pass,
        Err(reason) => Outcome::Fail(reason),
    }
}

/// The start and every step, compared in turn; the reason at the first
/// that disagrees.
fn drive_case(item: &Value, host: &Host) -> Result<(), String> {
    let source = item
        .get("source")
        .and_then(Value::as_str)
        .ok_or("the case has no source")?;
    let machine = statifier::compile(source).map_err(|refusals| {
        let messages: Vec<&str> = refusals.iter().map(|refusal| refusal.message()).collect();
        format!("the source does not compile: {}", messages.join("; "))
    })?;
    accepts(&machine, item)?;
    let options = options(item, host)?;
    let mut sends = Sends::new(options.send_types.clone(), expect_sends(item)?);
    let steps = item
        .get("steps")
        .and_then(Value::as_array)
        .ok_or("the case has no steps")?;
    positions(steps)?;
    let expected = configuration(item.get("initial_configuration"))?;
    let started = machine
        .start(options)
        .map_err(|error| format!("the driver refused the start: {error}"))?;
    sends.take(&started.effects);
    let state = report(&started.state, &mut sends)?;
    let mut state = reach(&state, 0, &expected, &mut sends)?;
    compare(&state, &expected, "the initial configuration")?;
    for (index, step) in steps.iter().enumerate() {
        let event = step.get("event");
        let name = event
            .and_then(|event| event.get("name"))
            .and_then(Value::as_str)
            .ok_or_else(|| format!("step {} has no event name", index + 1))?;
        let data = event
            .and_then(|event| event.get("data"))
            .map_or(statifier::Value::Undefined, value_of);
        let expected = configuration(step.get("configuration"))?;
        let mut fired = Vec::new();
        let settled = clock::settle(&state, &mut fired)
            .map_err(|error| format!("the settle window: {error}"))?;
        sends.take(&fired);
        let settled = report(&settled, &mut sends)?;
        let since = settled.now_ms();
        let moved = settled
            .step(name, data)
            .map_err(|error| format!("step {} (event {name:?}): {error}", index + 1))?;
        sends.take(&moved.effects);
        let reported = report(&moved.state, &mut sends)?;
        state = reach(&reported, since, &expected, &mut sends)?;
        compare(
            &state,
            &expected,
            &format!("step {} (event {name:?})", index + 1),
        )?;
    }
    sends.compare()
}

/// The case's `expect_sends`, empty when its host object has none, as the
/// reference's runner reads them. An item whose outcome the drive does not
/// observe is the reason the case fails.
fn expect_sends(item: &Value) -> Result<Vec<Value>, String> {
    let Some(expected) = item.get("host").and_then(|host| host.get(EXPECT_SENDS)) else {
        return Ok(Vec::new());
    };
    let expected = expected
        .as_array()
        .ok_or("the case's host expect_sends is not a list")?;
    for (index, send) in expected.iter().enumerate() {
        match send.get("outcome") {
            None => {}
            Some(Value::String(outcome)) if outcome == CANCELLED || outcome == FAILED => {}
            Some(outcome) => {
                return Err(format!(
                    "expect_sends item {} has the outcome {outcome}, which this runner does not observe",
                    index + 1
                ));
            }
        }
    }
    Ok(expected.clone())
}

/// Nothing when no step carries an [`EXPECT_POSITION`]; else the reason the
/// case fails, naming the first step that does.
fn positions(steps: &[Value]) -> Result<(), String> {
    match steps
        .iter()
        .position(|step| step.get(EXPECT_POSITION).is_some())
    {
        None => Ok(()),
        Some(index) => Err(format!(
            "step {} carries {EXPECT_POSITION}, which needs the position rendering this runner does not compare yet",
            index + 1
        )),
    }
}

/// The host's recording processor for a case's registered send types: every
/// send of one the driver hands the host, in the order handed, written in
/// the case's item shape, as the reference's runner writes it (`item/2` in
/// `lib/mix/statifier/corpus/host_case.ex` at the vendored tag). A delayed
/// send is the host's timer, and nothing fires it.
#[derive(Debug, Clone)]
pub struct Sends {
    send_types: BTreeSet<String>,
    expected: Vec<Value>,
    handed: Vec<Handed>,
    /// The sends handed whose expected item says [`FAILED`], each as the
    /// effect the host was handed, not yet reported.
    failing: Vec<Effect>,
}

/// One send handed to the host: its send id, and its item.
#[derive(Debug, Clone)]
struct Handed {
    send_id: String,
    item: Map<String, Value>,
}

impl Sends {
    /// A processor registered for `send_types`, expecting `expected`.
    pub fn new(send_types: BTreeSet<String>, expected: Vec<Value>) -> Self {
        Self {
            send_types,
            expected,
            handed: Vec::new(),
            failing: Vec::new(),
        }
    }

    /// Whether a send handed is still to be reported failed.
    pub fn has_failing(&self) -> bool {
        !self.failing.is_empty()
    }

    /// The sends to report failed, in the order handed, each taken once.
    pub fn failing(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.failing)
    }

    /// Reads one call's effects, in order: each send of a registered type is
    /// handed to the host, and a cancel marks the delayed sends handed under
    /// its send id whose expected item, at the same position, says
    /// [`CANCELLED`], as the reference's runner marks them
    /// (`cancel_named/2`). An invoked child's effects are read as its
    /// parent's, since a child inherits its parent's send types.
    pub fn take(&mut self, effects: &[Effect]) {
        for effect in effects {
            self.take_one(effect, effect);
        }
    }

    /// Reads `effect`, which the host was handed as `handed`: the effect
    /// itself, or the [`Effect::Child`] carrying it.
    fn take_one(&mut self, effect: &Effect, handed: &Effect) {
        match effect {
            Effect::Child { effect, .. } => self.take_one(effect, handed),
            Effect::Send {
                event,
                target,
                send_type,
                data,
                send_id,
                id_from_author,
                ..
            } => self.hand(send_type.as_ref(), handed, || {
                (
                    send_id.clone(),
                    item(
                        send_type,
                        target,
                        event,
                        data,
                        None,
                        *id_from_author,
                        send_id,
                    ),
                )
            }),
            Effect::SendDelayed {
                event,
                target,
                send_type,
                data,
                send_id,
                id_from_author,
                delay_ms,
                ..
            } => self.hand(send_type.as_ref(), handed, || {
                (
                    send_id.clone(),
                    item(
                        send_type,
                        target,
                        event,
                        data,
                        Some(*delay_ms),
                        *id_from_author,
                        send_id,
                    ),
                )
            }),
            Effect::Cancel {
                send_id: statifier::Value::String(send_id),
                ..
            } => self.cancel(send_id),
            _ => {}
        }
    }

    /// Records the send `made` builds when `send_type` is a registered type.
    /// A send whose expected item, at the same position, says [`FAILED`]
    /// is recorded with that outcome and queued to be reported.
    fn hand(
        &mut self,
        send_type: Option<&statifier::Value>,
        handed: &Effect,
        made: impl FnOnce() -> (String, Map<String, Value>),
    ) {
        // A match guard rather than a let chain, which the minimum Rust lacks.
        if matches!(send_type, Some(statifier::Value::String(send_type)) if self.send_types.contains(send_type))
        {
            let (send_id, mut item) = made();
            let failed = self
                .expected
                .get(self.handed.len())
                .and_then(|expected| expected.get("outcome"))
                .and_then(Value::as_str)
                == Some(FAILED);
            if failed {
                item.insert("outcome".to_owned(), Value::from(FAILED));
                self.failing.push(handed.clone());
            }
            self.handed.push(Handed { send_id, item });
        }
    }

    fn cancel(&mut self, send_id: &str) {
        for (index, handed) in self.handed.iter_mut().enumerate() {
            let claimed = self
                .expected
                .get(index)
                .and_then(|expected| expected.get("outcome"))
                .and_then(Value::as_str)
                == Some(CANCELLED);
            if handed.send_id == send_id && handed.item.contains_key("delay_ms") && claimed {
                handed
                    .item
                    .insert("outcome".to_owned(), Value::from(CANCELLED));
            }
        }
    }

    /// The sends handed, in order, each in the case's item shape.
    pub fn handed(&self) -> Vec<Value> {
        self.handed
            .iter()
            .map(|handed| Value::Object(handed.item.clone()))
            .collect()
    }

    /// Nothing when the sends handed are exactly the ones expected, outcomes
    /// included; else the reason.
    pub fn compare(&self) -> Result<(), String> {
        let handed = self.handed();
        if handed == self.expected {
            return Ok(());
        }
        Err(format!(
            "expected the sends handed to the host {}, got {}",
            Value::Array(self.expected.clone()),
            Value::Array(handed)
        ))
    }
}

/// A handed send in the case's item shape: its `type`, its `target`, its
/// `event` with the `name` and, when the send carries a payload, the
/// `data`; `delay_ms` for a delayed send; and `send_id` only when the
/// document named the send.
fn item(
    send_type: &Option<statifier::Value>,
    target: &Option<statifier::Value>,
    event: &Option<statifier::Value>,
    data: &statifier::Value,
    delay_ms: Option<i64>,
    id_from_author: bool,
    send_id: &str,
) -> Map<String, Value> {
    let mut named = Map::new();
    named.insert("name".to_owned(), optional_json(event.as_ref()));
    if !no_payload(data) {
        named.insert("data".to_owned(), json_of(data));
    }
    let mut item = Map::new();
    item.insert("type".to_owned(), optional_json(send_type.as_ref()));
    item.insert("target".to_owned(), optional_json(target.as_ref()));
    item.insert("event".to_owned(), Value::Object(named));
    if let Some(delay_ms) = delay_ms {
        item.insert("delay_ms".to_owned(), Value::from(delay_ms));
    }
    if id_from_author {
        item.insert("send_id".to_owned(), Value::from(send_id));
    }
    item
}

/// Whether `data` is no payload: undefined, null or an empty map, as the
/// reference's runner leaves those out.
fn no_payload(data: &statifier::Value) -> bool {
    match data {
        statifier::Value::Undefined | statifier::Value::Null => true,
        statifier::Value::Map(map) => map.is_empty(),
        _ => false,
    }
}

fn optional_json(value: Option<&statifier::Value>) -> Value {
    value.map_or(Value::Null, json_of)
}

/// `value` as JSON: undefined as null, a map's keys as text, and a value
/// JSON has no type for in its debug form.
fn json_of(value: &statifier::Value) -> Value {
    match value {
        statifier::Value::Null | statifier::Value::Undefined => Value::Null,
        statifier::Value::Bool(flag) => Value::Bool(*flag),
        statifier::Value::Integer(number) => Value::from(*number),
        statifier::Value::Float(number) => {
            serde_json::Number::from_f64(*number).map_or(Value::Null, Value::Number)
        }
        statifier::Value::String(text) => Value::String(text.clone()),
        statifier::Value::List(items) => Value::Array(items.iter().map(json_of).collect()),
        statifier::Value::Map(map) => Value::Object(
            map.iter()
                .map(|(key, value)| (key_text(key), json_of(value)))
                .collect(),
        ),
        other => Value::String(format!("{other:?}")),
    }
}

fn key_text(key: &statifier::Key) -> String {
    match key {
        statifier::Key::String(text) => text.clone(),
        statifier::Key::Integer(number) => number.to_string(),
        statifier::Key::Bool(flag) => flag.to_string(),
        other => format!("{other:?}"),
    }
}

/// A step's JSON `data` as the value the event carries: a number as an
/// integer when it is one, else a float.
fn value_of(json: &Value) -> statifier::Value {
    match json {
        Value::Null => statifier::Value::Null,
        Value::Bool(flag) => statifier::Value::Bool(*flag),
        Value::Number(number) => number.as_i64().map_or_else(
            || statifier::Value::Float(number.as_f64().unwrap_or(f64::NAN)),
            statifier::Value::Integer,
        ),
        Value::String(text) => statifier::Value::String(text.clone()),
        Value::Array(items) => statifier::Value::List(items.iter().map(value_of).collect()),
        Value::Object(members) => statifier::Value::Map(
            members
                .iter()
                .map(|(key, value)| (key.as_str(), value_of(value)))
                .collect(),
        ),
    }
}

/// The options a case starts with: its id as the session id, and the send
/// types its `host` object registers. A processor `provided` does not hold,
/// or a `host` key the drive does not honour (every one is named), is the
/// reason the case fails.
fn options(item: &Value, provided: &Host) -> Result<Options, String> {
    let mut options = Options::new(id_of(item).unwrap_or(""));
    let Some(host) = item.get("host") else {
        return Ok(options);
    };
    let host = host.as_object().ok_or("the case's host is not an object")?;
    let unhonoured: Vec<&str> = host
        .keys()
        .map(String::as_str)
        .filter(|key| {
            ![
                SEND_TYPES,
                EXPECT_SENDS,
                EVENT_IO_PROCESSORS,
                DECLARED_EVENTS,
                EXPECT_ACCEPTS,
            ]
            .contains(key)
        })
        .collect();
    match unhonoured.as_slice() {
        [] => {}
        [key] => {
            return Err(format!(
                "the case needs the host key {key}, which this runner does not honour"
            ));
        }
        keys => {
            return Err(format!(
                "the case needs the host keys {}, which this runner does not honour",
                keys.join(", ")
            ));
        }
    }
    for (key, value) in host {
        match key.as_str() {
            SEND_TYPES => {
                options.send_types = names(value)
                    .ok_or("the case's host send_types is not a list of names")?
                    .into_iter()
                    .collect();
            }
            EVENT_IO_PROCESSORS => {
                let processors = names(value)
                    .ok_or("the case's host event_io_processors is not a list of names")?;
                let missing: Vec<&str> = processors
                    .iter()
                    .map(String::as_str)
                    .filter(|name| !provided.provides(name))
                    .collect();
                if !missing.is_empty() {
                    return Err(format!(
                        "the case needs the Event I/O Processor {}, which this runner does not run",
                        missing.join(", ")
                    ));
                }
            }
            _ => {}
        }
    }
    Ok(options)
}

/// The accepts check of a case's `host` object, made on the compiled chart
/// before the chart starts, as the reference's host case runner makes it
/// (`accepts/2` in `lib/mix/statifier/corpus/host_case.ex` at the vendored
/// tag): [`statifier::Machine::check_accepts`] over its [`DECLARED_EVENTS`]
/// (a JSON `null` is no declaration) must answer exactly its
/// [`EXPECT_ACCEPTS`], both lists in order and no other key. Either key
/// without the other fails the case, each with its own reason; a case with
/// neither is not checked.
pub fn accepts(machine: &statifier::Machine, item: &Value) -> Result<(), String> {
    let host = item.get("host").and_then(Value::as_object);
    let declared = host.and_then(|host| host.get(DECLARED_EVENTS));
    let expected = host.and_then(|host| host.get(EXPECT_ACCEPTS));
    let (declared, expected) = match (declared, expected) {
        (None, None) => return Ok(()),
        (Some(_), None) => {
            return Err(format!(
                "{DECLARED_EVENTS} is present without {EXPECT_ACCEPTS}"
            ));
        }
        (None, Some(_)) => {
            return Err(format!(
                "{EXPECT_ACCEPTS} is present without {DECLARED_EVENTS}"
            ));
        }
        (Some(declared), Some(expected)) => (declared, expected),
    };
    let declared = if declared.is_null() {
        None
    } else {
        Some(names(declared).ok_or("the case's host declared_events is not a list of names")?)
    };
    let names: Option<Vec<&str>> = declared
        .as_ref()
        .map(|names| names.iter().map(String::as_str).collect());
    let check = machine.check_accepts(names.as_deref());
    let actual = serde_json::json!({
        "unreachable": check.unreachable,
        "undeclared": check.undeclared,
    });
    if actual == *expected {
        Ok(())
    } else {
        Err(format!(
            "Expected the accepts check {expected}, but got {actual}"
        ))
    }
}

/// The strings of a list, `None` when it is not a list of strings.
fn names(value: &Value) -> Option<Vec<String>> {
    value
        .as_array()?
        .iter()
        .map(|item| item.as_str().map(str::to_owned))
        .collect()
}

/// The state once every send `sends` holds to report failed is reported,
/// each through [`State::report_send_failed`], and the sends each report
/// hands the host are read in turn.
fn report(state: &State, sends: &mut Sends) -> Result<State, String> {
    let mut state = state.clone();
    loop {
        let failing = sends.failing();
        if failing.is_empty() {
            return Ok(state);
        }
        for send in &failing {
            let moved = state
                .report_send_failed(send, None)
                .map_err(|error| format!("the driver refused the failed-send report: {error}"))?;
            sends.take(&moved.effects);
            state = moved.state;
        }
    }
}

/// The state once `expected` is reached or the deadline after `since` has
/// passed; the sends the timers fired on the way hand the host go to
/// `sends`, and one to report failed is reported before the configuration
/// is read again.
fn reach(
    state: &State,
    since: u64,
    expected: &[String],
    sends: &mut Sends,
) -> Result<State, String> {
    let mut state = state.clone();
    loop {
        let mut fired = Vec::new();
        let reached = clock::await_until(
            &state,
            since,
            |current| leaves(current) == expected,
            &mut fired,
        )
        .map_err(|error| format!("the configuration deadline: {error}"))?;
        sends.take(&fired);
        if !sends.has_failing() {
            return Ok(reached);
        }
        state = report(&reached, sends)?;
    }
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
