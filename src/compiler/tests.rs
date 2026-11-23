use crate::machine::{
    Chart, Content, DataValue, Deferred, Expr, Kind, Machine, ParamKind, State, Transition,
};
use crate::refusal::{CompileReason, Reason, Refusal};
use crate::testing::corpus_documents;
use crate::{LoweringReason, ReadReason, ValidationReason, compile};
use predicator_rs::Value;

const OPEN: &str = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0""#;

// `body` inside an `<scxml>` root carrying `attributes`.
fn source(attributes: &str, body: &str) -> String {
    format!("{OPEN}{attributes}>{body}</scxml>")
}

fn machine(attributes: &str, body: &str) -> Machine {
    let text = source(attributes, body);
    match compile(&text) {
        Ok(machine) => machine,
        Err(refusals) => panic!("{text}: {refusals:#?}"),
    }
}

fn refusals(body: &str) -> Vec<Refusal> {
    let text = source("", body);
    match compile(&text) {
        Ok(_) => panic!("{text} compiled"),
        Err(refusals) => refusals,
    }
}

fn state<'c>(chart: &'c Chart, id: &str) -> &'c State {
    let index = chart.id_to_index[id];
    &chart.states[index]
}

fn transition<'c>(chart: &'c Chart, id: &str, position: usize) -> &'c Transition {
    &chart.transitions[state(chart, id).transitions[position]]
}

fn is_compiled(expr: &Expr, expected: &str) -> bool {
    matches!(expr, Expr::Compiled { source, .. } if source == expected)
}

fn compile_refusal(refusal: &Refusal) -> (&str, &str) {
    match refusal.reason() {
        Reason::Compile(CompileReason::ExpressionCompileError { source, cause, .. }) => {
            (source.as_str(), cause.as_str())
        }
        other => panic!("not a compile refusal: {other:?}"),
    }
}

fn sources(refusals: &[Refusal]) -> Vec<&str> {
    refusals
        .iter()
        .map(|refusal| compile_refusal(refusal).0)
        .collect()
}

// ---- interning

// A state's index, kind, parent, last descendant and children.
type Shape<'c> = (usize, Kind, Option<usize>, usize, &'c [usize]);

// Sabotage: with a state's last descendant set one past its subtree, this test fails.
#[test]
fn interns_states_in_document_order_with_their_hierarchy() {
    let machine = machine(
        "",
        r#"<state id="a"><state id="a1"/><parallel id="p"><state id="p1"/><history id="h">
           <transition target="p1"/></history></parallel></state><final id="f"/>"#,
    );
    let chart = machine.chart();
    let ids: Vec<Option<&str>> = chart.states.iter().map(|s| s.id.as_deref()).collect();
    assert_eq!(
        ids,
        [
            None,
            Some("a"),
            Some("a1"),
            Some("p"),
            Some("p1"),
            Some("h"),
            Some("f")
        ]
    );
    let shape: Vec<Shape<'_>> = chart
        .states
        .iter()
        .map(|s| (s.index, s.kind, s.parent, s.last, s.children.as_slice()))
        .collect();
    assert_eq!(
        shape,
        [
            (0, Kind::Scxml, None, 6, &[1, 6][..]),
            (1, Kind::State, Some(0), 5, &[2, 3][..]),
            (2, Kind::State, Some(1), 2, &[][..]),
            (3, Kind::Parallel, Some(1), 5, &[4, 5][..]),
            (4, Kind::State, Some(3), 4, &[][..]),
            (5, Kind::History, Some(3), 5, &[][..]),
            (6, Kind::Final, Some(0), 6, &[][..]),
        ]
    );
    assert_eq!(state(chart, "p").history_children, [5]);
    assert!(state(chart, "a").history_children.is_empty());
    assert_eq!(chart.id_to_index.len(), 6);
}

