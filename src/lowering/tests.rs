use super::error::Reason;
use super::{LoweringError, MAX_DEPTH, lower};
use crate::document::{
    Binding, Document, Executable, HistoryType, State, StateKind, TransitionType,
};
use crate::testing::corpus_documents;
use crate::xml::{Element, Location, Node, parse};

const SCXML: &str = "http://www.w3.org/2005/07/scxml";

fn read(source: &str) -> Result<Document, Vec<LoweringError>> {
    match parse(source.as_bytes()) {
        Ok(root) => lower(&root, source),
        Err(error) => panic!("expected {source:?} to parse, got {}", error.message),
    }
}

// The document `source` lowers to, failing the test when it is refused.
fn document(source: &str) -> Document {
    match read(source) {
        Ok(document) => document,
        Err(errors) => panic!("expected {source:?} to lower, got {errors:#?}"),
    }
}

// The errors `source` is refused with, failing the test when it lowers.
fn errors(source: &str) -> Vec<LoweringError> {
    match read(source) {
        Ok(_) => panic!("expected {source:?} to be refused"),
        Err(errors) => errors,
    }
}

// Each error's reason and the source text its span covers.
fn refusals(source: &str) -> Vec<(Reason, String)> {
    errors(source)
        .into_iter()
        .map(|error| (error.reason, slice(source, error.location)))
        .collect()
}

fn slice(source: &str, location: Location) -> String {
    location
        .slice(source)
        .expect("a span inside the source")
        .into()
}

// A document of `body` inside one state `s`, answering that state.
fn in_state(body: &str) -> (String, State) {
    let source = format!("<scxml xmlns=\"{SCXML}\"><state id=\"s\">{body}</state></scxml>");
    let state = document(&source).states.remove(0);
    (source, state)
}

// The executable content of one `<onentry>` holding `body`.
fn onentry(body: &str) -> (String, Vec<Executable>) {
    let (source, mut state) = in_state(&format!("<onentry>{body}</onentry>"));
    (source, state.onentry.remove(0).content)
}

fn misplaced(name: &str, parent: &str) -> Reason {
    Reason::MisplacedElement {
        name: name.into(),
        parent: parent.into(),
    }
}

