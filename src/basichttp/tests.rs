use super::*;
use std::cell::RefCell;

// A double that records each request and answers by address.
struct Double {
    seen: RefCell<Vec<Request>>,
}

impl Double {
    fn new() -> Self {
        Self {
            seen: RefCell::new(Vec::new()),
        }
    }
}

impl Transport for Double {
    fn send(&self, request: Request) -> Result<Response, TransportError> {
        self.seen.borrow_mut().push(request.clone());
        if request.address.ends_with("/answer/error") {
            return Err(TransportError::with_source(
                "connection failed",
                std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused"),
            ));
        }
        if let Some((_, status)) = request.address.rsplit_once("/answer/") {
            if let Ok(status) = status.parse::<u16>() {
                return Ok(Response::new(status, Vec::new(), Vec::new()));
            }
        }
        Ok(Response::new(204, Vec::new(), request.body))
    }
}

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn through(transport: &dyn Transport, request: Request) -> Result<Response, TransportError> {
    transport.send(request)
}

// Sabotage: swap method and address in Request::new and this goes red.
#[test]
fn a_request_reaches_the_transport_unchanged() {
    let headers = pairs(&[("X-A", "1"), ("X-B", "2"), ("X-A", "3")]);
    let request = Request::new("POST", "http://localhost:8080/s1", headers.clone(), "e.go");
    assert_eq!(request.method, "POST");
    assert_eq!(request.address, "http://localhost:8080/s1");
    assert_eq!(request.headers, headers);
    assert_eq!(request.body, b"e.go".to_vec());

    let double = Double::new();
    let response = double.send(request.clone()).unwrap();
    assert_eq!(response.status, 204);
    assert_eq!(response.body, b"e.go".to_vec());
    assert_eq!(*double.seen.borrow(), vec![request]);
}

// Sabotage: make Response::new set every status to 200 and this goes red.
#[test]
fn a_status_outside_2xx_is_a_response_not_an_error() {
    let double = Double::new();
    let request = Request::new("GET", "http://localhost:8080/answer/503", Vec::new(), "");
    let response = double.send(request).expect("a 503 is a response");
    assert_eq!(response.status, 503);

    let headers = pairs(&[("A", "1"), ("A", "2")]);
    let made = Response::new(404, headers.clone(), "gone");
    assert_eq!(made.status, 404);
    assert_eq!(made.headers, headers);
    assert_eq!(made.body, b"gone".to_vec());
}

// Sabotage: make TransportError's Display write nothing and this goes red.
#[test]
fn a_transport_error_displays_its_message_and_answers_its_cause() {
    let double = Double::new();
    let request = Request::new("GET", "http://localhost:8080/answer/error", Vec::new(), "");
    let error = double.send(request).unwrap_err();
    assert_eq!(error.to_string(), "connection failed");
    assert_eq!(error.message(), "connection failed");
    let source = Error::source(&error).expect("a cause");
    let io = source
        .downcast_ref::<std::io::Error>()
        .expect("an io error");
    assert_eq!(io.kind(), std::io::ErrorKind::ConnectionRefused);

    let bare = TransportError::new("no cause");
    assert_eq!(bare.to_string(), "no cause");
    assert!(Error::source(&bare).is_none());
}

// Sabotage: make Response::new set every status to 200 and this goes red.
#[test]
fn the_double_is_usable_as_a_dyn_transport() {
    let double = Double::new();
    let response = through(
        &double,
        Request::new("POST", "http://localhost:8080/answer/418", Vec::new(), ""),
    )
    .unwrap();
    assert_eq!(response.status, 418);
    assert_eq!(double.seen.borrow().len(), 1);
}

// Sabotage: make TransportError::with_source drop its cause and this goes red.
#[test]
fn a_clone_of_a_transport_error_keeps_the_message_and_the_cause() {
    let original = TransportError::with_source(
        "connection failed",
        std::io::Error::new(std::io::ErrorKind::TimedOut, "slow"),
    );
    let copy = original.clone();
    assert_eq!(copy.message(), "connection failed");
    let io = Error::source(&copy)
        .and_then(|source| source.downcast_ref::<std::io::Error>())
        .expect("the cause survives a clone");
    assert_eq!(io.kind(), std::io::ErrorKind::TimedOut);
}

