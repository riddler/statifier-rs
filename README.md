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
that refused the document, each with a reason, a message and a span. It
does not execute a machine yet.

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

`compile` does not yet refuse a chart whose expressions call a function no
evaluation can supply: statifier-ex refuses such a chart with the reason
`unknown_function`, and here the call fails only when it is evaluated.

## License

MIT, see [LICENSE](LICENSE).
