# statifier

An SCXML statechart interpreter for Rust, a conformant sibling of the Elixir
[statifier](https://github.com/riddler/statifier-ex) package. It will read an
SCXML document, run it as a value step (state and event in, new state and
effects out) and be checked against the conformance corpus that statifier-ex
publishes.

This crate is not usable yet. Version `0.0.0` holds the name `statifier` on
crates.io and contains no library code; the first real release follows.

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

## License

MIT, see [LICENSE](LICENSE).
