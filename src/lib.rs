//! An SCXML statechart interpreter core.
//!
//! This version holds the crate name and has no API yet.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

// Reading a chart is the reader, lowering into the typed document, then the
// validator. None of it has a caller in the library until the compiler reads
// the document; each expectation fails the build once one does, so it comes
// off then.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "no caller until the compiler reads the document")
)]
mod document;
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "no caller until the compiler reads the document")
)]
mod lowering;
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "no caller until the compiler reads the document")
)]
mod validator;
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "no caller until the compiler reads the document")
)]
mod xml;

#[cfg(test)]
mod testing;

#[cfg(test)]
mod tests {
    // Sabotage: rename the package in Cargo.toml and this assertion goes red.
    #[test]
    fn the_crate_is_named_statifier() {
        assert_eq!(env!("CARGO_PKG_NAME"), "statifier");
    }
}
