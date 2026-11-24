### Added

- `<invoke>` runs: an invocation of the SCXML type whose `<content>` is inline markup starts an in-process child session, its session id its parent's, a dot and the invoke id, inheriting its parent's registered send types; the child's effects are answered among the call's as `Effect::Child`, naming its session id. A child and its parent exchange events through `#_parent` and `#_` and the invoke id, its timers fire through its parent's `State::advance`, it returns `done.invoke.` and the invoke id with its donedata when it stops, and the exit of the invoking state cancels it.
- `Effect::Invoke`, `Effect::CancelInvoke`, `Effect::Autoforward` and `Effect::Child`, and `BlockOwner::Invoke`, the owner of an `<invoke>`'s `idlocation` write; with the `serde` feature each serializes with its own `kind`.
- `<finalize>` runs on an event from its invocation before the parent's selection; an empty one writes the returned values to its `namelist` and `<param location>` entries. An `autoforward` invocation is handed each external event its parent takes.
- `ImportError::UnknownInvoke`: `Machine::import_position` refuses a live invocation naming an `<invoke>` its state does not hold.

### Changed

- `State::next_due_ms` counts an invoked child's pending timers.
- A state's exit takes it off the states to invoke and cancels its live invocations, so a position exported after the exit names neither.
