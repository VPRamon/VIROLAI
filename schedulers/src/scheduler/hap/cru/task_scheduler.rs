//! Task Scheduling Cycle inner step for HAP/CRU.
//!
//! [`schedule_task`] places one task from the lobby into the schedule. It
//! computes all valid [`Candidate`] placements for the task given the current
//! schedule state, sorts them by insertion cost, runs the configured
//! [`Selector`] over the cheapest tier, evicts displaced tasks to the
//! [`Lobby`], and inserts the placement.
//!
//! [`task_scheduling_cycle`] drains a seeded [`Lobby`] (one initial task plus
//! any tasks evicted during the run) and tracks the lowest-`lobby_cost`
//! schedule seen — corresponding to `s_low`/`cost_min` in the CRU pseudo-code
//! — so the caller can roll back to it on exit.

use super::super::configuration::{Configuration, Selector};
use super::lobby::Lobby;
use crate::prescheduler::TaskPeriodMap;
use crate::schedule::{Schedule, TaskPlacement};
use crate::scheduling_block::SchedulingBlock;
use crate::task::Task;
use crate::time::{MJD, Period, PeriodSet, TaskId};
use qtty::Day;
use rand::Rng;
use std::collections::{HashMap, HashSet};

/// Errors that can occur during a single task-scheduling step.
#[derive(Debug)]
pub enum TaskSchedulerError {
    /// The task has no entry in the periods map, or its feasibility set is empty.
    NoFeasibilityWindows,
    /// Every candidate period conflicts with a protected (block) task.
    NoValidCandidates,
}

impl std::fmt::Display for TaskSchedulerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoFeasibilityWindows => {
                write!(f, "task has no feasibility windows in the periods map")
            }
            Self::NoValidCandidates => {
                write!(
                    f,
                    "no valid candidate periods: all conflict with protected (block) tasks"
                )
            }
        }
    }
}

impl std::error::Error for TaskSchedulerError {}

/// A fully-evaluated placement option for one task.
struct Candidate {
    /// Scheduled interval `[start, start + duration)`.
    period: Period<MJD>,
    /// Non-protected tasks displaced by this placement.
    conflicts: Vec<TaskId>,
    /// Insertion cost: number of tasks that would be displaced.
    ///
    /// Local conflict cost is intentionally simple: the count of evicted
    /// units. Callers wanting a richer cost (e.g. priority-weighted) can
    /// adjust this single computation site.
    cost: usize,
}

/// Build all valid, non-protected-conflicting [`Candidate`]s for `task`.
///
/// Two sources of candidate start times are considered per feasibility window:
///
/// 1. **Window start** – the beginning of the window, if the task fits.
/// 2. **After conflict** – immediately after each placed task that overlaps the
///    window, if the task still fits before the window closes.
///
/// Candidates that would displace a protected (block) task are excluded.
/// The returned list is sorted by `(cost, period.start, conflict count)`
/// ascending so every selector sees the same canonical ordering and the
/// deterministic selector can rely on lexicographic tie-break.
fn build_candidates(
    task: &Task,
    windows: &PeriodSet<MJD>,
    schedule: &Schedule,
    protected_ids: &HashSet<TaskId>,
) -> Vec<Candidate> {
    let duration = task.duration.to::<Day>();
    let mut candidates: Vec<Candidate> = Vec::new();

    for window in windows.iter() {
        if window.duration() < duration {
            continue;
        }

        let ws = window.start;
        let we = window.end;

        let mut starts = vec![ws];
        let window_interval = Period::new(ws, we);
        for overlapping_id in schedule.overlapping(&window_interval) {
            if let Some(placement) = schedule.get(overlapping_id) {
                let after = placement.end;
                if after > ws {
                    starts.push(after);
                }
            }
        }

        for start in starts {
            let end = start + duration;
            if end > we {
                continue;
            }
            let period = Period::new(start, end);
            let overlapping = schedule.overlapping(&period);

            if overlapping.iter().any(|id| protected_ids.contains(id)) {
                continue;
            }

            let cost = overlapping.len();
            candidates.push(Candidate {
                period,
                conflicts: overlapping,
                cost,
            });
        }
    }

    candidates.sort_by(|a, b| {
        a.cost
            .cmp(&b.cost)
            .then_with(|| a.period.start.value().total_cmp(&b.period.start.value()))
            .then_with(|| a.conflicts.len().cmp(&b.conflicts.len()))
    });
    candidates
}

