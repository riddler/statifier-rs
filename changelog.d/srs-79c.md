### Added

- `ActiveInvocation::new(state, invoke_index, invoke_id)` builds a live invocation for a position a host keeps in its own storage.
- `Position` implements `Default`: the empty position, which a host fills field by field from its own storage and passes to `Machine::import_position`, with or without the `serde` feature.

### Changed

- `basichttp::TransportError` is no longer `#[non_exhaustive]`: its fields are private, as `Refusal`'s and `Warning`'s are, so the attribute changed nothing a caller could do.
