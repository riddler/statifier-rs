//! The specification's system variables (section 5.10) as datamodel
//! values, as the reference writes them. A field with no value is
//! [`Value::Undefined`], never [`Value::Null`], which is the datamodel's
//! null.

use predicator_rs::{Map, Value};

/// The SCXML Event I/O Processor's type URI, the key of its
/// `_ioprocessors` entry.
pub(crate) const SCXML_EVENT_PROCESSOR: &str = "http://www.w3.org/TR/scxml/#SCXMLEventProcessor";

/// The address a session is reached at through the SCXML Event I/O
/// Processor.
pub(crate) fn scxml_location(session_id: &str) -> String {
    format!("#_scxml_{session_id}")
}

/// The four system variables before any event. `_sessionid` is the id the
/// host minted; `_name` the chart's name, undefined when it has none;
/// `_event` declared with no value, so a read before the first event
/// answers undefined rather than failing; `_ioprocessors` an entry per
/// registered send type (`entries`, by type) and the SCXML Event I/O
/// Processor's, which a registered type of the same spelling never
/// replaces.
pub(crate) fn initial(session_id: &str, name: Option<&str>, entries: Map) -> Map {
    let mut ioprocessors = entries;
    let mut scxml = Map::new();
    scxml.insert("location", Value::from(scxml_location(session_id)));
    ioprocessors.insert(SCXML_EVENT_PROCESSOR, Value::Map(scxml));
    let mut variables = Map::new();
    variables.insert("_sessionid", Value::from(session_id));
    variables.insert("_name", absent(name));
    variables.insert("_event", Value::Undefined);
    variables.insert("_ioprocessors", Value::Map(ioprocessors));
    variables
}

/// A session's starting datamodel: the host's roots with the system
/// variables over them, so a host value under a system variable's name
/// never survives.
pub(crate) fn seed(host: Map, variables: Map) -> Map {
    let mut data = host;
    for (root, value) in variables {
        data.insert(root, value);
    }
    data
}

/// An event's type, as `_event.type` spells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EventType {
    /// Raised by the chart itself.
    Internal,
    /// Sent from outside the session, or by a `<send>`.
    External,
    /// Raised by the platform, an `error.*` event among them.
    Platform,
}

impl EventType {
    /// The spelling `_event.type` holds.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            EventType::Internal => "internal",
            EventType::External => "external",
            EventType::Platform => "platform",
        }
    }
}

/// The fields `_event` is written from (spec 5.10.1).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EventFields<'a> {
    /// The event's name.
    pub(crate) name: &'a str,
    /// The event's type.
    pub(crate) event_type: EventType,
    /// The send id, when a `<send>` raised it.
    pub(crate) sendid: Option<&'a str>,
    /// Where it came from, when known.
    pub(crate) origin: Option<&'a str>,
    /// The type of `origin`, when known.
    pub(crate) origintype: Option<&'a str>,
    /// The invoke id, when an invoked session sent it.
    pub(crate) invokeid: Option<&'a str>,
    /// The payload, as it is.
    pub(crate) data: Value,
}

/// `_event`'s value for an event: its name, its type as text, each optional
/// field undefined when the event has none, and its data as it is.
pub(crate) fn event(fields: &EventFields<'_>) -> Value {
    let mut map = Map::new();
    map.insert("name", Value::from(fields.name));
    map.insert("type", Value::from(fields.event_type.as_str()));
    map.insert("sendid", absent(fields.sendid));
    map.insert("origin", absent(fields.origin));
    map.insert("origintype", absent(fields.origintype));
    map.insert("invokeid", absent(fields.invokeid));
    map.insert("data", fields.data.clone());
    Value::Map(map)
}

/// The datamodel with `_event` set to `event` ([`event`]'s value).
pub(crate) fn put_event(data: &Map, event: Value) -> Map {
    let mut data = data.clone();
    data.insert("_event", event);
    data
}

/// A field with no value, as undefined.
fn absent(value: Option<&str>) -> Value {
    value.map_or(Value::Undefined, Value::from)
}
