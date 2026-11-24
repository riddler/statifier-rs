### Added

- `State::active_leaf_count` and `Done::active_leaf_count` count the active leaf states, a state with no id counted too, so a caller can tell when `active_leaf_states`, which leaves such a state out, has left one out; with the `serde` feature a `Done` carries the count as `activeLeafCount`.