fn missing(element: &'static str, attribute: &'static str) -> Reason {
    Reason::MissingAttribute { element, attribute }
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

// ---- the root

// Sabotage: with the root's name read from its `version` attribute, this test fails.
#[test]
fn reads_the_roots_attributes_and_their_value_spans() {
    let source = format!(
        "<scxml xmlns=\"{SCXML}\" version=\"1.0\" name=\"n\" datamodel=\"predicator\" \
         binding=\"late\" initial=\" a\tb \"/>"
    );
    let document = document(&source);
    assert_eq!(document.name.as_deref(), Some("n"));
    assert_eq!(document.version.as_deref(), Some("1.0"));
    assert_eq!(document.xmlns.as_deref(), Some(SCXML));
    assert_eq!(document.namespace.as_deref(), Some(SCXML));
    assert_eq!(document.datamodel.as_deref(), Some("predicator"));
    assert_eq!(document.binding, Binding::Late);
    assert_eq!(document.initial, strings(&["a", "b"]));
    assert_eq!(slice(&source, document.location), source);
    let spans: Vec<(&str, String)> = document
        .attribute_locations
        .iter()
        .map(|(name, location)| (*name, slice(&source, *location)))
        .collect();
    assert_eq!(
        spans,
        vec![
            ("binding", "late".to_owned()),
            ("datamodel", "predicator".to_owned()),
            ("initial", " a\tb ".to_owned()),
            ("name", "n".to_owned()),
            ("version", "1.0".to_owned()),
            ("xmlns", SCXML.to_owned()),
        ]
    );
}

// Sabotage: with every written attribute recorded whatever its name, this test fails.
#[test]
fn defaults_what_was_not_written_and_records_only_what_was() {
    let document = document("<scxml binding=\"sideways\" other=\"x\"/>");
    assert_eq!(document.binding, Binding::Early);
    assert_eq!(document.initial, Vec::<String>::new());
    assert_eq!(
        (document.name, document.xmlns, document.namespace),
        (None, None, None)
    );
    assert_eq!(
        document
            .attribute_locations
            .keys()
            .copied()
            .collect::<Vec<_>>(),
        vec!["binding"]
    );
}

// Sabotage: with a root `<script>` refused as misplaced, this test fails.
#[test]
fn places_states_the_last_datamodel_and_scripts_in_document_order() {
    let source = format!(
        "<scxml xmlns=\"{SCXML}\"><state id=\"a\"/><datamodel/><parallel id=\"b\"/>\
         <script>x</script><final id=\"c\"/><history id=\"h\"/>\
         <datamodel><data id=\"d\"/></datamodel><script>y</script></scxml>"
    );
    let document = document(&source);
    let ids: Vec<_> = document
        .states
        .iter()
        .map(|state| state.id.clone())
        .collect();
    assert_eq!(
        ids,
        vec![
            Some("a".into()),
            Some("b".into()),
            Some("c".into()),
            Some("h".into())
        ]
    );
    let scripts: Vec<_> = document
        .scripts
        .iter()
        .map(|script| script.text.as_str())
        .collect();
    assert_eq!(scripts, vec!["x", "y"]);
    let datamodel = document.datamodel_element.expect("a datamodel");
    assert_eq!(datamodel.data.len(), 1);
}

// Sabotage: with the vocabulary test reading only the SCXML namespace, a root with none is refused and this test fails.
#[test]
fn lowers_a_prefixed_root_and_a_root_with_no_namespace_as_scxml() {
    let prefixed = document(&format!(
        "<s:scxml xmlns:s=\"{SCXML}\"><s:state id=\"a\"/></s:scxml>"
    ));
    assert_eq!(prefixed.xmlns, None);
    assert_eq!(prefixed.namespace.as_deref(), Some(SCXML));
    assert_eq!(prefixed.states.len(), 1);
    let bare = document("<scxml><state id=\"a\"/></scxml>");
    assert_eq!(bare.namespace, None);
    assert_eq!(bare.states.len(), 1);
}

// ---- the state family

// Sabotage: with `<parallel>` built as a `<state>`, this test fails.
#[test]
fn lowers_each_kind_with_its_id_initial_and_children() {
    let (source, state) = in_state(
        "<transition event=\"e\"/><initial><transition target=\"b\"/></initial>\
         <state id=\"a\"><final id=\"f\"/></state><parallel id=\"p\"/>\
         <datamodel/><donedata/><invoke/><transition event=\"g\"/>",
    );
    assert_eq!(state.kind, StateKind::State);
    assert_eq!(state.id.as_deref(), Some("s"));
    assert_eq!(slice(&source, state.attribute_locations["id"]), "s");
    let kinds: Vec<_> = state.states.iter().map(|child| child.kind).collect();
    assert_eq!(kinds, vec![StateKind::State, StateKind::Parallel]);
    assert_eq!(state.states[0].states[0].kind, StateKind::Final);
    let events: Vec<_> = state.transitions.iter().map(|t| t.event.clone()).collect();
    assert_eq!(events, vec![strings(&["e"]), strings(&["g"])]);
    let initial = state.initial_element.expect("an initial");
    assert_eq!(initial.transitions[0].target, strings(&["b"]));
    assert!(state.datamodel_element.is_some());
    assert!(state.donedata.is_some());
    assert_eq!(state.invoke.len(), 1);
    assert_eq!(state.history_type, None);
    let (_, compound) = in_state("<state id=\"a\" initial=\"x y\"/>");
    assert_eq!(compound.states[0].initial, strings(&["x", "y"]));
}

// Sabotage: with an unknown history type read as deep, this test fails.
#[test]
fn reads_a_historys_type_shallow_by_default_and_for_an_unknown_value() {
    let (source, state) = in_state(
        "<history id=\"a\"/><history id=\"b\" type=\"deep\"/><history id=\"c\" type=\"sideways\"/>",
    );
    let types: Vec<_> = state.states.iter().map(|h| h.history_type).collect();
    assert_eq!(
        types,
        vec![
            Some(HistoryType::Shallow),
            Some(HistoryType::Deep),
            Some(HistoryType::Shallow)
        ]
    );
    assert_eq!(state.states[0].kind, StateKind::History);
    assert!(!state.states[0].attribute_locations.contains_key("type"));
    assert_eq!(
        slice(&source, state.states[2].attribute_locations["type"]),
        "sideways"
    );
}

// Sabotage: with every `<onentry>` folded into the first block, this test fails.
#[test]
fn keeps_one_block_per_onentry_and_onexit_element() {
    let (source, state) = in_state(
        "<onentry><raise event=\"a\"/></onentry><onexit/><onentry><raise event=\"b\"/>\
         <raise event=\"c\"/></onentry>",
    );
    let blocks: Vec<Vec<String>> = state
        .onentry
        .iter()
        .map(|block| {
            block
                .content
                .iter()
                .map(|node| match node {
                    Executable::Raise(raise) => raise.event.clone(),
                    other => panic!("expected a raise, got {other:?}"),
                })
                .collect()
        })
        .collect();
    assert_eq!(blocks, vec![strings(&["a"]), strings(&["b", "c"])]);
    assert_eq!(state.onexit.len(), 1);
    assert_eq!(slice(&source, state.onexit[0].location), "<onexit/>");
}

// ---- transitions

// Sabotage: with `cond` split on whitespace and joined, this test fails.
#[test]
fn reads_event_and_target_as_lists_cond_raw_and_type() {
    let (source, state) = in_state(
        "<transition event=\"a.b  c\" target=\"x\ny\" cond=\" n &gt; 1 \" type=\"internal\">\
         <log expr=\"1\"/><raise event=\"r\"/></transition>",
    );
    let transition = &state.transitions[0];
    assert_eq!(transition.event, strings(&["a.b", "c"]));
    assert_eq!(transition.target, strings(&["x", "y"]));
    assert_eq!(transition.cond.as_deref(), Some(" n > 1 "));
    assert_eq!(transition.transition_type, TransitionType::Internal);
    assert_eq!(
        slice(&source, transition.attribute_locations["cond"]),
        " n &gt; 1 "
    );
    let names: Vec<_> = transition
        .content
        .iter()
        .map(Executable::element_name)
        .collect();
    assert_eq!(names, vec!["log", "raise"]);
}

// Sabotage: with an unknown transition type read as internal, this test fails.
#[test]
fn defaults_a_transitions_type_to_external_and_records_no_location_for_it() {
    let (_, state) = in_state("<transition/><transition type=\"sideways\"/>");
    let [plain, unknown] = &state.transitions[..] else {
        panic!("expected two transitions");
    };
    assert_eq!(plain.transition_type, TransitionType::External);
    assert!(plain.attribute_locations.is_empty());
    assert_eq!(
        (plain.event.len(), plain.target.len(), &plain.cond),
        (0, 0, &None)
    );
    assert_eq!(unknown.transition_type, TransitionType::External);
    assert!(unknown.attribute_locations.contains_key("type"));
}

// ---- the datamodel

// Sabotage: with a `<data>` text trimmed, this test fails.
#[test]
fn reads_a_datas_id_expr_src_and_untrimmed_in_line_value() {
    let source = format!(
        "<scxml xmlns=\"{SCXML}\"><datamodel><data id=\"a\" expr=\"1 + 1\" src=\"f\"/>\
         <data id=\"b\"> [1,\n 2] </data></datamodel></scxml>"
    );
    let data = document(&source)
        .datamodel_element
        .expect("a datamodel")
        .data;
    assert_eq!(data[0].id, "a");
    assert_eq!(data[0].expr.as_deref(), Some("1 + 1"));
    assert_eq!(data[0].src.as_deref(), Some("f"));
    assert_eq!(data[0].text, "");
    assert_eq!(data[1].text, " [1,\n 2] ");
    assert_eq!(data[1].expr, None);
    assert_eq!(
        data[0]
            .attribute_locations
            .keys()
            .copied()
            .collect::<Vec<_>>(),
        vec!["expr", "id", "src"]
    );
}

// Sabotage: with a `<data>`'s element children left unread, this test fails.
#[test]
fn refuses_a_data_with_no_id_and_any_element_inside_a_data() {
    let source = format!(
        "<scxml xmlns=\"{SCXML}\"><datamodel><data expr=\"1\"/>\
         <data id=\"b\"><x:v xmlns:x=\"urn:x\"/></data></datamodel></scxml>"
    );
    assert_eq!(
        refusals(&source),
        vec![
            (missing("data", "id"), "<data expr=\"1\"/>".into()),
            (misplaced("x:v", "data"), "<x:v xmlns:x=\"urn:x\"/>".into()),
        ]
    );
}

// ---- executable content

// Sabotage: with `<raise>`'s event split on whitespace, this test fails.
#[test]
fn reads_a_raises_event_as_one_name_never_split() {
    let (_, content) = onentry("<raise event=\"a b\"/>");
    let Executable::Raise(raise) = &content[0] else {
        panic!("expected a raise");
    };
    assert_eq!(raise.event, "a b");
}

// Sabotage: with the children of a `<raise>` with no event placed, a misplaced error is added and this test fails.
#[test]
fn refuses_a_raise_with_no_event_without_placing_its_children() {
    let source = format!(
        "<scxml xmlns=\"{SCXML}\"><state><onentry><raise><state/></raise></onentry></state></scxml>"
    );
    assert_eq!(
        refusals(&source),
        vec![(missing("raise", "event"), "<raise><state/></raise>".into())]
    );
}

// Sabotage: with an `<assign>`'s span taken from its first child, this test fails.
#[test]
fn keeps_an_assigns_location_attribute_apart_from_its_span() {
    let (source, content) =
        onentry("<assign location=\"x\" expr=\"1\"/><assign location=\"y\"> 2 </assign>");
    let (Executable::Assign(first), Executable::Assign(second)) = (&content[0], &content[1]) else {
        panic!("expected two assigns");
    };
    assert_eq!(first.location, "x");
    assert_eq!(first.expr.as_deref(), Some("1"));
    assert_eq!(
        slice(&source, first.node_location),
        "<assign location=\"x\" expr=\"1\"/>"
    );
    assert_eq!((second.text.as_str(), &second.markup), (" 2 ", &None));
    assert_eq!(second.markup_location, None);
}

// Sabotage: with the markup slice ending at the first child, this test fails.
#[test]
fn keeps_an_assigns_element_children_as_an_unfolded_source_slice() {
    let (source, content) =
        onentry("<assign location=\"x\">\n  t <a\r\n/><f:b xmlns:f=\"urn:f\"/>  \n</assign>");
    let Executable::Assign(assign) = &content[0] else {
        panic!("expected an assign");
    };
    assert_eq!(
        assign.markup.as_deref(),
        Some("\n  t <a\r\n/><f:b xmlns:f=\"urn:f\"/>")
    );
    assert_eq!(
        slice(&source, assign.markup_location.expect("a markup span")),
        "\n  t <a\r\n/><f:b xmlns:f=\"urn:f\"/>"
    );
}

// Sabotage: with a misplaced `<assign>` given the location of its parent, this test fails.
#[test]
fn refuses_an_assign_with_no_location_and_reports_a_misplaced_one_at_its_span() {
    let source = format!(
        "<scxml xmlns=\"{SCXML}\"><state><onentry><assign expr=\"1\"><bogus/></assign></onentry>\
         </state><assign location=\"x\"/></scxml>"
    );
    assert_eq!(
        refusals(&source),
        vec![
            (
                missing("assign", "location"),
                "<assign expr=\"1\"><bogus/></assign>".into()
            ),
            (
                misplaced("assign", "scxml"),
                "<assign location=\"x\"/>".into()
            ),
        ]
    );
}

// Sabotage: with `<log>`'s label read from `expr`, this test fails.
#[test]
fn reads_a_logs_label_and_expr_raw() {
    let (_, content) = onentry("<log label=\"l\" expr=\" a + b \"/><log/>");
    let (Executable::Log(first), Executable::Log(second)) = (&content[0], &content[1]) else {
        panic!("expected two logs");
    };
    assert_eq!(first.label.as_deref(), Some("l"));
    assert_eq!(first.expr.as_deref(), Some(" a + b "));
    assert_eq!((&second.label, &second.expr), (&None, &None));
}

// Sabotage: with every `<elseif>` content placed in the first branch, this test fails.
#[test]
fn partitions_an_ifs_content_into_one_branch_per_if_elseif_and_else() {
    let (source, content) = onentry(
        "<if cond=\"a\"><raise event=\"1\"/><raise event=\"2\"/><elseif cond=\"b\"/>\
         <raise event=\"3\"/><else/><raise event=\"4\"/></if>",
    );
    let Executable::If(node) = &content[0] else {
        panic!("expected an if");
    };
    let branches: Vec<(Option<&str>, usize)> = node
        .branches
        .iter()
        .map(|branch| (branch.cond.as_deref(), branch.content.len()))
        .collect();
    assert_eq!(branches, vec![(Some("a"), 2), (Some("b"), 1), (None, 1)]);
    assert_eq!(node.branches[0].location, node.location);
    assert_eq!(
        slice(&source, node.branches[1].location),
        "<elseif cond=\"b\"/>"
    );
    assert_eq!(
        slice(&source, node.branches[1].attribute_locations["cond"]),
        "b"
    );
}

// Sabotage: with every attribute on `<else>` refused rather than the first, this test fails.
#[test]
fn refuses_the_first_attribute_on_an_else_and_a_missing_cond() {
    let source = format!(
        "<scxml xmlns=\"{SCXML}\"><state><onentry><if cond=\"a\"><elseif>t</elseif>\
         <else cond=\"x\" y=\"z\"><raise event=\"e\"/></else></if><if/></onentry></state></scxml>"
    );
    assert_eq!(
        refusals(&source),
        vec![
            (missing("elseif", "cond"), "<elseif>t</elseif>".into()),
            (
                Reason::UnexpectedAttribute {
                    element: "else",
                    attribute: "cond".into()
                },
                "cond=\"x\"".into()
            ),
            (misplaced("raise", "else"), "<raise event=\"e\"/>".into()),
            (missing("if", "cond"), "<if/>".into()),
        ]
    );
}

// Sabotage: with an `<elseif>` accepted inside a block, this test fails.
#[test]
fn refuses_an_elseif_or_else_outside_an_if() {
    let source = format!(
        "<scxml xmlns=\"{SCXML}\"><state><onentry><elseif cond=\"a\"/><else/></onentry></state></scxml>"
    );
    assert_eq!(
        refusals(&source),
        vec![
            (
                misplaced("elseif", "onentry"),
                "<elseif cond=\"a\"/>".into()
            ),
            (misplaced("else", "onentry"), "<else/>".into()),
        ]
    );
}

// Sabotage: with `<foreach>`'s index left unread, this test fails.
#[test]
fn reads_a_foreachs_array_item_index_and_body() {
    let (_, content) = onentry(
        "<foreach array=\"xs\" item=\"x\" index=\"i\"><log expr=\"x\"/><if cond=\"x\"/></foreach>",
    );
    let Executable::Foreach(node) = &content[0] else {
        panic!("expected a foreach");
    };
    assert_eq!((node.array.as_str(), node.item.as_str()), ("xs", "x"));
    assert_eq!(node.index.as_deref(), Some("i"));
    let names: Vec<_> = node.content.iter().map(Executable::element_name).collect();
    assert_eq!(names, vec!["log", "if"]);
}

// Two errors at one offset keep the order the reference's accumulation gives
// them, which turns over with each level of nesting.
// Sabotage: with walk_children no longer reversing what it found, both orders turn over and this test fails.
#[test]
fn reports_both_missing_foreach_attributes_in_the_references_order() {
    let odd =
        format!("<scxml xmlns=\"{SCXML}\"><state><onentry><foreach/></onentry></state></scxml>");
    let reasons: Vec<Reason> = errors(&odd).into_iter().map(|e| e.reason).collect();
    assert_eq!(
        reasons,
        vec![missing("foreach", "array"), missing("foreach", "item")]
    );
    let even = format!(
        "<scxml xmlns=\"{SCXML}\"><state><onentry><if cond=\"c\"><foreach/></if></onentry>\
         </state></scxml>"
    );
    let reasons: Vec<Reason> = errors(&even).into_iter().map(|e| e.reason).collect();
    assert_eq!(
        reasons,
        vec![missing("foreach", "item"), missing("foreach", "array")]
    );
    let one = format!(
        "<scxml xmlns=\"{SCXML}\"><state><onentry><foreach item=\"x\"/></onentry></state></scxml>"
    );
    let reasons: Vec<Reason> = errors(&one).into_iter().map(|e| e.reason).collect();
    assert_eq!(reasons, vec![missing("foreach", "array")]);
}

// Sabotage: with a `<script>`'s text trimmed, this test fails.
#[test]
fn keeps_a_scripts_program_text_untrimmed() {
    let (_, content) = onentry("<script>\n  x = 1\n</script>");
    let Executable::Script(script) = &content[0] else {
        panic!("expected a script");
    };
    assert_eq!(script.text, "\n  x = 1\n");
}

// Sabotage: with a written `src` read as absent, the script lowers and this test fails.
#[test]
fn refuses_a_script_src_and_an_element_inside_a_script_and_a_script_in_a_state() {
    let source = format!(
        "<scxml xmlns=\"{SCXML}\"><state><onentry><script src=\"\"/><script><a/></script>\
         </onentry><script/></state></scxml>"
    );
    assert_eq!(
        refusals(&source),
        vec![
            (
                Reason::UnsupportedAttribute {
                    element: "script",
                    attribute: "src"
                },
                "<script src=\"\"/>".into()
            ),
            (misplaced("a", "script"), "<a/>".into()),
            (misplaced("script", "state"), "<script/>".into()),
        ]
    );
}

// Sabotage: with `<send>`'s `delay` read from `delayexpr`, this test fails.
#[test]
fn reads_every_send_attribute_raw_and_its_params_and_content() {
    let (_, content) = onentry(
        "<send event=\"e\" eventexpr=\"ee\" target=\"t\" targetexpr=\"te\" type=\"ty\" \
         typeexpr=\"tye\" id=\"i\" idlocation=\"il\" delay=\"1s\" delayexpr=\"de\" \
         namelist=\"a  b\"><param name=\"p\" expr=\"1\"/><param name=\"q\" location=\"l\"/>\
         <content>c</content></send>",
    );
    let Executable::Send(send) = &content[0] else {
        panic!("expected a send");
    };
    let read = [
        &send.event,
        &send.eventexpr,
        &send.target,
        &send.targetexpr,
        &send.send_type,
        &send.typeexpr,
        &send.id,
        &send.idlocation,
        &send.delay,
        &send.delayexpr,
    ]
    .map(|value| value.as_deref());
    assert_eq!(
        read,
        [
            Some("e"),
            Some("ee"),
            Some("t"),
            Some("te"),
            Some("ty"),
            Some("tye"),
            Some("i"),
            Some("il"),
            Some("1s"),
            Some("de")
        ]
    );
    assert_eq!(send.namelist, strings(&["a", "b"]));
    assert_eq!(send.attribute_locations.len(), 11);
    let params: Vec<_> = send
        .params
        .iter()
        .map(|param| {
            (
                param.name.as_str(),
                param.expr.as_deref(),
                param.param_location.as_deref(),
            )
        })
        .collect();
    assert_eq!(params, vec![("p", Some("1"), None), ("q", None, Some("l"))]);
    assert_eq!(
        send.content.as_ref().map(|content| content.text.as_str()),
        Some("c")
    );
}

// Sabotage: with a `<param>` built without its name, this test fails.
#[test]
fn refuses_a_param_with_no_name() {
    let source = format!(
        "<scxml xmlns=\"{SCXML}\"><state><onentry><send><param expr=\"1\"/></send></onentry>\
         </state></scxml>"
    );
    assert_eq!(
        refusals(&source),
        vec![(missing("param", "name"), "<param expr=\"1\"/>".into())]
    );
}

// Sabotage: with `<cancel>`'s sendidexpr read from `sendid`, this test fails.
#[test]
fn reads_a_cancels_sendid_and_sendidexpr_and_refuses_a_child() {
    let (_, content) = onentry("<cancel sendid=\"a\" sendidexpr=\"b\"/>");
    let Executable::Cancel(cancel) = &content[0] else {
        panic!("expected a cancel");
    };
    assert_eq!(cancel.sendid.as_deref(), Some("a"));
    assert_eq!(cancel.sendidexpr.as_deref(), Some("b"));
    let source = format!(
        "<scxml xmlns=\"{SCXML}\"><state><onentry><cancel sendid=\"a\"><log/></cancel>\
         </onentry></state></scxml>"
    );
    assert_eq!(
        refusals(&source),
        vec![(misplaced("log", "cancel"), "<log/>".into())]
    );
}

// ---- invocations and done data

// Sabotage: with `autoforward` read as true for any written value, this test fails.
#[test]
fn lowers_an_invocation_with_its_attributes_params_content_and_finalize() {
    let (_, state) = in_state(
        "<invoke type=\"t\" typeexpr=\"te\" src=\"s\" srcexpr=\"se\" id=\"i\" idlocation=\"il\" \
         namelist=\"a b\" autoforward=\"true\"><param name=\"p\"/><content>x</content>\
         <finalize><log/></finalize></invoke><invoke autoforward=\"yes\"/><invoke><finalize/></invoke>",
    );
    let [full, other, empty] = &state.invoke[..] else {
        panic!("expected three invocations");
    };
    let read = [
        &full.invoke_type,
        &full.typeexpr,
        &full.src,
        &full.srcexpr,
        &full.id,
        &full.idlocation,
    ]
    .map(|value| value.as_deref());
    assert_eq!(
        read,
        [
            Some("t"),
            Some("te"),
            Some("s"),
            Some("se"),
            Some("i"),
            Some("il")
        ]
    );
    assert_eq!(full.namelist, strings(&["a", "b"]));
    assert!(full.autoforward);
    assert_eq!(full.attribute_locations.len(), 8);
    assert_eq!(full.params[0].name, "p");
    assert_eq!(full.content.as_ref().map(|c| c.text.as_str()), Some("x"));
    assert_eq!(full.finalize.as_ref().map(|f| f.content.len()), Some(1));
    assert!(!other.autoforward);
    assert!(other.attribute_locations.contains_key("autoforward"));
    assert_eq!(other.finalize, None);
    assert_eq!(empty.finalize.as_ref().map(|f| f.content.len()), Some(0));
}

// Sabotage: with `<donedata>`'s params dropped, this test fails.
#[test]
fn lowers_donedata_with_its_params_and_content() {
    let source = format!(
        "<scxml xmlns=\"{SCXML}\"><final id=\"f\"><donedata><param name=\"a\" expr=\"1\"/>\
         <param name=\"b\" expr=\"2\"/><content expr=\"3\"/></donedata></final></scxml>"
    );
    let donedata = document(&source)
        .states
        .remove(0)
        .donedata
        .expect("donedata");
    let names: Vec<_> = donedata.params.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, vec!["a", "b"]);
    let content = donedata.content.expect("content");
    assert_eq!(content.expr.as_deref(), Some("3"));
    assert_eq!(slice(&source, content.attribute_locations["expr"]), "3");
}

