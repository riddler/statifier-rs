### Added

- The `basichttp` feature, off by default, adds the `basichttp` module: the seam a host fills to carry a Basic HTTP send. It adds no dependency, and the crate ships no transport.
- `basichttp::Transport`, a synchronous trait with one method, `send`, that takes a `Request` and answers a `Response` or a `TransportError`. A host implements it over an HTTP client it already has.
- `basichttp::Request` (method, address, headers in order, body bytes) and `basichttp::Response` (status, headers, body bytes). Every status the server answered is a response, 2xx or not.
- `basichttp::TransportError`, the one error a transport answers when no response came back, with its cause as `source`.
