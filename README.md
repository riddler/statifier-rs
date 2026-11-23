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

## License

MIT, see [LICENSE](LICENSE).
