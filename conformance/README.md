# conformance/

How this crate holds itself to the conformance corpus of statifier-ex, the
Elixir reference implementation. The corpus is the arbiter: a case this crate
answers differently from the corpus is this crate's bug.

| Path | What it is | Written by |
|---|---|---|
| `statifier/` | statifier-ex's `conformance/` directory at the tag the provenance record names, byte for byte (below) | `mise run corpus:refresh`, never by hand |
| `statifier.vendored.json` | the provenance record: the upstream repository, the tag, the commit the tag resolves to, and the corpus hash recomputed from the copy, as one line of JSON | `mise run corpus:refresh` |
| `registry.json` | this crate's registry: the corpus cases it claims to pass, against the pinned corpus | `mise run ratchet`, never by hand |
| `README.md` | this file | hand, reviewed like code |

The last three rows are this repository's own. Everything under `statifier/`
is copied.

## What is copied, and from where

The copy is the `conformance/` directory of
https://github.com/riddler/statifier-ex at the tag and commit
`statifier.vendored.json` records, taken by the sequence under "Vendoring the
corpus" in the reference's [`RATCHET.md`](statifier/RATCHET.md) with `DEST`
set to `conformance/statifier`: every file under `conformance/` at the tag,
written as git stores it. That is:

- the three suite files, `corpus/scion.json`, `corpus/w3c.json` and
  `corpus/statifier.json`, and `manifest.json`, which names them, gives each
  suite's case count and carries the corpus hash;
- the JSON Schemas under `schema/`;
- the cases the reference authors itself, under `cases/`;
- `exclusions.json`, the upstream documents the reference leaves out of the
  corpus, each with its reason;
- the reference's own `README.md`, `RATCHET.md` and `registry.json` (the
  reference's claim, never this crate's);
- `LICENSES/`, the licence texts the upstream cases are redistributed under.

What each of those files is, and who writes it upstream, is the reference's
[`README.md`](statifier/README.md); the contract this crate's registry is
written against is its `RATCHET.md`. Both are read in the copy, at the tag,
never at the reference's `main`.

**The copied files are never edited.** Re-running the sequence at the same
tag reproduces the copy exactly, so a copy that differs from a fresh run has
been edited. The fix for a copied file that is wrong is upstream, followed by
a refresh to the tag that carries it.

**The licences travel with the copy.** The `scion` suite's cases come from
the SCION scxml-test-framework and are redistributed under the Apache License
2.0 (`statifier/LICENSES/Apache-2.0.txt`); the `w3c` suite's come from the
W3C SCXML Implementation Report Plan test suite and are redistributed under
the W3C 3-clause BSD License (`statifier/LICENSES/BSD-3-Clause-W3C.txt`). The
copy's `manifest.json` names each upstream suite with its licence and notice
file, and each case's `upstream` field names its upstream document, its
licence and its notice. The `statifier` suite is the reference's own.

The copy is not part of the published crate: `Cargo.toml`'s `include`
packages `src/` and the top-level documents only.

## The commands

```bash
mise run gate:corpus      # the four hashes agree (a gate stage)
mise run gate:registry    # the registry check (a gate stage)
mise run gate:position    # the position round trip (a gate stage)
mise run ratchet          # record in registry.json the cases a run made now passes
STATIFIER_CORPUS_TAG=vX.Y.Z STATIFIER_CORPUS_CLONE=../statifier-ex \
  mise run corpus:refresh # move the copy to a tag
```

The code behind them lives in `tests/conformance/`, the `conformance` test
target, beside the tests that hold it; none of it ships. Each command runs
one test of that target on its own; `ratchet` and `refresh` write tracked
files, so they are ignored by every other test run.

**Check.** `mise run gate:corpus` recomputes the corpus hash from the copy -
SHA-256 over the bytes of the suite files concatenated in suite order, scion,
w3c, statifier - and requires it to equal four fields: the vendored
manifest's `corpus_hash`, the provenance record's, this registry's and the
vendored reference registry's. It also requires every suite file the
manifest lists to be there with the case count the manifest gives. A mismatch
fails naming the field, and is never repaired by rewriting a hash: restore
the copied bytes, or refresh the copy to a tag.

