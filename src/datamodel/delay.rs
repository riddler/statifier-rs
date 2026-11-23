//! A `<send>` delay in whole milliseconds, read through the predicator
//! crate's duration reader ([`predicator_rs::parse_duration`]), as the
//! reference reads it through its expression language's: one duration
//! vocabulary, so the units are the crate's eight (`y`, `mo`, `w`, `d`,
//! `h`, `m`, `s`, `ms`), a superset of the five the SCXML schema names.
//!
//! The one rewrite is the reference's: the schema admits a bare leading dot
//! (`.5s`) and the reader does not, so a leading `.` becomes `0.`. The sum
//! is the reference's expression language's conversion: a week is seven
//! days, a month thirty days and a year three hundred and sixty-five.

use super::Failure;
use predicator_rs::{Duration, Value};

/// `value` (a delay's text, or what a `delayexpr` evaluated to) in
/// milliseconds. Text is read by the duration reader; a duration is summed.
/// A map, a date or an instant is 0, as the reference reads any map, which
/// carries none of a duration's units. Anything else, text the reader
/// refuses, and a sum outside 64 bits are a [`Failure::InvalidDelay`]
/// carrying the value as given.
pub(crate) fn milliseconds(value: &Value) -> Result<i64, Failure> {
    let invalid = || Failure::InvalidDelay(value.clone());
    match value {
        Value::String(text) => {
            let text = match text.strip_prefix('.') {
                Some(rest) => format!("0.{rest}"),
                None => text.clone(),
            };
            let duration = predicator_rs::parse_duration(&text).map_err(|_| invalid())?;
            total(&duration).ok_or_else(invalid)
        }
        Value::Duration(duration) => total(duration).ok_or_else(invalid),
        Value::Map(_) | Value::Date(_) | Value::DateTime(_) => Ok(0),
        _ => Err(invalid()),
    }
}

/// A duration's milliseconds, `None` outside 64 bits.
fn total(duration: &Duration) -> Option<i64> {
    let weighted = [
        (duration.years, 31_536_000_000_i128),
        (duration.months, 2_592_000_000),
        (duration.weeks, 604_800_000),
        (duration.days, 86_400_000),
        (duration.hours, 3_600_000),
        (duration.minutes, 60_000),
        (duration.seconds, 1_000),
        (duration.milliseconds, 1),
    ];
    let sum: i128 = weighted
        .into_iter()
        .map(|(amount, weight)| i128::from(amount) * weight)
        .sum();
    i64::try_from(sum).ok()
}
