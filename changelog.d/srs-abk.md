### Added

- `State::report_send_failed`, which reports a send of a registered type the host could not deliver: the sending session takes `error.communication` as a platform event, its `sendid` the send's id whether the document named the send or not, and runs to a stable configuration within the call. A child's send is reported through the `Effect::Child` that carried it. The reason a host passes is written into no field of the event.
- `Error::Halted`, answered by a report while a spent round budget has halted the sending session, and `Error::NotAHandedSend`, answered for an effect that is not a send of a registered type handed to the host.
