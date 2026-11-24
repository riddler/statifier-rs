use super::*;

const OPEN: &str = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0""#;

fn machine(attributes: &str, body: &str) -> Machine {
    let text = format!("{OPEN}{attributes}>{body}</scxml>");
    match crate::compile(&text) {
        Ok(machine) => machine,
        Err(refusals) => panic!("{text}: {refusals:#?}"),
    }
}

// The vocabulary, read as an empty declaration reads it: every descriptor
// undeclared.
fn vocabulary_of(machine: &Machine) -> Vec<String> {
    machine.check_accepts(Some(&[])).undeclared
}

const TWO: &str = r#"<state id="a"><transition event="e.go" target="b"/></state>
    <state id="b"><transition event="x"/></state>"#;

// No declaration answers both lists empty, whatever the chart listens for.
// Sabotage: make check_accepts treat None as an empty declaration; the
// assert_eq on undeclared goes red.
#[test]
fn no_declaration_answers_both_lists_empty() {
    let check = machine("", TWO).check_accepts(None);
    assert_eq!(check.unreachable, Vec::<String>::new());
    assert_eq!(check.undeclared, Vec::<String>::new());
    assert_eq!(check, AcceptsCheck::default());
}

// An empty declaration is a declaration: nothing unreachable, every
// descriptor undeclared.
// Sabotage: make check_accepts answer the default for an empty slice too;
// the assert_eq on undeclared goes red.
#[test]
fn an_empty_declaration_leaves_every_descriptor_undeclared() {
    let check = machine("", TWO).check_accepts(Some(&[]));
    assert_eq!(check.unreachable, Vec::<String>::new());
    assert_eq!(check.undeclared, ["e.go", "x"]);
}

// Unreachable names keep the declaration's order and appear once.
// Sabotage: drop the dedupe filter on the declared names; the assert_eq
// goes red with q twice.
#[test]
fn unreachable_names_keep_the_declarations_order_each_once() {
    let check = machine("", TWO).check_accepts(Some(&["q", "x", "p", "q", "e.go"]));
    assert_eq!(check.unreachable, ["q", "p"]);
    assert_eq!(check.undeclared, Vec::<String>::new());
}

// A descriptor matches a declared name on token boundaries, a trailing `*`
// or dot dropped, as selection matches an event; a declared name is never a
// pattern.
// Sabotage: make descriptor_match compare with str::starts_with on the
// joined strings; the assert_eq on e.gone goes red.
#[test]
fn a_descriptor_matches_a_declared_name_on_token_boundaries() {
    for descriptor in ["e.go", "e.*", "e.", "e", "*"] {
        let chart = machine(
            "",
            &format!(r#"<state id="a"><transition event="{descriptor}"/></state>"#),
        );
        let check = chart.check_accepts(Some(&["e.go"]));
        assert_eq!(check.unreachable, Vec::<String>::new(), "{descriptor}");
        assert_eq!(check.undeclared, Vec::<String>::new(), "{descriptor}");
    }
    for descriptor in ["e.gone", "e.go.now"] {
        let chart = machine(
            "",
            &format!(r#"<state id="a"><transition event="{descriptor}"/></state>"#),
        );
        let check = chart.check_accepts(Some(&["e.go"]));
        assert_eq!(check.unreachable, ["e.go"], "{descriptor}");
        assert_eq!(check.undeclared, [descriptor], "{descriptor}");
    }
    let plain = machine("", r#"<state id="a"><transition event="e"/></state>"#);
    assert_eq!(
        plain.check_accepts(Some(&["e.*"])).unreachable,
        Vec::<String>::new()
    );
    let near = machine("", r#"<state id="a"><transition event="e.go"/></state>"#);
    assert_eq!(
        near.check_accepts(Some(&["e.gone"])).unreachable,
        ["e.gone"]
    );
    let deeper = machine("", r#"<state id="a"><transition event="e.go"/></state>"#);
    assert_eq!(deeper.check_accepts(Some(&["e.*"])).unreachable, ["e.*"]);
}

// The vocabulary is in transition order, a state's own transitions before
// its children's even when written after them, patterns as written, each
// descriptor once.
// Sabotage: drop the dedupe filter in vocabulary; the assert_eq goes red
// (x twice).
#[test]
fn the_vocabulary_is_in_transition_order_each_descriptor_once() {
    let chart = machine(
        r#" initial="p""#,
        r#"<state id="p">
             <state id="c"><transition event="x y.* e."/></state>
             <transition event="z x"/>
           </state>
           <state id="q"><transition event="w"/></state>
           <state id="r"><transition event="v" target="q"/></state>"#,
    );
    // r is entered by no path, so v is not in the vocabulary; q is reached
    // only through r, so w is not either.
    assert_eq!(vocabulary_of(&chart), ["z", "x", "y.*", "e."]);
}

// The vocabulary reads only the states some path enters: a transition's
// targets, their ancestors, every region of an entered parallel and a
// history's default target, and the walk ends on a cycle.
// Sabotage: make mark_entered queue no transition targets; the assert_eq
// goes red (only the initial path's descriptors remain).
#[test]
fn the_vocabulary_reads_only_states_some_path_enters() {
    let chart = machine(
        r#" initial="a""#,
        r#"<state id="a"><transition event="to.b" target="b1"/></state>
           <state id="b">
             <transition event="b.own"/>
             <state id="b0"><transition event="b0.only"/></state>
             <state id="b1"><transition event="back" target="a"/>
               <transition event="to.h" target="h"/></state>
             <history id="h"><transition target="d"/></history>
             <state id="d"><transition event="d.via.history"/></state>
           </state>
           <state id="never"><transition event="never.seen" target="p"/></state>
           <parallel id="p">
             <state id="p1"><transition event="p1.e"/></state>
             <state id="p2"><transition event="p2.e"/></state>
           </parallel>"#,
    );
    // b0 is b's first child, entered only by b's default, which no path
    // takes: b1 is the target, so b0.only is out.
    assert_eq!(
        vocabulary_of(&chart),
        ["to.b", "b.own", "back", "to.h", "d.via.history"]
    );
    let parallel = machine(
        r#" initial="a""#,
        r#"<state id="a"><transition event="go" target="p1"/></state>
           <parallel id="p">
             <state id="p1"><transition event="p1.e"/></state>
             <state id="p2"><transition event="p2.e"/></state>
           </parallel>"#,
    );
    // Entering p1 as a target enters its parallel ancestor's other region.
    assert_eq!(vocabulary_of(&parallel), ["go", "p1.e", "p2.e"]);
}

// The check refuses nothing: a declared name that is empty or all dots is a
// name like any other.
// Sabotage: make check_accepts skip an empty declared name; the last
// assert_eq goes red.
#[test]
fn an_odd_declared_name_is_compared_like_any_other() {
    let check = machine("", r#"<state id="a"><transition event="*"/></state>"#)
        .check_accepts(Some(&["", ".", "a..b"]));
    assert_eq!(check.unreachable, Vec::<String>::new());
    let check = machine("", TWO).check_accepts(Some(&["", "."]));
    assert_eq!(check.unreachable, ["", "."]);
}
