//! The position round-trip property, over every case of the scion suite: a
//! claim that the crate agrees with itself, that nothing is lost when a
//! position goes out and comes back in.
//!
//! Each case is driven as the run drives it (`runner.rs`): the settle window
//! before each event and the configuration deadline after the start and
//! after each event (`clock.rs`). A POINT is the state after the start and
//! after every step. At each point the property exports the position, takes
//! it through JSON text when the `serde` feature is on, compiles the case's
//! source again into a fresh machine and imports the position there, then
//! drives the remaining steps on the imported state the same way. The
//! imported state must agree with the unbroken drive at the point and after
//! every later step: the same export answer (the same position, or the same
//! refusal), the same configuration and active leaf states, the same answer
//! from `done` on whether the chart has stopped, and, for a chart that
//! stopped after the point, the same configuration it stopped in.
//!
//! A point a position cannot carry is counted and named with its reason,
//! never skipped: the export's two refusals (`internal_queue_not_empty`,
//! `unnameable_states`); a pending timer (`pending_timers`), which is the
//! driver's and travels in no position; a budget halt (`halted`), seen as
//! `Effect::BudgetExhausted` in a call's effects, which is the driver's too
//! and is the only way an external event stays queued between calls; and a
//! datamodel value the JSON text cannot carry (`unencodable_value`). Only
//! the first and the third are expected; the stage fails on any other,
//! on any disagreement, and on a run where no point agrees, since a property
//! that compared nothing proves nothing.
//!
//! It says nothing about whether the export matches the reference's.

use crate::clock::{CONFIGURATION_DEADLINE_MS, SETTLE_WINDOW_MS};
use crate::corpus::id_of;
use serde_json::Value as Json;
use statifier::{Effect, Error, ExportError, Machine, Options, Position, State};
use std::collections::BTreeMap;

/// Why a point's state cannot travel in a position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum NotCarried {
    /// The export refused: the internal queue holds an event.
    InternalQueueNotEmpty,
    /// The export refused: a state with no id is in the position.
    UnnameableStates,
    /// A timer is pending.
    PendingTimers,
    /// A spent budget halted the driver.
    Halted,
    /// A datamodel value has no JSON text.
    #[cfg(feature = "serde")]
    UnencodableValue,
}

impl NotCarried {
    /// The reason's name, as the stage prints it.
    pub fn name(self) -> &'static str {
        match self {
            NotCarried::InternalQueueNotEmpty => "internal_queue_not_empty",
            NotCarried::UnnameableStates => "unnameable_states",
            NotCarried::PendingTimers => "pending_timers",
            NotCarried::Halted => "halted",
            #[cfg(feature = "serde")]
            NotCarried::UnencodableValue => "unencodable_value",
        }
    }
}

/// The reasons a point may go uncarried with the stage still passing.
pub const EXPECTED: [NotCarried; 2] =
    [NotCarried::InternalQueueNotEmpty, NotCarried::PendingTimers];

/// What became of one point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Carried, and every later state agreed.
    Agree,
    /// Not carried, and why.
    NotCarried(NotCarried),
    /// Carried, and something disagreed: where and how.
    Disagree(String),
}

/// One point of one case: the start (point 0) or after step `point`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Point {
    /// The case.
    pub case_id: String,
    /// The point.
    pub point: usize,
    /// What became of it.
    pub outcome: Outcome,
}

/// The property's counts over a set of cases.
#[derive(Debug, Clone, Default)]
pub struct Report {
    /// The cases.
    pub cases: usize,
    /// Their steps.
    pub steps: usize,
    /// Every point, in case order.
    pub points: Vec<Point>,
}

/// How the property takes a position out and back in: the crate's own
/// export, or one a test plants to show the stage catches a loss.
pub type Exporter = fn(&State) -> Result<Position, ExportError>;

/// The crate's export.
pub fn export(state: &State) -> Result<Position, ExportError> {
    state.export_position()
}

