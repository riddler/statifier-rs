use super::finding::{Owner, Reason, WarningReason};
use super::{Options, Refused, Warning, validate};
use crate::lowering::lower;
use crate::testing::corpus_documents;
use crate::xml::parse;

const SCXML: &str = "http://www.w3.org/2005/07/scxml";

// `source` read, lowered and validated with `options`.
fn run(source: &str, options: Options) -> Result<Vec<Warning>, Refused> {
    let root = match parse(source.as_bytes()) {
        Ok(root) => root,
        Err(error) => panic!("expected {source:?} to parse, got {}", error.message),
    };
    match lower(&root, source) {
        Ok(document) => validate(&document, source, options),
        Err(errors) => panic!("expected {source:?} to lower, got {errors:#?}"),
    }
}

// A conformant root holding `body`, with `attributes` on it.
fn chart_with(attributes: &str, body: &str) -> String {
    format!("<scxml xmlns=\"{SCXML}\" version=\"1.0\"{attributes}>{body}</scxml>")
}

fn chart(body: &str) -> String {
    chart_with("", body)
}

// The errors `source` is refused with, each as its reason, the text its span
// covers and where that span starts.
fn findings(source: &str) -> Vec<(Reason, String, usize)> {
    match run(source, Options::default()) {
        Ok(_) => panic!("expected {source:?} to be refused"),
        Err(refused) => refused
            .errors
            .into_iter()
            .map(|error| {
                let text = error.location.slice(source).expect("a span in the source");
                (error.reason, text.to_owned(), error.location.start_offset)
            })
            .collect(),
    }
}

// Each error's reason and the text its span covers.
fn refusals(source: &str) -> Vec<(Reason, String)> {
    findings(source)
        .into_iter()
        .map(|(reason, text, _)| (reason, text))
        .collect()
}

fn one(reason: Reason, text: &str) -> Vec<(Reason, String)> {
    vec![(reason, text.to_owned())]
}

fn some(id: &str) -> Option<String> {
    Some(id.to_owned())
}

// ---- ids, targets and initials

// Sabotage: with every id read as a repeat, and this test fails.
#[test]
fn a_repeated_id_is_reported_at_each_later_occurrence_a_data_id_included() {
    let source = chart(r#"<datamodel><data id="x" expr="1"/></datamodel><state id="x"/>"#);
    let found = findings(&source);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].0, Reason::DuplicateId { id: "x".into() });
    assert_eq!(
        found[0].2,
        source.rfind("\"x\"").expect("the state's id") + 1
    );
}

