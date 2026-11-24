//! The Basic HTTP Event I/O Processor in the run, carried by an in-memory
//! loopback: no socket, no port.
//!
//! A case whose `host` object names the processor's URI
//! (`event_io_processors`) starts with the processor registered under its
//! URI and its short form ([`statifier::basichttp::register`]), at
//! [`BASE_ADDRESS`], as the reference's host case runner registers it
//! against the loopback front it starts for the case
//! (`with_event_io_processors/2` in `lib/mix/statifier/corpus/host_case.ex`
//! at the vendored tag). Every send of either spelling the driver hands the
//! host is delivered with [`statifier::basichttp::deliver`] through a
//! [`Loopback`] transport, whose `send` reads the request with
//! [`statifier::basichttp::decode`], answers it and keeps the event for the
//! session the address names; once the call that made the sends has
//! returned, the run steps that session with each kept event, in the order
//! received. A miss is reported to the chart by `deliver` itself.
//!
//! An exchange round is one pass of delivering the sends handed and
//! stepping the events received. A run of rounds that keeps handing new
//! sends to the processor ends at [`ROUND_BOUND`] rounds, and the case fails
//! naming the bound.
//!
//! This proves the processor's encoding and decoding and the chart's event
//! path between them, nothing about a network: no request leaves the
//! process.

use statifier::basichttp::{
    self, DecodeError, Delivery, Received, Request, Response, Transport, TransportError,
};
use statifier::{Effect, Error, Options, State, Value};
use std::cell::RefCell;
use std::collections::VecDeque;

/// The base address the processor is registered at: the chart's
/// `_ioprocessors` location is this, `/` and the session's id. The `.test`
/// name is reserved, so no network ever answers it.
pub const BASE_ADDRESS: &str = "http://loopback.test/scxml";

/// The most exchange rounds one call's sends may take before the case
/// fails.
pub const ROUND_BOUND: usize = 100;

/// The in-memory transport: it answers each request as a front answers it
/// and keeps the event it reads, with the session its address names.
#[derive(Debug)]
pub struct Loopback {
    session_id: String,
    received: RefCell<VecDeque<(String, Received)>>,
}

impl Loopback {
    /// A loopback for the case whose session is `session_id`.
    pub fn new(session_id: &str) -> Self {
        Self {
            session_id: session_id.to_owned(),
            received: RefCell::new(VecDeque::new()),
        }
    }

    /// The events received, each with the session its address names, in
    /// the order received; each taken once.
    pub fn take_received(&self) -> Vec<(String, Received)> {
        self.received.borrow_mut().drain(..).collect()
    }
}

impl Transport for Loopback {
    /// An address outside [`BASE_ADDRESS`] is an error, as an unreachable
    /// host is; a request [`basichttp::decode`] refuses for its method is
    /// answered 405 with `Allow: POST`, and one it refuses otherwise 400;
    /// a decoded event is kept and answered 204.
    fn send(&self, request: Request) -> Result<Response, TransportError> {
        let Some(session) = session_of(&request.address) else {
            return Err(TransportError::new(format!(
                "the loopback answers only under {BASE_ADDRESS}, not {}",
                request.address
            )));
        };
        match basichttp::decode(&request) {
            Ok(received) => {
                self.received.borrow_mut().push_back((session, received));
                Ok(Response::new(204, Vec::new(), Vec::new()))
            }
            Err(DecodeError::MethodNotAllowed { .. }) => Ok(Response::new(
                405,
                vec![("allow".to_owned(), "POST".to_owned())],
                Vec::new(),
            )),
            Err(_) => Ok(Response::new(400, Vec::new(), Vec::new())),
        }
    }
}

/// The session id an address under [`BASE_ADDRESS`] names: what follows
/// the base and `/`, up to a `?` or a `#`.
fn session_of(address: &str) -> Option<String> {
    let rest = address.strip_prefix(BASE_ADDRESS)?.strip_prefix('/')?;
    let end = rest.find(['?', '#']).unwrap_or(rest.len());
    Some(rest[..end].to_owned())
}

/// Whether `effect` is a send of the processor's URI or short form, itself
/// or carried by an [`Effect::Child`].
pub fn is_processor_send(effect: &Effect) -> bool {
    match effect {
        Effect::Child { effect, .. } => is_processor_send(effect),
        Effect::Send { send_type, .. } | Effect::SendDelayed { send_type, .. } => matches!(
            send_type,
            Some(Value::String(send_type)) if send_type == basichttp::URI || send_type == basichttp::SHORT_FORM
        ),
        _ => false,
    }
}

