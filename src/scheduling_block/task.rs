//! Astronomical observation task.

use crate::constraints::ConstraintBlocks;
use crate::constraints::SoftConstraintExpr;
use crate::error::ScheduleError;
use crate::time::TaskId;
use qtty::Seconds;
use siderust::coordinates::frames::ICRS;
use siderust::coordinates::spherical::Direction;

/// An ICRS sky direction — the positional target of an observation.
pub type IcrsTarget = Direction<ICRS>;

/// A single schedulable observation task.
///
/// A task couples a sky target, an integration duration, and separate hard
/// and soft observing constraints. It does **not** carry a time placement;
/// that is held by [`TaskPlacement`](crate::schedule::TaskPlacement).
#[derive(Debug)]
pub struct Task {
    pub id: TaskId,
    pub name: String,
    pub target: IcrsTarget,
    /// Required total on-sky time (must be positive).
    pub duration: Seconds,
    /// Hard constraints evaluated before placement.
    pub hard_constraints: ConstraintBlocks,
    /// Soft constraints used for scoring and ranking.
    pub soft_constraints: Option<SoftConstraintExpr>,
}

impl Task {
    /// Construct a new task, validating invariants.
    pub fn new(
        id: TaskId,
        name: impl Into<String>,
        target: IcrsTarget,
        duration: Seconds,
        hard_constraints: impl Into<ConstraintBlocks>,
        soft_constraints: Option<SoftConstraintExpr>,
    ) -> Result<Self, ScheduleError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(ScheduleError::InvalidTask(
                "task name must not be empty".into(),
            ));
        }
        if duration.value() <= 0.0 {
            return Err(ScheduleError::InvalidDuration);
        }
        Ok(Task {
            id,
            name,
            target,
            duration,
            hard_constraints: hard_constraints.into(),
            soft_constraints,
        })
    }
}