// Sabotage: with an empty id sent to the duplicate set, and this test fails.
#[test]
fn an_empty_id_is_reported_and_two_empty_ids_are_no_duplicate() {
    assert_eq!(
        refusals(&chart(r#"<state id=""/><state id=""/>"#)),
        vec![
            (Reason::EmptyId, String::new()),
            (Reason::EmptyId, String::new())
        ]
    );
}

// Sabotage: with an unresolved target skipped, and this test fails.
#[test]
fn a_transition_target_that_names_no_state_is_reported_at_the_target() {
    assert_eq!(
        refusals(&chart(
            r#"<state id="a"><transition target="a b"/></state>"#
        )),
        one(Reason::UnresolvedTarget { id: "b".into() }, "a b")
    );
}

// Sabotage: with the root's initial left unchecked, and this test fails.
#[test]
fn an_initial_that_names_no_state_is_reported_on_a_state_and_on_the_root() {
    let source = chart_with(
        r#" initial="q""#,
        r#"<state id="a" initial="z"><state id="b"/></state>"#,
    );
    assert_eq!(
        refusals(&source),
        vec![
            (Reason::UnresolvedInitial { id: "q".into() }, "q".into()),
            (Reason::UnresolvedInitial { id: "z".into() }, "z".into()),
        ]
    );
}

// Sabotage: with an initial attribute's target read as always inside its state, and this test fails.
#[test]
fn an_initial_outside_its_state_is_not_a_descendant() {
    let source = chart(
        r#"<state id="a" initial="c"><state id="b"/></state><state><initial><transition target="b"/></initial><state id="c"/></state>"#,
    );
    assert_eq!(
        refusals(&source),
        vec![
            (
                Reason::InitialNotDescendant {
                    id: "c".into(),
                    parent_id: some("a"),
                },
                "c".into()
            ),
            (
                Reason::InitialNotDescendant {
                    id: "b".into(),
                    parent_id: None,
                },
                "b".into()
            ),
        ]
    );
}

// Sabotage: with a parallel counted as able to take an initial, and this test fails.
#[test]
fn an_initial_on_a_state_with_nothing_to_enter_is_reported_once() {
    assert_eq!(
        refusals(&chart(
            r#"<state id="a" initial="nowhere"/><parallel id="p" initial="x"><state id="x"/></parallel>"#
        )),
        vec![
            (
                Reason::InitialOnAtomicState { id: some("a") },
                "nowhere".into()
            ),
            (Reason::InitialOnAtomicState { id: some("p") }, "x".into()),
        ]
    );
}

// Sabotage: with the attribute-and-element check never firing, and this test fails.
#[test]
fn an_initial_element_beside_an_initial_attribute_is_reported() {
    let initial = r#"<initial><transition target="b"/></initial>"#;
    let source = chart(&format!(
        r#"<state id="a" initial="b">{initial}<state id="b"/></state>"#
    ));
    assert_eq!(
        refusals(&source),
        one(
            Reason::InitialAttributeAndElement { id: some("a") },
            initial
        )
    );
}

// Sabotage: with the count check firing only on two or more, and this test fails.
#[test]
fn an_initial_element_with_no_transition_has_the_wrong_count() {
    assert_eq!(
        refusals(&chart(r#"<state id="a"><initial/><state id="b"/></state>"#)),
        one(
            Reason::TransitionCount {
                owner: Owner::Initial(some("a")),
                count: 0,
            },
            "<initial/>"
        )
    );
}

// Sabotage: with a missing target accepted, and this test fails.
#[test]
fn a_default_transition_without_a_target_is_reported() {
    assert_eq!(
        refusals(&chart(
            r#"<state id="a"><initial><transition target=""/></initial><state id="b"/></state>"#
        )),
        one(
            Reason::TransitionMissingTarget {
                owner: Owner::Initial(some("a")),
            },
            r#"<transition target=""/>"#
        )
    );
}

// Sabotage: with a written event read from the event list, which an empty one leaves empty, and this test fails.
#[test]
fn a_default_transition_with_event_or_cond_is_reported_at_each() {
    let owner = Owner::Initial(some("a"));
    assert_eq!(
        refusals(&chart(
            r#"<state id="a"><initial><transition event="" cond="c1" target="b"/></initial><state id="b"/></state>"#
        )),
        vec![
            (
                Reason::TransitionForbiddenAttribute {
                    owner: owner.clone(),
                    attribute: "event",
                },
                String::new()
            ),
            (
                Reason::TransitionForbiddenAttribute {
                    owner,
                    attribute: "cond",
                },
                "c1".into()
            ),
        ]
    );
}

// ---- histories, finals and default entry

// Sabotage: with the root counted as a compound parent, and this test fails.
#[test]
fn a_history_outside_a_compound_state_is_reported() {
    let history = r#"<history id="h"><transition target="a"/></history>"#;
    assert_eq!(
        refusals(&chart(&format!(r#"{history}<state id="a"/>"#))),
        one(
            Reason::HistoryBadParent {
                id: some("h"),
                parent_kind: "scxml",
            },
            history
        )
    );
}

// Sabotage: with the descendancy test against the history's parent inverted, and this test fails.
#[test]
fn a_history_default_target_outside_its_parent_is_not_a_descendant() {
    assert_eq!(
        refusals(&chart(
            r#"<state id="p"><state id="a"/><history id="h"><transition target="o"/></history></state><state id="o"/>"#
        )),
        one(
            Reason::InitialNotDescendant {
                id: "o".into(),
                parent_id: some("p"),
            },
            "o"
        )
    );
}

// Sabotage: with `sideways` added to the history types read, and this test fails.
#[test]
fn a_history_type_out_of_range_is_reported_as_written() {
    assert_eq!(
        refusals(&chart(
            r#"<state id="p"><state id="a"/><history id="h" type="sideways"><transition target="a"/></history></state>"#
        )),
        one(
            Reason::HistoryBadType {
                raw: "sideways".into()
            },
            "sideways"
        )
    );
}

// Sabotage: with a final's first state child skipped, and this test fails.
#[test]
fn a_state_inside_a_final_is_reported_at_the_child() {
    assert_eq!(
        refusals(&chart(r#"<final id="f"><state id="x"/></final>"#)),
        one(
            Reason::FinalHasStates { id: some("x") },
            r#"<state id="x"/>"#
        )
    );
}

// Sabotage: with a final's first transition skipped, and this test fails.
#[test]
fn a_transition_inside_a_final_is_reported_at_the_transition() {
    assert_eq!(
        refusals(&chart(r#"<final id="f"><transition target="f"/></final>"#)),
        one(
            Reason::FinalHasTransitions { id: some("f") },
            r#"<transition target="f"/>"#
        )
    );
}

// Sabotage: with the first final child's id replaced by none, and this test fails.
#[test]
fn a_state_holding_a_final_without_an_id_is_reported_once() {
    let state = r#"<state><final id="f"/><final id="g"/></state>"#;
    assert_eq!(
        refusals(&chart(state)),
        one(
            Reason::FinalParentMissingId {
                final_id: some("f"),
            },
            state
        )
    );
}

// Sabotage: with a parallel held to default entry too, and this test fails.
#[test]
fn a_state_entered_at_a_leading_history_is_reported_and_a_parallel_is_not() {
    let history = r#"<history id="h"><transition target="a"/></history>"#;
    let source = chart(&format!(
        r#"<state id="p">{history}<state id="a"/></state><parallel id="q"><history id="g"><transition target="b"/></history><state id="b"/></parallel>"#
    ));
    assert_eq!(
        refusals(&source),
        one(
            Reason::DefaultEntryNotEnterable {
                id: some("p"),
                child_kind: "history",
            },
            history
        )
    );
}

// ---- data and the root

// Sabotage: with a final counted as a state that may not carry donedata, and this test fails.
#[test]
fn donedata_on_a_state_that_is_not_final_is_reported() {
    let donedata = r#"<donedata><param name="n" expr="1"/></donedata>"#;
    assert_eq!(
        refusals(&chart(&format!(
            r#"<state id="a">{donedata}</state><final id="f">{donedata}</final>"#
        ))),
        one(Reason::DonedataNotOnFinal { id: some("a") }, donedata)
    );
}

// Sabotage: with the content half of the pair inverted, and this test fails.
#[test]
fn donedata_with_content_and_params_is_reported() {
    let donedata = r#"<donedata><content expr="1"/><param name="n" expr="1"/></donedata>"#;
    assert_eq!(
        refusals(&chart(&format!(r#"<final id="f">{donedata}</final>"#))),
        one(Reason::DonedataContentAndParams { id: some("f") }, donedata)
    );
}

// Sabotage: with blank text read as content, and this test fails.
#[test]
fn content_with_expr_and_inline_content_is_reported_with_its_form() {
    let source = chart(
        r#"<final id="f"><donedata><content expr="1">
  </content></donedata></final><state id="a"><invoke><content expr="2">x</content></invoke><invoke><content expr="3"><b/></content></invoke></state>"#,
    );
    let errors = run(&source, Options::default())
        .expect_err("refused")
        .errors;
    let found: Vec<(Reason, &str)> = errors
        .iter()
        .map(|error| (error.reason.clone(), error.message.as_str()))
        .collect();
    assert_eq!(
        found,
        vec![
            (
                Reason::ContentExprAndText { expr: "2".into() },
                "<content> must not specify both an expr attribute (\"2\") and inline text"
            ),
            (
                Reason::ContentExprAndText { expr: "3".into() },
                "<content> must not specify both an expr attribute (\"3\") and inline markup"
            ),
        ]
    );
}

// Sabotage: with a param holding both accepted, and this test fails.
#[test]
fn a_param_with_expr_and_location_is_reported() {
    let param = r#"<param name="n" expr="1" location="v"/>"#;
    assert_eq!(
        refusals(&chart(&format!(
            r#"<final id="f"><donedata>{param}</donedata></final>"#
        ))),
        one(Reason::ParamExprAndLocation { name: "n".into() }, param)
    );
}

// Sabotage: with the send params left out of the walk, and this test fails.
#[test]
fn a_param_with_no_value_is_reported_under_a_send_too() {
    let param = r#"<param name="n"/>"#;
    assert_eq!(
        refusals(&chart(&format!(
            r#"<state id="a"><onentry><foreach array="v" item="i"><send event="e">{param}</send></foreach></onentry></state>"#
        ))),
        one(Reason::ParamNoValue { name: "n".into() }, param)
    );
}

// Sabotage: with the relaxed mode ignored, and this test fails.
#[test]
fn a_root_with_no_namespace_is_reported_unless_invoke_content_markup_is_on() {
    let source = r#"<scxml version="1.0"><state id="a"/></scxml>"#;
    assert_eq!(
        refusals(source),
        one(Reason::BadNamespace { uri: None }, source)
    );
    let relaxed = Options {
        invoke_content_markup: true,
    };
    assert_eq!(run(source, relaxed), Ok(Vec::new()));
}

// Sabotage: with an absent version accepted, and this test fails.
#[test]
fn a_root_version_other_than_one_point_oh_is_reported() {
    let absent = format!(r#"<scxml xmlns="{SCXML}"><state id="a"/></scxml>"#);
    assert_eq!(
        refusals(&absent),
        one(Reason::BadVersion { version: None }, &absent)
    );
    assert_eq!(
        refusals(&format!(r#"<scxml xmlns="{SCXML}" version="2.0"/>"#)),
        one(
            Reason::BadVersion {
                version: some("2.0")
            },
            "2.0"
        )
    );
}

// Sabotage: with `sideways` added to the transition types read, and this test fails.
#[test]
fn a_transition_type_out_of_range_is_reported_as_written() {
    assert_eq!(
        refusals(&chart(
            r#"<state id="a"><transition type="sideways" target="a"/></state>"#
        )),
        one(
            Reason::TransitionBadType {
                raw: "sideways".into()
            },
            "sideways"
        )
    );
}

// Sabotage: with `whenever` added to the bindings read, and this test fails.
#[test]
fn a_binding_out_of_range_is_reported_as_written() {
    assert_eq!(
        refusals(&chart_with(r#" binding="whenever""#, "")),
        one(
            Reason::ScxmlBadBinding {
                raw: "whenever".into()
            },
            "whenever"
        )
    );
}

// Sabotage: with `javascript` added to the datamodels read, and this test fails.
#[test]
fn a_datamodel_out_of_range_is_reported_as_written() {
    assert_eq!(
        refusals(&chart_with(r#" datamodel="javascript""#, "")),
        one(
            Reason::ScxmlBadDatamodel {
                raw: "javascript".into()
            },
            "javascript"
        )
    );
}

// Sabotage: with `yes` added to the autoforward values read, and this test fails.
#[test]
fn an_autoforward_out_of_range_is_reported_as_written() {
    assert_eq!(
        refusals(&chart(
            r#"<state id="a"><invoke autoforward="yes"/></state>"#
        )),
        one(Reason::InvokeBadAutoforward { raw: "yes".into() }, "yes")
    );
}

// Sabotage: with the src half of the pair inverted, and this test fails.
#[test]
fn data_with_expr_and_src_is_reported() {
    assert_eq!(
        refusals(&chart(
            r#"<datamodel><data id="d" expr="1" src="s"/></datamodel>"#
        )),
        one(Reason::DataExprAndSrc { id: "d".into() }, "d")
    );
}

// Sabotage: with a data's child text left unread, and this test fails.
#[test]
fn data_with_a_value_and_child_text_is_reported() {
    assert_eq!(
        refusals(&chart(
            r#"<state id="a"><datamodel><data id="d" expr="1">2</data></datamodel></state>"#
        )),
        one(Reason::DataValueAndChildren { id: "d".into() }, "d")
    );
}

// Sabotage: with the reserved prefix read as `$`, and this test fails.
#[test]
fn a_data_id_starting_with_an_underscore_is_reported() {
    assert_eq!(
        refusals(&chart(r#"<datamodel><data id="_d"/></datamodel>"#)),
        one(Reason::DataReservedId { id: "_d".into() }, "_d")
    );
}

// Sabotage: with a final allowed a datamodel, and this test fails.
#[test]
fn a_datamodel_on_a_final_is_reported() {
    let datamodel = r#"<datamodel><data id="d"/></datamodel>"#;
    assert_eq!(
        refusals(&chart(&format!(r#"<final id="f">{datamodel}</final>"#))),
        one(Reason::DatamodelBadParent { kind: "final" }, datamodel)
    );
}

// ---- executable content

// Sabotage: with the walk not descending into an `<if>`, and this test fails.
#[test]
fn an_assign_with_expr_and_children_is_reported_inside_an_if() {
    let assign = r#"<assign location="v" expr="1">2</assign>"#;
    assert_eq!(
        refusals(&chart(&format!(
            r#"<state id="a"><onentry><if cond="c">{assign}</if></onentry></state>"#
        ))),
        one(Reason::AssignExprAndText { expr: "1".into() }, assign)
    );
}

// Sabotage: with an elseif after an else read as a duplicate else, and this test fails.
#[test]
fn an_elseif_after_an_else_is_reported() {
    assert_eq!(
        refusals(&chart(
            r#"<state id="a"><transition target="a"><if cond="c"><else/><elseif cond="d"/></if></transition></state>"#
        )),
        one(Reason::IfElseifAfterElse, r#"<elseif cond="d"/>"#)
    );
}

// Sabotage: with the else already seen left unset, and this test fails.
#[test]
fn a_second_else_is_reported_at_the_second() {
    let source = chart(
        r#"<state id="a"><onexit><if cond="c"><if cond="d"><else/><else/></if></if></onexit></state>"#,
    );
    let found = findings(&source);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].0, Reason::IfDuplicateElse);
    assert_eq!(
        found[0].2,
        source.rfind("<else/>").expect("the second else")
    );
}

// Sabotage: with the root's scripts left out, and this test fails.
#[test]
fn a_script_with_no_text_is_reported_in_a_state_and_under_the_root() {
    assert_eq!(
        refusals(&chart(
            r#"<state id="a"><onentry><script>  </script></onentry></state><script/>"#
        )),
        vec![
            (Reason::ScriptNoSrcOrText, "<script>  </script>".into()),
            (Reason::ScriptNoSrcOrText, "<script/>".into()),
        ]
    );
}

// Sabotage: with the invoke type pair read from `type` alone, and this test fails.
#[test]
fn an_invoke_with_type_and_typeexpr_is_reported() {
    let invoke = r#"<invoke type="t" typeexpr="x"/>"#;
    assert_eq!(
        refusals(&chart(&format!(
            r#"<state id="a">{invoke}<invoke type="t"/></state>"#
        ))),
        one(Reason::InvokeTypeAndTypeexpr, invoke)
    );
}

// Sabotage: with the invoke src pair removed, and this test fails.
#[test]
fn an_invoke_with_src_and_srcexpr_is_reported() {
    let invoke = r#"<invoke src="s" srcexpr="x"/>"#;
    assert_eq!(
        refusals(&chart(&format!(r#"<state id="a">{invoke}</state>"#))),
        one(Reason::InvokeSrcAndSrcexpr, invoke)
    );
}

// Sabotage: with a srcexpr beside content accepted, and this test fails.
#[test]
fn an_invoke_with_a_source_and_content_is_reported() {
    let invoke = r#"<invoke srcexpr="x"><content>c</content></invoke>"#;
    assert_eq!(
        refusals(&chart(&format!(r#"<state id="a">{invoke}</state>"#))),
        one(Reason::InvokeSrcAndContent, invoke)
    );
}

// Sabotage: with the invoke id pair removed, and this test fails.
#[test]
fn an_invoke_with_id_and_idlocation_is_reported() {
    let invoke = r#"<invoke id="i" idlocation="v"/>"#;
    assert_eq!(
        refusals(&chart(&format!(r#"<state id="a">{invoke}</state>"#))),
        one(Reason::InvokeIdAndIdlocation, invoke)
    );
}

// Sabotage: with the invoke namelist pair removed, and this test fails.
#[test]
fn an_invoke_with_namelist_and_param_is_reported() {
    let invoke = r#"<invoke namelist="v"><param name="n" expr="1"/></invoke>"#;
    assert_eq!(
        refusals(&chart(&format!(r#"<state id="a">{invoke}</state>"#))),
        one(Reason::InvokeNamelistAndParam, invoke)
    );
}

// A `<send>` in an `<onentry>` of one state, answering the errors.
fn send_refusals(send: &str) -> Vec<(Reason, String)> {
    refusals(&chart(&format!(
        r#"<state id="a"><onentry>{send}</onentry></state>"#
    )))
}

// Sabotage: with the send event pair removed, and this test fails.
#[test]
fn a_send_with_event_and_eventexpr_is_reported() {
    let send = r#"<send event="e" eventexpr="x"/>"#;
    assert_eq!(
        send_refusals(send),
        one(Reason::SendEventAndEventexpr, send)
    );
}

// Sabotage: with the send target pair removed, and this test fails.
#[test]
fn a_send_with_target_and_targetexpr_is_reported() {
    let send = r#"<send event="e" target="t" targetexpr="x"/>"#;
    assert_eq!(
        send_refusals(send),
        one(Reason::SendTargetAndTargetexpr, send)
    );
}

// Sabotage: with the send type pair removed, and this test fails.
#[test]
fn a_send_with_type_and_typeexpr_is_reported() {
    let send = r#"<send event="e" type="t" typeexpr="x"/>"#;
    assert_eq!(send_refusals(send), one(Reason::SendTypeAndTypeexpr, send));
}

// Sabotage: with the send id pair removed, and this test fails.
#[test]
fn a_send_with_id_and_idlocation_is_reported() {
    let send = r#"<send event="e" id="i" idlocation="v"/>"#;
    assert_eq!(send_refusals(send), one(Reason::SendIdAndIdlocation, send));
}

// Sabotage: with the send delay pair removed, and this test fails.
#[test]
fn a_send_with_delay_and_delayexpr_is_reported() {
    let send = r#"<send event="e" delay="1s" delayexpr="x"/>"#;
    assert_eq!(
        send_refusals(send),
        one(Reason::SendDelayAndDelayexpr, send)
    );
}

// Sabotage: with `#_internal` left out of the internal target, and this test fails.
#[test]
fn a_delayed_send_to_the_internal_target_is_reported_in_both_spellings() {
    let plain = r#"<send event="e" target="_internal" delay="1s"/>"#;
    let hashed = r##"<send event="e" target="#_internal" delayexpr="x"/>"##;
    assert_eq!(
        send_refusals(&format!("{plain}{hashed}")),
        vec![
            (Reason::SendDelayAndInternalTarget, plain.into()),
            (Reason::SendDelayAndInternalTarget, hashed.into()),
        ]
    );
}

// Sabotage: with the send namelist pair removed, and this test fails.
#[test]
fn a_send_with_namelist_and_content_is_reported() {
    let send = r#"<send event="e" namelist="v"><content>c</content></send>"#;
    assert_eq!(
        send_refusals(send),
        one(Reason::SendNamelistAndContent, send)
    );
}

// Sabotage: with the send param pair removed, and this test fails.
#[test]
fn a_send_with_param_and_content_is_reported() {
    let send = r#"<send event="e"><param name="n" expr="1"/><content>c</content></send>"#;
    assert_eq!(send_refusals(send), one(Reason::SendParamAndContent, send));
}

// Sabotage: with a cancel holding both accepted, and this test fails.
#[test]
fn a_cancel_with_sendid_and_sendidexpr_is_reported() {
    let cancel = r#"<cancel sendid="s" sendidexpr="x"/>"#;
    assert_eq!(
        send_refusals(cancel),
        one(Reason::CancelSendidAndSendidexpr, cancel)
    );
}

// Sabotage: with the finalize blocks left out of the cancel walk, and this test fails.
#[test]
fn a_cancel_with_no_sendid_is_reported_inside_finalize_too() {
    assert_eq!(
        refusals(&chart(
            r#"<state id="a"><invoke><finalize><cancel/></finalize></invoke></state>"#
        )),
        one(Reason::CancelNoSendid, "<cancel/>")
    );
}

// ---- the markup

// Sabotage: with the work list not descending into children, and this test fails.
#[test]
fn a_repeated_attribute_is_reported_at_each_repeat_in_content_markup_too() {
    let source = chart(
        r#"<state id="a" id="b"><invoke><content><x k="1" k="2"/></content></invoke></state>"#,
    );
    assert_eq!(
        refusals(&source),
        vec![
            (
                Reason::DuplicateAttribute {
                    element: "state".into(),
                    attribute: "id".into(),
                },
                r#"id="b""#.into()
            ),
            (
                Reason::DuplicateAttribute {
                    element: "x".into(),
                    attribute: "k".into(),
                },
                r#"k="2""#.into()
            ),
        ]
    );
}

// ---- the warning and the contracts

// Sabotage: with the finalize content read without descending into an `<if>`, and this test fails.
#[test]
fn a_raise_or_send_inside_finalize_is_a_warning_and_the_document_validates() {
    let raise = r#"<raise event="e"/>"#;
    let send = r#"<send event="e"/>"#;
    let source = chart(&format!(
        r#"<state id="a"><invoke><finalize>{raise}<if cond="c">{send}</if></finalize></invoke></state>"#
    ));
    let warnings = run(&source, Options::default()).expect("the document validates");
    let found: Vec<(WarningReason, &str, &str)> = warnings
        .iter()
        .map(|warning| {
            let text = warning.location.slice(&source).expect("a span");
            (warning.reason.clone(), text, warning.message.as_str())
        })
        .collect();
    assert_eq!(
        found,
        vec![
            (
                WarningReason::FinalizeForbiddenContent { element: "raise" },
                raise,
                "<raise> must not occur inside <finalize>"
            ),
            (
                WarningReason::FinalizeForbiddenContent { element: "send" },
                send,
                "<send> must not occur inside <finalize>"
            ),
        ]
    );
}

// Sabotage: with the warnings dropped when an error fires, and this test fails.
#[test]
fn a_refused_document_still_answers_its_warnings() {
    let source = chart(
        r#"<state id="a"><transition target="z"/><invoke><finalize><raise event="e"/></finalize></invoke></state>"#,
    );
    let refused = run(&source, Options::default()).expect_err("refused");
    assert_eq!(refused.errors.len(), 1, "{refused:#?}");
    assert_eq!(refused.warnings.len(), 1, "{refused:#?}");
}

// Sabotage: with the errors left in the order the checks ran, and this test fails.
#[test]
fn every_check_runs_and_the_errors_come_in_document_order_check_order_at_one_offset() {
    let invoke = r#"<invoke type="t" typeexpr="x" src="s" srcexpr="y"/>"#;
    let source = chart(&format!(
        r#"<state id="a">{invoke}<transition target="z"/></state><state id="a"/>"#
    ));
    let reasons: Vec<Reason> = findings(&source)
        .into_iter()
        .map(|(reason, ..)| reason)
        .collect();
    assert_eq!(
        reasons,
        vec![
            Reason::InvokeTypeAndTypeexpr,
            Reason::InvokeSrcAndSrcexpr,
            Reason::UnresolvedTarget { id: "z".into() },
            Reason::DuplicateId { id: "a".into() },
        ]
    );
}

// Sabotage: with an absent id printed as `null`, and this test fails.
#[test]
fn a_message_reads_as_the_reference_writes_it() {
    let source = chart(r#"<history><transition target="a"/></history><state id="a"/>"#);
    let errors = run(&source, Options::default())
        .expect_err("refused")
        .errors;
    assert_eq!(
        errors[0].message,
        "history nil must be a child of a compound <state> or <parallel>, not scxml"
    );
}

// Sabotage: with every transition target reported as unresolved, and this test fails.
#[test]
fn every_corpus_document_validates() {
    let documents = corpus_documents();
    let refused: Vec<String> = documents
        .iter()
        .filter_map(|(name, bytes)| {
            let source = std::str::from_utf8(bytes).expect("UTF-8");
            let root = parse(bytes).expect("a corpus document parses");
            let document = lower(&root, source).expect("a corpus document lowers");
            validate(&document, source, Options::default())
                .err()
                .map(|refused| format!("{name}: {:#?}", refused.errors))
        })
        .collect();
    println!("corpus documents validated: {}", documents.len());
    assert!(documents.len() > 300, "{}", documents.len());
    assert!(refused.is_empty(), "{refused:#?}");
}
