//! The concrete schedule result: placements plus the overlap index.
//!
//! [`Schedule`] owns:
//! - a set of [`TaskPlacement`]s keyed by [`TaskId`]
//! - an [`IntervalTree<MJD>`](crate::time::IntervalTree) for fast overlap queries

mod output;
mod problem;
mod serde;
mod task_placement;

pub use output::ScheduleOutput;
pub use problem::SchedulingProblem;
pub use task_placement::TaskPlacement;

use crate::error::ScheduleError;
use crate::time::{IntervalTree, MJD, TaskId, TimeInterval};
use std::collections::HashMap;

/// The placement result.
///
/// All mutations keep the placement map and interval tree consistent.
#[derive(Debug, Default, Clone)]
pub struct Schedule {
    placements: HashMap<TaskId, TaskPlacement>,
    interval_tree: IntervalTree<MJD>,
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

    pub fn insert_placement(&mut self, placement: TaskPlacement) {
        self.index_insert(&placement);
        self.placements.insert(placement.task_id, placement);
    }

    /// Iterate over all placed tasks.
    pub fn placements(&self) -> impl Iterator<Item = &TaskPlacement> {
        self.placements.values()
    }

    /// Look up the placement for `task_id`, if any.
    pub fn get(&self, task_id: TaskId) -> Option<&TaskPlacement> {
        self.placements.get(&task_id)
    }

    /// Number of placed tasks.
    pub fn len(&self) -> usize {
        self.placements.len()
    }

    /// `true` when no tasks have been placed.
    pub fn is_empty(&self) -> bool {
        self.placements.is_empty()
    }

    /// `true` when `task_id` has been placed.
    pub fn contains(&self, task_id: TaskId) -> bool {
        self.placements.contains_key(&task_id)
    }
}
