//! Deserialize [`SchedulingProblem`] from scheduler JSON formats.
//!
//! Accepted inputs:
//! - Legacy: an array of scheduling blocks, each containing full task objects.
//! - Envelope: an object with `location`, optional `schedule_time_window`, and
//!   `scheduling_blocks`.
//!
//! Legacy array example:
//!
//! ```json
//! [
//!   {
//!     "id": 1,
//!     "tasks": [
//!       {
//!         "id": 101,
//!         "name": "my-task",
//!         "requested_duration_sec": 1200.0,
//!         "target": { "ra_deg": 83.8, "dec_deg": 22.0 },
//!         "hard_constraints": {
//!           "altitude_min_deg": 20.0,
//!           "time_window": { "start_mjd_utc": 62000.0, "end_mjd_utc": 62001.0 }
//!         },
//!         "soft_constraints": { "priority": 5.0 }
//!       }
//!     ],
//!     "dependencies": []
//!   }
//! ]
//! ```
//!
//! `detected_horizon` on the returned [`SchedulingProblem`] is:
//! - `schedule_time_window` when provided by the envelope format.
//! - otherwise, the union of all task `time_window` hard constraints.
//! - `None` when no horizon information is present.
//!
//! `location` on the returned [`SchedulingProblem`] is populated from the
//! envelope `location` object when available.

use super::SchedulingProblem;
use crate::constraints::{
    AltitudeConstraint, AzimuthConstraint, ConstraintExpr, PrioritySoftConstraint,
    SoftConstraintExpr, TimeConstraint,
};
use crate::scheduling_block::{Dependency, SchedulingBlock};
use crate::task::Task;
use crate::time::{MJD, Period, SchedulingBlockId, TaskId, Time};
use qtty::{Degrees, Meters, Seconds};
use serde::{Deserialize, Deserializer};
use siderust::coordinates::centers::Geodetic;
use siderust::coordinates::frames::{ECEF, ICRS};
use siderust::coordinates::spherical::Direction;

// ── JSON repr types (schema) ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct BlockRepr {
    id: u64,
    tasks: Vec<TaskRepr>,
    #[serde(default)]
    dependencies: Vec<DepRepr>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ProblemRepr {
    BlockList(Vec<BlockRepr>),
    Envelope(ProblemEnvelopeRepr),
}

#[derive(Debug, Deserialize)]
struct ProblemEnvelopeRepr {
    location: LocationRepr,
    #[serde(default)]
    schedule_time_window: Option<TimeWindowRepr>,
    scheduling_blocks: Vec<BlockRepr>,
}

#[derive(Debug, Deserialize)]
struct LocationRepr {
    longitude_deg: f64,
    latitude_deg: f64,
    height_m: f64,
}

#[derive(Debug, Deserialize)]
struct TaskRepr {
    id: u64,
    #[serde(default)]
    name: Option<String>,
    requested_duration_sec: f64,
    target: TargetRepr,
    #[serde(default)]
    hard_constraints: HardConstraintsRepr,
    #[serde(default)]
    soft_constraints: Option<SoftConstraintsRepr>,
}

#[derive(Debug, Deserialize)]
struct TargetRepr {
    ra_deg: f64,
    dec_deg: f64,
}

