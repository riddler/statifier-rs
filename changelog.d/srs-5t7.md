### Added

- `State::export_position` answers a chart's `Position` in the reference's string-id vocabulary: the configuration, the entered states, the states to invoke, the history values, the active invocations, the counters, the datamodel, the running flag and status, the trace flag and the round budget; it refuses with `ExportError` a state whose internal queue holds an event or that holds a state with no id.
- `Machine::import_position` reads a `Position` back into a `State` on any machine that holds every state it names, the virtual clock at 0 and nothing pending; it refuses with `ImportError` an unknown state id (naming every one), a datamodel with no `_sessionid` string, a history value the chart cannot restore and two invocations of one `<invoke>`.
- The `serde` feature, off by default, implements `Serialize` and `Deserialize` for `Position`, `Effect`, `Done` and the types they reach; a position's JSON form has the TypeScript sibling's keys.
