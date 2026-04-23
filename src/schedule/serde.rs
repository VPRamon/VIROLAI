//! Deserialize [`SchedulingProblem`] from scheduler JSON formats.
//!
//! Accepted inputs:
//! - Legacy: an array of scheduling blocks, each containing full task objects.
//! - Envelope: an object with `resources`, optional `schedule_time_window`, and
//!   `scheduling_blocks`.
//! - Backward-compatible envelope: an object with legacy top-level `location`
//!   instead of `resources`.
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
//! The telescope resource (if any) is stored on the returned
//! [`SchedulingProblem`] as a [`Telescope`]. Telescope-level hard constraints
//! remain on the telescope and are not merged into individual tasks.

use super::SchedulingProblem;
use crate::constraints::{PrioritySoftConstraint, SoftConstraintExpr};
use crate::scheduling_block::{Dependency, SchedulingBlock};
use crate::serde_repr::{
    HardConstraintsRepr, LocationRepr, TimeWindowRepr, geodetic_location_from_repr,
    hard_constraint_blocks_from_repr, mjd_period,
};
use crate::task::Task;
use crate::telescope::Telescope;
use crate::telescope::serde_impl::TelescopeRepr;
use crate::time::{SchedulingBlockId, TaskId};
use qtty::{Degrees, Seconds};
use serde::{Deserialize, Deserializer};
use siderust::coordinates::frames::ICRS;
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
    #[serde(default)]
    location: Option<LocationRepr>,
    #[serde(default)]
    resources: Vec<TelescopeRepr>,
    #[serde(default)]
    schedule_time_window: Option<TimeWindowRepr>,
    scheduling_blocks: Vec<BlockRepr>,
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
struct SoftConstraintsRepr {
    priority: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct DepRepr {
    from: u64,
    to: u64,
}

// ── Conversion ───────────────────────────────────────────────────────────────

fn task_from_repr(repr: TaskRepr) -> Result<Task, String> {
    let hard_constraints = hard_constraint_blocks_from_repr(&repr.hard_constraints)?;

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
        let (block_reprs, explicit_horizon, telescope) = match ProblemRepr::deserialize(
            deserializer,
        )? {
            ProblemRepr::BlockList(blocks) => (blocks, None, None),
            ProblemRepr::Envelope(envelope) => {
                let horizon = envelope
                    .schedule_time_window
                    .map(|tw| mjd_period(tw.start_mjd_utc, tw.end_mjd_utc))
                    .transpose()
                    .map_err(serde::de::Error::custom)?;

                let mut resources = envelope.resources.into_iter();
                let telescope = if let Some(primary) = resources.next() {
                    if resources.next().is_some() {
                        return Err(serde::de::Error::custom(
                            "multiple resources are not supported yet; expected exactly one telescope resource",
                        ));
                    }
                    Some(primary.into_telescope().map_err(serde::de::Error::custom)?)
                } else if let Some(location) = envelope.location {
                    let location =
                        geodetic_location_from_repr(location).map_err(serde::de::Error::custom)?;
                    Some(Telescope::new(
                        0,
                        "telescope-0",
                        location,
                        Default::default(),
                    ))
                } else {
                    return Err(serde::de::Error::custom(
                        "missing observing site; expected resources[0].location or legacy location",
                    ));
                };

                (envelope.scheduling_blocks, horizon, telescope)
            }
        };

        let mut blocks = Vec::with_capacity(block_reprs.len());
        let mut min_start = f64::INFINITY;
        let mut max_end = f64::NEG_INFINITY;

        for block_repr in block_reprs {
            let block_id = block_repr.id;
            let mut block = SchedulingBlock::new(SchedulingBlockId(block_id));

            for task_repr in block_repr.tasks {
                // Capture time_window before task_repr is consumed.
                let time_window = task_repr.hard_constraints.time_window;

                let task = task_from_repr(task_repr).map_err(serde::de::Error::custom)?;
                block.push_task(task).map_err(serde::de::Error::custom)?;

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

            blocks.push(block);
        }

        let mut problem =
            SchedulingProblem::from_blocks(blocks).map_err(serde::de::Error::custom)?;
        problem.telescope = telescope;

        if let Some(horizon) = explicit_horizon {
            problem.detected_horizon = Some(horizon);
        } else if min_start.is_finite() && max_end.is_finite() {
            problem.detected_horizon = mjd_period(min_start, max_end).ok();
        }

        Ok(problem)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constraints::{
        AltitudeConstraint, ConstraintExpr, MoonAltitudeConstraint, NightConstraint,
    };
    use crate::time::{MJD, Period, Time};
    use qtty::Meters;
    use siderust::calculus::solar::Twilight;
    use siderust::coordinates::centers::Geodetic;
    use siderust::coordinates::frames::ECEF;

    fn roque() -> Geodetic<ECEF> {
        Geodetic::<ECEF>::new(
            Degrees::new(-17.892),
            Degrees::new(28.762),
            Meters::new(2396.0),
        )
    }

    #[test]
    fn telescope_hard_constraints_parse_from_envelope() {
        let payload = serde_json::json!({
            "resources": [
                {
                    "id": 1,
                    "name": "CTA-N",
                    "location": {
                        "longitude_deg": -17.892,
                        "latitude_deg": 28.762,
                        "height_m": 2396.0
                    },
                    "hard_constraints": {
                        "night_time": {"twilight": "Nautical"},
                        "moon_altitude": {"min_deg": -90.0, "max_deg": 0.0}
                    }
                }
            ],
            "schedule_time_window": {
                "start_mjd_utc": 60000.0,
                "end_mjd_utc": 60001.0
            },
            "scheduling_blocks": [
                {
                    "id": 1,
                    "tasks": [
                        {
                            "id": 10,
                            "name": "task-10",
                            "requested_duration_sec": 600.0,
                            "target": {"ra_deg": 101.287, "dec_deg": -16.716},
                            "hard_constraints": {
                                "altitude_min_deg": 20.0,
                                "altitude_max_deg": 90.0
                            }
                        }
                    ],
                    "dependencies": []
                }
            ]
        });

        let problem: SchedulingProblem = serde_json::from_value(payload).unwrap();

        let telescope = problem.telescope.as_ref().expect("telescope should parse");
        let timeline = Period::new(Time::<MJD>::new(60000.0), Time::<MJD>::new(60001.0));
        let telescope_out = telescope
            .hard_constraints
            .check_hard(&timeline, None, Some(&telescope.location))
            .expect("telescope constraints should evaluate");
        let expected_telescope = ConstraintExpr::Intersection(vec![
            ConstraintExpr::atom(NightConstraint {
                twilight: Twilight::Nautical,
            }),
            ConstraintExpr::atom(MoonAltitudeConstraint {
                min: Degrees::new(-90.0),
                max: Degrees::new(0.0),
            }),
        ])
        .check(&timeline, Some(&telescope.location), None)
        .expect("expected telescope constraints should evaluate");
        assert_eq!(telescope_out, expected_telescope);

        // Task carries only its own constraints — telescope constraints are
        // not merged in at deserialization time.
        let task = problem.task(TaskId(10)).expect("task parsed");
        let task_out = task
            .hard_constraints
            .check_hard(&timeline, Some(&task.target), Some(&roque()))
            .expect("task constraints should evaluate");
        let expected_task = ConstraintExpr::atom(AltitudeConstraint {
            min: Degrees::new(20.0),
            max: Degrees::new(90.0),
        })
        .check(&timeline, Some(&roque()), Some(&task.target))
        .expect("expected task constraints should evaluate");
        assert_eq!(task_out, expected_task);
    }

    #[test]
    fn deserialize_rejects_moon_altitude_above_horizon_limit() {
        let payload = serde_json::json!({
            "resources": [
                {
                    "id": 1,
                    "location": {
                        "longitude_deg": -17.892,
                        "latitude_deg": 28.762,
                        "height_m": 2396.0
                    },
                    "hard_constraints": {
                        "moon_altitude": {
                            "min_deg": -90.0,
                            "max_deg": 5.0
                        }
                    }
                }
            ],
            "schedule_time_window": {
                "start_mjd_utc": 60000.0,
                "end_mjd_utc": 60001.0
            },
            "scheduling_blocks": [
                {
                    "id": 1,
                    "tasks": [
                        {
                            "id": 10,
                            "requested_duration_sec": 600.0,
                            "target": { "ra_deg": 101.287, "dec_deg": -16.716 },
                            "hard_constraints": {}
                        }
                    ],
                    "dependencies": []
                }
            ]
        });

        let err = serde_json::from_value::<SchedulingProblem>(payload)
            .expect_err("payload should be rejected")
            .to_string();

        assert!(err.contains("must be <= 0"));
    }
}
