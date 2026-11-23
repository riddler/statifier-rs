use super::delay::milliseconds;
use super::system_variables::{initial, put_event, seed};
use super::*;
use crate::machine::Machine;

const CHART: &str = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="a">
  <state id="a"><transition event="e.go" target="b"/></state>
  <state id="b"/>
</scxml>"#;

fn machine() -> Machine {
    crate::compile(CHART).expect("the chart compiles")
}

fn expr(source: &str) -> Expr {
    Expr::Compiled {
        instructions: predicator_rs::compile(source).expect("the expression compiles"),
        source: source.to_owned(),
    }
}

fn program(source: &str) -> Deferred<Program> {
    Deferred::Ready(Program {
        instructions: predicator_rs::compile_program(source).expect("the program compiles"),
        source: source.to_owned(),
    })
}

fn seeded(host: &[(&str, Value)]) -> Map {
    let host = host
        .iter()
        .map(|(root, value)| (*root, value.clone()))
        .collect();
    seed(host, initial("s-1", Some("chart"), Map::new()))
}

fn get<'a>(data: &'a Map, root: &str) -> Option<&'a Value> {
    data.get(&Key::from(root))
}

fn field<'a>(value: &'a Value, name: &str) -> &'a Value {
    match value {
        Value::Map(map) => map.get(&Key::from(name)).expect("the field"),
        other => panic!("not a map: {other:?}"),
    }
}

// Sabotage: with in_state answering configuration.is_empty() in place of
// the membership test, the active and inactive answers swap and this fails.
#[test]
fn in_answers_whether_a_declared_state_is_active() {
    let machine = machine();
    let chart = machine.chart();
    let configuration: BTreeSet<usize> = [chart.id_to_index["a"]].into_iter().collect();
    let data = seeded(&[]);
    let scope = Scope {
        data: &data,
        chart,
        configuration: &configuration,
    };
    assert_eq!(evaluate(&scope, &expr("In('a')")), Ok(Value::Bool(true)));
    assert_eq!(evaluate(&scope, &expr("In('b')")), Ok(Value::Bool(false)));
    assert_eq!(
        evaluate(&scope, &expr("In('nowhere')")),
        Ok(Value::Bool(false))
    );
    assert_eq!(evaluate(&scope, &expr("In(1)")), Ok(Value::Bool(false)));
}

// Sabotage: with in_state accepting any number of arguments as one, the
// two-argument call answers a value and this fails.
#[test]
fn in_refuses_any_count_of_arguments_but_one() {
    let machine = machine();
    let configuration = BTreeSet::new();
    let data = seeded(&[]);
    let scope = Scope {
        data: &data,
        chart: machine.chart(),
        configuration: &configuration,
    };
    let Err(Failure::Evaluation { source, error }) = evaluate(&scope, &expr("In('a', 'b')")) else {
        panic!("In with two arguments is refused");
    };
    assert_eq!(source, "In('a', 'b')");
    assert_eq!(error.type_name(), "EvaluationError");
}

// Sabotage: with UNBOUND set to OnUnbound::Undefined, the unbound load
// answers undefined and this fails.
#[test]
fn a_load_of_an_unbound_root_fails_and_a_seeded_one_reads_undefined() {
    let machine = machine();
    let configuration = BTreeSet::new();
    let data = seeded(&[("x", Value::from(2))]);
    let scope = Scope {
        data: &data,
        chart: machine.chart(),
        configuration: &configuration,
    };
    let Err(Failure::Evaluation { error, .. }) = evaluate(&scope, &expr("y is defined")) else {
        panic!("an unbound root is refused");
    };
    assert_eq!(error.type_name(), "UndefinedVariableError");
    assert_eq!(evaluate(&scope, &expr("x + 1")), Ok(Value::from(3)));
    assert_eq!(
        evaluate(&scope, &expr("_event is defined")),
        Ok(Value::Bool(false))
    );
}