// ---- content

// Sabotage: with `<content>`'s element children dispatched, the foreign one is refused and this test fails.
#[test]
fn keeps_contents_markup_children_as_opaque_source_in_any_namespace() {
    let (source, state) = in_state(
        "<invoke><content>  <scxml><state id=\"x\"/></scxml> <f:a xmlns:f=\"urn:f\"/>\n</content></invoke>",
    );
    let content = state.invoke[0].content.as_ref().expect("content");
    let markup = "<scxml><state id=\"x\"/></scxml> <f:a xmlns:f=\"urn:f\"/>";
    assert_eq!(content.markup.as_deref(), Some(markup));
    assert_eq!(
        slice(&source, content.markup_location.expect("a span")),
        markup
    );
    assert_eq!(content.text, "   \n");
}

// Sabotage: with `<content>`'s text trimmed, this test fails.
#[test]
fn keeps_text_only_content_untrimmed_with_no_markup() {
    let (_, state) = in_state("<invoke><content> a &lt; b </content></invoke>");
    let content = state.invoke[0].content.as_ref().expect("content");
    assert_eq!(content.text, " a < b ");
    assert_eq!((&content.markup, &content.markup_location), (&None, &None));
}

// ---- refusals

// Sabotage: with an unknown name answered as nothing, this test fails.
#[test]
fn reports_an_unknown_element_by_its_name_as_written() {
    let source = format!(
        "<scxml xmlns=\"{SCXML}\" xmlns:s=\"{SCXML}\"><state><bogus/></state><s:other/>\
         <scxml/></scxml>"
    );
    assert_eq!(
        refusals(&source),
        vec![
            (
                Reason::UnsupportedElement {
                    name: "bogus".into()
                },
                "<bogus/>".into()
            ),
            (
                Reason::UnsupportedElement {
                    name: "s:other".into()
                },
                "<s:other/>".into()
            ),
            (
                Reason::UnsupportedElement {
                    name: "scxml".into()
                },
                "<scxml/>".into()
            ),
        ]
    );
    assert_eq!(
        refusals("<scxml><bogus/></scxml>"),
        vec![(
            Reason::UnsupportedElement {
                name: "bogus".into()
            },
            "<bogus/>".into()
        )]
    );
}

