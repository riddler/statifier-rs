//! The seam a host fills to carry a Basic HTTP send.
//!
//! The Basic HTTP Event I/O Processor will hand a host's transport one
//! [`Request`] per message and read back the [`Response`]. The crate opens
//! no socket and ships no transport, the standard library having no HTTP
//! client: a host implements [`Transport`] over a client it already has.
//! The module exists only behind the `basichttp` feature, off by default,
//! and the feature adds no dependency.

use std::error::Error;
use std::fmt;
use std::sync::Arc;

/// One HTTP request for a transport to send.
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