// The processor over the driver: registration, delivery and the miss.

const OPEN: &str = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0""#;

fn machine(body: &str) -> crate::Machine {
    let text = format!(r#"{OPEN} initial="a">{body}</scxml>"#);
    match crate::compile(&text) {
        Ok(machine) => machine,
        Err(refusals) => panic!("{text}: {refusals:#?}"),
    }
}

fn started(body: &str) -> Moved {
    machine(body)
        .start(register(Options::new("s-1"), "http://h/base"))
        .expect("a start")
}

// A chart whose state `a` makes one send of `send_type` to `target`, and
// moves to `failed` on its `error.communication`.
fn sender(send_type: &str, target: &str) -> String {
    format!(
        r#"<state id="a">
             <onentry><send id="k" event="e.go" type="{send_type}" {target}/></onentry>
             <transition event="error.communication" cond="_event.sendid == 'k'" target="failed"/>
           </state>
           <state id="failed"/>"#
    )
}

fn sends(effects: &[Effect]) -> Vec<Effect> {
    effects
        .iter()
        .filter(|effect| match effect {
            Effect::Send { .. } | Effect::SendDelayed { .. } => true,
            Effect::Child { effect, .. } => matches!(**effect, Effect::Send { .. }),
            _ => false,
        })
        .cloned()
        .collect()
}

fn target(send: &Effect) -> Option<Value> {
    match send {
        Effect::Send { target, .. } => target.clone(),
        Effect::Child { effect, .. } => target(effect),
        _ => None,
    }
}

// Sabotage: with a session location written without the session's id, the
// targets read `http://h/base` and this goes red.
#[test]
fn register_writes_a_location_under_both_spellings() {
    let moved = started(&format!(
        r#"<state id="a"><onentry>
             <send event="x" type="basichttp" targetexpr="_ioprocessors['basichttp']['location']"/>
             <send event="y" type="{URI}" targetexpr="_ioprocessors['{URI}']['location']"/>
           </onentry></state>"#
    ));
    let targets: Vec<Option<Value>> = sends(&moved.effects).iter().map(target).collect();
    assert_eq!(
        targets,
        vec![
            Some(Value::from("http://h/base/s-1")),
            Some(Value::from("http://h/base/s-1"))
        ]
    );
    let options = register(Options::new("s-1"), "http://h/base");
    assert_eq!(
        options.send_types.iter().collect::<Vec<_>>(),
        [SHORT_FORM, URI]
    );
}

// Sabotage: with the child's entry written for its parent's session, the
// child's target reads `http://h/base/s-1`.
#[test]
fn an_invoked_childs_location_names_the_child() {
    let moved = started(
        r#"<state id="a"><invoke id="i1"><content><scxml version="1.0" initial="c">
             <state id="c"><onentry>
               <send event="x" type="basichttp" targetexpr="_ioprocessors['basichttp']['location']"/>
             </onentry></state>
           </scxml></content></invoke></state>"#,
    );
    let child_sends = sends(&moved.effects);
    assert_eq!(child_sends.len(), 1);
    assert!(matches!(&child_sends[0], Effect::Child { session_id, .. } if session_id == "s-1.i1"));
    assert_eq!(
        target(&child_sends[0]),
        Some(Value::from("http://h/base/s-1.i1"))
    );
}

// Sabotage: with deliver building the request for the wrong session, the
// key's first field reads otherwise and this goes red.
#[test]
fn a_2xx_response_is_delivered_and_moves_nothing() {
    let moved = started(&sender("basichttp", r#"target="http://h/s""#));
    let send = &sends(&moved.effects)[0];
    let before = send.clone();
    let double = Double::new();
    let delivery = deliver(&moved.state, send, &double).expect("a delivery");
    assert!(matches!(delivery, Delivery::Delivered { response } if response.status == 204));
    assert_eq!(*send, before);
    assert_eq!(moved.state.configuration(), ["a"]);
    let seen = double.seen.borrow();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].address, "http://h/s");
    assert_eq!(seen[0].body, b"_scxmleventname=e.go".to_vec());
    assert_eq!(
        seen[0].headers,
        pairs(&[
            ("content-type", "application/x-www-form-urlencoded"),
            ("scxml-send-key", "s-1/k/1/1/0/0/onentry.1.0/1"),
        ])
    );
}

// Sabotage: with a status outside 2xx answered as delivered, every row goes
// red; with the range checked as 200..299 exclusive, the 299 row does.
#[test]
fn a_status_outside_2xx_is_a_miss_the_chart_takes() {
    for status in [0_u16, 99, 100, 199, 300, 404, 500, 599, 600, u16::MAX] {
        let moved = started(&sender(
            "basichttp",
            &format!(r#"target="http://h/answer/{status}""#),
        ));
        let send = &sends(&moved.effects)[0];
        let delivery = deliver(&moved.state, send, &Double::new()).expect("a delivery");
        let Delivery::Missed {
            miss: Miss::Status { response },
            moved: Some(reported),
        } = delivery
        else {
            panic!("status {status}: {delivery:?}");
        };
        assert_eq!(response.status, status);
        assert_eq!(
            reported.state.configuration(),
            ["failed"],
            "status {status}"
        );
    }
    for status in [200_u16, 299] {
        let moved = started(&sender(
            "basichttp",
            &format!(r#"target="http://h/answer/{status}""#),
        ));
        let send = &sends(&moved.effects)[0];
        let delivery = deliver(&moved.state, send, &Double::new()).expect("a delivery");
        assert!(
            matches!(delivery, Delivery::Delivered { .. }),
            "status {status}"
        );
    }
}

// Sabotage: with a transport error answered as delivered, this goes red.
#[test]
fn a_transport_error_is_a_miss_the_chart_takes() {
    let moved = started(&sender(URI, r#"target="http://h/answer/error""#));
    let send = &sends(&moved.effects)[0];
    let delivery = deliver(&moved.state, send, &Double::new()).expect("a delivery");
    let Delivery::Missed {
        miss: Miss::Transport { error },
        moved: Some(reported),
    } = delivery
    else {
        panic!("{delivery:?}");
    };
    assert_eq!(error.message(), "connection failed");
    assert_eq!(reported.state.configuration(), ["failed"]);
}

// Sabotage: with a send of no target handed to the transport, the double
// records a request and this goes red.
#[test]
fn a_send_with_no_target_is_a_miss_and_requests_nothing() {
    let moved = started(&sender("basichttp", ""));
    let send = &sends(&moved.effects)[0];
    let double = Double::new();
    let delivery = deliver(&moved.state, send, &double).expect("a delivery");
    let Delivery::Missed {
        miss: Miss::NoTarget,
        moved: Some(reported),
    } = delivery
    else {
        panic!("{delivery:?}");
    };
    assert_eq!(reported.state.configuration(), ["failed"]);
    assert!(double.seen.borrow().is_empty());
}

// Sabotage: with deliver taking a send of any registered type, the first
// refusal is answered as a delivery.
#[test]
fn a_send_of_another_type_or_another_effect_is_refused() {
    let machine = machine(&sender("x-other", r#"target="http://h/s""#));
    let mut options = register(Options::new("s-1"), "http://h/base");
    options.send_types.insert("x-other".to_owned());
    let moved = machine.start(options).expect("a start");
    let send = &sends(&moved.effects)[0];
    let double = Double::new();
    assert!(matches!(
        deliver(&moved.state, send, &double),
        Err(crate::Error::NotAHandedSend)
    ));
    let not_a_send = moved
        .effects
        .iter()
        .find(|effect| !matches!(effect, Effect::Send { .. }))
        .expect("another effect");
    assert!(matches!(
        deliver(&moved.state, not_a_send, &double),
        Err(crate::Error::NotAHandedSend)
    ));
    assert!(double.seen.borrow().is_empty());
}

// Sabotage: with a refused report answered as an error, the stopped chart's
// miss is an Err and this goes red.
#[test]
fn a_miss_from_a_stopped_chart_carries_no_report() {
    let moved = machine(
        r#"<final id="a"><onentry>
             <send type="basichttp" target="http://h/answer/500"/>
           </onentry></final>"#,
    )
    .start(register(Options::new("s-1"), "http://h/base"))
    .expect("a start");
    assert!(moved.state.done().is_some());
    let send = &sends(&moved.effects)[0];
    let delivery = deliver(&moved.state, send, &Double::new()).expect("a delivery");
    assert!(matches!(
        delivery,
        Delivery::Missed {
            miss: Miss::Status { .. },
            moved: None
        }
    ));
}

// Sabotage: with a child's miss reported to its parent, the child logs
// nothing and this goes red.
#[test]
fn a_childs_miss_is_reported_to_the_child() {
    let moved = started(
        r#"<state id="a"><invoke id="i1"><content><scxml version="1.0" initial="c">
             <state id="c">
               <onentry><send id="k" type="basichttp" target="http://h/answer/503"/></onentry>
               <transition event="error.communication" target="m"/>
             </state>
             <state id="m"><onentry><log label="missed" expr="_event.sendid"/></onentry></state>
           </scxml></content></invoke></state>"#,
    );
    let send = &sends(&moved.effects)[0];
    let delivery = deliver(&moved.state, send, &Double::new()).expect("a delivery");
    let Delivery::Missed {
        moved: Some(reported),
        ..
    } = delivery
    else {
        panic!("{delivery:?}");
    };
    assert_eq!(reported.state.configuration(), ["a"]);
    assert!(reported.effects.iter().any(|effect| matches!(
        effect,
        Effect::Child { session_id, effect }
            if session_id == "s-1.i1"
                && matches!(&**effect, Effect::Log { label: Some(label), value: Some(value), .. }
                    if label == "missed" && *value == Value::from("k"))
    )));
}

// The doc example's round trip with a delayed send: the host delivers it
// when its delay has passed. Sabotage: with a delayed send refused, this
// goes red.
#[test]
fn a_delayed_send_is_delivered_when_the_host_calls() {
    let moved = started(
        r#"<state id="a"><onentry>
             <send event="e.go" type="basichttp" delay="1s" target="http://h/s"/>
           </onentry></state>"#,
    );
    let send = &sends(&moved.effects)[0];
    assert!(matches!(send, Effect::SendDelayed { .. }));
    let double = Double::new();
    let delivery = deliver(&moved.state, send, &double).expect("a delivery");
    assert!(matches!(delivery, Delivery::Delivered { .. }));
    assert_eq!(
        double.seen.borrow()[0].headers[1].1,
        "s-1/send_1/1/1/0/0/onentry.1.0/1"
    );
}

// Sabotage: with Miss's Display writing nothing, this goes red.
#[test]
fn a_miss_and_a_decode_error_display_what_they_are() {
    assert_eq!(Miss::NoTarget.to_string(), "the send names no target");
    assert_eq!(
        Miss::Status {
            response: Response::new(500, Vec::new(), Vec::new())
        }
        .to_string(),
        "the server answered 500"
    );
    assert_eq!(
        Miss::Transport {
            error: TransportError::new("down")
        }
        .to_string(),
        "the transport failed: down"
    );
    assert_eq!(
        DecodeError::MethodNotAllowed {
            method: "GET".to_owned()
        }
        .to_string(),
        "the method \"GET\" is not allowed"
    );
    assert_eq!(
        DecodeError::NotUtf8 { part: Part::Query }.to_string(),
        "the query is not UTF-8"
    );
    assert_eq!(
        DecodeError::NotUtf8 { part: Part::Body }.to_string(),
        "the body is not UTF-8"
    );
    assert_eq!(
        DecodeError::MalformedSendKey {
            value: "x".to_owned()
        }
        .to_string(),
        "the send key \"x\" is malformed"
    );
}
