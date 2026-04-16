//! The aggregate schedule: tasks, blocks, placements, and the interval index.
//!
//! [`Schedule`] is the central data structure.  It owns:
//! - a set of [`Task`]s keyed by [`TaskId`]
//! - a set of [`SchedulingBlock`]s keyed by [`SchedulingBlockId`]
//! - a set of [`TaskPlacement`]s keyed by [`TaskId`]
//! - an [`IntervalTree<JD>`](crate::interval_tree::IntervalTree) for fast
//!   overlap queries
//!
//! ## Interval index
//!
//! The interval index is an [`IntervalTree<JD>`](crate::interval_tree::IntervalTree)
//! that stores each placed task as a [`Period<JD>`](tempoch::Period) typed
//! interval.  Overlap queries run in O(log n + k) where k is the number of
//! results, using binary-search start-pruning and suffix-max-end pruning.

use crate::error::ScheduleError;
use crate::interval_tree::IntervalTree;
use crate::scheduling_block::SchedulingBlock;
use crate::task::Task;
use crate::time::{Period, PeriodSet, SchedulingBlockId, TaskId, TimeInterval, TimePoint, JD};
use siderust::coordinates::centers::Geodetic;
use siderust::coordinates::frames::ECEF;
use siderust::time::MJD;
use std::collections::HashMap;

// ─── TaskPlacement ───────────────────────────────────────────────────────────

/// The concrete scheduled slot for a task.
#[derive(Debug, Clone)]
pub struct TaskPlacement {
    pub task_id: TaskId,
    pub start: TimePoint,
    pub end: TimePoint,
    /// The scheduling block this placement belongs to, if any.
    pub block_id: Option<SchedulingBlockId>,
}

impl TaskPlacement {
    /// Convenience: return the interval `[start, end)` as a [`Period<JD>`].
    pub fn interval(&self) -> TimeInterval {
        TimeInterval::new(self.start, self.end)
    }
}

// ─── Schedule ────────────────────────────────────────────────────────────────

/// The aggregate schedule.
///
/// All mutating operations go through the high-level methods, which keep
/// the placements and interval tree consistent.
#[derive(Debug, Default)]
pub struct Schedule {
    pub tasks: HashMap<TaskId, Task>,
    pub blocks: HashMap<SchedulingBlockId, SchedulingBlock>,
    pub placements: HashMap<TaskId, TaskPlacement>,

    /// Interval tree index for O(log n + k) overlap queries.
    ///
    /// Each entry maps a [`Period<JD>`](tempoch::Period) to a [`TaskId`].
    /// The tree is updated atomically with `placements` by every mutation.
    interval_tree: IntervalTree<JD>,
}

impl Schedule {
    pub fn new() -> Self {
        Self::default()
    }

    // ── Task registry ─────────────────────────────────────────────────────

    /// Register a task.  Replaces any existing task with the same id.
    pub fn add_task(&mut self, task: Task) {
        self.tasks.insert(task.id, task);
    }

    // ── Block registry ────────────────────────────────────────────────────

    /// Register a scheduling block.
    pub fn add_block(&mut self, block: SchedulingBlock) {
        self.blocks.insert(block.id, block);
    }

    // ── Overlap query ─────────────────────────────────────────────────────

    /// Return the IDs of all currently placed tasks whose interval overlaps
    /// `[start, end)`.
    ///
    /// Two intervals `[a, b)` and `[c, d)` overlap iff `a < d && c < b`.
    ///
    /// Delegates to [`IntervalTree::query_overlapping`] for O(log n + k)
    /// performance.
    pub fn overlapping(&self, interval: &TimeInterval) -> Vec<TaskId> {
        self.interval_tree.query_overlapping(interval)
    }

    // ── Validation ────────────────────────────────────────────────────────

