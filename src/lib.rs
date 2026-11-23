//! An SCXML statechart interpreter core.
//!
//! This version holds the crate name and has no API yet.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

// The XML reader has no caller in the library until lowering uses it; the
// expectation fails the build once one does, so it comes off then.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "no caller until lowering uses the XML reader")
)]
mod xml;

#[cfg(test)]
mod tests {
    // Sabotage: rename the package in Cargo.toml and this assertion goes red.
    #[test]
    fn the_crate_is_named_statifier() {
        assert_eq!(env!("CARGO_PKG_NAME"), "statifier");
    }
}