/// Whether `effect` is a delayed send, itself or carried by an
/// [`Effect::Child`].
fn is_delayed(effect: &Effect) -> bool {
    match effect {
        Effect::Child { effect, .. } => is_delayed(effect),
        Effect::SendDelayed { .. } => true,
        _ => false,
    }
}

/// `options` with the processor registered at [`BASE_ADDRESS`], and the
/// loopback that carries its sends, when the case's `processors` name the
/// processor's URI; else `options` as they are and no loopback.
pub fn attach(processors: &[String], options: Options) -> (Options, Option<Loopback>) {
    if !processors.iter().any(|name| name == basichttp::URI) {
        return (options, None);
    }
    let loopback = Loopback::new(&options.session_id);
    (basichttp::register(options, BASE_ADDRESS), Some(loopback))
}

/// One exchange round over `effects`, the effects one call handed the host
/// from `state`: each processor send is delivered through `loopback`, a
/// miss reported by the delivery itself, then the root session is stepped
/// with each event received, in order. The effects the reports and the
/// steps hand the host are appended to `produced`. Answers the new state
/// and whether the round carried anything. An event for a session other
/// than the root fails the case, since the run steps the root alone; an
/// event for a root that has stopped goes nowhere, as a stopped session
/// takes no event.
pub fn exchange(
    loopback: &Loopback,
    state: State,
    effects: &[Effect],
    produced: &mut Vec<Effect>,
) -> Result<(State, bool), String> {
    let mut state = state;
    let mut carried = false;
    for effect in effects.iter().filter(|effect| is_processor_send(effect)) {
        if is_delayed(effect) {
            return Err(
                "the case hands the host a delayed Basic HTTP send, which this loopback does not time"
                    .to_owned(),
            );
        }
        carried = true;
        let delivery = basichttp::deliver(&state, effect, loopback)
            .map_err(|error| format!("the Basic HTTP delivery was refused: {error}"))?;
        if let Delivery::Missed {
            moved: Some(moved), ..
        } = delivery
        {
            produced.extend(moved.effects.iter().cloned());
            state = moved.state;
        }
    }
    for (session, received) in loopback.take_received() {
        carried = true;
        if session != loopback.session_id {
            return Err(format!(
                "the loopback received {:?} for the session {session:?}, which the run cannot step",
                received.name
            ));
        }
        match state.step(&received.name, received.data) {
            Ok(moved) => {
                produced.extend(moved.effects);
                state = moved.state;
            }
            Err(Error::NotRunning) => {}
            Err(error) => {
                return Err(format!(
                    "the event {:?} the loopback received: {error}",
                    received.name
                ));
            }
        }
    }
    Ok((state, carried))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::{self, Outcome};

    /// The w3c cases the reference's registry claims for the processor.
    const CLAIMED: [&str; 11] = [
        "w3c/test509",
        "w3c/test510",
        "w3c/test518",
        "w3c/test519",
        "w3c/test520",
        "w3c/test522",
        "w3c/test531",
        "w3c/test532",
        "w3c/test534",
        "w3c/test567",
        "w3c/test577",
    ];

    // The runner carries every case naming the processor through the
    // loopback: the eleven the reference claims pass, and test201, which it
    // does not claim, fails in the configuration its own timeout reaches.
    // Sabotage: make Loopback::send keep the event under the name "x"
    // instead of the decoded one; every case but test577 goes red.
    #[test]
    fn the_runner_carries_the_basic_http_cases_through_the_loopback() {
        let corpus = crate::corpus();
        let cases = corpus.by_id();
        let named: Vec<&str> = cases
            .iter()
            .filter(|(_, item)| {
                item.get("host")
                    .and_then(|host| host.get(runner::EVENT_IO_PROCESSORS))
                    .is_some()
            })
            .map(|(id, _)| *id)
            .collect();
        let mut expected: Vec<&str> = CLAIMED.to_vec();
        expected.push("w3c/test201");
        expected.sort_unstable();
        assert_eq!(named, expected);
        for id in CLAIMED {
            assert_eq!(runner::drive(cases[id]), Outcome::Pass, "{id}");
        }
        assert_eq!(
            runner::drive(cases["w3c/test201"]),
            Outcome::Fail(
                "the initial configuration: expected active leaf states [\"pass\"], got [\"fail\"]"
                    .to_owned()
            )
        );
    }

    /// A w3c case naming the processor whose chart sends itself `e.ping`
    /// through it `pings` times in all, one exchange round each, then stops
    /// in `pass`.
    fn pinging(pings: usize) -> serde_json::Value {
        let source = format!(
            r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="a">
  <datamodel><data id="n" expr="1"/></datamodel>
  <state id="a">
    <onentry>
      <send event="e.ping" type="basichttp" targetexpr="_ioprocessors['basichttp']['location']"/>
    </onentry>
    <transition event="e.ping" cond="n &lt; {pings}">
      <assign location="n" expr="n + 1"/>
      <send event="e.ping" type="basichttp" targetexpr="_ioprocessors['basichttp']['location']"/>
    </transition>
    <transition event="e.ping" target="pass"/>
  </state>
  <final id="pass"/>
</scxml>"#
        );
        serde_json::json!({
            "id": "w3c/x",
            "suite": "w3c",
            "source": source,
            "initial_configuration": ["pass"],
            "steps": [],
            "host": {"event_io_processors": [basichttp::URI]},
        })
    }

    // The exchange is bounded at ROUND_BOUND rounds: a chart whose sends
    // take exactly that many passes, and one taking one more fails naming
    // the bound.
    // Sabotage: compare rounds with `>=` in runner::absorb; the first
    // assert_eq goes red (the case at the bound fails). Compare it with
    // ROUND_BOUND * 2; the second goes red (the case past the bound
    // passes).
    #[test]
    fn the_exchange_fails_a_case_past_its_round_bound_naming_it() {
        assert_eq!(ROUND_BOUND, 100);
        assert_eq!(runner::drive(&pinging(ROUND_BOUND)), Outcome::Pass);
        assert_eq!(
            runner::drive(&pinging(ROUND_BOUND + 1)),
            Outcome::Fail("the Basic HTTP exchange passed its bound of 100 rounds".to_owned())
        );
    }

    // The transport answers as a front does: an address outside the base
    // is an error, a method other than POST 405 with Allow, a body decode
    // refuses 400, and a decoded event is kept with the session its
    // address names, the query left out.
    // Sabotage: make session_of keep the query; the kept session's
    // assert_eq goes red.
    #[test]
    fn the_loopback_answers_a_request_as_a_front_does() {
        let loopback = Loopback::new("s-1");
        let post = |address: &str, body: &[u8]| {
            Request::new(
                "POST",
                address,
                vec![(
                    "content-type".to_owned(),
                    "application/x-www-form-urlencoded".to_owned(),
                )],
                body.to_vec(),
            )
        };
        assert!(
            loopback
                .send(post(
                    "http://elsewhere.test/scxml/s-1",
                    b"_scxmleventname=e.go"
                ))
                .is_err()
        );
        let get = Request::new("GET", format!("{BASE_ADDRESS}/s-1"), Vec::new(), "");
        let refused = loopback.send(get).unwrap();
        assert_eq!(refused.status, 405);
        assert_eq!(
            refused.headers,
            vec![("allow".to_owned(), "POST".to_owned())]
        );
        let malformed = loopback
            .send(post(&format!("{BASE_ADDRESS}/s-1"), &[0xff]))
            .unwrap();
        assert_eq!(malformed.status, 400);
        assert!(loopback.take_received().is_empty());
        let kept = loopback
            .send(post(
                &format!("{BASE_ADDRESS}/s-2?x=1"),
                b"_scxmleventname=e.go",
            ))
            .unwrap();
        assert_eq!(kept.status, 204);
        let received = loopback.take_received();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].0, "s-2");
        assert_eq!(received[0].1.name, "e.go");
        assert!(loopback.take_received().is_empty());
    }

    // An event the loopback receives for a session the run cannot step
    // fails the case naming that session, never dropped.
    // Sabotage: make exchange step the root whatever session the event
    // names; the assert_eq goes red (the chart reaches b).
    #[test]
    fn the_exchange_fails_an_event_for_a_session_it_cannot_step() {
        let source = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="a">
  <state id="a">
    <onentry>
      <send event="e.go" type="basichttp" targetexpr="_ioprocessors['basichttp']['location'] + '-other'"/>
    </onentry>
    <transition event="e.go" target="b"/>
  </state>
  <final id="b"/>
</scxml>"#;
        let case = serde_json::json!({
            "id": "w3c/x",
            "suite": "w3c",
            "source": source,
            "initial_configuration": ["b"],
            "steps": [],
            "host": {"event_io_processors": [basichttp::URI]},
        });
        assert_eq!(
            runner::drive(&case),
            Outcome::Fail(
                "the loopback received \"e.go\" for the session \"w3c/x-other\", which the run cannot step"
                    .to_owned()
            )
        );
    }
}
