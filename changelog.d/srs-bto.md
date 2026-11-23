### Added

- `compile(source)` reads an SCXML document into an immutable `Machine`, compiling every expression through the `predicator-rs` crate, or answers the `Refusal`s of the first stage that refused it, each with a reason, a message and a span; a machine carries the validator's warnings.
- The crate takes its one runtime dependency, `predicator-rs` 0.1.
