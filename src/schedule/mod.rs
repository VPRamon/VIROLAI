//! The concrete schedule result: placements plus the overlap index.
//!
//! [`Schedule`] owns:
//! - a set of [`TaskPlacement`]s keyed by [`TaskId`]
//! - an [`IntervalTree<JD>`](crate::time::IntervalTree) for fast overlap queries

mod output;
mod problem;
mod serde;
mod task_placement;

pub use output::ScheduleOutput;
pub use problem::SchedulingProblem;
pub use task_placement::TaskPlacement;

use crate::error::ScheduleError;
use crate::time::{IntervalTree, JD, TaskId, TimeInterval};
use std::collections::HashMap;

/// The placement result.
///
/// All mutations keep the placement map and interval tree consistent.
#[derive(Debug, Default, Clone)]
pub struct Schedule {
    pub placements: HashMap<TaskId, TaskPlacement>,
    interval_tree: IntervalTree<JD>,
}

impl Schedule {
    pub fn new() -> Self {
        Self::default()
    }

    /// Return the IDs of all currently placed tasks whose interval overlaps
    /// `[start, end)`.
    pub fn overlapping(&self, interval: &TimeInterval) -> Vec<TaskId> {
        self.interval_tree.query_overlapping(interval)
    }

    /// Remove the current placement of `task_id`, if any.
    ///
    /// Returns `Ok(())` even if the task was not placed.
    pub fn unplace_task(&mut self, task_id: TaskId) -> Result<(), ScheduleError> {
        if let Some(placement) = self.placements.remove(&task_id) {
            self.index_remove(&placement);
        }
        Ok(())
    }

    fn index_insert(&mut self, placement: &TaskPlacement) {
        self.interval_tree
            .insert(placement.interval(), placement.task_id);
    }

    fn index_remove(&mut self, placement: &TaskPlacement) {
        self.interval_tree.remove(placement.task_id);
    }

    pub(crate) fn insert_placement(&mut self, placement: TaskPlacement) {
        self.index_insert(&placement);
        self.placements.insert(placement.task_id, placement);
    }
}