#[derive(Debug, Default, Deserialize)]
struct HardConstraintsRepr {
    altitude_min_deg: Option<f64>,
    altitude_max_deg: Option<f64>,
    azimuth_min_deg: Option<f64>,
    azimuth_max_deg: Option<f64>,
    time_window: Option<TimeWindowRepr>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
struct TimeWindowRepr {
    start_mjd_utc: f64,
    end_mjd_utc: f64,
}

#[derive(Debug, Default, Deserialize)]
struct SoftConstraintsRepr {
    priority: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct DepRepr {
    from: u64,
    to: u64,
}

// ── Conversion helpers ────────────────────────────────────────────────────────

fn mjd_period(start: f64, end: f64) -> Result<Period<MJD>, String> {
    if !start.is_finite() || !end.is_finite() {
        return Err("time_window bounds must be finite".to_string());
    }
    if start >= end {
        return Err(format!(
            "time_window start ({start}) must be before end ({end})"
        ));
    }
    Ok(Period::new(Time::<MJD>::new(start), Time::<MJD>::new(end)))
}

fn geodetic_location_from_repr(repr: LocationRepr) -> Result<Geodetic<ECEF>, String> {
    if !repr.longitude_deg.is_finite()
        || !repr.latitude_deg.is_finite()
        || !repr.height_m.is_finite()
    {
        return Err("location coordinates must be finite".to_string());
    }

    Ok(Geodetic::<ECEF>::new(
        Degrees::new(repr.longitude_deg),
        Degrees::new(repr.latitude_deg),
        Meters::new(repr.height_m),
    ))
}

fn hard_constraints_from_repr(repr: &HardConstraintsRepr) -> Result<ConstraintExpr, String> {
    let mut constraints = Vec::new();

    if repr.altitude_min_deg.is_some() || repr.altitude_max_deg.is_some() {
        let min = repr.altitude_min_deg.unwrap_or(0.0);
        let max = repr.altitude_max_deg.unwrap_or(90.0);
        if min > max {
            return Err(format!("invalid altitude bounds: min {min} > max {max}"));
        }
        constraints.push(ConstraintExpr::atom(AltitudeConstraint {
            min: Degrees::new(min),
            max: Degrees::new(max),
        }));
    }

    if repr.azimuth_min_deg.is_some() || repr.azimuth_max_deg.is_some() {
        let min = repr.azimuth_min_deg.unwrap_or(0.0);
        let max = repr.azimuth_max_deg.unwrap_or(360.0);
        constraints.push(ConstraintExpr::atom(AzimuthConstraint {
            min: Degrees::new(min),
            max: Degrees::new(max),
        }));
    }

    if let Some(tw) = repr.time_window {
        let window = mjd_period(tw.start_mjd_utc, tw.end_mjd_utc)?;
        constraints.push(ConstraintExpr::atom(TimeConstraint { window }));
    }

    Ok(ConstraintExpr::Intersection(constraints))
}

fn task_from_repr(repr: TaskRepr) -> Result<Task, String> {
    let hard_constraints = hard_constraints_from_repr(&repr.hard_constraints)?;
    let soft_constraints = repr
        .soft_constraints
        .as_ref()
        .and_then(|s| s.priority)
        .map(|p| SoftConstraintExpr::atom(PrioritySoftConstraint::new(p)));

    let name = repr
        .name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("task-{}", repr.id));

    let target = Direction::<ICRS>::new_raw(
        Degrees::new(repr.target.dec_deg),
        Degrees::new(repr.target.ra_deg),
    );

    Task::new(
        TaskId(repr.id),
        name,
        target,
        Seconds::new(repr.requested_duration_sec),
        hard_constraints,
        soft_constraints,
    )
    .map_err(|e| format!("invalid task {}: {e}", repr.id))
}

// ── Deserialize for SchedulingProblem ─────────────────────────────────────────

impl<'de> Deserialize<'de> for SchedulingProblem {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let (block_reprs, explicit_horizon, location) =
            match ProblemRepr::deserialize(deserializer)? {
                ProblemRepr::BlockList(blocks) => (blocks, None, None),
                ProblemRepr::Envelope(envelope) => {
                    let horizon = envelope
                        .schedule_time_window
                        .map(|tw| mjd_period(tw.start_mjd_utc, tw.end_mjd_utc))
                        .transpose()
                        .map_err(serde::de::Error::custom)?;
                    let location = geodetic_location_from_repr(envelope.location)
                        .map_err(serde::de::Error::custom)?;
                    (envelope.scheduling_blocks, horizon, Some(location))
                }
            };

        let mut problem = SchedulingProblem::new();
        problem.location = location;
        let mut min_start = f64::INFINITY;
        let mut max_end = f64::NEG_INFINITY;

        for block_repr in block_reprs {
            let block_id = block_repr.id;
            let mut block = SchedulingBlock::new(SchedulingBlockId(block_id));

            for task_repr in block_repr.tasks {
                let task_id = TaskId(task_repr.id);

                // Capture time_window before task_repr is consumed.
                let time_window = task_repr.hard_constraints.time_window;

                let task = task_from_repr(task_repr).map_err(serde::de::Error::custom)?;

                if problem.tasks.contains_key(&task_id) {
                    return Err(serde::de::Error::custom(format!(
                        "duplicate task id {} in block {}",
                        task_id.0, block_id
                    )));
                }

                problem.add_task(task);
                block.add_task(task_id);

                if let Some(tw) = time_window {
                    min_start = min_start.min(tw.start_mjd_utc);
                    max_end = max_end.max(tw.end_mjd_utc);
                }
            }

            for dep in block_repr.dependencies {
                let (from, to) = (TaskId(dep.from), TaskId(dep.to));
                if !block.contains_task(from) || !block.contains_task(to) {
                    return Err(serde::de::Error::custom(format!(
                        "block {block_id} dependency references unknown task {} -> {}",
                        dep.from, dep.to
                    )));
                }
                block
                    .add_dependency(from, to, Dependency::DependsOn)
                    .map_err(|e| {
                        serde::de::Error::custom(format!(
                            "block {block_id} dependency {} -> {} is invalid: {e}",
                            dep.from, dep.to
                        ))
                    })?;
            }

            problem.add_block(block);
        }

        if let Some(horizon) = explicit_horizon {
            problem.detected_horizon = Some(horizon);
        } else if min_start.is_finite() && max_end.is_finite() {
            problem.detected_horizon = mjd_period(min_start, max_end).ok();
        }

        Ok(problem)
    }
}
