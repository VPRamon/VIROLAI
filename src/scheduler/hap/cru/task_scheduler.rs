//! Task Scheduling Cycle inner step for HAP/CRU.
//!
//! [`schedule_task`] places one task from the lobby into the schedule.
//! It computes all valid [`Candidate`] placements for the task given the
//! current schedule state, sorts them by insertion cost, selects one
//! stochastically from the cheapest tier, evicts displaced tasks to the
//! [`Lobby`], and inserts the placement.

use super::super::configuration::Configuration;
use super::lobby::Lobby;
use crate::prescheduler::TaskPeriodMap;
use crate::schedule::{Schedule, TaskPlacement};
use crate::scheduling_block::SchedulingBlock;
use crate::task::Task;
use crate::time::{MJD, Period, PeriodSet, TaskId};
use qtty::Day;
use rand::Rng;
use std::collections::HashSet;

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
/// The returned list is sorted by `cost` ascending.
///
/// # Note on block dependencies
///
/// Inter-task ordering constraints within a [`SchedulingBlock`] are **not**
/// applied here. A future iteration will filter or shift candidates according
/// to predecessor end times; this is the designated extension point.
fn build_candidates(
    task: &Task,
    windows: &PeriodSet<MJD>,
    schedule: &Schedule,
    protected_ids: &HashSet<TaskId>,
) -> Vec<Candidate> {
    let duration = task.duration.to::<Day>();
    let mut candidates: Vec<Candidate> = Vec::new();

    for window in windows.iter() {
        // Skip windows too short to fit the task.
        if window.duration() < duration {
            continue;
        }

        let ws = window.start;
        let we = window.end;

        // Candidate start times: window start, plus immediately after each
        // placed task that overlaps this window.
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

        // Evaluate each start time and build a Candidate if it fits and is valid.
        for start in starts {
            let end = start + duration;
            if end > we {
                continue;
            }
            let period = Period::new(start, end);
            let overlapping = schedule.overlapping(&period);

            // Discard candidates that would evict a protected task.
            if overlapping.iter().any(|id| protected_ids.contains(id)) {
                continue;
            }

            // All remaining overlapping tasks are non-protected conflicts.
            let cost = overlapping.len();
            candidates.push(Candidate {
                period,
                conflicts: overlapping,
                cost,
            });
        }
    }

    // Sort by cost ascending so the stochastic picker works on a prefix.
    candidates.sort_by_key(|c| c.cost);
    candidates
}

/// Choose a candidate index from a cost-sorted slice stochastically.
///
/// - If zero-cost candidates exist, pick uniformly among them.
/// - Otherwise, pick uniformly from the `stochastic_range` cheapest.
fn choose_candidate_idx(
    candidates: &[Candidate],
    stochastic_range: usize,
    rng: &mut impl Rng,
) -> usize {
    debug_assert!(!candidates.is_empty());
    // `partition_point` returns the number of elements where cost == 0.
    let zero_count = candidates.partition_point(|c| c.cost == 0);
    let range = if zero_count > 0 {
        zero_count
    } else {
        stochastic_range.min(candidates.len())
    };
    rng.gen_range(0..range)
}

/// Place `task` into `schedule`, evicting non-protected conflicts to `lobby`.
///
/// # Steps
///
/// 1. **Assert membership** – verifies that `task` belongs to `block`.
/// 2. **Build candidates** – evaluates all valid start times, computing the
///    displaced task set and cost for each.
/// 3. **Stochastic selection** – picks from the cheapest cost tier.
/// 4. **Evict conflicts** – unplaces all non-protected tasks that overlap the
///    chosen interval and enqueues them in `lobby`.
/// 5. **Insert placement** – records the task in `schedule`.
///
/// # Errors
///
/// Returns a [`TaskSchedulerError`] if the task has no feasibility windows or
/// if all candidate periods conflict with a protected task.
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

    // Protected set: block tasks + tasks already placed in this inner run.
    let mut protected_ids: HashSet<TaskId> = block.iter().collect();
    protected_ids.extend(run_protected.iter().copied());

    let candidates = build_candidates(task, windows, schedule, &protected_ids);
    if candidates.is_empty() {
        return Err(TaskSchedulerError::NoValidCandidates);
    }

    let chosen_idx = choose_candidate_idx(&candidates, config.stochastic_range, rng);
    let chosen = &candidates[chosen_idx];

    // Evict displaced tasks and send them to the lobby.
    for &conflict_id in &chosen.conflicts {
        let _ = schedule.unplace_task(conflict_id);
        lobby.push(conflict_id);
    }

    // Place the task.
    schedule.insert_placement(TaskPlacement {
        task_id: task.id,
        start: chosen.period.start,
        end: chosen.period.end,
    });

    Ok(())
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

    /// Build a minimal `Task` with the given id and duration (in days).
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

    // ── helper: build a minimal SchedulingBlock containing `task` ────────────

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
        let periods_map = TaskPeriodMap::new(); // empty
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
        let task = make_task(1, 1.0); // 1-day task
        let mut block = SchedulingBlock::new(make_block_id(1));
        block.push_task(make_task(1, 1.0)).unwrap();

        let mut schedule = Schedule::new();
        let mut periods_map = TaskPeriodMap::new();
        // Window [0, 5) — task fits at start
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
    ///
    /// Candidate starts:
    ///   - ws = 0.0  →  end = 2.0 ≤ 2.5  →  overlaps squatter  →  cost = 1
    ///   - after squatter: start = 2.0  →  end = 4.0 > 2.5  →  doesn't fit
    ///
    /// The only valid candidate has cost = 1, so the squatter must be evicted.
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
        assert!(
            !schedule.contains(squatter.id),
            "squatter must have been evicted"
        );
        assert_eq!(lobby.len(), 1, "squatter must be in the lobby");
    }

    /// Window [0, 5), squatter at [0, 1), duration = 2.0 days.
    ///
    /// Candidates:
    ///   - start = 0.0  →  end = 2.0  →  overlaps squatter  →  cost = 1
    ///   - start = 1.0  →  end = 3.0  →  no overlap         →  cost = 0  ← preferred
    ///
    /// Zero-cost candidates always win, so task is placed at [1, 3) and
    /// the squatter is left untouched.
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
        assert!(
            schedule.contains(squatter.id),
            "squatter must not be evicted"
        );
        assert!(lobby.is_empty(), "lobby must remain empty");
        // Task must have been placed at [1.0, 3.0) — the only zero-cost slot.
        let placement = schedule.get(task.id).unwrap();
        assert_eq!(placement.start, mjd(1.0));
        assert_eq!(placement.end, mjd(3.0));
    }
}
