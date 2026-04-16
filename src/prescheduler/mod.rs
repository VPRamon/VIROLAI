//! Pre-scheduling pass that computes feasibility windows per task.
//!
//! The prescheduler iterates task ids from the provided scheduling blocks,
//! evaluates each task hard constraints over a global timeline and observing
//! site, and stores the result in a task->period-set map.
//!
//! Output is a map `TaskId -> PeriodSet<MJD>` that can be used as input for a
//! downstream placement optimizer.

use crate::error::ScheduleError;
use crate::scheduling_block::SchedulingBlock;
use crate::task::Task;
use crate::time::{MJD, Period, PeriodSet, TaskId};
use rayon::prelude::*;
use siderust::coordinates::centers::Geodetic;
use siderust::coordinates::frames::ECEF;
use std::collections::HashMap;

pub type TaskPeriodMap = HashMap<TaskId, PeriodSet<MJD>>;

#[derive(Debug, Default)]
pub struct Prescheduler;

impl Prescheduler {
    /// Compute feasible period sets per task.
    ///
    /// For each block, this performs a single pass over task ids and stores
    /// `check_hard` feasibility windows in the output map.
    pub fn run(
        blocks: &[SchedulingBlock],
        tasks: &HashMap<TaskId, Task>,
        timeline: &Period<MJD>,
        location: &Geodetic<ECEF>,
    ) -> Result<TaskPeriodMap, ScheduleError> {
        let mut out = TaskPeriodMap::new();
        let task_ids: Vec<TaskId> = blocks
            .par_iter()
            .flat_map_iter(SchedulingBlock::iter)
            .collect();

        for task_id in task_ids {
            let task = tasks.get(&task_id).ok_or(ScheduleError::TaskNotFound)?;
            let feasible =
                task.hard_constraints
                    .check_hard(timeline, Some(&task.target), Some(location))?;

            if out.insert(task_id, feasible).is_some() {
                return Err(ScheduleError::InvalidTask(format!(
                    "duplicate task id {} across scheduling blocks",
                    task_id.0
                )));
            }
        }

        Ok(out)
    }
}

pub fn preschedule(
    blocks: &[SchedulingBlock],
    tasks: &HashMap<TaskId, Task>,
    timeline: &Period<MJD>,
    location: &Geodetic<ECEF>,
) -> Result<TaskPeriodMap, ScheduleError> {
    Prescheduler::run(blocks, tasks, timeline, location)
}