// Sabotage: with an empty namespace read as SCXML's, this test fails.
#[test]
fn reports_an_element_in_a_foreign_or_empty_namespace() {
    let source =
        format!("<scxml xmlns=\"{SCXML}\"><f:state xmlns:f=\"urn:f\"/><state xmlns=\"\"/></scxml>");
    assert_eq!(
        refusals(&source),
        vec![
            (
                Reason::ForeignElement {
                    name: "f:state".into(),
                    uri: "urn:f".into()
                },
                "<f:state xmlns:f=\"urn:f\"/>".into()
            ),
            (
                Reason::ForeignElement {
                    name: "state".into(),
                    uri: String::new()
                },
                "<state xmlns=\"\"/>".into()
            ),
        ]
    );
}

// Sabotage: with the root's local name left unchecked, a `<state>` root lowers and this test fails.
#[test]
fn refuses_a_root_that_is_not_scxml_and_a_foreign_root() {
    assert_eq!(
        refusals("<p:state xmlns:p=\"http://www.w3.org/2005/07/scxml\"/>"),
        vec![(
            Reason::UnexpectedRoot {
                name: "state".into()
            },
            "<p:state xmlns:p=\"http://www.w3.org/2005/07/scxml\"/>".into()
        )]
    );
    let errors = errors("<scxml xmlns=\"urn:f\"/>");
    assert_eq!(
        errors[0].reason,
        Reason::ForeignElement {
            name: "scxml".into(),
            uri: "urn:f".into()
        }
    );
    assert_eq!(
        errors[0].message,
        "element \"scxml\" is bound to foreign namespace \"urn:f\", not SCXML"
    );
}

