//! Legacy proposal compatibility types for the HAP scheduler.
//!
//! The HAP implementation now operates directly on
//! [`SchedulingBlock`](crate::scheduling_block::SchedulingBlock) values.
//! This module remains public for compatibility with existing callers that
//! still build or inspect a proposal wrapper.

use crate::prescheduler::TaskPeriodMap;
use crate::schedule::{Schedule, SchedulingProblem};
use crate::scheduling_block::SchedulingBlock;
use crate::time::{MJD, SchedulingBlockId, TaskId, Time};

/// Legacy wrapper around one [`SchedulingBlock`].
///
/// HAP no longer uses this type internally, but it remains available for
/// external callers that depend on the precomputed proposal metadata.
#[derive(Debug, Clone)]
pub struct Proposal {
    /// The scheduling block this proposal was derived from.
    pub id: SchedulingBlockId,
    /// Tasks in topological order (predecessors first).
    pub task_ids: Vec<TaskId>,
    /// Sum of member-task soft-constraint priorities evaluated at the horizon
    /// start.  Tasks without soft constraints contribute `0.0`.
    pub priority: f64,
    /// `max(1, total feasible period count across block tasks)`.
    ///
    /// Used as the denominator in the impatience calculation.
    pub impatience_denominator: usize,
}

impl Proposal {
    /// Derive a proposal from a scheduling block.
    pub fn from_block(
        block: &SchedulingBlock,
        problem: &SchedulingProblem,
        possible_periods: &TaskPeriodMap,
        horizon_start: Time<MJD>,
    ) -> Self {
        let task_ids = block
            .topological_order()
            .unwrap_or_else(|_| block.iter().collect());

        let priority: f64 = task_ids
            .iter()
            .map(|id| {
                problem
                    .task(*id)
                    .and_then(|t| {
                        t.soft_constraints
                            .as_ref()
                            .map(|sc| sc.score(&horizon_start, None, Some(&t.target)))
                    })
                    .unwrap_or(0.0)
            })
            .sum();

        let total_periods: usize = task_ids
            .iter()
            .map(|id| {
                possible_periods
                    .get(id)
                    .map(|p| p.as_slice().len())
                    .unwrap_or(0)
            })
            .sum();

        Self {
            id: block.id,
            task_ids,
            priority,
            impatience_denominator: total_periods.max(1),
        }
    }

    /// Return `true` if all proposal tasks are placed in `schedule`.
    pub fn is_complete(&self, schedule: &Schedule) -> bool {
        self.task_ids.iter().all(|id| schedule.contains(*id))
    }

    /// Number of tasks in this proposal.
    pub fn task_count(&self) -> usize {
        self.task_ids.len()
    }

    /// Fractional completion contribution: `(placed / total) * priority`.
    pub fn completion_contribution(&self, schedule: &Schedule) -> f64 {
        if self.task_ids.is_empty() {
            return 0.0;
        }
        let placed = self
            .task_ids
            .iter()
            .filter(|id| schedule.contains(**id))
            .count();
        (placed as f64 / self.task_ids.len() as f64) * self.priority
    }
}

/// `completion_fitness` = sum over all proposals of `(placed/total) * priority`.
pub fn completion_fitness(schedule: &Schedule, proposals: &[Proposal]) -> f64 {
    proposals
        .iter()
        .map(|p| p.completion_contribution(schedule))
        .sum()
}

/// `science_time_fitness` = total scheduled duration in MJD days.
pub fn science_time_fitness(schedule: &Schedule) -> f64 {
    schedule
        .placements()
        .map(|p| p.end.value() - p.start.value())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schedule::TaskPlacement;
    use crate::time::{MJD, SchedulingBlockId, TaskId, Time};

    fn make_proposal(task_ids: Vec<TaskId>, priority: f64) -> Proposal {
        Proposal {
            id: SchedulingBlockId(1),
            task_ids,
            priority,
            impatience_denominator: 10,
        }
    }

    fn placement(task_id: TaskId, start: f64, end: f64) -> TaskPlacement {
        TaskPlacement {
            task_id,
            start: Time::<MJD>::new(start),
            end: Time::<MJD>::new(end),
        }
    }

    #[test]
    fn completion_contribution_zero_when_nothing_placed() {
        let p = make_proposal(vec![TaskId(1), TaskId(2)], 3.0);
        let schedule = Schedule::new();
        assert_eq!(p.completion_contribution(&schedule), 0.0);
    }

    #[test]
    fn completion_contribution_full_when_all_placed() {
        let p = make_proposal(vec![TaskId(1)], 2.0);
        let mut schedule = Schedule::new();
        schedule.insert_placement(placement(TaskId(1), 0.0, 1.0));
        assert_eq!(p.completion_contribution(&schedule), 2.0);
    }

    #[test]
    fn completion_contribution_partial() {
        let p = make_proposal(vec![TaskId(1), TaskId(2)], 4.0);
        let mut schedule = Schedule::new();
        schedule.insert_placement(placement(TaskId(1), 0.0, 1.0));
        assert_eq!(p.completion_contribution(&schedule), 2.0);
    }

    #[test]
    fn is_complete_checks_all_tasks() {
        let p = make_proposal(vec![TaskId(1), TaskId(2)], 1.0);
        let mut schedule = Schedule::new();
        schedule.insert_placement(placement(TaskId(1), 0.0, 1.0));
        assert!(!p.is_complete(&schedule));
        schedule.insert_placement(placement(TaskId(2), 1.0, 2.0));
        assert!(p.is_complete(&schedule));
    }

    #[test]
    fn completion_fitness_sums_proposals() {
        let p1 = make_proposal(vec![TaskId(1)], 2.0);
        let p2 = make_proposal(vec![TaskId(2)], 3.0);
        let mut schedule = Schedule::new();
        schedule.insert_placement(placement(TaskId(1), 0.0, 1.0));
        // p1 complete (2.0), p2 incomplete (0.0)
        assert_eq!(completion_fitness(&schedule, &[p1, p2]), 2.0);
    }

    #[test]
    fn science_time_fitness_sums_durations() {
        let mut schedule = Schedule::new();
        schedule.insert_placement(placement(TaskId(1), 0.0, 2.0));
        schedule.insert_placement(placement(TaskId(2), 5.0, 7.0));
        assert!((science_time_fitness(&schedule) - 4.0).abs() < 1e-10);
    }
}