// Sabotage: with initial binding _name to Null for a chart with no name,
// or _sessionid to the location, this fails.
#[test]
fn the_system_variables_are_seeded_over_the_host_roots() {
    let data = seed(
        [("_name", Value::from(9)), ("x", Value::from(1))]
            .into_iter()
            .collect(),
        initial("s-1", None, Map::new()),
    );
    assert_eq!(get(&data, "x"), Some(&Value::from(1)));
    assert_eq!(get(&data, "_sessionid"), Some(&Value::from("s-1")));
    assert_eq!(get(&data, "_name"), Some(&Value::Undefined));
    assert_eq!(get(&data, "_event"), Some(&Value::Undefined));
    let ioprocessors = get(&data, "_ioprocessors").expect("_ioprocessors");
    assert_eq!(
        field(field(ioprocessors, SCXML_EVENT_PROCESSOR_KEY), "location"),
        &Value::from("#_scxml_s-1")
    );
    let named = initial("s-2", Some("chart"), Map::new());
    assert_eq!(get(&named, "_name"), Some(&Value::from("chart")));
}

const SCXML_EVENT_PROCESSOR_KEY: &str = system_variables::SCXML_EVENT_PROCESSOR;

// Sabotage: with initial inserting the SCXML entry before the registered
// entries, a registered type under the processor's URI replaces it and
// this fails.
#[test]
fn a_registered_entry_never_replaces_the_scxml_entry() {
    let entries: Map = [
        (SCXML_EVENT_PROCESSOR_KEY, Value::from("taken")),
        ("x-kind", Value::from("there")),
    ]
    .into_iter()
    .collect();
    let data = initial("s-1", None, entries);
    let ioprocessors = get(&data, "_ioprocessors").expect("_ioprocessors");
    assert_eq!(field(ioprocessors, "x-kind"), &Value::from("there"));
    assert_eq!(
        field(field(ioprocessors, SCXML_EVENT_PROCESSOR_KEY), "location"),
        &Value::from("#_scxml_s-1")
    );
}

// Sabotage: with event writing Null for a missing send id, the read of
// _event.sendid answers null and this fails.
#[test]
fn event_reads_back_through_an_evaluation() {
    let machine = machine();
    let configuration = BTreeSet::new();
    let fields = EventFields {
        name: "e.go",
        event_type: EventType::External,
        sendid: None,
        origin: Some("#_scxml_s-0"),
        origintype: None,
        invokeid: None,
        data: Value::from(7),
    };
    let data = put_event(&seeded(&[]), system_variables::event(&fields));
    let scope = Scope {
        data: &data,
        chart: machine.chart(),
        configuration: &configuration,
    };
    assert_eq!(
        evaluate(&scope, &expr("_event.name")),
        Ok(Value::from("e.go"))
    );
    assert_eq!(
        evaluate(&scope, &expr("_event.type")),
        Ok(Value::from("external"))
    );
    assert_eq!(
        evaluate(&scope, &expr("_event.sendid is defined")),
        Ok(Value::Bool(false))
    );
    assert_eq!(
        evaluate(&scope, &expr("_event.origin")),
        Ok(Value::from("#_scxml_s-0"))
    );
    assert_eq!(evaluate(&scope, &expr("_event.data")), Ok(Value::from(7)));
    assert_eq!(EventType::Internal.as_str(), "internal");
    assert_eq!(EventType::Platform.as_str(), "platform");
}

// Sabotage: with evaluate_cond answering Ok(false) for a value that is not
// a boolean, the non-boolean failure is lost and this fails.
#[test]
fn a_condition_is_true_when_absent_and_fails_when_not_a_boolean() {
    let machine = machine();
    let configuration = BTreeSet::new();
    let data = seeded(&[("x", Value::from(2))]);
    let scope = Scope {
        data: &data,
        chart: machine.chart(),
        configuration: &configuration,
    };
    assert_eq!(evaluate_cond(&scope, None), Ok(true));
    assert_eq!(evaluate_cond(&scope, Some(&expr("x > 1"))), Ok(true));
    assert_eq!(evaluate_cond(&scope, Some(&expr("x > 5"))), Ok(false));
    assert_eq!(
        evaluate_cond(&scope, Some(&expr("x"))),
        Err(Failure::NonBooleanCond(Value::from(2)))
    );
    assert!(matches!(
        evaluate_cond(&scope, Some(&expr("1 / 0"))),
        Err(Failure::Evaluation { .. })
    ));
    assert_eq!(
        evaluate(&scope, &Expr::Static(Value::from("lit"))),
        Ok(Value::from("lit"))
    );
}