// Sabotage: with blank read as ASCII whitespace only, the no-break space run is refused and this test fails.
#[test]
fn reports_stray_text_trimmed_and_passes_blank_text() {
    let source = format!("<scxml xmlns=\"{SCXML}\">\n\u{A0}\u{3000}<state> x  y\n</state></scxml>");
    let errors = errors(&source);
    assert_eq!(errors.len(), 1);
    assert_eq!(
        errors[0].reason,
        Reason::StrayText {
            text: "x  y".into()
        }
    );
    assert_eq!(slice(&source, errors[0].location), " x  y\n");
    assert_eq!(errors[0].message, "stray text \"x  y\" is not allowed here");
}

// Sabotage: with the preview cut at thirty code points, this test fails.
#[test]
fn previews_long_stray_text_at_forty_code_points() {
    let long = "\u{E9}".repeat(45);
    let errors = errors(&format!("<scxml>{long}</scxml>"));
    let preview = "\u{E9}".repeat(40);
    assert_eq!(
        errors[0].message,
        format!("stray text \"{preview}...\" is not allowed here")
    );
    let exact = "a".repeat(40);
    let errors = self::errors(&format!("<scxml>{exact}</scxml>"));
    assert_eq!(
        errors[0].message,
        format!("stray text \"{exact}\" is not allowed here")
    );
}