/// A state the drive reached, and whether a spent budget has halted it.
#[derive(Debug, Clone)]
struct Reached {
    state: State,
    halted: bool,
}

fn halts(effects: &[Effect]) -> bool {
    effects
        .iter()
        .any(|effect| matches!(effect, Effect::BudgetExhausted { .. }))
}

fn where_(point: usize) -> String {
    if point == 0 {
        "the start".to_owned()
    } else {
        format!("step {point}")
    }
}

/// `clock::settle`, keeping the halt.
fn settle(reached: &Reached) -> Result<Reached, Error> {
    if reached.state.next_due_ms().is_none() {
        return Ok(reached.clone());
    }
    let moved = reached.state.advance(SETTLE_WINDOW_MS)?;
    Ok(Reached {
        halted: reached.halted || halts(&moved.effects),
        state: moved.state,
    })
}

/// `clock::await_until` on the expected leaves, keeping the halt.
fn reach(reached: Reached, since: u64, expected: &[String]) -> Result<Reached, Error> {
    let deadline = since.saturating_add(CONFIGURATION_DEADLINE_MS);
    let mut current = reached;
    loop {
        if leaves(&current.state) == expected || current.state.done().is_some() {
            return Ok(current);
        }
        match current.state.next_due_ms() {
            Some(due) if due <= deadline => {
                let moved = current
                    .state
                    .advance(due.saturating_sub(current.state.now_ms()))?;
                current = Reached {
                    halted: current.halted || halts(&moved.effects),
                    state: moved.state,
                };
            }
            _ => return Ok(current),
        }
    }
}

fn leaves(state: &State) -> Vec<String> {
    state.done().map_or_else(
        || state.active_leaf_states(),
        |done| done.active_leaf_states.clone(),
    )
}

/// A case's source, its expected start leaves, and each step's event and
/// expected leaves.
struct Case {
    id: String,
    source: String,
    initial: Vec<String>,
    steps: Vec<(String, Vec<String>)>,
}