**Run.** The conformance run is the rest of the `conformance` test target. It
reads every case of every suite with no skip list, refuses a case carrying a
key the copy's `schema/case.json` does not name (at every object the schema
closes) or one its per-suite arms forbid, hands every case to the drive,
requires every case this registry claims to pass, and reports per suite how
many cases are not yet claimed. The drive (`drive` in
`tests/conformance/runner.rs`) is the one place the crate is plugged into the
run, through the crate's public surface alone. It drives every suite as
the reference's harness drives a case: it compiles the source, starts the
chart with the send types the case's `host` object registers, sends each
step's event with the step's `data` when it gives one, and compares the
active leaf states (`State::active_leaf_states`, or
`Done::active_leaf_states` once the chart has stopped, as the reference's
harness reads its done effect) with the case's initial configuration
and, after each step's event, with that step's configuration, settling the
virtual clock before each event and waiting up to the configuration
deadline after it
(`tests/conformance/clock.rs`). Each comparison first counts the active
leaves (`State::active_leaf_count`, or `Done::active_leaf_count`) against
the ids read, so a case whose active leaf has no id fails, as the
reference's harness fails it. A case whose `host` object needs what the
drive does not honour, such as an Event I/O Processor it does not run,
fails naming it. Every send of a registered type the driver hands the
host over the whole execution is recorded in the case's item shape, a
cancel the driver hands the host marking a delayed send whose expected
item says `cancelled`, as the reference's host case runner
(`lib/mix/statifier/corpus/host_case.ex` at the tag) records them; once
every configuration agrees, the sends recorded must be exactly the
case's `expect_sends`, in order. A send whose expected item says `fail`
is reported failed through `State::report_send_failed` as soon as the
drive reads it. Before the chart starts, a case carrying
`declared_events` and `expect_accepts` is checked with
`Machine::check_accepts` on the compiled chart (a JSON `null` being no
declaration), and both lists it answers must be exactly the case's,
order included; either key without the other fails the case, as the
reference's host case runner checks it. A case the drive cannot compare
yet fails before the chart starts, naming what it waits on: a host key
it does not honour (the chart diff's `to_source`, `expect_diff`,
`mapping` and `expect_compatible_at`) and a step carrying
`expect_position` (the position rendering). The registry check prints, for each driven suite,
how many cases pass, and the gap list: every case of every suite the
registry does not claim, in the reference's order, with its conformance
class when the corpus gives one, the features it needs, the keys of its
`host` object when it has one, and whether it passes or why it fails.

An excluded document is not in the corpus at all, so it is not a case and
is counted nowhere; `exclusions.json` is copied for its reasons. A case in
the corpus that this registry has no entry for is unclaimed, whether or not
the reference claims it.

**Ratchet.** `mise run ratchet` runs the whole corpus now and writes the
union of the recorded entries and the cases the run passed, pinned to the
vendored manifest's hash, with exactly the claims those entries count toward,
one entry per line in the reference's encoding. It refuses, writing nothing,
when an entry already recorded did not pass, when it is recorded under
another suite or the corpus does not hold it, and when the run is not of the
whole corpus. There is no command that adds a case by id. With nothing
passing, it writes the registry it found.

**Registry check.** `mise run gate:registry` runs the checks under "The
check" in the copy's `RATCHET.md` - the pin, rule 1, the claims, every entry
passing in a run made now, and something to check - and refuses entries out
of order, a case entered twice, another implementation's name and a file the
ratchet would not have written. It also refuses a registry with no entry
(rule 3: nothing to verify), as the copy's `schema/registry.json` refuses
that file; before its first claim this registry had none, and the check
passed it as the registry that makes no claim.

**Position round trip.** `mise run gate:position` drives every scion case
as the run does and, after the start and after every step, exports the
chart's position, writes it to JSON text and reads it back (the stage
runs with every feature, `serde` among them), compiles the source again,
imports the position into the new machine and drives the remaining steps
on it. The imported drive must agree with the unbroken one at the point
and after every later step: the same export, the same configuration and
active leaf states, and the same answer on whether the chart has
stopped. A point a position cannot carry is counted with its reason and
named, never skipped: the export's two refusals, a pending timer, a
budget halt, or a datamodel value JSON text cannot hold. A pending timer
and a non-empty internal queue are the reasons the stage expects; it
fails on any other, on any disagreement, and when no point agrees. It
shows the crate agreeing with itself, not its positions agreeing with
the reference's.

**Refresh.** Moving the copy to another tag is a change of its own, reviewed
like code. `mise run corpus:refresh` reads the tag named in
`STATIFIER_CORPUS_TAG` from the clone of statifier-ex named in
`STATIFIER_CORPUS_CLONE` (for one, `git clone --bare
https://github.com/riddler/statifier-ex`), refuses while the copy or its
record has uncommitted changes, refuses a name that is not a tag, writes the
copy into a temporary directory under the build's `target/`, requires it to
hash to its manifest's hash, and only then replaces `statifier/` and rewrites
the provenance record, which names the public repository, never the clone.
With `STATIFIER_CORPUS_CHECK=1` it stops after the hash and compares instead:
it names every file of the copy that is missing, changed or not at the tag,
and a record that is not the one it would write, and writes nothing. A new
tag changes the corpus hash: the corpus check then fails on this registry's
pin until `mise run ratchet` re-verifies every entry against the new corpus
and re-pins it.
