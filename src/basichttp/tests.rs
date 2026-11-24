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
