//! The specification's Basic HTTP Event I/O Processor, and the seam a host
//! fills to carry its sends.
//!
//! The processor is host-side code over the driver's ordinary surface, as
//! the reference's `Statifier.Send.BasicHTTP` is (statifier-ex):
//!
//! - [`register`] registers it in a host's [`Options`] under its URI
//!   ([`URI`]) and its short form ([`SHORT_FORM`]), with the base address
//!   the host's own front answers at. The chart's `_ioprocessors` then
//!   carries an entry under each spelling whose `location` is that address,
//!   `/` and the session's id.
//! - [`deliver`] takes one send of either spelling the driver handed the
//!   host, encodes it as the reference encodes it and hands the request to
//!   the host's [`Transport`], passed per call and never kept. A transport
//!   error, a status outside 2xx, or a send with no target, is reported to
//!   the chart as [`State::report_send_failed`] reports it.
//! - [`decode`] reads a request the host's front received into the name and
//!   data of an external event, which the host passes to [`State::step`].
//!
//! The crate opens no socket and ships no transport, the standard library
//! having no HTTP client: a host implements [`Transport`] over a client it
//! already has. The module exists only behind the `basichttp` feature, off
//! by default, and the feature adds no dependency.
//!
//! ```
//! use statifier::basichttp::{self, Delivery, Request, Response, Transport, TransportError};
//! use statifier::{Effect, Options};
//! use std::cell::RefCell;
//!
//! // A transport that keeps each request in memory and answers 204.
//! struct Kept(RefCell<Vec<Request>>);
//!
//! impl Transport for Kept {
//!     fn send(&self, request: Request) -> Result<Response, TransportError> {
//!         self.0.borrow_mut().push(request);
//!         Ok(Response::new(204, Vec::new(), Vec::new()))
//!     }
//! }
//!
//! let machine = statifier::compile(
//!     r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="a">
//!          <state id="a">
//!            <onentry>
//!              <send event="e.go" type="basichttp"
//!                    targetexpr="_ioprocessors['basichttp']['location']"/>
//!            </onentry>
//!            <transition event="e.go" target="b"/>
//!          </state>
//!          <final id="b"/>
//!        </scxml>"#,
//! )
//! .map_err(|refusals| format!("{refusals:?}"))?;
//! let options = basichttp::register(Options::new("s-1"), "http://localhost:8080/scxml");
//! let started = machine.start(options)?;
//! let send = started
//!     .effects
//!     .iter()
//!     .find(|effect| matches!(effect, Effect::Send { .. }))
//!     .ok_or("the send is handed to the host")?;
//!
//! let transport = Kept(RefCell::new(Vec::new()));
//! let delivery = basichttp::deliver(&started.state, send, &transport)?;
//! assert!(matches!(delivery, Delivery::Delivered { .. }));
//!
//! // The host's front receives the request and steps the chart with it.
//! let request = transport.0.borrow()[0].clone();
//! assert_eq!(request.address, "http://localhost:8080/scxml/s-1");
//! assert_eq!(request.body, b"_scxmleventname=e.go".to_vec());
//! let received = basichttp::decode(&request)?;
//! let moved = started.state.step(&received.name, received.data)?;
//! assert!(moved.state.done().is_some());
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod decode;
mod encode;

use crate::{Effect, IoProcessorEntry, Moved, Options, State, Value};
use std::error::Error;
use std::fmt;
use std::sync::Arc;

/// The processor's URI, the send type the specification names it by.
pub const URI: &str = "http://www.w3.org/TR/scxml/#BasicHTTPEventProcessor";

/// The processor's short form, the send type and `_ioprocessors` key a
/// chart may name it by.
pub const SHORT_FORM: &str = "basichttp";