// Sabotage: with the `initial` attribute ignored, so the first child is entered, this test fails.
#[test]
fn resolves_each_default_entry_in_the_specification_s_precedence() {
    let machine = machine(
        r#" initial="b""#,
        r#"<state id="a"/><state id="b" initial="b2"><state id="b1"/><state id="b2"/></state>
           <state id="c"><initial><transition target="c2"/></initial><state id="c1"/><state id="c2"/></state>
           <state id="d"><state id="d1"/><state id="d2"/></state>
           <parallel id="p"><state id="p1"/></parallel>"#,
    );
    let chart = machine.chart();
    let index = |id: &str| chart.id_to_index[id];
    assert_eq!(chart.states[0].initial, [index("b")]);
    assert_eq!(state(chart, "b").initial, [index("b2")]);
    assert_eq!(state(chart, "c").initial, [index("c2")]);
    assert_eq!(state(chart, "d").initial, [index("d1")]);
    assert!(state(chart, "a").initial.is_empty());
    assert!(state(chart, "p").initial.is_empty());

    let defaulted = machine_root_initial();
    assert_eq!(defaulted, [1]);
}

fn machine_root_initial() -> Vec<usize> {
    machine("", r#"<state id="x"/><state id="y"/>"#)
        .chart()
        .states[0]
        .initial
        .clone()
}

// Sabotage: with a history's transitions also kept as selectable ones, this test fails.
#[test]
fn numbers_transitions_own_then_initial_and_a_history_s_as_its_default() {
    let machine = machine(
        "",
        r#"<state id="a"><initial><transition target="a2"/></initial>
           <transition event="e.one" target="z"/><transition event="e.two" target="a"/>
           <state id="a1"/><state id="a2"/>
           <history id="h"><transition target="a1"/></history></state><state id="z"/>"#,
    );
    let chart = machine.chart();
    let a = state(chart, "a");
    assert_eq!(a.transitions, [0, 1]);
    assert_eq!(a.initial_transition, Some(2));
    assert_eq!(a.history_default, None);
    let h = state(chart, "h");
    assert!(h.transitions.is_empty());
    assert_eq!(h.history_default, Some(3));
    // A forward reference resolves: `z` comes after `a`.
    assert_eq!(chart.transitions[0].targets, [chart.id_to_index["z"]]);
    assert_eq!(chart.transitions[0].source, chart.id_to_index["a"]);
    assert_eq!(chart.transitions[3].source, chart.id_to_index["h"]);
    assert_eq!(chart.transitions.len(), 4);
}

// Sabotage: with event descriptors split on commas rather than dots, this test fails.
#[test]
fn splits_each_event_descriptor_on_its_dots_keeping_empty_parts() {
    let machine = machine(
        "",
        r#"<state id="a"><transition event="e.go a..b *" target="a"/></state>"#,
    );
    let events = &machine.chart().transitions[0].events;
    assert_eq!(
        events,
        &[
            vec!["e".to_owned(), "go".to_owned()],
            vec!["a".to_owned(), String::new(), "b".to_owned()],
            vec!["*".to_owned()],
        ]
    );
}

// Sabotage: with a state's onexit content numbered before its onentry content, this test fails.
#[test]
fn numbers_content_containers_before_their_children_in_document_order() {
    let machine = machine(
        "",
        r#"<state id="a"><onentry><raise event="r0"/><if cond="true"><raise event="r2"/>
           <else/><foreach array="[1]" item="i"><raise event="r4"/></foreach></if></onentry>
           <onexit><raise event="r5"/></onexit>
           <transition target="a"><raise event="r6"/></transition>
           <invoke type="scxml"><finalize><raise event="r7"/></finalize>
           <content><scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0"/></content></invoke>
           <state id="b"><onentry><raise event="r8"/></onentry></state></state>"#,
    );
    let chart = machine.chart();
    let kinds: Vec<String> = chart
        .contents
        .iter()
        .map(|content| match content {
            Content::Raise { event, .. } => event.clone(),
            Content::If { .. } => "if".to_owned(),
            Content::Foreach(_) => "foreach".to_owned(),
            other => format!("{other:?}"),
        })
        .collect();
    assert_eq!(
        kinds,
        ["r0", "if", "r2", "foreach", "r4", "r5", "r6", "r7", "r8"]
    );
    let a = state(chart, "a");
    assert_eq!(a.onentry[0].content, [0, 1]);
    assert_eq!(a.onexit[0].content, [5]);
    assert_eq!(chart.transitions[0].content, [6]);
    assert_eq!(
        a.invoke[0].finalize.as_ref().map(|b| b.content.clone()),
        Some(vec![7])
    );
    let Content::If { branches, .. } = &chart.contents[1] else {
        panic!("an if")
    };
    assert_eq!(branches.len(), 2);
    assert_eq!(branches[0].content, [2]);
    assert!(branches[1].cond.is_none());
    assert_eq!(branches[1].content, [3]);
    let Content::Foreach(foreach) = &chart.contents[3] else {
        panic!("a foreach")
    };
    assert_eq!(foreach.content, [4]);
    assert_eq!(foreach.item, "i");
}

// ---- expressions and values

// Sabotage: with a compiled condition dropped from its transition, this test fails.
#[test]
fn compiles_conditions_and_values_through_the_expression_compiler() {
    let text = source(
        "",
        r##"<state id="a"><onentry><log label="l" expr="x + 1"/><cancel sendidexpr="y"/>
           <send eventexpr="'e.' + x" target="#_internal"><param name="p" expr="x"/>
           <param name="q" location="y"/></send></onentry>
           <transition cond="x > 1" target="a"/></state>"##,
    );
    let machine = compile(&text).expect("compiles");
    let chart = machine.chart();
    let cond = transition(chart, "a", 0);
    assert!(cond.cond.as_ref().is_some_and(|c| is_compiled(c, "x > 1")));
    let at = text.find("x > 1").expect("the condition");
    assert_eq!(cond.cond_location.map(|l| l.start_offset), Some(at));
    let Content::Log { expr, label, .. } = &chart.contents[0] else {
        panic!("a log")
    };
    assert!(expr.as_ref().is_some_and(|e| is_compiled(e, "x + 1")));
    assert_eq!(label.as_deref(), Some("l"));
    let Content::Cancel { sendid, .. } = &chart.contents[1] else {
        panic!("a cancel")
    };
    assert!(sendid.as_ref().is_some_and(|e| is_compiled(e, "y")));
    let Content::Send(send) = &chart.contents[2] else {
        panic!("a send")
    };
    assert!(
        send.event
            .as_ref()
            .is_some_and(|e| is_compiled(e, "'e.' + x"))
    );
    assert_eq!(
        send.target,
        Some(Expr::Static(Value::String("#_internal".to_owned())))
    );
    let kinds: Vec<(&str, ParamKind)> = send
        .params
        .iter()
        .map(|p| (p.name.as_str(), p.kind))
        .collect();
    assert_eq!(kinds, [("p", ParamKind::Expr), ("q", ParamKind::Location)]);
}

// Sabotage: with an in-line value kept as its trimmed text even when it evaluates, this test fails.
#[test]
fn folds_in_line_values_and_keeps_data_in_document_order() {
    let machine = machine(
        "",
        r#"<datamodel><data id="list">[1, 2]</data><data id="word"> hello </data>
           <data id="blank">  </data><data id="big">99999999999999999999</data></datamodel>
           <state id="a"><datamodel><data id="e" expr="x +"/><data id="s" src="file:x"/>
           <data id="n" expr="1 + 2"/></datamodel></state>"#,
    );
    let chart = machine.chart();
    let ids: Vec<&str> = chart.data.iter().map(|d| d.id.as_str()).collect();
    assert_eq!(ids, ["list", "word", "blank", "big", "e", "s", "n"]);
    assert_eq!(chart.states[0].data, [0, 1, 2, 3]);
    assert_eq!(state(chart, "a").data, [4, 5, 6]);
    let value = |i: usize| &chart.data[i].value;
    assert_eq!(
        value(0),
        &DataValue::Expr(Expr::Static(Value::List(vec![
            Value::Integer(1),
            Value::Integer(2)
        ])))
    );
    assert_eq!(
        value(1),
        &DataValue::Expr(Expr::Static(Value::String("hello".to_owned())))
    );
    assert_eq!(value(2), &DataValue::Expr(Expr::Static(Value::Null)));
    // The predicator crate's integers are 64-bit: the literal does not
    // compile, so the text is kept as a string.
    assert_eq!(
        value(3),
        &DataValue::Expr(Expr::Static(Value::String(
            "99999999999999999999".to_owned()
        )))
    );
    assert!(matches!(value(4), DataValue::Invalid(_)));
    assert_eq!(value(5), &DataValue::Src("file:x".to_owned()));
    assert!(matches!(value(6), DataValue::Expr(e) if is_compiled(e, "1 + 2")));
}

// Sabotage: with an `<assign>` expression compiled to null rather than deferred, this test fails.
#[test]
fn defers_assign_values_namelist_entries_and_scripts() {
    let machine = machine(
        "",
        r#"<script>x = ;</script><script>y = 1</script>
           <state id="a"><onentry><assign location="v" expr="1 +"/>
           <assign location="w"><p>m</p></assign><assign location="u">3</assign>
           <script>if {</script><send event="e" namelist="a.b c["/></onentry></state>"#,
    );
    let chart = machine.chart();
    assert!(matches!(chart.global_scripts[0], Deferred::Invalid(_)));
    assert!(matches!(&chart.global_scripts[1], Deferred::Ready(p) if p.source == "y = 1"));
    let assign = |i: usize| match &chart.contents[i] {
        Content::Assign(assign) => assign,
        other => panic!("not an assign: {other:?}"),
    };
    assert!(matches!(assign(0).value, Deferred::Invalid(_)));
    assert_eq!(assign(0).location, "v");
    assert!(assign(0).expr_location.is_some());
    assert_eq!(
        assign(1).value,
        Deferred::Ready(Expr::Static(Value::String("<p>m</p>".to_owned())))
    );
    assert_eq!(
        assign(2).value,
        Deferred::Ready(Expr::Static(Value::Integer(3)))
    );
    assert!(assign(2).expr_location.is_none());
    assert!(matches!(
        &chart.contents[3],
        Content::Script {
            program: Deferred::Invalid(_),
            ..
        }
    ));
    let Content::Send(send) = &chart.contents[4] else {
        panic!("a send")
    };
    assert!(matches!(&send.namelist[0].expr, Deferred::Ready(e) if is_compiled(e, "a.b")));
    assert!(matches!(&send.namelist[1].expr, Deferred::Invalid(_)));
    assert_eq!(send.namelist[1].name, "c[");
    assert_eq!(
        send.event,
        Some(Expr::Static(Value::String("e".to_owned())))
    );
}

// Sabotage: with a `<content>` body's text dropped for null, this test fails.
#[test]
fn compiles_donedata_and_invoke_content_and_params() {
    let machine = machine(
        "",
        r#"<state id="a"><final id="f1"><donedata><content expr="x"/></donedata></final>
           <final id="f2"><donedata><param name="p" expr="1"/></donedata></final>
           <final id="f3"><donedata><content>text</content></donedata></final></state>
           <state id="b"><invoke typeexpr="t" src="s" id="i" autoforward="true" namelist="n"/>
           <invoke type="x"><param name="q" expr="2"/></invoke></state>"#,
    );
    let chart = machine.chart();
    let done = |id: &str| state(chart, id).donedata.as_ref().expect("donedata");
    assert!(
        done("f1")
            .expr
            .as_ref()
            .is_some_and(|e| is_compiled(e, "x"))
    );
    assert_eq!(done("f2").params[0].name, "p");
    assert_eq!(
        done("f3").expr,
        Some(Expr::Static(Value::String("text".to_owned())))
    );
    let invoke = &state(chart, "b").invoke[0];
    assert!(
        invoke
            .invoke_type
            .as_ref()
            .is_some_and(|e| is_compiled(e, "t"))
    );
    assert_eq!(
        invoke.src,
        Some(Expr::Static(Value::String("s".to_owned())))
    );
    assert_eq!(invoke.id.as_deref(), Some("i"));
    assert!(invoke.autoforward);
    assert_eq!(invoke.namelist[0].name, "n");
    assert_eq!(invoke.index, 0);
    let second = &state(chart, "b").invoke[1];
    assert_eq!(second.params[0].name, "q");
    assert_eq!(second.index, 1);
}

// ---- refusals

// Sabotage: with a refused condition reported at its element rather than its attribute, this test fails.
#[test]
fn refuses_a_condition_the_predicator_crate_refuses_at_its_attribute() {
    let text = source(
        "",
        r#"<state id="a"><transition cond="x >" target="a"/></state>"#,
    );
    let refusals = compile(&text).expect_err("refused");
    assert_eq!(refusals.len(), 1);
    let refusal = &refusals[0];
    let Reason::Compile(CompileReason::ExpressionCompileError {
        source,
        cause,
        position,
    }) = refusal.reason()
    else {
        panic!("{refusal:?}")
    };
    assert_eq!(source, "x >");
    assert!(!cause.is_empty());
    assert_eq!(position.map(|p| p.0), Some(1));
    assert!(
        refusal
            .message()
            .starts_with("failed to compile expression \"x >\": "),
        "{}",
        refusal.message()
    );
    assert!(refusal.message().contains("(predicator line 1, column"));
    let at = text.find("x >").expect("the condition");
    assert_eq!(refusal.span().start_offset, at);
    assert_eq!(refusal.span().end_offset, at + 3);
}

// Sabotage: with the refusals left in the order the groups were gathered, this test fails.
#[test]
fn collects_every_load_time_refusal_sorted_by_where_it_starts() {
    let refusals = refusals(
        r#"<state id="a"><onentry><log expr="l +"/><if cond="i1 +"><elseif cond="i2 +"/></if>
           <foreach array="f +" item="x"/><cancel sendidexpr="c +"/></onentry>
           <transition cond="t +" target="a"/>
           <final id="f"><donedata><param name="p" expr="d1 +"/><param name="q" location="d2 +"/></donedata></final>
           <invoke typeexpr="v1 +" srcexpr="v2 +"><param name="r" expr="v3 +"/></invoke>
           <invoke type="x"><content expr="v4 +"/></invoke>
           </state>"#,
    );
    assert_eq!(
        sources(&refusals),
        [
            "l +", "i1 +", "i2 +", "f +", "c +", "t +", "d1 +", "d2 +", "v1 +", "v2 +", "v3 +",
            "v4 +"
        ]
    );
    let offsets: Vec<usize> = refusals.iter().map(|r| r.span().start_offset).collect();
    let mut sorted = offsets.clone();
    sorted.sort_unstable();
    assert_eq!(offsets, sorted);
}

// Sabotage: with a `<send>` compiling on past its first refused attribute, this test fails.
#[test]
fn a_send_stops_at_its_first_refused_attribute() {
    let refusals = refusals(
        r#"<state id="a"><onentry><send eventexpr="a +" targetexpr="b +"/>
           <send event="e" delayexpr="d +"><content expr="c +"/></send>
           <send event="e"><param name="p" expr="p1 +"/><param name="q" expr="p2 +"/></send>
           <send event="e"><content expr="k +"/></send></onentry></state>"#,
    );
    assert_eq!(sources(&refusals), ["a +", "d +", "p1 +", "p2 +", "k +"]);
}

// Sabotage: with the content-expression span taken from the element rather than its `expr`, this test fails.
#[test]
fn reports_a_content_expression_at_its_expr_attribute() {
    let text = source(
        "",
        r#"<state id="a"><onentry><send event="e"><content expr="k +"/></send></onentry></state>"#,
    );
    let refusals = compile(&text).expect_err("refused");
    assert_eq!(
        refusals[0].span().start_offset,
        text.find("k +").expect("k")
    );
}

// Sabotage: with the reader's refusal dropped in favor of an empty list, this test fails.
#[test]
fn answers_the_refusals_of_the_first_stage_that_refused() {
    let read = compile("<scxml").expect_err("refused");
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].reason(), &Reason::Read(ReadReason::UnexpectedEnd));

    let lowered = compile(&source("", "<bogus/>")).expect_err("refused");
    assert!(matches!(
        lowered[0].reason(),
        Reason::Lowering(LoweringReason::UnsupportedElement { .. })
    ));

    // An unresolved target and a refused condition: only the validator's
    // refusal is answered, since the compiler never runs.
    let validated =
        refusals_or_panic(r#"<state id="a"><transition cond="x >" target="z"/></state>"#);
    assert_eq!(validated.len(), 1);
    assert!(matches!(
        validated[0].reason(),
        Reason::Validation(ValidationReason::UnresolvedTarget { id }) if id == "z"
    ));
    assert!(!validated[0].message().is_empty());
}

fn refusals_or_panic(body: &str) -> Vec<Refusal> {
    compile(&source("", body)).expect_err("refused")
}

// Sabotage: with the validator's warnings dropped from the machine, this test fails.
#[test]
fn carries_the_validator_s_warnings_on_the_machine() {
    let text = source(
        "",
        r#"<state id="a"><invoke type="scxml"><finalize><raise event="e"/><send event="f"/></finalize>
           <content><scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0"/></content></invoke></state>"#,
    );
    let machine = compile(&text).expect("compiles");
    let found: Vec<(&str, usize)> = machine
        .warnings()
        .iter()
        .map(|w| (w.message(), w.span().start_offset))
        .collect();
    assert_eq!(
        found,
        [
            (
                "<raise> must not occur inside <finalize>",
                text.find("<raise").expect("raise")
            ),
            (
                "<send> must not occur inside <finalize>",
                text.find("<send").expect("send")
            ),
        ]
    );
    assert!(matches!(
        machine.warnings()[0].reason(),
        crate::WarningReason::FinalizeForbiddenContent { element: "raise" }
    ));
}

// ---- the machine as a value

fn assert_send_sync<T: Send + Sync + Clone + 'static>() {}

// Sabotage: with the machine's chart held in an `Rc`, this test no longer compiles.
#[test]
fn a_machine_is_a_shared_immutable_value() {
    assert_send_sync::<Machine>();
    assert_send_sync::<Refusal>();
    let machine = machine("", r#"<state id="a"/>"#);
    let clone = machine.clone();
    assert!(std::ptr::eq(machine.chart(), clone.chart()));
    let joined = std::thread::spawn(move || clone.chart().states.len())
        .join()
        .expect("the thread");
    assert_eq!(joined, 2);
}

// ---- the corpus and the no-panic promise

// Sabotage: with every expression refused by the compiler, this test fails.
#[test]
fn every_corpus_document_compiles() {
    let documents = corpus_documents();
    let refused: Vec<String> = documents
        .iter()
        .filter_map(|(name, bytes)| {
            let text = std::str::from_utf8(bytes).expect("UTF-8");
            compile(text)
                .err()
                .map(|refusals| format!("{name}: {refusals:#?}"))
        })
        .collect();
    println!("corpus documents compiled: {}", documents.len());
    assert!(documents.len() > 300, "{}", documents.len());
    assert!(refused.is_empty(), "{refused:#?}");
}

// Every prefix of `source` that is UTF-8 compiles to a value: a machine or
// refusals, never a panic. Answers how many prefixes were compiled.
fn assert_every_prefix_answers(source: &str) -> usize {
    let mut compiled = 0;
    for cut in 0..=source.len() {
        if let Some(prefix) = source.get(..cut) {
            if let Err(refusals) = compile(prefix) {
                assert!(!refusals.is_empty(), "{prefix:?}");
            }
            compiled += 1;
        }
    }
    compiled
}

// Sabotage: with compile panicking on a source that ends with `<s`, a cut document panics and this test fails.
#[test]
fn answers_every_prefix_of_every_corpus_document_without_panicking() {
    let documents = corpus_documents();
    let threads = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let share = documents.len().div_ceil(threads);
    let prefixes: usize = std::thread::scope(|scope| {
        let handles: Vec<_> = documents
            .chunks(share)
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .map(|(_, bytes)| {
                            assert_every_prefix_answers(std::str::from_utf8(bytes).expect("UTF-8"))
                        })
                        .sum::<usize>()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("every prefix answers"))
            .sum()
    });
    println!(
        "corpus prefixes compiled: {prefixes} over {} documents",
        documents.len()
    );
    assert!(documents.len() > 300, "{}", documents.len());
    let expected: usize = documents
        .iter()
        .map(|(_, bytes)| {
            (0..=bytes.len())
                .filter(|&cut| {
                    std::str::from_utf8(bytes).is_ok_and(|text| text.is_char_boundary(cut))
                })
                .count()
        })
        .sum();
    assert_eq!(prefixes, expected);
}
