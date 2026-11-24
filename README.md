# statifier

[![MIT License][badge_license_url]](LICENSE)
[![CI][badge_ci_url]][ci_url]

An SCXML statechart interpreter for Rust, a conformant sibling of the Elixir
[statifier](https://github.com/riddler/statifier-ex) package. It reads an
SCXML document, executes it as a value step (a state and an event in, a new
state and the effects the step produced out), and is checked against the
conformance corpus that statifier-ex publishes. It has no runtime, thread,
clock or I/O of its own: a host drives it, and everything the chart does that
the outside world can see comes back as plain data.

## Why statifier

A workflow, a form flow or a protocol is easier to get right as a statechart
than as a tangle of flags, and SCXML is the W3C's standard notation for one.
Running a chart as a process, as the reference does, ties it to that
runtime. Here a running chart is an immutable value: a call takes a state
and answers a new one, so a host can keep a state, keep several, try an
event against one and discard the answer, move it between machines, or
store it and resume it later, with no scheduler to coordinate with. The
semantics are not this crate's to invent: they are the reference's, and a
shared corpus decides every disagreement.

## Install

Add the crate to your `Cargo.toml`:

```toml
[dependencies]
statifier = "0.1"
```

The crate is `statifier` on crates.io and the library is imported as
`statifier`. It is not published yet: a `0.0.0` release will hold the name
and the first real release is `0.1.0`, so until then depend on the
repository by path or git. The optional `serde` feature is off by default;
turn it on with `statifier = { version = "0.1", features = ["serde"] }`.

## Basic usage

```rust
use statifier::{Options, Value};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Compile once: a machine is an immutable value that holds the chart.
    let machine = statifier::compile(
        r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="a">
             <state id="a"><transition event="e.go" target="b"/></state>
             <final id="b"/>
           </scxml>"#,
    )
    .map_err(|refusals| format!("{refusals:?}"))?;

    // Start it, then move it with events. Each call answers a new state.
    let started = machine.start(Options::new("s-1"))?;
    assert_eq!(started.state.configuration(), ["a"]);

    let moved = started.state.step("e.go", Value::Undefined)?;
    assert_eq!(started.state.configuration(), ["a"], "the old state is unchanged");
    assert_eq!(moved.state.done().map(|done| done.configuration.clone()), Some(vec!["b".to_owned()]));
    Ok(())
}
```

## The driver and its results

A host holds a [`Machine`] and moves a chart in execution through four
calls. Each answers a `Moved`: the new `State` and the `effects` the call
produced, in the order they were produced.

- `Machine::start(options)` enters the chart's initial states and runs it to
  a stable configuration. `Options::new(session_id)` is the options, with
  starting datamodel values, the send types the host delivers itself, the
  round budget and a trace flag as fields to set.
- `State::step(name, data)` takes one external event and runs it, and every
  event the chart sends itself, to a stable configuration.
- `State::advance(ms)` moves the virtual clock forward and fires every timer
  due by then, earliest first. The clock is a number the host moves: nothing
  here reads the system clock, so a test advances an hour in no time.
- `State::export_position()` and `Machine::import_position(&position)` move
  a chart between machines (below).

A state is read without moving it: `configuration()` (the active states'
ids), `active_leaf_states()` and `active_leaf_count()`, `done()` (`None`
while the chart runs; once it has stopped, its donedata and the
configuration it stopped in), `now_ms()` and `next_due_ms()`, the virtual
time the earliest pending timer fires at, which is when a host next calls
`advance`.

```rust
use statifier::{Options, Value};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let machine = statifier::compile(
        r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="a">
             <state id="a"><onentry><send event="e.go" delay="1s"/></onentry>
               <transition event="e.go" target="b"/></state>
             <final id="b"/>
           </scxml>"#,
    )
    .map_err(|refusals| format!("{refusals:?}"))?;
    let started = machine.start(Options::new("s-1"))?;
    assert_eq!(started.state.next_due_ms(), Some(1_000));

    // Nothing is due yet, and the clock moves.
    let early = started.state.advance(999)?;
    assert_eq!(early.state.configuration(), ["a"]);
    assert_eq!(early.state.now_ms(), 999);

    // The delayed send fires, and the chart stops in its final state.
    let moved = early.state.advance(1)?;
    assert_eq!(moved.state.now_ms(), 1_000);
    assert_eq!(moved.state.done().map(|done| done.configuration.clone()), Some(vec!["b".to_owned()]));
    Ok(())
}
```

A chart's datamodel holds predicator values: `statifier::Value` is the
predicator-rs crate's `Value`, re-exported rather than mirrored, so a value
a host passes in and a value an expression answers are one type. That type
is part of this crate's public API, so a release of this crate that moves
to a new minor version of predicator-rs is a new minor version of this
crate too.

## Refusals are values

Nothing a host passes in makes the crate panic: a document, an event, a
duration, a position, a send result. A call that cannot proceed answers an
`Err`, and every public enum is `#[non_exhaustive]`, so a host matches with
a wildcard arm.

- `compile` answers the `Refusal`s of the first stage that refused the
  document (the reader, lowering, the validator or the compiler), every one
  that stage found, each with a `reason()` (the reference's reason, under
  its name), a `message()` for a person and a `span()` of the source.
- A call that moves a chart answers an `Error`: `NotRunning` once the chart
  has stopped, `InvalidDuration` for an `advance` that would move the clock
  past its largest value, `BuiltInSendTypes` for options that register a
  type the crate delivers itself.
- A position that cannot be exported or imported answers an `ExportError`
  or an `ImportError`.

```rust
use statifier::{CompileReason, Options, Reason, Value};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // A document is refused with a reason, a message and a span.
    let refusals = statifier::compile(
        r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0">
             <state id="a"><transition cond="x >" target="a"/></state>
           </scxml>"#,
    )
    .unwrap_err();
    assert!(matches!(
        refusals[0].reason(),
        Reason::Compile(CompileReason::ExpressionCompileError { .. })
    ));
    assert_eq!(refusals[0].span().start_line, 2);

    // A stopped chart takes no more events.
    let machine = statifier::compile(
        r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="a">
             <final id="a"/>
           </scxml>"#,
    )
    .map_err(|refusals| format!("{refusals:?}"))?;
    let stopped = machine.start(Options::new("s-1"))?.state;
    assert_eq!(
        stopped.step("e.go", Value::Undefined).unwrap_err(),
        statifier::Error::NotRunning
    );
    Ok(())
}
```

## Effects are data

Interpreting never logs, never writes anywhere and never delivers an event
itself. Everything the chart did that a host may observe is an `Effect`, an
owned value the host reads in the order the chart made it: a `<log>` that
ran, a write to the datamodel, a `<send>` for the host to deliver, a delayed
send, a cancel, an invoke, and the chart's own stop. A host matches the
variants it acts on, with a wildcard arm and `..` for the fields, since each
is `#[non_exhaustive]`.

A host registers the send types it delivers itself in
`Options::send_types`. A `<send>` of one of them is answered as an
`Effect::Send` and is never routed or scheduled here, delayed or not; the
crate calls no host code. The host delivers the message however it likes
and, if the answer is an event for the chart, passes it back with `step`.

```rust
use statifier::{Effect, Options};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let machine = statifier::compile(
        r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="a">
             <state id="a">
               <onentry><send type="x-example" target="t1" event="e.out"/></onentry>
             </state>
           </scxml>"#,
    )
    .map_err(|refusals| format!("{refusals:?}"))?;

    let mut options = Options::new("s-1");
    options.send_types.insert("x-example".to_owned());
    let started = machine.start(options)?;

    let sends: Vec<_> = started
        .effects
        .iter()
        .filter(|effect| matches!(effect, Effect::Send { .. }))
        .collect();
    assert_eq!(sends.len(), 1);
    Ok(())
}
```

With the `serde` feature an effect serializes as a map whose `kind` is the
variant in snake case beside its fields in camel case.

## Positions

A chart's position moves between machines in the reference's string-id
vocabulary: `State::export_position` answers a `Position` (every state named
by the id the document wrote, the datamodel, the counters, the running flag
and status, the trace flag and the round budget), and
`Machine::import_position` reads one back into a state, onto the same chart
or another that holds every state it names. A position carries the
interpreter's state only: the virtual clock, a pending timer and a queued
external event are the driver's, so an imported state starts with none, and
the host schedules what it still expects on its own clock. The export
refuses a state whose internal queue holds an event or that holds a state
with no id; the import refuses, among others, every state id the chart does
not hold, named at once.

```rust
use statifier::{Options, Value};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let source = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="a">
                      <state id="a"><transition event="e.go" target="b"/></state>
                      <state id="b"><transition event="e.go" target="c"/></state>
                      <final id="c"/>
                    </scxml>"#;
    let machine = statifier::compile(source).map_err(|refusals| format!("{refusals:?}"))?;
    let moved = machine.start(Options::new("s-1"))?.state.step("e.go", Value::Undefined)?;
    let position = moved.state.export_position()?;

    // Resume on a machine compiled afresh, and drive on: it answers what the
    // state it was taken from answers.
    let again = statifier::compile(source).map_err(|refusals| format!("{refusals:?}"))?;
    let resumed = again.import_position(&position)?;
    assert_eq!(resumed.configuration(), ["b"]);
    let next = resumed.step("e.go", Value::Undefined)?.state;
    let original = moved.state.step("e.go", Value::Undefined)?.state;
    assert_eq!(next.done(), original.done());
    Ok(())
}
```

The proof that a position carries what it needs is a stage of the gate
(`mise run gate:position`). For every scion case of the corpus, the stage
exports the position after the start and after every step, takes it through
JSON text, compiles the case's source again into a fresh machine, imports
the position there and drives the rest of the case's events on it. The
imported state must agree with the unbroken drive at that point and after
every later step: the same export answer, the same configuration and active
leaf states, and the same answer on whether the chart has stopped. A point a
position cannot carry (a pending timer is the driver's, not the position's)
is counted and named, never skipped, and the stage fails on any
disagreement.

The optional `serde` feature implements `Serialize` and `Deserialize` for a
position, an effect and a stopped chart's `Done`, and turns on
predicator-rs's own for a value. The implementations are written by hand, so
the feature adds the `serde` crate alone. A position's JSON form has the keys
of the TypeScript sibling's, in camel case, each datamodel root written as
predicator's tagged text of its value, so a position one sibling writes
reads in the other.

## Compatibility

- **Rust 1.85 or later** (`rust-version` in `Cargo.toml`, edition 2024). CI
  runs the test suite with every feature on that Rust, and the full quality
  gate on the toolchain `mise.toml` pins.
- **One runtime dependency**, the [predicator-rs](https://crates.io/crates/predicator-rs)
  crate, which evaluates a chart's conditions, expressions and assignments.
  `serde` is optional, behind the `serde` feature, off by default. The
  SCXML reader is written here: no XML crate, no JSON crate, no HTTP client
  and no async runtime.
- **The standard library is required.** The crate is not `no_std`.
- **wasm32-unknown-unknown.** CI builds the library for that target, with the
  default features and with every feature. Nothing here reads the system
  clock: time is a number the host passes to `advance`.
- **No unsafe code.** The crate is `#![forbid(unsafe_code)]`, and has no
  global state: no `static mut`, no thread-local and no global
  configuration; the state is an immutable value and the options are passed
  per call.

## Conformance

The chart semantics are defined by the reference implementation's
conformance corpus, vendored under `conformance/` at the release tag
`v2.16.0` of statifier-ex (`conformance/statifier.vendored.json` names the
tag, the commit and the corpus hash, and `conformance/README.md` says what
is copied and from where). `conformance/registry.json` records one entry per
corpus case this crate passes, and the claims those entries make. At this
head:

| Claim | Cases passing |
|---|---|
| scion | 119 |
| statifier | 20 |
| w3c-mandatory | 152 |
| w3c-optional | 2 |
| Total | 293 |

No case is skipped. A case this crate cannot pass is a defect to bring to
the reference, never an entry on a skip list. The full gate re-runs the
corpus and fails when the registry and a run made now disagree, and a test
fails when the table above and the registry disagree.

## Differences from the reference

Where the corpus does not fix an answer, this crate follows statifier-ex at
the vendored tag. It differs in these readings of malformed or unusual XML,
none of which any corpus document holds:

- A processing instruction whose target starts with `xml` at the very start
  of a document, such as `<?xmlfoo?>`, is read as a processing instruction;
  statifier-ex refuses the document.
- A character reference to a code point outside XML 1.0's `Char` production,
  such as `&#0;`, `&#1;` or `&#xFFFE;`, is refused; statifier-ex decodes it.
- A three-character processing instruction target that is not `xml` in
  some case but starts with `l`, has `m` second or ends with `x`, in either
  case (such as `<?lab?>` or `<?amb?>`), is read as a processing
  instruction; statifier-ex refuses the document.

It differs in these readings of a document's elements, none of which any
corpus document holds either:

- An element nested more than 128 elements below the `<scxml>` root is
  refused; statifier-ex reads any depth. Markup kept as text inside
  `<content>` or `<assign>`, and an element inside `<data>`, `<script>`,
  `<elseif>` or `<else>`, is not counted, since neither port descends into
  it.
- The message refusing stray text previews its first forty code points;
  statifier-ex previews forty grapheme clusters.

It differs in these readings of a chart's expressions, which the predicator
crate decides and statifier-ex's predicator does not share, none of which
any corpus document holds:

- An integer is 64-bit. A condition or value holding an integer literal
  outside that range is refused at compile time; statifier-ex compiles it.
  An in-line `<data>` or `<assign>` value holding one is kept as its text,
  a string, where statifier-ex reads the integer.
- An expression nests 400 levels at most; a deeper one is refused at
  compile time, and statifier-ex compiles it.
- A `<send>` delay is a 64-bit count of milliseconds. A delay whose
  milliseconds fall outside that range is refused as an invalid delay;
  statifier-ex reads any size.

It differs in these writes to a location, such as an `<assign>`'s, none
of which any corpus document holds:

- A write at a list index of 1,048,576 or more that lies past the list's
  end, such as `a[2000000]` against a short list, is refused as an invalid
  index; statifier-ex pads the list to any length.
- A location is read by this crate's own reader, since the predicator crate
  has no location API: a root name followed by property and bracket
  accesses. Text of any other shape is refused as not assignable, where
  statifier-ex refuses some of it as a parse error. Both raise
  `error.execution`; only the reason its data carries differs.

It differs in this reading of a chart's history states, which no corpus
document holds either:

- A history whose default transition leads back to it through other
  history states of the same parent, such as two histories each naming the
  other as its default target, stands for no state once the chain returns
  to it: a transition targeting it alone leaves and enters nothing, and
  entering it enters nothing. statifier-ex never finishes resolving such a
  history.

It differs in this count of a session's sends, which no corpus document
reaches:

- The send counter, which a generated send id (`send_1`, `send_2`, ...)
  counts from, and the timer counter, which a delayed send's, a cancel's
  and a registered type's ordinal counts from, are 64-bit and stop at their
  largest value; statifier-ex counts without bound.

It differs in these parts of the driver, the reference's session process
made a synchronous value step:

- One call also spends the round budget on three counts, each kept at one
  virtual time and started over whenever `State::advance` moves the clock
  on: the events it takes that an immediate send to the session itself
  queued in the same call, the immediate sends it delivers onto the
  internal queue, and the timers it fires that were scheduled at that same
  virtual time in the same call. One more than the budget on any of the
  three halts the driver as a spent macrostep does, answering
  `Effect::BudgetExhausted`. Under a bounded budget every call therefore
  returns, a timer rescheduled due at once included; timers with a delay,
  a chain or a fan-out, are bounded by the duration advanced and never halt
  on these counts, however the host splits time. Under
  `RoundBudget::Unbounded` no count ever halts the driver, and such a call
  may never return. A timer scheduled by an earlier call, or at an earlier
  virtual time, is not counted. The reference's session never returns, so
  it counts none of them.
- `State::advance` takes a whole number of milliseconds, so no duration is
  negative; it refuses one that moves the clock past its largest value
  with `Error::InvalidDuration`, the driver's name for what the reference
  calls an invalid delay.
- A send of a registered type is answered as an effect and nothing more:
  the driver calls no host processor, takes no report of a failed send,
  and writes an empty `_ioprocessors` entry for each registered type.
- The `trace` option is taken and ignored: no trace effect is ported, so
  none is answered.
- `Effect::SendDelayed`, `Effect::Cancel`, `Effect::Invoke` and
  `Effect::CancelInvoke` do not carry the caller context the reference
  copies from the external event that started the macrostep, and a host
  event carries none, so no `done.invoke` event carries one either.
- An invoked child's session id is its parent's, a dot and the invoke id;
  the reference gives an invoked child a session id of its own.
- An invoked child inherits its parent's registered send types; the
  reference makes that inheritance opt-in, off by default.
- An invoked child runs inside its parent's call, and what it sends its
  parent waits in the parent's mailbox until the parent's external queue is
  empty; the reference's child is a process of its own, whose messages
  reach the parent as the runtime delivers them.
- `State::configuration` and `Done::configuration` leave out a state the
  document gave no id, as the reference leaves it out where it turns
  states into ids.
- `CompileReason::ExpressionCompileError` does not name the expression's
  owner, which the reference's refusal does; the refusal's span locates
  the expression.

It differs in these parts of a position, the reference's
`Statifier.Position.export/1` and `import/2`:

- An export's `identity` is null: this crate computes no chart identity.
  The import reads no identity, as the reference's reads none, so a
  position another sibling wrote keeps the one it carried.
- The import refuses a position whose datamodel `_sessionid` is not a
  string, since the state it answers runs as that session; the reference
  takes the session from the process it resumes in.
- The import refuses a recorded history value whose key is not a
  `<history>`, or that holds a history or a state outside the history's
  parent, two live invocations naming one `<invoke>`, and a live
  invocation naming an `<invoke>` its state does not hold; the reference
  imports each, and entering such a history need never finish.
- The registered send types of an imported state are the entries of its
  datamodel's `_ioprocessors` other than the SCXML Event I/O Processor's,
  as `Machine::start` writes them; the reference's driver stamps them
  again from its own options.
- With the `serde` feature, a JSON position missing a key or holding a
  value of the wrong shape is refused by the deserializer, naming the
  first it meets; the reference names every missing key at once.
- Every counter of a position (the invoke, send and timer counters, the
  macrostep, the microstep and the round) and an active invocation's
  invoke index is a non-negative 64-bit count, so a position cannot hold a
  negative one; the reference takes any integer. With the `serde` feature
  the reader refuses a negative one, and one above the 64-bit signed range
  too, since every value is read through the predicator crate's reader.

It differs in this part of its conformance check, which no user of the
crate sees:

- The registry check started on an empty registry, the pin with no claims
  and no entries, and passed it as the registry that makes no claim, where
  the corpus's `RATCHET.md` (rule 3) fails a check with no entries to
  verify. From the first claim on, it refuses an empty registry.

`compile` does not yet refuse a chart whose expressions call a function no
evaluation can supply: statifier-ex refuses such a chart with the reason
`unknown_function`, and here the call fails only when it is evaluated.

## Documentation

- [The API reference on docs.rs](https://docs.rs/statifier): every public
  type, function and method, with examples that run as tests.
- [The changelog](CHANGELOG.md): what changed in each version.
- [The conformance corpus](conformance/README.md): what is vendored, from
  where, and what is this repository's own.

## Development

After checking out the repository, run `mise install` for the Rust toolchain
and the cargo tools `mise.toml` pins. Then `mise run gate:loop` runs the
inner loop (format, lints, tests), and `mise run gate` runs the full quality
gate: the inner loop, the corpus, registry and position checks, the docs
with every feature and with none, line coverage and `cargo deny check`. The
README's Rust examples run as doctests.

## Contributing

Bug reports and pull requests are welcome on GitHub at
https://github.com/riddler/statifier-rs.

## License

MIT, see [LICENSE](LICENSE).

[badge_license_url]: https://img.shields.io/badge/license-MIT-blue.svg
[badge_ci_url]: https://github.com/riddler/statifier-rs/actions/workflows/ci.yml/badge.svg
[ci_url]: https://github.com/riddler/statifier-rs/actions/workflows/ci.yml
