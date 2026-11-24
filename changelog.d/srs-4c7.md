### Added

- `Machine::start(options)` starts a chart in execution and answers a `Moved`: the new `State` and the effects the start produced. `Options` carries the session id, starting datamodel values, the send types the host delivers itself, the `RoundBudget` and the trace flag.
- `State::step(name, data)` sends the chart one external event, and `State::advance(ms)` moves its virtual clock, firing every pending timer due by then; each answers a new state and its effects and leaves the state it was given unchanged.
- `State::configuration()`, `State::done()`, `State::now_ms()` and `State::next_due_ms()` read the active state ids, the stopped chart's donedata and final configuration, the virtual clock and the earliest pending timer.
- `Error`, the refusal of a driver call: `NotRunning`, `InvalidDuration` and `BuiltInSendTypes`.
- `Map` and `Key`, the predicator crate's map and key types, are re-exported beside `Value`.

### Changed

- `Owner` is renamed `DefaultTransitionOwner`.
- Every validation, lowering and compile reason variant that carries detail is `#[non_exhaustive]`: match its fields with `..`.
- `Effect::Send` and `Effect::SendDelayed` carry `None` for an event, target or type expression that evaluates to null, as for one not written.
