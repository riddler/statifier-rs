### Added

- `Effect::Send`, `Effect::SendDelayed` and `Effect::Cancel`: a `<send>` answers the message it sends (its event, target, type, data and send id, a delayed one with its delay in milliseconds) for the host to deliver, the internal queue included, and a `<cancel>` answers the send id it cancels; each carries its ordinal from the session's timer counter where the reference stamps one, and a send refused for its type or its target raises `error.execution` or `error.communication` carrying its send id instead.