// Sabotage: with a misplaced child named by its element rather than its slot, `<parallel>` is reported as itself and this test fails.
#[test]
fn reports_a_misplaced_element_under_its_slots_name_and_its_parent_as_written() {
    let source = format!(
        "<s:scxml xmlns:s=\"{SCXML}\"><s:initial/><s:state><s:onentry><s:parallel/>\
         </s:onentry><s:data id=\"d\"/></s:state></s:scxml>"
    );
    assert_eq!(
        refusals(&source),
        vec![
            (misplaced("initial", "s:scxml"), "<s:initial/>".into()),
            (misplaced("state", "s:onentry"), "<s:parallel/>".into()),
            (misplaced("data", "s:state"), "<s:data id=\"d\"/>".into()),
        ]
    );
    assert_eq!(
        errors(&source)[0].message,
        "element \"initial\" is not allowed inside \"s:scxml\""
    );
}

// Sabotage: with the errors left unsorted, this test fails.
#[test]
fn collects_every_error_in_source_order() {
    let source = format!(
        "<scxml xmlns=\"{SCXML}\"><state><onentry><raise/></onentry><bogus/></state>\
         text<final><transition><log><x/></log></transition></final></scxml>"
    );
    let offsets: Vec<usize> = errors(&source)
        .iter()
        .map(|error| error.location.start_offset)
        .collect();
    assert_eq!(offsets.len(), 4);
    assert!(
        offsets.windows(2).all(|pair| pair[0] < pair[1]),
        "{offsets:?}"
    );
}

