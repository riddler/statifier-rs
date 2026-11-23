//! An SCXML statechart interpreter core.
//!
//! This version holds the crate name and has no API yet.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

#[cfg(test)]
mod tests {
    // Sabotage: rename the package in Cargo.toml and this assertion goes red.
    #[test]
    fn the_crate_is_named_statifier() {
        assert_eq!(env!("CARGO_PKG_NAME"), "statifier");
    }
}