/// `options` with the processor registered under [`URI`] and
/// [`SHORT_FORM`], each with the `_ioprocessors` entry
/// [`IoProcessorEntry::SessionLocation`] writes from `base_address`: a
/// `location` that is the address, `/` and the session's id. Any entry
/// `options` already gives either spelling is replaced; nothing else
/// changes.
///
/// ```
/// use statifier::basichttp;
/// use statifier::{IoProcessorEntry, Options};
///
/// let options = basichttp::register(Options::new("s-1"), "http://localhost:8080/scxml");
/// assert!(options.send_types.contains(basichttp::URI));
/// assert_eq!(
///     options.ioprocessors.get(basichttp::SHORT_FORM),
///     Some(&IoProcessorEntry::SessionLocation("http://localhost:8080/scxml".to_owned()))
/// );
/// ```
pub fn register(options: Options, base_address: &str) -> Options {
    let mut options = options;
    for send_type in [URI, SHORT_FORM] {
        options.send_types.insert(send_type.to_owned());
        options.ioprocessors.insert(
            send_type.to_owned(),
            IoProcessorEntry::SessionLocation(base_address.to_owned()),
        );
    }
    options
}

/// What [`deliver`] answers for a send it took.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum Delivery {
    /// The server answered a 2xx status. The chart is not moved.
    #[non_exhaustive]
    Delivered {
        /// The response.
        response: Response,
    },
    /// The send missed, and the sending session took `error.communication`
    /// for it, carrying the send's id, as [`State::report_send_failed`]
    /// reports it.
    #[non_exhaustive]
    Missed {
        /// Why.
        miss: Miss,
        /// The report: the new state and the effects it produced, boxed. `None`
        /// when the sending session takes no report, having stopped or
        /// been halted by a spent round budget since it made the send.
        moved: Option<Box<Moved>>,
    },
}

/// Why a send missed.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum Miss {
    /// The send named no target, so there was no address to request, and
    /// no request was made.
    NoTarget,
    /// The server answered a status outside 2xx. A status outside the 100
    /// to 599 HTTP defines is a miss the same way.
    #[non_exhaustive]
    Status {
        /// The response.
        response: Response,
    },
    /// The transport answered an error: no response came back.
    #[non_exhaustive]
    Transport {
        /// The transport's error.
        error: TransportError,
    },
}

impl fmt::Display for Miss {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Miss::NoTarget => f.write_str("the send names no target"),
            Miss::Status { response } => write!(f, "the server answered {}", response.status),
            Miss::Transport { error } => write!(f, "the transport failed: {error}"),
        }
    }
}

/// Delivers `send`, a send of [`URI`] or [`SHORT_FORM`] the driver handed
/// the host from `state`: encodes it as the reference encodes it, hands the
/// request to `transport` once, and reports a miss to the chart.
///
/// `send` is the [`Effect::Send`] or [`Effect::SendDelayed`] as the host
/// was handed it, or, for an invoked child's send, the [`Effect::Child`]
/// carrying it. A delayed send is delivered at once: the host calls this
/// when its delay has passed, unless a `<cancel>` named it first.
///
/// The request is a `POST` to the send's target. A `data` that is a map,
/// the `namelist` and `<param>` values or a `<content>` whose value is a
/// map, is a form body (`application/x-www-form-urlencoded`): the event's
/// name as `_scxmleventname` first, then each member in the order the
/// reference walks a map of up to 32 members (integer keys, then `false`,
/// `true` and undefined, then string keys, each in its own order). No data
/// sends `_scxmleventname` alone as a form body. Any other `<content>` is a
/// `text/plain` body, and the event's name travels as a `_scxmleventname`
/// query parameter of the target. Names and values are percent-encoded,
/// every byte outside letters, digits, `-`, `.`, `_` and `~`, a space as
/// `+`. A value is written as text: a string as it is, a number or a
/// boolean as its literal, null as `null`, undefined as the empty string,
/// a list or a map as JSON text. The request carries the `content-type`
/// header and the `scxml-send-key` header, the reference's dedup key: the
/// session's id, the send id, the macrostep, microstep and round, the
/// content index, the owner and the ordinal, joined by `/`.
///
/// A 2xx response answers [`Delivery::Delivered`]. A send with no target,
/// a transport error, and a response whose status is outside 2xx answer
/// [`Delivery::Missed`]: the miss is reported with
/// [`State::report_send_failed`], whose new state the delivery carries.
///
/// Refused, with nothing requested, as [`State::report_send_failed`]
/// refuses an effect that is not a send handed to the host, and with
/// [`crate::Error::NotAHandedSend`] for a send of another type.
pub fn deliver(
    state: &State,
    send: &Effect,
    transport: &dyn Transport,
) -> Result<Delivery, crate::Error> {
    let (session_id, bare) = state.handed_send(send)?;
    let ours = match bare {
        Effect::Send { send_type, .. } | Effect::SendDelayed { send_type, .. } => {
            matches!(send_type, Some(Value::String(send_type)) if send_type == URI || send_type == SHORT_FORM)
        }
        _ => false,
    };
    if !ours {
        return Err(crate::Error::NotAHandedSend);
    }
    let miss = match encode::request(session_id, bare) {
        None => Miss::NoTarget,
        Some(request) => match transport.send(request) {
            Ok(response) if (200..=299).contains(&response.status) => {
                return Ok(Delivery::Delivered { response });
            }
            Ok(response) => Miss::Status { response },
            Err(error) => Miss::Transport { error },
        },
    };
    let moved = state
        .report_send_failed(send, Some(&miss.to_string()))
        .ok()
        .map(Box::new);
    Ok(Delivery::Missed { miss, moved })
}

