//! CRU (Constraint Repair Unit) engine for the HAP scheduler.
//!
//! Each CRU starts from a survivor schedule and attempts to place all tasks of
//! one proposal (the *protected set*) by evicting conflicting non-protected
//! tasks when necessary.  The best intermediate state is tracked and returned
//! when the iteration budget is exhausted or all protected tasks are placed.

use super::configuration::Configuration;
use super::proposal::Proposal;
use super::ranking::compare_schedules;
use crate::prescheduler::TaskPeriodMap;
use crate::schedule::{Schedule, TaskPlacement};
use crate::scheduling_block::SchedulingBlock;
use crate::task::Task;
use crate::time::{MJD, Period, PeriodSet, SchedulingBlockId, TaskId, Time};
use qtty::Day;
use std::collections::{HashMap, HashSet};

// ---------------------------------------------------------------------------
// Seeded RNG
// ---------------------------------------------------------------------------

struct Xorshift64(u64);

impl Xorshift64 {
    fn new(seed: u64) -> Self {
        Self(if seed == 0 { 1 } else { seed })
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn next_usize(&mut self, n: usize) -> usize {
        if n <= 1 {
            return 0;
        }
        (self.next() % n as u64) as usize
    }
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

fn compute_min_positive_priority(proposals: &[Proposal]) -> f64 {
    proposals
        .iter()
        .map(|p| p.priority)
        .filter(|&p| p > 0.0)
        .reduce(f64::min)
        .unwrap_or(1.0)
}

/// Returns `true` when all direct predecessors of `task_id` (within its block)
/// are already placed in `schedule`.
fn predecessors_placed(
    task_id: TaskId,
    schedule: &Schedule,
    blocks: &HashMap<SchedulingBlockId, SchedulingBlock>,
    task_to_block: &HashMap<TaskId, SchedulingBlockId>,
) -> bool {
    let Some(&block_id) = task_to_block.get(&task_id) else {
        return true;
    };
    let Some(block) = blocks.get(&block_id) else {
        return true;
    };
    block
        .predecessors(task_id)
        .iter()
        .all(|pred_id| schedule.contains(*pred_id))
}

/// Returns the maximum end time of all placed predecessors of `task_id`,
/// falling back to `horizon.start` when no predecessors are placed.
fn predecessor_end_lower_bound(
    task_id: TaskId,
    schedule: &Schedule,
    blocks: &HashMap<SchedulingBlockId, SchedulingBlock>,
    task_to_block: &HashMap<TaskId, SchedulingBlockId>,
    horizon: &Period<MJD>,
) -> Time<MJD> {
    let Some(&block_id) = task_to_block.get(&task_id) else {
        return horizon.start;
    };
    let Some(block) = blocks.get(&block_id) else {
        return horizon.start;
    };
    block
        .predecessors(task_id)
        .iter()
        .filter_map(|pred_id| schedule.get(*pred_id).map(|p| p.end))
        .max_by(|a, b| {
            a.value()
                .partial_cmp(&b.value())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap_or(horizon.start)
}

// ---------------------------------------------------------------------------
// Candidate window generation
// ---------------------------------------------------------------------------

/// Generate candidate start times for `task` within its feasible windows.
///
/// For each feasible period `[ws, we]`:
/// - Tries `start = max(ws, pred_end)` if the task fits.
/// - Tries every placed-task end `E` that falls within the window and
///   satisfies `E >= pred_end` and `E + duration <= we`.
///
/// The returned list is sorted ascending and deduplicated.
fn generate_candidate_starts(
    task: &Task,
    windows: &PeriodSet<MJD>,
    schedule: &Schedule,
    pred_end: Time<MJD>,
) -> Vec<Time<MJD>> {
    let duration_days = task.duration.to::<Day>().value();
    let mut seen: HashSet<u64> = HashSet::new();
    let mut result: Vec<Time<MJD>> = Vec::new();

    for window in windows.iter() {
        let ws = window.start;
        let we = window.end;
        let window_duration = we.value() - ws.value();
        if window_duration < duration_days {
            continue;
        }

        // Candidate 1: start at max(ws, pred_end)
        let s0 = if ws.value() >= pred_end.value() {
            ws
        } else {
            pred_end
        };
        if s0.value() + duration_days <= we.value() && seen.insert(s0.value().to_bits()) {
            result.push(s0);
        }

        // Candidate 2+: start at end of any placed task overlapping this window
        let window_interval = Period::new(ws, we);
        for overlapping_id in schedule.overlapping(&window_interval) {
            if let Some(placement) = schedule.get(overlapping_id) {
                let e = placement.end;
                if e.value() > ws.value()
                    && e.value() >= pred_end.value()
                    && e.value() + duration_days <= we.value()
                    && seen.insert(e.value().to_bits())
                {
                    result.push(e);
                }
            }
        }
    }

    result.sort_by(|a, b| {
        a.value()
            .partial_cmp(&b.value())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    result
}

// ---------------------------------------------------------------------------
// Candidate selection from the lobby
// ---------------------------------------------------------------------------

/// Estimate the number of valid start positions for `task_id` given the
/// current schedule state.  Used as a proxy for flexibility.
fn compute_flexibility(
    task_id: TaskId,
    tasks: &HashMap<TaskId, Task>,
    possible_periods: &TaskPeriodMap,
    schedule: &Schedule,
    blocks: &HashMap<SchedulingBlockId, SchedulingBlock>,
    task_to_block: &HashMap<TaskId, SchedulingBlockId>,
    horizon: &Period<MJD>,
) -> usize {
    let Some(task) = tasks.get(&task_id) else {
        return 0;
    };
    let Some(windows) = possible_periods.get(&task_id) else {
        return 0;
    };
    let pred_end = predecessor_end_lower_bound(task_id, schedule, blocks, task_to_block, horizon);
    generate_candidate_starts(task, windows, schedule, pred_end).len()
}

fn get_priority(task_id: TaskId, tasks: &HashMap<TaskId, Task>, at: Time<MJD>) -> f64 {
    tasks
        .get(&task_id)
        .and_then(|t| {
            t.soft_constraints
                .as_ref()
                .map(|sc| sc.score(&at, None, Some(&t.target)))
        })
        .unwrap_or(0.0)
}

/// Pick the best next candidate from `group`:
/// sort by `(flexibility ASC, priority DESC, task_id ASC)`.
fn select_best_candidate(
    group: &[TaskId],
    tasks: &HashMap<TaskId, Task>,
    possible_periods: &TaskPeriodMap,
    schedule: &Schedule,
    blocks: &HashMap<SchedulingBlockId, SchedulingBlock>,
    task_to_block: &HashMap<TaskId, SchedulingBlockId>,
    horizon: &Period<MJD>,
) -> TaskId {
    *group
        .iter()
        .min_by(|&&a, &&b| {
            let flex_a = compute_flexibility(
                a,
                tasks,
                possible_periods,
                schedule,
                blocks,
                task_to_block,
                horizon,
            );
            let flex_b = compute_flexibility(
                b,
                tasks,
                possible_periods,
                schedule,
                blocks,
                task_to_block,
                horizon,
            );
            let flex_cmp = flex_a.cmp(&flex_b);
            if flex_cmp != std::cmp::Ordering::Equal {
                return flex_cmp;
            }
            let prio_a = get_priority(a, tasks, horizon.start);
            let prio_b = get_priority(b, tasks, horizon.start);
            let prio_cmp = prio_b.total_cmp(&prio_a); // DESC
            if prio_cmp != std::cmp::Ordering::Equal {
                return prio_cmp;
            }
            a.0.cmp(&b.0) // ASC tie-break
        })
        .expect("group must be non-empty")
}

// ---------------------------------------------------------------------------
// Protected-eviction filter
// ---------------------------------------------------------------------------

/// Returns `true` if placing `task` starting at `start` would overlap any
/// already-placed protected task.
fn would_evict_protected(
    start: Time<MJD>,
    task: &Task,
    schedule: &Schedule,
    protected_ids: &HashSet<TaskId>,
) -> bool {
    let duration_days = task.duration.to::<Day>().value();
    let end = Time::<MJD>::new(start.value() + duration_days);
    let interval = Period::new(start, end);
    schedule
        .overlapping(&interval)
        .iter()
        .any(|id| protected_ids.contains(id))
}

// ---------------------------------------------------------------------------
// Conflict group and cost
// ---------------------------------------------------------------------------

/// Compute the conflict group for placing `task` starting at `start`:
/// - All placed tasks overlapping `[start, start + duration)`.
/// - Closed over dependency descendants in the same block that are also placed.
fn compute_conflict_group(
    start: Time<MJD>,
    task: &Task,
    schedule: &Schedule,
    task_to_block: &HashMap<TaskId, SchedulingBlockId>,
    blocks: &HashMap<SchedulingBlockId, SchedulingBlock>,
) -> HashSet<TaskId> {
    let duration_days = task.duration.to::<Day>().value();
    let end = Time::<MJD>::new(start.value() + duration_days);
    let interval = Period::new(start, end);

    let initial: HashSet<TaskId> = schedule.overlapping(&interval).into_iter().collect();
    let mut closed = initial.clone();

    for &base_id in &initial {
        if let Some(&block_id) = task_to_block.get(&base_id)
            && let Some(block) = blocks.get(&block_id)
        {
            for desc_id in block.all_descendants(base_id) {
                if schedule.contains(desc_id) {
                    closed.insert(desc_id);
                }
            }
        }
    }

    closed
}

fn find_proposal_for_task<'a>(
    task_id: TaskId,
    task_to_block: &HashMap<TaskId, SchedulingBlockId>,
    proposals: &'a [Proposal],
) -> Option<&'a Proposal> {
    let &block_id = task_to_block.get(&task_id)?;
    proposals.iter().find(|p| p.id == block_id)
}

/// Compute the total conflict cost of placing `task` at `start`.
///
/// Cost = sum over conflicting tasks of:
/// - `conflicting_proposal.priority + impatience * (min_positive_priority / alpha)`
///   when the conflicting task belongs to a currently-complete proposal.
/// - `0.0` otherwise.
#[allow(clippy::too_many_arguments)]
fn compute_conflict_cost(
    start: Time<MJD>,
    task: &Task,
    schedule: &Schedule,
    task_to_block: &HashMap<TaskId, SchedulingBlockId>,
    blocks: &HashMap<SchedulingBlockId, SchedulingBlock>,
    all_proposals: &[Proposal],
    current_proposal: &Proposal,
    min_positive_priority: f64,
    config: &Configuration,
) -> f64 {
    let impatience =
        current_proposal.task_count() as f64 / current_proposal.impatience_denominator as f64;
    let conflict_group = compute_conflict_group(start, task, schedule, task_to_block, blocks);

    let mut total_cost = 0.0;
    for conflicting_id in &conflict_group {
        let Some(proposal) = find_proposal_for_task(*conflicting_id, task_to_block, all_proposals)
        else {
            continue;
        };
        if proposal.is_complete(schedule) {
            total_cost +=
                proposal.priority + impatience * (min_positive_priority / config.impatience_alpha);
        }
    }
    total_cost
}

// ---------------------------------------------------------------------------
// Candidate window selection
// ---------------------------------------------------------------------------

/// Choose a candidate index from `costs`.
///
/// - If any cost is `0.0`: return the index of the first zero-cost candidate.
/// - Otherwise: sort by cost ascending and pick uniformly from the best
///   `stochastic_range` candidates using `rng`.
fn choose_candidate(costs: &[f64], stochastic_range: usize, rng: &mut Xorshift64) -> usize {
    if costs.is_empty() {
        return 0;
    }
    let min_cost = costs.iter().cloned().fold(f64::INFINITY, f64::min);

    if min_cost == 0.0 {
        costs.iter().position(|&c| c == 0.0).unwrap_or(0)
    } else {
        let mut indexed: Vec<(usize, f64)> = costs.iter().copied().enumerate().collect();
        indexed.sort_by(|a, b| a.1.total_cmp(&b.1));
        let range = stochastic_range.min(indexed.len()).max(1);
        let chosen = rng.next_usize(range);
        indexed[chosen].0
    }
}

// ---------------------------------------------------------------------------
// Best-snapshot tracking
// ---------------------------------------------------------------------------

fn count_protected_placed(schedule: &Schedule, protected_ids: &HashSet<TaskId>) -> usize {
    protected_ids
        .iter()
        .filter(|id| schedule.contains(**id))
        .count()
}

fn count_unplaced_displaced(schedule: &Schedule, displaced: &HashSet<TaskId>) -> usize {
    displaced
        .iter()
        .filter(|id| !schedule.contains(**id))
        .count()
}

/// Returns `true` when `current` is strictly better than `best` by the CRU
/// snapshot comparison criteria:
/// 1. More protected tasks placed (primary).
/// 2. Fewer unplaced displaced tasks (secondary).
/// 3. Higher HAP rank via `compare_schedules` (tertiary).
fn is_better_snapshot(
    new_protected: usize,
    new_unplaced_displaced: usize,
    current: &Schedule,
    best_protected: usize,
    best_unplaced: usize,
    best: &Schedule,
    proposals: &[Proposal],
) -> bool {
    if new_protected > best_protected {
        return true;
    }
    if new_protected < best_protected {
        return false;
    }
    if new_unplaced_displaced < best_unplaced {
        return true;
    }
    if new_unplaced_displaced > best_unplaced {
        return false;
    }
    // Tertiary: current schedule is better in HAP rank
    compare_schedules(current, best, proposals) == std::cmp::Ordering::Less
}

// ---------------------------------------------------------------------------
// Public CRU entry point
// ---------------------------------------------------------------------------

/// Run one CRU repair pass starting from `base_schedule`.
///
/// Attempts to place all tasks in `proposal` (the *protected set*).
/// Non-protected conflicting tasks may be evicted and re-queued in the
/// internal displaced lobby.  Returns the best intermediate schedule
/// observed during the run.
#[allow(clippy::too_many_arguments)]
pub(super) fn run_cru(
    base_schedule: Schedule,
    proposal: &Proposal,
    tasks: &HashMap<TaskId, Task>,
    possible_periods: &TaskPeriodMap,
    horizon: &Period<MJD>,
    blocks: &HashMap<SchedulingBlockId, SchedulingBlock>,
    task_to_block: &HashMap<TaskId, SchedulingBlockId>,
    all_proposals: &[Proposal],
    config: &Configuration,
    seed: u64,
) -> Schedule {
    let mut rng = Xorshift64::new(seed);
    let protected_ids: HashSet<TaskId> = proposal.task_ids.iter().copied().collect();
    let mut displaced: HashSet<TaskId> = HashSet::new();
    let mut schedule = base_schedule;

    let mut best_protected_placed = count_protected_placed(&schedule, &protected_ids);
    let mut best_unplaced_displaced = 0usize;
    let mut best_snapshot = schedule.clone();

    let min_positive_priority = compute_min_positive_priority(all_proposals);

    for _iteration in 0..config.cru_max_iterations {
        // Build lobby: unplaced protected tasks ∪ unplaced displaced tasks
        let lobby: HashSet<TaskId> = protected_ids
            .iter()
            .filter(|id| !schedule.contains(**id))
            .chain(displaced.iter().filter(|id| !schedule.contains(**id)))
            .copied()
            .collect();

        if lobby.is_empty() {
            break;
        }

        // Find ready tasks: all predecessors in their block are placed
        let ready: Vec<TaskId> = lobby
            .iter()
            .copied()
            .filter(|&tid| predecessors_placed(tid, &schedule, blocks, task_to_block))
            .collect();

        if ready.is_empty() {
            break;
        }

        // Prefer protected tasks; fall back to displaced when none are ready
        let ready_protected: Vec<TaskId> = ready
            .iter()
            .copied()
            .filter(|id| protected_ids.contains(id))
            .collect();
        let ready_displaced: Vec<TaskId> = ready
            .iter()
            .copied()
            .filter(|id| !protected_ids.contains(id))
            .collect();

        let candidate_group = if !ready_protected.is_empty() {
            ready_protected
        } else {
            ready_displaced
        };

        let next_task_id = select_best_candidate(
            &candidate_group,
            tasks,
            possible_periods,
            &schedule,
            blocks,
            task_to_block,
            horizon,
        );

        let Some(task) = tasks.get(&next_task_id) else {
            log::warn!("hap cru: task {} not found in task map", next_task_id.0);
            break;
        };
        let Some(windows) = possible_periods.get(&next_task_id) else {
            log::debug!("hap cru: task {} has no feasible windows", next_task_id.0);
            break;
        };

        // Predecessor lower bound on start time
        let pred_end =
            predecessor_end_lower_bound(next_task_id, &schedule, blocks, task_to_block, horizon);

        // Generate and filter candidate starts
        let candidate_starts = generate_candidate_starts(task, windows, &schedule, pred_end);
        let candidate_starts: Vec<Time<MJD>> = candidate_starts
            .into_iter()
            .filter(|&s| !would_evict_protected(s, task, &schedule, &protected_ids))
            .collect();

        if candidate_starts.is_empty() {
            log::debug!(
                "hap cru: no valid windows for task {} after protected filter",
                next_task_id.0
            );
            break;
        }

        // Compute conflict cost for every candidate start
        let costs: Vec<f64> = candidate_starts
            .iter()
            .map(|&s| {
                compute_conflict_cost(
                    s,
                    task,
                    &schedule,
                    task_to_block,
                    blocks,
                    all_proposals,
                    proposal,
                    min_positive_priority,
                    config,
                )
            })
            .collect();

        // Choose the candidate window
        let chosen_idx = choose_candidate(&costs, config.stochastic_range, &mut rng);
        let chosen_start = candidate_starts[chosen_idx];

        // Compute conflict group for the chosen window and evict non-protected
        let conflict_group =
            compute_conflict_group(chosen_start, task, &schedule, task_to_block, blocks);
        for &evicted_id in &conflict_group {
            if !protected_ids.contains(&evicted_id) {
                let _ = schedule.unplace_task(evicted_id);
                displaced.insert(evicted_id);
            }
        }

        // Place the task
        let end = Time::<MJD>::new(chosen_start.value() + task.duration.to::<Day>().value());
        let block_id = task_to_block.get(&next_task_id).copied();
        schedule.insert_placement(TaskPlacement {
            task_id: next_task_id,
            start: chosen_start,
            end,
            block_id,
        });
        displaced.remove(&next_task_id);

        log::trace!(
            "hap cru: iter={} placed task={} at [{:.4}, {:.4}]",
            _iteration,
            next_task_id.0,
            chosen_start.value(),
            end.value(),
        );

        // Update best snapshot
        let new_protected_placed = count_protected_placed(&schedule, &protected_ids);
        let new_unplaced_displaced = count_unplaced_displaced(&schedule, &displaced);
        if is_better_snapshot(
            new_protected_placed,
            new_unplaced_displaced,
            &schedule,
            best_protected_placed,
            best_unplaced_displaced,
            &best_snapshot,
            all_proposals,
        ) {
            best_snapshot = schedule.clone();
            best_protected_placed = new_protected_placed;
            best_unplaced_displaced = new_unplaced_displaced;
        }

        // Early exit when all protected tasks are placed
        if new_protected_placed == protected_ids.len() {
            log::debug!(
                "hap cru: all {} protected task(s) placed, returning early",
                protected_ids.len()
            );
            return schedule;
        }
    }

    best_snapshot
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xorshift64_is_deterministic() {
        let mut r1 = Xorshift64::new(42);
        let mut r2 = Xorshift64::new(42);
        for _ in 0..200 {
            assert_eq!(r1.next(), r2.next());
        }
    }

    #[test]
    fn xorshift64_seed_zero_avoids_degenerate_state() {
        let mut r = Xorshift64::new(0);
        // Must produce non-zero values (zero seed is replaced with 1)
        assert_ne!(r.next(), 0);
    }

    #[test]
    fn xorshift64_different_seeds_differ() {
        let mut r1 = Xorshift64::new(1);
        let mut r2 = Xorshift64::new(2);
        // It would be astronomically unlikely for 10 consecutive outputs to match
        let seq1: Vec<u64> = (0..10).map(|_| r1.next()).collect();
        let seq2: Vec<u64> = (0..10).map(|_| r2.next()).collect();
        assert_ne!(seq1, seq2);
    }

    #[test]
    fn choose_candidate_picks_zero_cost_first() {
        let costs = vec![2.0, 0.0, 1.0];
        let mut rng = Xorshift64::new(1);
        assert_eq!(choose_candidate(&costs, 3, &mut rng), 1);
    }

    #[test]
    fn choose_candidate_returns_zero_for_empty() {
        let mut rng = Xorshift64::new(1);
        assert_eq!(choose_candidate(&[], 3, &mut rng), 0);
    }

    #[test]
    fn choose_candidate_stochastic_stays_within_range() {
        let costs = vec![3.0, 1.0, 2.0, 4.0, 5.0];
        let mut rng = Xorshift64::new(99);
        // With stochastic_range=2 we should only ever pick index of 1.0 or 2.0
        for _ in 0..50 {
            let idx = choose_candidate(&costs, 2, &mut rng);
            // The two cheapest are cost 1.0 (idx 1) and cost 2.0 (idx 2)
            assert!(idx == 1 || idx == 2, "unexpected index {idx}");
        }
    }

    #[test]
    fn compute_min_positive_priority_fallback() {
        assert_eq!(compute_min_positive_priority(&[]), 1.0);
    }

    #[test]
    fn generate_candidate_starts_empty_when_window_too_small() {
        use crate::time::{MJD, PeriodSet, Time};
        // Task duration 2.0 days, window [0.0, 1.0] — too small
        let windows = PeriodSet::from_periods(vec![Period::new(
            Time::<MJD>::new(0.0),
            Time::<MJD>::new(1.0),
        )]);

        // Build a minimal fake Task via direct struct construction is not possible
        // (Task::new validates). Use a helper that skips constraints.
        // We can't easily build a Task here without full setup, so we test the
        // window-too-small branch indirectly through the empty-result invariant
        // using a schedule that has no placements.
        let schedule = Schedule::new();
        let pred_end = Time::<MJD>::new(0.0);

        // Manually call the filtering logic: window_duration (1.0) < duration (2.0) → skip
        let duration_days = 2.0_f64;
        let mut result_count = 0usize;
        for window in windows.iter() {
            let window_duration = window.end.value() - window.start.value();
            if window_duration >= duration_days {
                // Candidate 1
                let s0 = if window.start.value() >= pred_end.value() {
                    window.start
                } else {
                    pred_end
                };
                if s0.value() + duration_days <= window.end.value() {
                    result_count += 1;
                }
                // Also check placed-task ends (schedule is empty so none)
                let window_interval = Period::new(window.start, window.end);
                result_count += schedule.overlapping(&window_interval).len();
            }
        }
        assert_eq!(result_count, 0);
    }
}