    /// Validate that `task_id` can be placed at `candidate`.
    ///
    /// Checks (in order):
    /// 1. Task exists.
    /// 2. Interval length matches task duration (within 1 ms tolerance).
    /// 3. No overlap with existing placements.
    /// 4. Hard-Static and Hard-Dynamic task constraints.
    /// 5. Built-in Hard-Dynamic dependency filtering (if a block is provided).
    /// 6. Candidate interval is fully covered by resulting feasible periods.
    /// 7. Soft-Static and Soft-Dynamic qualification (currently non-blocking).
    pub fn can_place(
        &self,
        task_id: TaskId,
        candidate: &TimeInterval,
        block_id: Option<SchedulingBlockId>,
        site: &Geodetic<ECEF>,
    ) -> Result<(), ScheduleError> {
        // 1. Task must exist.
        let task = self.tasks.get(&task_id).ok_or(ScheduleError::TaskNotFound)?;

        // 2. Duration check: interval length in seconds must match task.duration.
        let interval_days = candidate.end.value() - candidate.start.value();
        let interval_secs = interval_days * 86_400.0;
        let required_secs = task.duration.value();
        if (interval_secs - required_secs).abs() > 1e-3 {
            return Err(ScheduleError::IntervalDurationMismatch);
        }

        // 3. Overlap check via interval tree.
        if !self.overlapping(candidate).is_empty() {
            return Err(ScheduleError::OverlapConflict);
        }

        // 4. Constraint tree.
        let candidate_period = Period::new(candidate.start.to::<MJD>(), candidate.end.to::<MJD>());
        let hard_task_feasible = task
            .hard_constraints
            .check_hard(&candidate_period, Some(&task.target), Some(site))?;
        let dependency_feasible =
            self.block_dependency_feasible_periods(task_id, candidate, block_id)?;
        let feasible = hard_task_feasible.intersection(&dependency_feasible);

        let candidate_start = candidate.start.to::<MJD>();
        let candidate_end = candidate.end.to::<MJD>();

        let fully_covered = feasible
            .as_slice()
            .iter()
            .any(|period| period.start <= candidate_start && period.end >= candidate_end);

        if !fully_covered {
            return Err(ScheduleError::ConstraintViolation(
                "candidate interval is not fully covered by constraint periods".into(),
            ));
        }

        // 7. Soft scoring is computed but does not block placement decisions.
        if let Some(soft_constraints) = &task.soft_constraints {
            let _ = soft_constraints.score(&candidate_start, Some(site), Some(&task.target));
        }

        Ok(())
    }

    /// Built-in hard-dynamic filter for intra-block dependencies.
    ///
    /// Returns the candidate interval as a singleton set if dependencies are
    /// satisfied, or an empty set if they are not.
    fn block_dependency_feasible_periods(
        &self,
        task_id: TaskId,
        candidate: &TimeInterval,
        block_id: Option<SchedulingBlockId>,
    ) -> Result<PeriodSet<MJD>, ScheduleError> {
        let candidate_set = PeriodSet::from_periods(vec![Period::new(
            candidate.start.to::<MJD>(),
            candidate.end.to::<MJD>(),
        )]);

        let Some(bid) = block_id else {
            return Ok(candidate_set);
        };

        let block = self.blocks.get(&bid).ok_or(ScheduleError::BlockNotFound)?;
        if !block.contains_task(task_id) {
            return Ok(candidate_set);
        }

        let order = block.topological_order()?;
        let task_pos = order.iter().position(|&t| t == task_id).unwrap_or(0);

        for predecessor_id in order.iter().take(task_pos) {
            match self.placements.get(predecessor_id) {
                None => return Ok(PeriodSet::new()),
                Some(prev) => {
                    if prev.end > candidate.start {
                        return Ok(PeriodSet::new());
                    }
                }
            }
        }

        Ok(candidate_set)
    }

    // ── Placement ─────────────────────────────────────────────────────────

    /// Place `task_id` at `candidate` after successful validation.
    ///
    /// Calls [`can_place`](Self::can_place) internally; returns its error if
    /// validation fails.
    pub fn place_task(
        &mut self,
        task_id: TaskId,
        candidate: TimeInterval,
        block_id: Option<SchedulingBlockId>,
        site: &Geodetic<ECEF>,
    ) -> Result<(), ScheduleError> {
        self.can_place(task_id, &candidate, block_id, site)?;

        let placement = TaskPlacement {
            task_id,
            start: candidate.start,
            end: candidate.end,
            block_id,
        };

        self.index_insert(&placement);
        self.placements.insert(task_id, placement);
        Ok(())
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

    /// Move `task_id` to a new `candidate` interval.
    ///
    /// Equivalent to `unplace_task` + `place_task`, but atomic: if
    /// `place_task` fails the original placement is restored.
    pub fn move_task(
        &mut self,
        task_id: TaskId,
        new_interval: TimeInterval,
        block_id: Option<SchedulingBlockId>,
        site: &Geodetic<ECEF>,
    ) -> Result<(), ScheduleError> {
        let old = self.placements.remove(&task_id);
        if let Some(ref p) = old {
            self.index_remove(p);
        }

        match self.place_task(task_id, new_interval, block_id, site) {
            Ok(()) => Ok(()),
            Err(e) => {
                if let Some(p) = old {
                    self.index_insert(&p);
                    self.placements.insert(task_id, p);
                }
                Err(e)
            }
        }
    }

    // ── Index helpers ─────────────────────────────────────────────────────

    fn index_insert(&mut self, p: &TaskPlacement) {
        self.interval_tree.insert(p.interval(), p.task_id);
    }

    fn index_remove(&mut self, p: &TaskPlacement) {
        self.interval_tree.remove(p.task_id);
    }
}