/// An external event read from a request: the host passes its name and
/// data to [`State::step`].
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub struct Received {
    /// The event's name.
    pub name: String,
    /// The event's data.
    pub data: Value,
}

/// Which part of a request [`DecodeError::NotUtf8`] names.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Part {
    /// The query string of the address.
    Query,
    /// The body.
    Body,
}

/// Why [`decode`] read no event from a request.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// The method is not `POST`; a front answers 405 with `Allow: POST`.
    #[non_exhaustive]
    MethodNotAllowed {
        /// The method as the request gave it.
        method: String,
    },
    /// The part is not UTF-8 once its percent escapes are decoded.
    #[non_exhaustive]
    NotUtf8 {
        /// The part.
        part: Part,
    },
    /// The `scxml-send-key` header is not eight fields joined by `/` whose
    /// second is UTF-8 once decoded.
    #[non_exhaustive]
    MalformedSendKey {
        /// The header's value.
        value: String,
    },
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::MethodNotAllowed { method } => {
                write!(f, "the method {method:?} is not allowed")
            }
            DecodeError::NotUtf8 { part: Part::Query } => f.write_str("the query is not UTF-8"),
            DecodeError::NotUtf8 { part: Part::Body } => f.write_str("the body is not UTF-8"),
            DecodeError::MalformedSendKey { value } => {
                write!(f, "the send key {value:?} is malformed")
            }
        }
    }
}

impl Error for DecodeError {}

/// Reads a request a host's front received into an external event, as the
/// reference reads one. Pure: it knows no session, so the front resolves
/// the address to the session and steps it.
///
/// - The method must be `POST` in any case.
/// - The event's name is the first `_scxmleventname`, the query's before
///   the body's, else `HTTP.` and the method in upper case (`HTTP.POST`).
/// - A form body (a `content-type` header starting
///   `application/x-www-form-urlencoded`, in any case) and the query give
///   their other parameters as the data, a map, the last of a repeated name
///   kept, each value read as a `<content>` text is (a predicator literal,
///   else the text with its whitespace runs one space), a parameter with
///   no `=` undefined; with none the data is undefined.
/// - Any other body is the data, read the same way, and the query then
///   gives the name alone.
/// - An `scxml-send-key` header, the dedup key, sets nothing: a front
///   deduplicates on it itself. Only its shape is checked.
///
/// The query is what follows the address's first `?`, up to a `#`. The
/// first header of a name counts, the name read in any case.
///
/// ```
/// use statifier::basichttp::{self, Request};
/// use statifier::Value;
///
/// let request = Request::new(
///     "POST",
///     "http://localhost:8080/scxml/s-1",
///     vec![("content-type".to_owned(), "application/x-www-form-urlencoded".to_owned())],
///     "_scxmleventname=e.go&n=2",
/// );
/// let received = basichttp::decode(&request)?;
/// assert_eq!(received.name, "e.go");
/// let Value::Map(data) = &received.data else { panic!("a map") };
/// assert_eq!(data.get(&"n".into()), Some(&Value::from(2)));
/// # Ok::<(), basichttp::DecodeError>(())
/// ```
pub fn decode(request: &Request) -> Result<Received, DecodeError> {
    decode::decode(request)
}