// Sabotage: with the first of two attributes passed over for the last, this test fails.
#[test]
fn reads_the_first_of_two_attributes_and_ignores_attributes_it_does_not_read() {
    let (source, state) = in_state("<state id=\"a\" id=\"b\" colour=\"c\" onentry=\"d\"/>");
    let child = &state.states[0];
    assert_eq!(child.id.as_deref(), Some("a"));
    assert_eq!(slice(&source, child.attribute_locations["id"]), "a");
    assert_eq!(child.attribute_locations.len(), 1);
}

// ---- whitespace, as the reference reads it

// Sabotage: with a list split on every Unicode whitespace character, the no-break spaces split and this test fails.
#[test]
fn splits_a_list_on_unicode_whitespace_other_than_the_no_break_spaces() {
    let (_, state) =
        in_state("<transition event=\"a\u{2003}b\u{3000}c\u{85}d\u{A0}e\u{2007}f\u{202F}g\"/>");
    assert_eq!(
        state.transitions[0].event,
        strings(&["a", "b", "c", "d\u{A0}e\u{2007}f\u{202F}g"])
    );
}

// Sabotage: with blank read as ASCII whitespace only, this test fails.
#[test]
fn reads_text_as_blank_by_the_references_set() {
    for blank in [
        "\u{85}", "\u{A0}", "\u{2007}", "\u{202F}", "\u{2028}", "\u{3000}", "\t\r\n ",
    ] {
        let source = format!("<scxml>{blank}</scxml>");
        assert!(read(&source).is_ok(), "{blank:?}");
    }
    for not_blank in ["\u{200B}", "\u{FEFF}", "\u{180E}"] {
        let source = format!("<scxml>{not_blank}</scxml>");
        assert!(read(&source).is_err(), "{not_blank:?}");
    }
}

// ---- nesting

fn nested(depth: usize) -> String {
    let mut source = String::from("<scxml>");
    source.push_str(&"<state>".repeat(depth));
    source.push_str(&"</state>".repeat(depth));
    source.push_str("</scxml>");
    source
}

// Sabotage: with the bound checked one level later, the first refused element lowers and this test fails.
#[test]
fn refuses_an_element_nested_deeper_than_the_bound() {
    let mut document = document(&nested(MAX_DEPTH));
    let mut depth = 0;
    while let Some(state) = document.states.pop() {
        depth += 1;
        document.states = state.states;
    }
    assert_eq!(depth, MAX_DEPTH);
    let source = nested(MAX_DEPTH + 1);
    let errors = errors(&source);
    assert_eq!(errors.len(), 1);
    assert_eq!(
        errors[0].reason,
        Reason::NestingTooDeep {
            name: "state".into()
        }
    );
    assert_eq!(slice(&source, errors[0].location), "<state></state>");
}

// A tree far deeper than the bound is refused at the bound, without the walk
// going below it; the reader builds and drops the tree without recursion.
// Sabotage: with the refusal placed at the parent of the first element beyond the bound, this test fails.
#[test]
fn refuses_a_very_deep_document_without_walking_below_the_bound() {
    let source = nested(100_000);
    let errors = errors(&source);
    assert_eq!(errors.len(), 1);
    assert_eq!(
        errors[0].location.start_offset,
        "<scxml>".len() + "<state>".len() * MAX_DEPTH
    );
}

// ---- the corpus and the no-panic promise

