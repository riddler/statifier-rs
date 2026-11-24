//! The run's virtual clock: the reference harness's two knobs, applied on
//! the driver's virtual clock rather than on a wall clock, so the run never
//! sleeps.
//!
//! The constants are the reference's, read in statifier-ex at the vendored
//! tag (`@default_settle_window_ms` and `@default_configuration_deadline_ms`
//! in `lib/statifier/testing/case.ex`, the harness a scion case runs
//! through, and `@settle_window_ms` and `@configuration_deadline_ms` in
//! `lib/mix/statifier/corpus/host_case.ex`). The drive in `runner.rs` steps
//! every case through them.

use statifier::{Effect, Error, State};

/// Before each event, the window within which a pending timer may fire.
pub const SETTLE_WINDOW_MS: u64 = 100;

/// After the start and after each event, how long the run waits for the
/// configuration it expects.
pub const CONFIGURATION_DEADLINE_MS: u64 = 4_000;

/// The settle window: every timer due within [`SETTLE_WINDOW_MS`] of the
/// clock's time fires, earliest first; a state with no timer pending is
/// answered as it is. The effects the timers fired produce are appended to
/// `effects`, in order.
pub fn settle(state: &State, effects: &mut Vec<Effect>) -> Result<State, Error> {
    // As the reference's settle_short_timers_until/2 returns at once with no timer pending.
    if state.next_due_ms().is_none() {
        return Ok(state.clone());
    }
    let moved = state.advance(SETTLE_WINDOW_MS)?;
    effects.extend(moved.effects);
    Ok(moved.state)
}

/// The configuration deadline: due timers fire, earliest first, until
/// `reached` holds, the chart has stopped, no timer is pending, or the next
/// one falls due more than [`CONFIGURATION_DEADLINE_MS`] after `since`. It
/// ends under a bounded budget because every advance does: a chart that
/// keeps rescheduling a timer due at once spends the call's budget at that
/// virtual time and halts, and a halted chart schedules nothing more; a
/// timer due later than the deadline ends the wait. The effects the timers
/// fired produce are appended to `effects`, in order.
pub fn await_until(
    state: &State,
    since: u64,
    reached: impl Fn(&State) -> bool,
    effects: &mut Vec<Effect>,
) -> Result<State, Error> {
    let deadline = since.saturating_add(CONFIGURATION_DEADLINE_MS);
    let mut current = state.clone();
    loop {
        if reached(&current) || current.done().is_some() {
            return Ok(current);
        }
        match current.next_due_ms() {
            Some(due) if due <= deadline => {
                let moved = current.advance(due.saturating_sub(current.now_ms()))?;
                effects.extend(moved.effects);
                current = moved.state;
            }
            _ => return Ok(current),
        }
    }
}