/// Pick a candidate index from a cost-sorted slice using `selector`.
///
/// Zero-cost candidates always win, regardless of the selector — this
/// implements the spec's "prefer zero-conflict insertion" requirement.
fn choose_candidate_idx(candidates: &[Candidate], selector: Selector, rng: &mut impl Rng) -> usize {
    debug_assert!(!candidates.is_empty());

    let zero_count = candidates.partition_point(|c| c.cost == 0);
    if zero_count > 0 {
        return 0;
    }

    match selector {
        Selector::Deterministic => 0,
        Selector::Stochastic { rho } => {
            let range = rho.max(1).min(candidates.len());
            rng.gen_range(0..range)
        }
        Selector::Random => weighted_random_pick(candidates, rng),
    }
}

/// Weight ∝ 1 / (1 + cost). Lower cost ⇒ higher probability.
fn weighted_random_pick(candidates: &[Candidate], rng: &mut impl Rng) -> usize {
    let weights: Vec<f64> = candidates
        .iter()
        .map(|c| 1.0 / (1.0 + c.cost as f64))
        .collect();
    let total: f64 = weights.iter().sum();
    if total <= 0.0 {
        return 0;
    }
    let mut r = rng.gen_range(0.0..total);
    for (i, w) in weights.iter().enumerate() {
        if r < *w {
            return i;
        }
        r -= w;
    }
    candidates.len() - 1
}

/// Place `task` into `schedule`, evicting non-protected conflicts to `lobby`.
///
/// # Errors
///
/// Returns a [`TaskSchedulerError`] if the task has no feasibility windows
/// or if all candidate periods conflict with a protected task.
#[allow(clippy::too_many_arguments)]
pub fn schedule_task(
    task: &Task,
    block: &SchedulingBlock,
    schedule: &mut Schedule,
    periods_map: &TaskPeriodMap,
    lobby: &mut Lobby,
    run_protected: &HashSet<TaskId>,
    config: &Configuration,
    rng: &mut impl Rng,
) -> Result<(), TaskSchedulerError> {
    debug_assert!(
        block.contains_task(task.id),
        "task {} is not a member of block {}",
        task.id.0,
        block.id.0
    );

    let windows = periods_map
        .get(&task.id)
        .filter(|w| !w.is_empty())
        .ok_or(TaskSchedulerError::NoFeasibilityWindows)?;

    let mut protected_ids: HashSet<TaskId> = block.iter().collect();
    protected_ids.extend(run_protected.iter().copied());

    let candidates = build_candidates(task, windows, schedule, &protected_ids);
    if candidates.is_empty() {
        return Err(TaskSchedulerError::NoValidCandidates);
    }

    let chosen_idx = choose_candidate_idx(&candidates, config.selector, rng);
    let chosen = &candidates[chosen_idx];

    for &conflict_id in &chosen.conflicts {
        let _ = schedule.unplace_task(conflict_id);
        lobby.push(conflict_id);
    }

    schedule.insert_placement(TaskPlacement {
        task_id: task.id,
        start: chosen.period.start,
        end: chosen.period.end,
    });

    Ok(())
}

/// Cheapest insertion cost for `task` against the current `schedule`.
fn min_window_cost(
    task: &Task,
    windows: &PeriodSet<MJD>,
    schedule: &Schedule,
    protected_ids: &HashSet<TaskId>,
) -> Option<usize> {
    let candidates = build_candidates(task, windows, schedule, protected_ids);
    candidates.first().map(|c| c.cost)
}

/// Sum of cheapest insertion costs across the lobby.
///
/// This is the CRU `cost_lobby` — a local, monotone proxy for "how much
/// repair is still pending". A task with no valid window contributes
/// `usize::MAX` (saturating) so its presence cannot lower the metric.
fn lobby_cost(
    lobby: &Lobby,
    schedule: &Schedule,
    task_index: &HashMap<TaskId, &Task>,
    periods_map: &TaskPeriodMap,
    protected_ids: &HashSet<TaskId>,
) -> usize {
    let mut total: usize = 0;
    for id in lobby.iter() {
        let cost = match (task_index.get(id), periods_map.get(id)) {
            (Some(task), Some(windows)) if !windows.is_empty() => {
                min_window_cost(task, windows, schedule, protected_ids).unwrap_or(usize::MAX)
            }
            _ => usize::MAX,
        };
        total = total.saturating_add(cost);
    }
    total
}

