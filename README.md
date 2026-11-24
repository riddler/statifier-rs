# statifier

An SCXML statechart interpreter for Rust, a conformant sibling of the Elixir
[statifier](https://github.com/riddler/statifier-ex) package. It will read an
SCXML document, run it as a value step (state and event in, new state and
effects out) and be checked against the conformance corpus that statifier-ex
publishes.

This crate is not usable yet. Version `0.0.0` holds the name `statifier` on
crates.io; the first real release follows. On `main` the crate reads a
chart: `statifier::compile` takes an SCXML document's source and answers an
immutable machine (every state interned, every expression compiled through
the [predicator-rs](https://crates.io/crates/predicator-rs) crate, the
validator's warnings carried on it), or the refusals of the first stage
that refused the document, each with a reason, a message and a span.
`Machine::start` starts the chart in execution, and the state it answers
moves by `State::step` (an external event) and `State::advance` (the
virtual clock); each call answers a new state and the effects it produced,
and leaves the state it was given unchanged. The driver takes no runtime,
thread, clock or I/O of its own: a `<send>` of a type the host registered
is answered as an effect for the host to deliver.

```rust
use statifier::Options;

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
    assert_eq!(started.state.configuration(), ["a"]);
    let moved = started.state.advance(1_000)?;
    assert_eq!(moved.state.done().map(|done| done.configuration.clone()), Some(vec!["b".to_owned()]));
    Ok(())
}
```

A chart's datamodel holds predicator values: `statifier::Value` is the
predicator-rs crate's `Value`, re-exported rather than mirrored, so a
value a host passes in and a value an expression answers are one type.
That type is part of this crate's public API, so a release of this crate
that moves to a new minor version of predicator-rs is a new minor version
of this crate too.

The conformance corpus this crate is checked against is vendored under
`conformance/`, whose README says what is copied and from where.

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

- One call also spends the round budget on the events it takes that an
  immediate send to the session itself queued in the same call, on the
  immediate sends it delivers onto the internal queue, and on the timers it
  fires at one virtual time that were scheduled in the same call (the count
  starts over whenever the clock moves on); one more than the budget on any
  of the three halts the driver as a spent macrostep does, answering
  `Effect::BudgetExhausted`. Under a bounded budget every call therefore
  returns, a timer rescheduled due at once included; a chain of timers with
  a delay is bounded by the duration advanced and never halts on this
  count. Under `RoundBudget::Unbounded` no count ever halts the driver, and
  such a call may never return. A timer an earlier call scheduled is not counted. The
  reference's session never returns, so it counts none of them.
- `State::advance` takes a whole number of milliseconds, so no duration is
  negative; it refuses one that moves the clock past its largest value
  with `Error::InvalidDuration`, the driver's name for what the reference
  calls an invalid delay.
- A send of a registered type is answered as an effect and nothing more:
  the driver calls no host processor, takes no report of a failed send,
  and writes an empty `_ioprocessors` entry for each registered type.
- The `trace` option is taken and ignored: no trace effect is ported, so
  none is answered.
- `Effect::SendDelayed` and `Effect::Cancel` do not carry the caller
  context the reference copies from the external event that started the
  macrostep, and a host event carries none.
- `State::configuration` and `Done::configuration` leave out a state the
  document gave no id, as the reference leaves it out where it turns
  states into ids.
- `CompileReason::ExpressionCompileError` does not name the expression's
  owner, which the reference's refusal does; the refusal's span locates
  the expression.

`compile` does not yet refuse a chart whose expressions call a function no
evaluation can supply: statifier-ex refuses such a chart with the reason
`unknown_function`, and here the call fails only when it is evaluated.

## License

MIT, see [LICENSE](LICENSE).
