### Added

- `Effect`, the plain-data effects interpreting answers, starts with `Effect::Log` (a `<log>`'s label and value) and `Effect::DatamodelChange` (a datamodel write's resolved path, source, new and prior values), each carrying the macrostep, microstep and round counters; `BlockOwner` names the block an effect came from and `PathSegment` one segment of a resolved location.