// Sabotage: with evaluate_deferred evaluating nothing and answering
// Ok(Value::Null) for an invalid value, this fails.
#[test]
fn a_deferred_compile_refusal_fails_when_evaluated() {
    let machine = machine();
    let configuration = BTreeSet::new();
    let data = seeded(&[]);
    let scope = Scope {
        data: &data,
        chart: machine.chart(),
        configuration: &configuration,
    };
    let refusals = crate::compile(
        r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0">
             <state id="a"><transition cond="x >" target="a"/></state>
           </scxml>"#,
    )
    .unwrap_err();
    let refusal = Box::new(refusals[0].clone());
    let invalid = Deferred::Invalid(refusal.clone());
    assert_eq!(
        evaluate_deferred(&scope, &invalid),
        Err(Failure::Compile(refusal.clone()))
    );
    assert_eq!(
        evaluate_deferred(&scope, &Deferred::Ready(expr("1 + 1"))),
        Ok(Value::from(2))
    );
    let executed = execute(&scope, &Deferred::Invalid(refusal.clone()));
    assert_eq!(executed.data, data);
    assert_eq!(executed.failure, Some(Failure::Compile(refusal)));
}

// Sabotage: with execute keeping the before datamodel rather than merging
// the program's writes, x stays 1 and this fails.
#[test]
fn a_program_writes_through_and_may_create_a_root() {
    let machine = machine();
    let configuration = BTreeSet::new();
    let data = seeded(&[("x", Value::from(1))]);
    let scope = Scope {
        data: &data,
        chart: machine.chart(),
        configuration: &configuration,
    };
    let executed = execute(&scope, &program("x = x + 1; y = In('a'); x = x"));
    assert_eq!(executed.failure, None);
    assert_eq!(get(&executed.data, "x"), Some(&Value::from(2)));
    assert_eq!(get(&executed.data, "y"), Some(&Value::Bool(false)));
    assert_eq!(get(&data, "x"), Some(&Value::from(1)));
}

// Sabotage: with execute building its options without protected_roots, the
// store into _sessionid lands and the later statement runs, so this fails.
#[test]
fn a_store_into_a_seeded_system_root_is_refused_at_the_attempt() {
    let machine = machine();
    let configuration = BTreeSet::new();
    let data = seeded(&[("x", Value::from(1))]);
    let scope = Scope {
        data: &data,
        chart: machine.chart(),
        configuration: &configuration,
    };
    let executed = execute(&scope, &program("x = 5; _sessionid = 'other'; x = 9"));
    assert_eq!(
        executed.failure,
        Some(Failure::SystemVariable("_sessionid".to_owned()))
    );
    assert_eq!(get(&executed.data, "x"), Some(&Value::from(5)));
    assert_eq!(get(&executed.data, "_sessionid"), Some(&Value::from("s-1")));
    let nested = execute(&scope, &program("_event.name = 'x'"));
    assert_eq!(
        nested.failure,
        Some(Failure::SystemVariable("_event".to_owned()))
    );
}

// Sabotage: with the system_changed check dropped from execute, a fresh
// underscore root is merged and no failure is answered, so this fails.
#[test]
fn a_fresh_system_root_is_found_after_the_run_and_never_kept() {
    let machine = machine();
    let configuration = BTreeSet::new();
    let data = seeded(&[]);
    let scope = Scope {
        data: &data,
        chart: machine.chart(),
        configuration: &configuration,
    };
    let executed = execute(&scope, &program("_zed = 1; _alpha = 2; x = _alpha + 1"));
    assert_eq!(
        executed.failure,
        Some(Failure::SystemVariable("_alpha".to_owned()))
    );
    assert_eq!(get(&executed.data, "x"), Some(&Value::from(3)));
    assert_eq!(get(&executed.data, "_alpha"), None);
    assert_eq!(get(&executed.data, "_zed"), None);
}

// Sabotage: with run_failure mapping every error to SystemVariable, the
// division failure loses its source and this fails.
#[test]
fn a_program_that_stops_keeps_the_writes_before_the_stop() {
    let machine = machine();
    let configuration = BTreeSet::new();
    let data = seeded(&[]);
    let scope = Scope {
        data: &data,
        chart: machine.chart(),
        configuration: &configuration,
    };
    let source = "a = 1; b = 1 / 0";
    let executed = execute(&scope, &program(source));
    let Some(Failure::Evaluation {
        source: failed,
        error,
    }) = &executed.failure
    else {
        panic!("the run stops on its division");
    };
    assert_eq!(failed, source);
    assert_eq!(error.reason(), "division_by_zero");
    assert_eq!(get(&executed.data, "a"), Some(&Value::from(1)));
    assert_eq!(get(&executed.data, "b"), None);
}