/// One HTTP request: one a transport sends, or one a front received.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Request {
    /// The HTTP method token, such as `POST`.
    pub method: String,
    /// The absolute URL the request goes to.
    pub address: String,
    /// The header names and values, in the order they are sent. A name may
    /// repeat.
    pub headers: Vec<(String, String)>,
    /// The body bytes, already encoded; empty for none.
    pub body: Vec<u8>,
}

impl Request {
    /// Builds a request from its method, address, headers and body.
    pub fn new(
        method: impl Into<String>,
        address: impl Into<String>,
        headers: Vec<(String, String)>,
        body: impl Into<Vec<u8>>,
    ) -> Self {
        Self {
            method: method.into(),
            address: address.into(),
            headers,
            body: body.into(),
        }
    }
}

/// The response a server answered.
///
/// Every status the server answered is a response, 2xx or not (100 to 599
/// are HTTP's). A status outside 2xx is answered as a `Response`, never as a
/// [`TransportError`].
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Response {
    /// The HTTP status code.
    pub status: u16,
    /// The header names and values, in the order received.
    pub headers: Vec<(String, String)>,
    /// The body bytes; empty for none.
    pub body: Vec<u8>,
}

impl Response {
    /// Builds a response from its status, headers and body.
    pub fn new(status: u16, headers: Vec<(String, String)>, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            headers,
            body: body.into(),
        }
    }
}

/// The one error a transport answers when no response came back: an address
/// it cannot request, a connection refused or failed, a time bound passed,
/// or an exchange broken off.
///
/// The cause, when there was one, is its [`source`](Error::source). It
/// implements no `PartialEq`, because a cause is not comparable.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct TransportError {
    message: String,
    source: Option<Arc<dyn Error + Send + Sync + 'static>>,
}

impl TransportError {
    /// Builds an error with a message and no cause.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            source: None,
        }
    }

    /// Builds an error with a message and the cause that produced it.
    pub fn with_source(
        message: impl Into<String>,
        source: impl Into<Box<dyn Error + Send + Sync + 'static>>,
    ) -> Self {
        Self {
            message: message.into(),
            source: Some(Arc::from(source.into())),
        }
    }

    /// The message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for TransportError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self.source {
            Some(source) => Some(source.as_ref()),
            None => None,
        }
    }
}

/// What a host implements to carry a request to a server.
///
/// `send` is synchronous. It makes one attempt and does not retry. It
/// answers the response that came back, whatever its status, or a
/// [`TransportError`] when none came back.
///
/// The trait is object safe and used as `&dyn Transport`. It takes `&self`
/// so one transport serves many sends; a host whose client needs mutation
/// uses interior mutability. It has no `Send` or `Sync` supertrait: a host
/// that shares one transport across threads names the bound where it holds
/// it, as `&(dyn Transport + Sync)`, and a single-threaded host's transport
/// needs neither.
///
/// ```
/// use statifier::basichttp::{Request, Response, Transport, TransportError};
///
/// struct InMemory;
///
/// impl Transport for InMemory {
///     fn send(&self, request: Request) -> Result<Response, TransportError> {
///         if request.address.starts_with("http://") {
///             Ok(Response::new(200, Vec::new(), request.body))
///         } else {
///             Err(TransportError::new("not an http address"))
///         }
///     }
/// }
///
/// let transport: &dyn Transport = &InMemory;
/// let request = Request::new("POST", "http://localhost:8080/s1", Vec::new(), "e.go");
/// assert_eq!(transport.send(request).unwrap().status, 200);
/// ```
pub trait Transport {
    /// Sends one request and answers the response that came back, or the
    /// error when none did.
    fn send(&self, request: Request) -> Result<Response, TransportError>;
}

#[cfg(test)]
mod tests;
