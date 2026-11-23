//! The event-name matcher of the specification's section 3.13, the
//! pseudocode's `nameMatch`, as the reference matches
//! (`Statifier.Interpreter.NameMatch`).
//!
//! A descriptor matches an event name when its tokens are a prefix of the
//! event's tokens on token boundaries: `error` matches `error.execution` and
//! not `errors.execution`. A trailing `*` token, or the empty token a
//! trailing dot leaves, is dropped before the prefix test, so `*` matches
//! every event, `a.*` matches `a` and `a.b`, and `a.` matches as `a` does. A
//! `*` anywhere else is matched as written.
//!
//! A transition's descriptors arrive split on their dots by the compiler
//! (the machine's `Transition::events`), so both sides are token lists; an
//! event name is split once per selection round by [`tokenize`].

/// Whether any of `descriptors` (one token list per descriptor of a
/// transition's `event` attribute) matches the event whose name is
/// `event_tokens`.
pub(crate) fn name_match(descriptors: &[Vec<String>], event_tokens: &[&str]) -> bool {
    descriptors
        .iter()
        .any(|descriptor| descriptor_match(descriptor, event_tokens))
}

/// Whether one descriptor's tokens, with a trailing `*` or empty token
/// dropped, are a prefix of `event_tokens`.
fn descriptor_match(descriptor: &[String], event_tokens: &[&str]) -> bool {
    let normalized = match descriptor.split_last() {
        Some((last, rest)) if last == "*" || last.is_empty() => rest,
        _ => descriptor,
    };
    normalized.len() <= event_tokens.len()
        && normalized
            .iter()
            .zip(event_tokens)
            .all(|(descriptor, event)| descriptor == event)
}

/// An event name split on its dots, every empty part kept: the empty name
/// is one empty token, as the reference splits it.
pub(crate) fn tokenize(name: &str) -> Vec<&str> {
    name.split('.').collect()
}