// Sabotage: with `<send>` missing from the dispatch, the corpus's sends are refused and this test fails.
#[test]
fn lowers_every_scxml_document_in_the_vendored_corpus() {
    let documents = corpus_documents();
    let refused: Vec<String> = documents
        .iter()
        .filter_map(|(name, bytes)| {
            let source = std::str::from_utf8(bytes).expect("UTF-8");
            let root = parse(bytes).expect("a corpus document parses");
            lower(&root, source)
                .err()
                .map(|errors| format!("{name}: {errors:#?}"))
        })
        .collect();
    println!("corpus documents lowered: {}", documents.len());
    assert!(documents.len() > 300, "{}", documents.len());
    assert!(refused.is_empty(), "{refused:#?}");
}

fn elements_of(root: &Element) -> Vec<&Element> {
    let mut found = vec![root];
    let mut index = 0;
    while let Some(element) = found.get(index).copied() {
        found.extend(element.children.iter().filter_map(|child| match child {
            Node::Element(child) => Some(child),
            Node::Text(_) => None,
        }));
        index += 1;
    }
    found
}

// Every element of every corpus document, taken as a root, lowers to a value:
// a document or its refusals, never a panic.
// Sabotage: with lower's root check skipped, a `<state>` taken as a root is built as `<scxml>` and this test fails.
#[test]
fn answers_every_element_of_every_corpus_document_taken_as_a_root() {
    let mut lowered = 0;
    let mut refused = 0;
    for (_, bytes) in corpus_documents() {
        let source = std::str::from_utf8(&bytes).expect("UTF-8");
        let root = parse(&bytes).expect("a corpus document parses");
        for element in elements_of(&root) {
            match lower(element, source) {
                Ok(document) => {
                    assert_eq!(super::local_name(&element.name), "scxml");
                    assert_eq!(document.location, element.location);
                    lowered += 1;
                }
                Err(errors) => {
                    assert!(!errors.is_empty());
                    assert!(
                        errors
                            .iter()
                            .all(|error| error.location.slice(source).is_some())
                    );
                    refused += 1;
                }
            }
        }
    }
    println!("corpus elements taken as roots: {lowered} lowered, {refused} refused");
    assert!(lowered > 300 && refused > 1000, "{lowered} {refused}");
}

// Every lowered element, placed in a `<log>`, which has no slot for any, is
// reported under its slot's name at its own span.
// Sabotage: with a misplaced `<finalize>` reported under the name `onentry`, this test fails.
#[test]
fn reports_every_lowered_element_misplaced_under_its_name() {
    let children = [
        ("state", "<parallel/>"),
        ("transition", "<transition/>"),
        ("initial", "<initial/>"),
        ("onentry", "<onentry/>"),
        ("onexit", "<onexit/>"),
        ("finalize", "<finalize/>"),
        ("donedata", "<donedata/>"),
        ("datamodel", "<datamodel/>"),
        ("data", "<data id=\"d\"/>"),
        ("param", "<param name=\"p\"/>"),
        ("content", "<content/>"),
        ("invoke", "<invoke/>"),
        ("raise", "<raise event=\"e\"/>"),
        ("log", "<log/>"),
        ("assign", "<assign location=\"l\"/>"),
        ("if", "<if cond=\"c\"/>"),
        ("foreach", "<foreach array=\"a\" item=\"i\"/>"),
        ("script", "<script/>"),
        ("send", "<send/>"),
        ("cancel", "<cancel/>"),
        ("elseif", "<elseif cond=\"c\"/>"),
        ("else", "<else/>"),
    ];
    let body: String = children.iter().map(|(_, markup)| *markup).collect();
    let source = format!(
        "<scxml xmlns=\"{SCXML}\"><state><onentry><log>{body}</log></onentry></state></scxml>"
    );
    let expected: Vec<(Reason, String)> = children
        .iter()
        .map(|(name, markup)| (misplaced(name, "log"), (*markup).to_owned()))
        .collect();
    assert_eq!(refusals(&source), expected);
}

// Each node with slots of its own refuses a child that fits none of them.
// Sabotage: with `<invoke>` placing a `<log>` into its finalize block, this test fails.
#[test]
fn refuses_a_child_that_fits_none_of_its_parents_slots() {
    let cases = [
        ("initial", "<initial><log/></initial>"),
        ("donedata", "<donedata><log/></donedata>"),
        ("datamodel", "<datamodel><log/></datamodel>"),
        ("invoke", "<invoke><log/></invoke>"),
        ("transition", "<transition><state/></transition>"),
    ];
    for (parent, body) in cases {
        let source = format!("<scxml xmlns=\"{SCXML}\"><state>{body}</state></scxml>");
        let reasons: Vec<Reason> = errors(&source).into_iter().map(|e| e.reason).collect();
        let child = if parent == "transition" {
            "state"
        } else {
            "log"
        };
        assert_eq!(reasons, vec![misplaced(child, parent)], "{body}");
    }
    for (parent, body) in [
        ("send", "<send><log/></send>"),
        ("if", "<if cond=\"c\"><state/></if>"),
        (
            "foreach",
            "<foreach array=\"a\" item=\"i\"><param name=\"p\"/></foreach>",
        ),
    ] {
        let source =
            format!("<scxml xmlns=\"{SCXML}\"><state><onentry>{body}</onentry></state></scxml>");
        let reasons: Vec<Reason> = errors(&source).into_iter().map(|e| e.reason).collect();
        assert_eq!(reasons.len(), 1, "{body}");
        let Reason::MisplacedElement { parent: found, .. } = &reasons[0] else {
            panic!("expected a misplaced element, got {reasons:?}");
        };
        assert_eq!(found, parent);
    }
}
