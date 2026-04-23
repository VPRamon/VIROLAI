//! Block-based evaluation helpers for the HAP scheduler.

use crate::prescheduler::TaskPeriodMap;
use crate::schedule::{Schedule, SchedulingProblem};
use crate::scheduling_block::SchedulingBlock;
use crate::time::{MJD, Time};

pub(super) fn block_priority(
    block: &SchedulingBlock,
    problem: &SchedulingProblem,
    horizon_start: Time<MJD>,
) -> f64 {
    block
        .iter()
        .map(|id| {
            problem
                .task(id)
                .and_then(|task| {
                    task.soft_constraints.as_ref().map(|constraints| {
                        constraints.score(&horizon_start, None, Some(&task.target))
                    })
                })
                .unwrap_or(0.0)
        })
        .sum()
}

pub(super) fn block_task_count(block: &SchedulingBlock) -> usize {
    block.iter().count()
}

pub(super) fn block_impatience_denominator(
    block: &SchedulingBlock,
    possible_periods: &TaskPeriodMap,
) -> usize {
    block
        .iter()
        .map(|id| {
            possible_periods
                .get(&id)
                .map(|periods| periods.as_slice().len())
                .unwrap_or(0)
        })
        .sum::<usize>()
        .max(1)
}

pub(super) fn block_is_complete(block: &SchedulingBlock, schedule: &Schedule) -> bool {
    block.iter().all(|id| schedule.contains(id))
}

pub(super) fn block_completion_contribution(
    block: &SchedulingBlock,
    schedule: &Schedule,
    problem: &SchedulingProblem,
    horizon_start: Time<MJD>,
) -> f64 {
    let task_count = block_task_count(block);
    if task_count == 0 {
        return 0.0;
    }

    let placed = block.iter().filter(|id| schedule.contains(*id)).count();
    (placed as f64 / task_count as f64) * block_priority(block, problem, horizon_start)
}

pub(super) fn completion_fitness(
    schedule: &Schedule,
    problem: &SchedulingProblem,
    horizon_start: Time<MJD>,
) -> f64 {
    problem
        .blocks()
        .iter()
        .map(|block| block_completion_contribution(block, schedule, problem, horizon_start))
        .sum()
}

pub(super) fn min_positive_block_priority(
    problem: &SchedulingProblem,
    horizon_start: Time<MJD>,
) -> f64 {
    problem
        .blocks()
        .iter()
        .map(|block| block_priority(block, problem, horizon_start))
        .filter(|&priority| priority > 0.0)
        .reduce(f64::min)
        .unwrap_or(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constraints::ConstraintExpr;
    use crate::constraints::SoftConstraintExpr;
    use crate::constraints::soft::PrioritySoftConstraint;
    use crate::schedule::SchedulingProblem;
    use crate::schedule::TaskPlacement;
    use crate::task::Task;
    use crate::time::{PeriodSet, SchedulingBlockId, TaskId};
    use qtty::{Degrees, Seconds};
    use siderust::coordinates::frames::ICRS;
    use siderust::coordinates::spherical::Direction;

    fn make_task(id: u64, priority: f64) -> Task {
        Task::new(
            TaskId(id),
            format!("task-{id}"),
            Direction::<ICRS>::new_raw(Degrees::new(-16.716), Degrees::new(101.287)),
            Seconds::new(600.0),
            ConstraintExpr::Intersection(vec![]),
            Some(SoftConstraintExpr::atom(PrioritySoftConstraint::new(
                priority,
            ))),
        )
        .expect("task construction must succeed")
    }

    fn make_block(id: u64, task_specs: &[(u64, f64)]) -> SchedulingBlock {
        SchedulingBlock::from_tasks(
            SchedulingBlockId(id),
            task_specs
                .iter()
                .map(|(task_id, priority)| make_task(*task_id, *priority))
                .collect(),
        )
        .unwrap()
    }

    fn make_problem(blocks: Vec<SchedulingBlock>) -> SchedulingProblem {
        SchedulingProblem::from_blocks(blocks).unwrap()
    }

    fn placement(task_id: u64, start: f64, end: f64) -> TaskPlacement {
        TaskPlacement {
            task_id: TaskId(task_id),
            start: Time::<MJD>::new(start),
            end: Time::<MJD>::new(end),
        }
    }

    #[test]
    fn block_priority_sums_member_task_priorities() {
        let horizon_start = Time::<MJD>::new(60000.0);
        let problem = make_problem(vec![make_block(1, &[(1, 2.0), (2, 3.5)])]);
        let block = problem.block(SchedulingBlockId(1)).unwrap();

        assert_eq!(block_priority(block, &problem, horizon_start), 5.5);
    }

    #[test]
    fn block_completion_contribution_tracks_fractional_progress() {
        let horizon_start = Time::<MJD>::new(60000.0);
        let problem = make_problem(vec![make_block(1, &[(1, 2.0), (2, 4.0)])]);
        let block = problem.block(SchedulingBlockId(1)).unwrap();

        let mut schedule = Schedule::new();
        schedule.insert_placement(placement(1, 0.0, 1.0));

        assert_eq!(
            block_completion_contribution(block, &schedule, &problem, horizon_start),
            3.0
        );
    }

    #[test]
    fn completion_fitness_sums_block_contributions() {
        let horizon_start = Time::<MJD>::new(60000.0);
        let problem = make_problem(vec![make_block(1, &[(1, 2.0)]), make_block(2, &[(2, 3.0)])]);

        let mut schedule = Schedule::new();
        schedule.insert_placement(placement(1, 0.0, 1.0));

        assert_eq!(completion_fitness(&schedule, &problem, horizon_start), 2.0);
    }

    #[test]
    fn min_positive_block_priority_uses_block_scores() {
        let horizon_start = Time::<MJD>::new(60000.0);
        let problem = make_problem(vec![
            make_block(1, &[(1, 4.0)]),
            make_block(2, &[(2, 0.0)]),
            make_block(3, &[(3, 2.5)]),
        ]);

        assert_eq!(min_positive_block_priority(&problem, horizon_start), 2.5);
    }

    #[test]
    fn block_impatience_denominator_uses_total_feasible_periods() {
        let block = make_block(1, &[(1, 0.0), (2, 0.0)]);
        let possible_periods = std::collections::HashMap::from([
            (TaskId(1), PeriodSet::from_periods(vec![])),
            (
                TaskId(2),
                PeriodSet::from_periods(vec![
                    crate::Period::new(Time::<MJD>::new(1.0), Time::<MJD>::new(2.0)),
                    crate::Period::new(Time::<MJD>::new(3.0), Time::<MJD>::new(4.0)),
                ]),
            ),
        ]);

        assert_eq!(block_impatience_denominator(&block, &possible_periods), 2);
    }
}