fn ids(value: Option<&Json>) -> Vec<String> {
    let mut ids: Vec<String> = value
        .and_then(Json::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    ids.sort();
    ids.dedup();
    ids
}

fn case_of(item: &Json) -> Result<Case, String> {
    let id = id_of(item).ok_or("a case with no id")?.to_owned();
    let source = item
        .get("source")
        .and_then(Json::as_str)
        .ok_or_else(|| format!("{id}: no source"))?
        .to_owned();
    let steps = item
        .get("steps")
        .and_then(Json::as_array)
        .ok_or_else(|| format!("{id}: no steps"))?
        .iter()
        .map(|step| {
            let name = step
                .get("event")
                .and_then(|event| event.get("name"))
                .and_then(Json::as_str)
                .unwrap_or("")
                .to_owned();
            (name, ids(step.get("configuration")))
        })
        .collect();
    Ok(Case {
        initial: ids(item.get("initial_configuration")),
        id,
        source,
        steps,
    })
}

fn machine(case: &Case) -> Result<Machine, String> {
    statifier::compile(&case.source)
        .map_err(|_| format!("{}: the source does not compile", case.id))
}

/// The run's drive, keeping the state at every point.
fn unbroken(case: &Case) -> Result<Vec<Reached>, String> {
    let started = machine(case)?
        .start(Options::new(case.id.clone()))
        .map_err(|error| format!("{}: the start was refused: {error}", case.id))?;
    let first = Reached {
        halted: halts(&started.effects),
        state: started.state,
    };
    let mut current = reach(first, 0, &case.initial).map_err(|error| error.to_string())?;
    let mut points = vec![current.clone()];
    for (index, (name, expected)) in case.steps.iter().enumerate() {
        current = next(&current, name, expected)
            .map_err(|error| format!("{} at {}: {error}", case.id, where_(index + 1)))?;
        points.push(current.clone());
    }
    Ok(points)
}

/// One step as the run takes it: settle, send, wait for the expectation.
fn next(reached: &Reached, name: &str, expected: &[String]) -> Result<Reached, Error> {
    let settled = settle(reached)?;
    let since = settled.state.now_ms();
    let moved = settled.state.step(name, statifier::Value::Undefined)?;
    let stepped = Reached {
        halted: settled.halted || halts(&moved.effects),
        state: moved.state,
    };
    reach(stepped, since, expected)
}

/// The position through JSON text and back, with the `serde` feature; as it
/// is, without.
#[cfg(feature = "serde")]
fn travel(position: &Position) -> Result<Position, NotCarried> {
    let text = serde_json::to_string(position).map_err(|_| NotCarried::UnencodableValue)?;
    serde_json::from_str(&text).map_err(|_| NotCarried::UnencodableValue)
}

#[cfg(not(feature = "serde"))]
fn travel(position: &Position) -> Result<Position, NotCarried> {
    Ok(position.clone())
}

/// `None` when `imported` agrees with `original`, else the first thing that
/// differs. A chart stopped at the point is not compared on the
/// configuration it stopped in: that travels in the done effect, never in a
/// position.
pub fn disagreement(original: &State, imported: &State, stopped_at_point: bool) -> Option<String> {
    let (was, is) = (original.export_position(), imported.export_position());
    if was != is {
        return Some(format!(
            "the export is {is:?}, the unbroken drive's {was:?}"
        ));
    }
    if original.configuration() != imported.configuration() {
        return Some(format!(
            "the configuration is {:?}, the unbroken drive's {:?}",
            imported.configuration(),
            original.configuration()
        ));
    }
    if original.active_leaf_states() != imported.active_leaf_states() {
        return Some(format!(
            "the active leaf states are {:?}, the unbroken drive's {:?}",
            imported.active_leaf_states(),
            original.active_leaf_states()
        ));
    }
    match (original.done(), imported.done()) {
        (None, None) => None,
        (Some(_), Some(_)) if stopped_at_point => None,
        (Some(was), Some(is)) if was.configuration == is.configuration => None,
        (was, is) => Some(format!("done answers {is:?}, the unbroken drive's {was:?}")),
    }
}

fn round_trip(case: &Case, points: &[Reached], point: usize, exporter: Exporter) -> Outcome {
    let original = &points[point];
    let position = match exporter(&original.state) {
        Ok(position) => position,
        Err(ExportError::InternalQueueNotEmpty) => {
            return Outcome::NotCarried(NotCarried::InternalQueueNotEmpty);
        }
        Err(_) => return Outcome::NotCarried(NotCarried::UnnameableStates),
    };
    if original.state.next_due_ms().is_some() {
        return Outcome::NotCarried(NotCarried::PendingTimers);
    }
    if original.halted {
        return Outcome::NotCarried(NotCarried::Halted);
    }
    let position = match travel(&position) {
        Ok(position) => position,
        Err(reason) => return Outcome::NotCarried(reason),
    };
    let fresh = match machine(case) {
        Ok(fresh) => fresh,
        Err(reason) => return Outcome::Disagree(reason),
    };
    let imported = match fresh.import_position(&position) {
        Ok(state) => state,
        Err(error) => return Outcome::Disagree(format!("the import was refused: {error}")),
    };
    let stopped_at_point = original.state.done().is_some();
    if let Some(found) = disagreement(&original.state, &imported, stopped_at_point) {
        return Outcome::Disagree(format!("at the import: {found}"));
    }
    let mut current = Reached {
        state: imported,
        halted: false,
    };
    for (later, unbroken) in points.iter().enumerate().skip(point + 1) {
        let (name, expected) = &case.steps[later - 1];
        current = match next(&current, name, expected) {
            Ok(reached) => reached,
            Err(error) => {
                return Outcome::Disagree(format!(
                    "{}: the imported chart refused: {error}",
                    where_(later)
                ));
            }
        };
        if let Some(found) = disagreement(&unbroken.state, &current.state, stopped_at_point) {
            return Outcome::Disagree(format!("{}: {found}", where_(later)));
        }
    }
    Outcome::Agree
}

/// The property over every case given.
pub fn run(cases: &[Json], exporter: Exporter) -> Result<Report, String> {
    let mut report = Report {
        cases: cases.len(),
        ..Report::default()
    };
    for item in cases {
        let case = case_of(item)?;
        report.steps += case.steps.len();
        let points = unbroken(&case)?;
        for point in 0..points.len() {
            report.points.push(Point {
                case_id: case.id.clone(),
                point,
                outcome: round_trip(&case, &points, point, exporter),
            });
        }
    }
    Ok(report)
}

impl Report {
    /// The points that agree.
    pub fn agreeing(&self) -> usize {
        self.points
            .iter()
            .filter(|point| point.outcome == Outcome::Agree)
            .count()
    }

    /// The points not carried, by reason.
    pub fn not_carried(&self) -> BTreeMap<NotCarried, Vec<&Point>> {
        let mut by_reason: BTreeMap<NotCarried, Vec<&Point>> = BTreeMap::new();
        for point in &self.points {
            if let Outcome::NotCarried(reason) = point.outcome {
                by_reason.entry(reason).or_default().push(point);
            }
        }
        by_reason
    }

    /// The points that disagree.
    pub fn disagreements(&self) -> Vec<&Point> {
        self.points
            .iter()
            .filter(|point| matches!(point.outcome, Outcome::Disagree(_)))
            .collect()
    }

    /// Why the stage fails: every disagreement, every reason outside
    /// [`EXPECTED`], and a run where no point agrees. Empty when it passes.
    pub fn failures(&self) -> Vec<String> {
        let mut failures: Vec<String> = self
            .disagreements()
            .into_iter()
            .map(|point| {
                let Outcome::Disagree(reason) = &point.outcome else {
                    return String::new();
                };
                format!(
                    "position: {} at {} disagrees: {reason}",
                    point.case_id,
                    where_(point.point)
                )
            })
            .collect();
        for (reason, points) in self.not_carried() {
            if !EXPECTED.contains(&reason) {
                failures.push(format!(
                    "position: {} points not carried for {}, outside the expected {} and {}",
                    points.len(),
                    reason.name(),
                    EXPECTED[0].name(),
                    EXPECTED[1].name()
                ));
            }
        }
        if self.agreeing() == 0 {
            failures.push(format!(
                "position: no round trip agrees over {} points, so nothing was compared",
                self.points.len()
            ));
        }
        failures
    }

    /// The stage's lines: the counts, then every point not carried, named.
    pub fn lines(&self) -> Vec<String> {
        let not_carried = self.not_carried();
        let total: usize = not_carried.values().map(Vec::len).sum();
        let reasons: Vec<String> = not_carried
            .iter()
            .map(|(reason, points)| format!("{} {}", reason.name(), points.len()))
            .collect();
        let mut lines = vec![
            format!(
                "position: {} points over {} scion cases ({} starts and {} steps)",
                self.points.len(),
                self.cases,
                self.cases,
                self.steps
            ),
            format!(
                "position: {} round trips agree, {} disagree",
                self.agreeing(),
                self.disagreements().len()
            ),
            format!(
                "position: {total} points not carried{}",
                if reasons.is_empty() {
                    String::new()
                } else {
                    format!(": {}", reasons.join(", "))
                }
            ),
        ];
        for (reason, points) in &not_carried {
            for point in points {
                lines.push(format!(
                    "position: not carried ({}): {} at {}",
                    reason.name(),
                    point.case_id,
                    where_(point.point)
                ));
            }
        }
        lines
    }
}
