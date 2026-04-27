//! Lexicographic schedule ranking and survivor selection for the HAP scheduler.

use super::block_eval::completion_fitness;
use crate::schedule::{Schedule, SchedulingProblem};
use crate::time::{MJD, Time};

/// Compare two schedules lexicographically by the HAP ranking criteria.
///
/// Ordering (earlier = "better", comes first in a sorted survivor list):
/// 1. `completion_fitness` **DESC**
/// 2. `science_time_fitness` **DESC**
/// 3. placed task count **DESC**
/// 4. sorted `(task_id, start_bits, end_bits)` tuples **ASC** (deterministic
///    tie-breaker)
///
/// Since this is used with `sort_by`, `Ordering::Less` means "comes first",
/// i.e., `a < b` means `a` is the better schedule.
pub fn compare_schedules(
    a: &Schedule,
    b: &Schedule,
    problem: &SchedulingProblem,
    horizon_start: Time<MJD>,
) -> std::cmp::Ordering {
    let cf_a = completion_fitness(a, problem, horizon_start);
    let cf_b = completion_fitness(b, problem, horizon_start);
    let cf_cmp = cf_b.total_cmp(&cf_a); // DESC: higher cf = Less (better)
    if cf_cmp != std::cmp::Ordering::Equal {
        return cf_cmp;
    }

    let st_a = science_time_fitness(a);
    let st_b = science_time_fitness(b);
    let st_cmp = st_b.total_cmp(&st_a); // DESC
    if st_cmp != std::cmp::Ordering::Equal {
        return st_cmp;
    }

    let count_cmp = b.len().cmp(&a.len()); // DESC
    if count_cmp != std::cmp::Ordering::Equal {
        return count_cmp;
    }

    // Deterministic tie-breaker: sorted placement tuples ASC
    let tuples_a = sorted_placement_tuples(a);
    let tuples_b = sorted_placement_tuples(b);
    tuples_a.cmp(&tuples_b)
}

fn science_time_fitness(schedule: &Schedule) -> f64 {
    schedule
        .placements()
        .map(|placement| placement.end.value() - placement.start.value())
        .sum()
}

fn sorted_placement_tuples(schedule: &Schedule) -> Vec<(u64, u64, u64)> {
    let mut tuples: Vec<_> = schedule
        .placements()
        .map(|p| {
            (
                p.task_id.0,
                p.start.value().to_bits(),
                p.end.value().to_bits(),
            )
        })
        .collect();
    tuples.sort_unstable();
    tuples
}

/// Select the best `n` schedules from `candidates` by HAP ranking.
pub fn select_top_n(
    mut candidates: Vec<Schedule>,
    n: usize,
    problem: &SchedulingProblem,
    horizon_start: Time<MJD>,
) -> Vec<Schedule> {
    candidates.sort_by(|a, b| compare_schedules(a, b, problem, horizon_start));
    candidates.truncate(n);
    candidates
}

/// Check whether two survivor sets are equivalent by comparing their sorted
/// placement-tuple keys.
pub fn survivor_sets_equal(a: &[Schedule], b: &[Schedule]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let key = |schedules: &[Schedule]| -> Vec<Vec<(u64, u64, u64)>> {
        let mut keys: Vec<_> = schedules.iter().map(sorted_placement_tuples).collect();
        keys.sort();
        keys
    };
    key(a) == key(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constraints::ConstraintExpr;
    use crate::constraints::SoftConstraintExpr;
    use crate::constraints::soft::PrioritySoftConstraint;
    use crate::schedule::{SchedulingProblem, TaskPlacement};
    use crate::scheduling_block::SchedulingBlock;
    use crate::task::Task;
    use crate::time::{SchedulingBlockId, TaskId};
    use qtty::{Degrees, Seconds};
    use siderust::coordinates::frames::ICRS;
    use siderust::coordinates::spherical::Direction;

    fn placement(task_id: TaskId, start: f64, end: f64) -> TaskPlacement {
        TaskPlacement {
            task_id,
            start: Time::<MJD>::new(start),
            end: Time::<MJD>::new(end),
        }
    }

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

    #[test]
    fn compare_prefers_more_completed_blocks() {
        let horizon_start = Time::<MJD>::new(60000.0);
        let problem = make_problem(vec![make_block(1, &[(1, 1.0)])]);

        let mut s_complete = Schedule::new();
        s_complete.insert_placement(placement(TaskId(1), 0.0, 1.0));

        let s_empty = Schedule::new();

        // s_complete should be "Less" (better, comes first)
        assert_eq!(
            compare_schedules(&s_complete, &s_empty, &problem, horizon_start),
            std::cmp::Ordering::Less
        );
        assert_eq!(
            compare_schedules(&s_empty, &s_complete, &problem, horizon_start),
            std::cmp::Ordering::Greater
        );
    }

    #[test]
    fn compare_equal_schedules_are_equal() {
        let horizon_start = Time::<MJD>::new(60000.0);
        let problem = SchedulingProblem::new();
        let s = Schedule::new();
        assert_eq!(
            compare_schedules(&s, &s, &problem, horizon_start),
            std::cmp::Ordering::Equal
        );
    }

    #[test]
    fn select_top_n_keeps_best() {
        let horizon_start = Time::<MJD>::new(60000.0);
        let problem = make_problem(vec![make_block(1, &[(1, 1.0)])]);

        let mut s1 = Schedule::new();
        s1.insert_placement(placement(TaskId(1), 0.0, 1.0));
        let s2 = Schedule::new();

        let result = select_top_n(vec![s2, s1], 1, &problem, horizon_start);
        assert_eq!(result.len(), 1);
        // The selected schedule must contain task 1
        assert!(result[0].contains(TaskId(1)));
    }

    #[test]
    fn survivor_sets_equal_detects_same_and_different() {
        let mut s1 = Schedule::new();
        s1.insert_placement(placement(TaskId(1), 0.0, 1.0));
        let s2 = Schedule::new();

        assert!(survivor_sets_equal(&[s1.clone()], &[s1.clone()]));
        assert!(!survivor_sets_equal(&[s1.clone()], &[s2]));
    }
}