// Sabotage: with protected_roots keeping every root, x is protected and
// this fails.
#[test]
fn the_protected_roots_are_the_underscore_roots_the_datamodel_holds() {
    let data = seeded(&[("x", Value::from(1))]);
    let mut roots = protected_roots(&data);
    roots.sort();
    assert_eq!(roots, ["_event", "_ioprocessors", "_name", "_sessionid"]);
    assert!(system_root("_x") && !system_root("x"));
}

// Sabotage: with the leading-dot rewrite removed from milliseconds, ".5s"
// is refused and this fails.
#[test]
fn a_delay_is_read_through_the_duration_reader() {
    assert_eq!(milliseconds(&Value::from("1.5s")), Ok(1_500));
    assert_eq!(milliseconds(&Value::from(".5s")), Ok(500));
    assert_eq!(milliseconds(&Value::from("2w")), Ok(1_209_600_000));
    assert_eq!(milliseconds(&Value::from("1mo1y")), Ok(34_128_000_000));
    assert_eq!(milliseconds(&Value::from("1d1h1m1s1ms")), Ok(90_061_001));
    assert_eq!(
        milliseconds(&Value::from("soon")),
        Err(Failure::InvalidDelay(Value::from("soon")))
    );
    assert_eq!(
        milliseconds(&Value::from(5)),
        Err(Failure::InvalidDelay(Value::from(5)))
    );
    let duration = predicator_rs::Duration {
        minutes: 2,
        milliseconds: -5,
        ..predicator_rs::Duration::default()
    };
    assert_eq!(milliseconds(&Value::Duration(duration)), Ok(119_995));
    assert_eq!(milliseconds(&Value::Map(Map::new())), Ok(0));
    let huge = predicator_rs::Duration {
        years: i64::MAX,
        ..predicator_rs::Duration::default()
    };
    assert_eq!(
        milliseconds(&Value::Duration(huge)),
        Err(Failure::InvalidDelay(Value::Duration(huge)))
    );
}

// Sabotage: with Failure::event naming the event "error.platform", or
// typing it internal, this fails.
#[test]
fn a_failure_raises_error_execution_carrying_its_reason() {
    let machine = machine();
    let configuration = BTreeSet::new();
    let data = seeded(&[]);
    let scope = Scope {
        data: &data,
        chart: machine.chart(),
        configuration: &configuration,
    };
    let failure = evaluate(&scope, &expr("nope")).unwrap_err();
    let event = failure.event();
    assert_eq!(field(&event, "name"), &Value::from("error.execution"));
    assert_eq!(field(&event, "type"), &Value::from("platform"));
    assert_eq!(field(&event, "sendid"), &Value::Undefined);
    let reason = field(&event, "data");
    assert_eq!(field(reason, "kind"), &Value::from("evaluator_error"));
    assert_eq!(field(reason, "source"), &Value::from("nope"));
    assert_eq!(
        field(reason, "type"),
        &Value::from("UndefinedVariableError")
    );
    let kinds = [
        (Failure::NonBooleanCond(Value::from(1)), "non_boolean_cond"),
        (Failure::SystemVariable("_x".to_owned()), "system_variable"),
        (Failure::InvalidDelay(Value::from("x")), "invalid_delay"),
    ];
    for (failure, kind) in kinds {
        assert_eq!(field(&failure.data(), "kind"), &Value::from(kind));
    }
    assert_eq!(
        field(&Failure::SystemVariable("_x".to_owned()).data(), "root"),
        &Value::from("_x")
    );
    let refusals = crate::compile("<scxml").unwrap_err();
    let compile = Failure::Compile(Box::new(refusals[0].clone()));
    assert_eq!(
        field(&compile.data(), "kind"),
        &Value::from("compile_error")
    );
}

// Sabotage: with the re-export pointing at a statifier-owned type, the
// value built here no longer is predicator's and this fails to compile.
#[test]
fn the_value_type_is_predicators() {
    let value: crate::Value = predicator_rs::Value::from(1);
    assert_eq!(value, predicator_rs::Value::Integer(1));
}
