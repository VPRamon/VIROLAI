//! Planner-level survivor selectors.
//!
//! After CRU has produced its candidate set for a block, the accumulative
//! planner reduces that set down to the schedules that survive into the
//! next block iteration. The reduction policy is one of:
//!
//! - [`SurvivorSelector::GreedyOne`] — keep the single best (AP).
//! - [`SurvivorSelector::ElitistTopK`] — keep the top `k` by scalar fitness.
//! - [`SurvivorSelector::ParetoFront`] — keep the non-dominated frontier
//!   over `(scheduling_rate, priority_sum)`, with crowding-distance
//!   pruning when the front exceeds `cap`.

use super::configuration::SurvivorSelector;
use super::eval::{
    completion_fitness, placement_fingerprint, priority_sum, scheduling_rate, science_time,
};
use crate::schedule::{Schedule, SchedulingProblem};
use crate::time::{MJD, Time};

/// Apply `selector` to `candidates` and return the surviving schedules.
///
/// The returned vector is non-empty whenever `candidates` is non-empty.
pub fn select(
    selector: SurvivorSelector,
    candidates: Vec<Schedule>,
    problem: &SchedulingProblem,
    horizon_start: Time<MJD>,
) -> Vec<Schedule> {
    if candidates.is_empty() {
        return candidates;
    }
    match selector {
        SurvivorSelector::GreedyOne => greedy_one(candidates, problem, horizon_start),
        SurvivorSelector::ElitistTopK { k } => {
            elitist_top_k(candidates, k.max(1), problem, horizon_start)
        }
        SurvivorSelector::ParetoFront { cap } => {
            pareto_front(candidates, cap.max(1), problem, horizon_start)
        }
    }
}

/// Lexicographic compare for greedy/elitist: `(completion_fitness DESC,
/// science_time DESC, len DESC, fingerprint ASC)`.
///
/// Returns `Less` when `a` is *better* than `b` (so `sort_by` puts the
/// best first).
fn cmp_better_first(
    a: &Schedule,
    b: &Schedule,
    problem: &SchedulingProblem,
    horizon_start: Time<MJD>,
) -> std::cmp::Ordering {
    let cf = completion_fitness(b, problem, horizon_start).total_cmp(&completion_fitness(
        a,
        problem,
        horizon_start,
    ));
    if cf != std::cmp::Ordering::Equal {
        return cf;
    }
    let st = science_time(b).total_cmp(&science_time(a));
    if st != std::cmp::Ordering::Equal {
        return st;
    }
    let len = b.len().cmp(&a.len());
    if len != std::cmp::Ordering::Equal {
        return len;
    }
    placement_fingerprint(a).cmp(&placement_fingerprint(b))
}

fn greedy_one(
    mut candidates: Vec<Schedule>,
    problem: &SchedulingProblem,
    horizon_start: Time<MJD>,
) -> Vec<Schedule> {
    candidates.sort_by(|a, b| cmp_better_first(a, b, problem, horizon_start));
    candidates.truncate(1);
    candidates
}

fn elitist_top_k(
    mut candidates: Vec<Schedule>,
    k: usize,
    problem: &SchedulingProblem,
    horizon_start: Time<MJD>,
) -> Vec<Schedule> {
    candidates.sort_by(|a, b| cmp_better_first(a, b, problem, horizon_start));
    candidates.truncate(k);
    candidates
}

/// `(scheduling_rate, priority_sum)` — both maximised.
fn objectives(
    schedule: &Schedule,
    problem: &SchedulingProblem,
    horizon_start: Time<MJD>,
) -> (f64, f64) {
    (
        scheduling_rate(schedule, problem),
        priority_sum(schedule, problem, horizon_start),
    )
}

/// `a` dominates `b` iff `a` is `>=` on every objective and `>` on at
/// least one.
fn dominates(a: (f64, f64), b: (f64, f64)) -> bool {
    let (a1, a2) = a;
    let (b1, b2) = b;
    let ge = a1 >= b1 && a2 >= b2;
    let gt = a1 > b1 || a2 > b2;
    ge && gt
}

fn pareto_front(
    candidates: Vec<Schedule>,
    cap: usize,
    problem: &SchedulingProblem,
    horizon_start: Time<MJD>,
) -> Vec<Schedule> {
    let scored: Vec<((f64, f64), Schedule)> = candidates
        .into_iter()
        .map(|s| (objectives(&s, problem, horizon_start), s))
        .collect();

    let mut front: Vec<((f64, f64), Schedule)> = Vec::new();
    for (obj, sched) in scored {
        if front.iter().any(|(o, _)| dominates(*o, obj)) {
            continue;
        }
        front.retain(|(o, _)| !dominates(obj, *o));
        front.push((obj, sched));
    }

    if front.len() <= cap {
        return front.into_iter().map(|(_, s)| s).collect();
    }

    crowding_prune(front, cap)
}

/// NSGA-II-style crowding-distance pruning: keep the `cap` schedules with
/// the largest crowding distance. Boundary points get +∞ to preserve the
/// extremes of every objective.
fn crowding_prune(mut front: Vec<((f64, f64), Schedule)>, cap: usize) -> Vec<Schedule> {
    let n = front.len();
    let mut distance = vec![0.0_f64; n];

    for axis in 0..2 {
        let mut idx: Vec<usize> = (0..n).collect();
        idx.sort_by(|&i, &j| {
            let (oi, oj) = if axis == 0 {
                (front[i].0.0, front[j].0.0)
            } else {
                (front[i].0.1, front[j].0.1)
            };
            oi.total_cmp(&oj)
        });
        distance[idx[0]] = f64::INFINITY;
        distance[idx[n - 1]] = f64::INFINITY;
        let lo = if axis == 0 {
            front[idx[0]].0.0
        } else {
            front[idx[0]].0.1
        };
        let hi = if axis == 0 {
            front[idx[n - 1]].0.0
        } else {
            front[idx[n - 1]].0.1
        };
        let range = (hi - lo).abs();
        if range == 0.0 {
            continue;
        }
        for k in 1..n - 1 {
            let prev = if axis == 0 {
                front[idx[k - 1]].0.0
            } else {
                front[idx[k - 1]].0.1
            };
            let next = if axis == 0 {
                front[idx[k + 1]].0.0
            } else {
                front[idx[k + 1]].0.1
            };
            distance[idx[k]] += (next - prev) / range;
        }
    }

    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&i, &j| distance[j].total_cmp(&distance[i]));
    order.truncate(cap);
    order.sort();

    front
        .drain(..)
        .enumerate()
        .filter_map(|(i, (_, s))| {
            if order.binary_search(&i).is_ok() {
                Some(s)
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_problem() -> SchedulingProblem {
        SchedulingProblem::new()
    }

    fn t0() -> Time<MJD> {
        Time::<MJD>::new(60000.0)
    }

    #[test]
    fn select_returns_empty_when_input_empty() {
        let out = select(SurvivorSelector::GreedyOne, vec![], &empty_problem(), t0());
        assert!(out.is_empty());
    }

    #[test]
    fn greedy_one_keeps_single_schedule() {
        let s = Schedule::new();
        let out = select(SurvivorSelector::GreedyOne, vec![s], &empty_problem(), t0());
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn dominates_basic() {
        assert!(dominates((0.5, 10.0), (0.4, 9.0)));
        assert!(dominates((0.5, 10.0), (0.5, 9.0)));
        assert!(!dominates((0.5, 10.0), (0.5, 10.0)));
        assert!(!dominates((0.5, 9.0), (0.4, 10.0)));
    }
}