/// Run the inner Task Scheduling Cycle for one block task.
///
/// Drains `lobby` until it is empty or `config.max_iter` iterations have
/// been used. Tasks evicted by [`schedule_task`] re-enter the lobby and are
/// re-attempted in subsequent iterations. While the cycle runs, the
/// schedule with the lowest observed `lobby_cost` is snapshotted and
/// restored on exit (the CRU `s_low`/`cost_min` recovery rule).
///
/// `task_index` resolves any [`TaskId`] in the lobby (or evicted from the
/// schedule) back to its owning [`Task`] / [`SchedulingBlock`] pair so the
/// inner cycle can keep using each task's own block as its protected set.
pub fn task_scheduling_cycle(
    initial_task_id: TaskId,
    schedule: &mut Schedule,
    task_index: &HashMap<TaskId, (&Task, &SchedulingBlock)>,
    periods_map: &TaskPeriodMap,
    config: &Configuration,
    rng: &mut impl Rng,
) {
    let mut lobby = Lobby::new();
    lobby.push(initial_task_id);

    let mut run_protected: HashSet<TaskId> = HashSet::new();
    let mut min_cost: usize = usize::MAX;
    let mut best_schedule: Schedule = schedule.clone();

    let task_only_index: HashMap<TaskId, &Task> =
        task_index.iter().map(|(k, (t, _))| (*k, *t)).collect();

    let mut iter = 0usize;
    while let Some(task_id) = lobby.pop() {
        if iter >= config.max_iter {
            break;
        }
        iter += 1;

        let Some((task, owner_block)) = task_index.get(&task_id).copied() else {
            continue;
        };

        let attempt = schedule_task(
            task,
            owner_block,
            schedule,
            periods_map,
            &mut lobby,
            &run_protected,
            config,
            rng,
        );

        if attempt.is_ok() {
            run_protected.insert(task_id);
        }

        let current_cost = lobby_cost(
            &lobby,
            schedule,
            &task_only_index,
            periods_map,
            &run_protected,
        );

        if current_cost < min_cost {
            min_cost = current_cost;
            best_schedule = schedule.clone();
        }
    }

    *schedule = best_schedule;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduling_block::SchedulingBlock;
    use crate::time::{MJD, SchedulingBlockId, TaskId, Time};
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    fn make_block_id(n: u64) -> SchedulingBlockId {
        SchedulingBlockId(n)
    }

    fn make_task_id(n: u64) -> TaskId {
        TaskId(n)
    }

    fn mjd(v: f64) -> Time<MJD> {
        Time::<MJD>::new(v)
    }

    fn period(s: f64, e: f64) -> Period<MJD> {
        Period::new(mjd(s), mjd(e))
    }

    fn make_task(id: u64, duration_days: f64) -> crate::task::Task {
        use crate::constraints::ConstraintBlocks;
        use qtty::Seconds;
        use siderust::coordinates::{frames::ICRS, spherical::Direction};

        let duration_secs = duration_days * 86400.0;
        crate::task::Task::new(
            make_task_id(id),
            format!("task-{id}"),
            Direction::<ICRS>::new_raw(0.0.into(), 0.0.into()),
            Seconds::new(duration_secs),
            ConstraintBlocks::default(),
            None,
        )
        .unwrap()
    }

    fn seeded_rng() -> StdRng {
        StdRng::seed_from_u64(42)
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic]
    fn panics_when_task_not_in_block() {
        let task = make_task(1, 1.0);
        let empty_block = SchedulingBlock::new(make_block_id(99));
        let mut schedule = Schedule::new();
        let mut periods_map = TaskPeriodMap::new();
        periods_map.insert(task.id, PeriodSet::from_periods(vec![period(0.0, 5.0)]));
        let mut lobby = Lobby::new();
        let mut rng = seeded_rng();

        let _ = schedule_task(
            &task,
            &empty_block,
            &mut schedule,
            &periods_map,
            &mut lobby,
            &HashSet::new(),
            &Configuration::default(),
            &mut rng,
        );
    }

    #[test]
    fn returns_error_when_no_windows() {
        let task = make_task(1, 1.0);
        let mut block = SchedulingBlock::new(make_block_id(1));
        block.push_task(make_task(1, 1.0)).unwrap();
        let mut schedule = Schedule::new();
        let periods_map = TaskPeriodMap::new();
        let mut lobby = Lobby::new();
        let mut rng = seeded_rng();

        let result = schedule_task(
            &task,
            &block,
            &mut schedule,
            &periods_map,
            &mut lobby,
            &HashSet::new(),
            &Configuration::default(),
            &mut rng,
        );
        assert!(matches!(
            result,
            Err(TaskSchedulerError::NoFeasibilityWindows)
        ));
    }

    #[test]
    fn places_task_in_empty_schedule() {
        let task = make_task(1, 1.0);
        let mut block = SchedulingBlock::new(make_block_id(1));
        block.push_task(make_task(1, 1.0)).unwrap();

        let mut schedule = Schedule::new();
        let mut periods_map = TaskPeriodMap::new();
        periods_map.insert(task.id, PeriodSet::from_periods(vec![period(0.0, 5.0)]));

        let mut lobby = Lobby::new();
        let mut rng = seeded_rng();

        schedule_task(
            &task,
            &block,
            &mut schedule,
            &periods_map,
            &mut lobby,
            &HashSet::new(),
            &Configuration::default(),
            &mut rng,
        )
        .unwrap();

        assert!(schedule.contains(task.id));
        assert!(lobby.is_empty());
    }

    /// Window [0, 2.5), squatter at [1, 2), duration = 2.0 days.
    /// Only valid candidate has cost 1, so the squatter is evicted.
    #[test]
    fn evicts_non_protected_conflict_to_lobby() {
        let task = make_task(1, 2.0);
        let squatter = make_task(2, 1.0);

        let mut block = SchedulingBlock::new(make_block_id(1));
        block.push_task(make_task(1, 2.0)).unwrap();

        let mut schedule = Schedule::new();
        schedule.insert_placement(TaskPlacement {
            task_id: squatter.id,
            start: mjd(1.0),
            end: mjd(2.0),
        });

        let mut periods_map = TaskPeriodMap::new();
        periods_map.insert(task.id, PeriodSet::from_periods(vec![period(0.0, 2.5)]));

        let mut lobby = Lobby::new();
        let mut rng = seeded_rng();

        schedule_task(
            &task,
            &block,
            &mut schedule,
            &periods_map,
            &mut lobby,
            &HashSet::new(),
            &Configuration::default(),
            &mut rng,
        )
        .unwrap();

        assert!(schedule.contains(task.id));
        assert!(!schedule.contains(squatter.id));
        assert_eq!(lobby.len(), 1);
    }

    /// Window [0, 5), squatter at [0, 1), duration = 2.0 days.
    /// Zero-cost candidate wins; squatter survives.
    #[test]
    fn prefers_zero_cost_slot_over_eviction() {
        let task = make_task(1, 2.0);
        let squatter = make_task(2, 1.0);

        let mut block = SchedulingBlock::new(make_block_id(1));
        block.push_task(make_task(1, 2.0)).unwrap();

        let mut schedule = Schedule::new();
        schedule.insert_placement(TaskPlacement {
            task_id: squatter.id,
            start: mjd(0.0),
            end: mjd(1.0),
        });

        let mut periods_map = TaskPeriodMap::new();
        periods_map.insert(task.id, PeriodSet::from_periods(vec![period(0.0, 5.0)]));

        let mut lobby = Lobby::new();
        let mut rng = seeded_rng();

        schedule_task(
            &task,
            &block,
            &mut schedule,
            &periods_map,
            &mut lobby,
            &HashSet::new(),
            &Configuration::default(),
            &mut rng,
        )
        .unwrap();

        assert!(schedule.contains(task.id));
        assert!(schedule.contains(squatter.id));
        assert!(lobby.is_empty());
        let placement = schedule.get(task.id).unwrap();
        assert_eq!(placement.start, mjd(1.0));
        assert_eq!(placement.end, mjd(3.0));
    }

    /// Two runs with the same inputs and seed must produce the same placement
    /// under the deterministic selector (which never consults the RNG).
    #[test]
    fn deterministic_selector_is_reproducible() {
        fn run_once() -> Period<MJD> {
            let task = make_task(1, 2.0);
            let mut block = SchedulingBlock::new(make_block_id(1));
            block.push_task(make_task(1, 2.0)).unwrap();
            let mut schedule = Schedule::new();
            let mut periods_map = TaskPeriodMap::new();
            periods_map.insert(task.id, PeriodSet::from_periods(vec![period(0.0, 5.0)]));
            let mut lobby = Lobby::new();
            let mut rng = StdRng::seed_from_u64(0);
            let cfg = Configuration {
                selector: Selector::Deterministic,
                ..Configuration::default()
            };
            schedule_task(
                &task,
                &block,
                &mut schedule,
                &periods_map,
                &mut lobby,
                &HashSet::new(),
                &cfg,
                &mut rng,
            )
            .unwrap();
            let p = schedule.get(task.id).unwrap();
            Period::new(p.start, p.end)
        }
        assert_eq!(run_once(), run_once());
    }
}
