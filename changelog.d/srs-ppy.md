### Added

- `basichttp::register`, which registers the Basic HTTP Event I/O Processor in a host's options under its URI (`basichttp::URI`) and its short form (`basichttp::SHORT_FORM`), with the base address whose `location` each `_ioprocessors` entry names, `/` and the session's id.
- `basichttp::deliver`, which takes a send of the processor's type the driver handed the host, encodes it as the reference encodes it (a form body or a text body, `_scxmleventname`, the same percent-encoding and field order, the `scxml-send-key` dedup header) and hands it to the host's transport, passed per call. It answers `basichttp::Delivery`: delivered on a 2xx response, or missed (`basichttp::Miss`: no target, a status outside 2xx, or a transport error), a miss reported to the chart as `error.communication`.
- `basichttp::decode`, which reads a request a host's front received into a `basichttp::Received`, the name and data of an external event for `State::step`, or refuses it with a `basichttp::DecodeError` naming a method other than `POST`, a part that is not UTF-8 (`basichttp::Part`), or a malformed send key.
- `IoProcessorEntry` and `Options::ioprocessors`, the `_ioprocessors` entry a host supplies for each registered send type: a fixed map, or a base address whose `location` names each session, an invoked child's included.
- `Error::UnregisteredEntries`, answered by `Machine::start` for an entry whose type the options do not register.

### Changed

- A registered send type's `_ioprocessors` entry is the one the host supplies in `Options::ioprocessors`; a type with none still has an empty map.
