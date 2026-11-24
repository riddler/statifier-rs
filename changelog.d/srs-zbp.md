### Added

- `Effect::DatamodelInit`, `Effect::BudgetExhausted` and `Effect::Done`, the effects of the interpreter loop: the datamodel a chart starts from, before any `<data>` binds; a macrostep that spent its round budget (10000 rounds unless the host sets another) without reaching a stable configuration, with the configuration, the budget and the events still on the internal queue, the chart still running; and a chart that stopped, with its top-level final's donedata, the error evaluating that donedata raised, and the configuration it stopped in.
