//! Lexicographic schedule ranking and survivor selection for the HAP scheduler.

use super::proposal::{Proposal, completion_fitness, science_time_fitness};
use crate::schedule::Schedule;

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
pub fn compare_schedules(a: &Schedule, b: &Schedule, proposals: &[Proposal]) -> std::cmp::Ordering {
    let cf_a = completion_fitness(a, proposals);
    let cf_b = completion_fitness(b, proposals);
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
    proposals: &[Proposal],
) -> Vec<Schedule> {
    candidates.sort_by(|a, b| compare_schedules(a, b, proposals));
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
    use crate::schedule::TaskPlacement;
    use crate::scheduler::hap::proposal::Proposal;
    use crate::time::{MJD, SchedulingBlockId, TaskId, Time};

    fn placement(task_id: TaskId, start: f64, end: f64) -> TaskPlacement {
        TaskPlacement {
            task_id,
            start: Time::<MJD>::new(start),
            end: Time::<MJD>::new(end),
            block_id: None,
        }
    }

    fn make_proposal(id: u64, task_ids: Vec<TaskId>, priority: f64) -> Proposal {
        Proposal {
            id: SchedulingBlockId(id),
            task_ids,
            priority,
            impatience_denominator: 10,
        }
    }

    #[test]
    fn compare_prefers_more_completed_proposals() {
        let proposals = [make_proposal(1, vec![TaskId(1)], 1.0)];

        let mut s_complete = Schedule::new();
        s_complete.insert_placement(placement(TaskId(1), 0.0, 1.0));

        let s_empty = Schedule::new();

        // s_complete should be "Less" (better, comes first)
        assert_eq!(
            compare_schedules(&s_complete, &s_empty, &proposals),
            std::cmp::Ordering::Less
        );
        assert_eq!(
            compare_schedules(&s_empty, &s_complete, &proposals),
            std::cmp::Ordering::Greater
        );
    }

    #[test]
    fn compare_equal_schedules_are_equal() {
        let proposals: Vec<Proposal> = vec![];
        let s = Schedule::new();
        assert_eq!(
            compare_schedules(&s, &s, &proposals),
            std::cmp::Ordering::Equal
        );
    }

    #[test]
    fn select_top_n_keeps_best() {
        let proposals = [make_proposal(1, vec![TaskId(1)], 1.0)];

        let mut s1 = Schedule::new();
        s1.insert_placement(placement(TaskId(1), 0.0, 1.0));
        let s2 = Schedule::new();

        let result = select_top_n(vec![s2, s1], 1, &proposals);
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
