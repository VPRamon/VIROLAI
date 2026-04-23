//! Best-snapshot tracking helpers for CRU.

use super::super::ranking::compare_schedules;
use crate::schedule::{Schedule, SchedulingProblem};
use crate::time::{MJD, TaskId, Time};
use std::collections::HashSet;

pub(super) fn count_protected_placed(
    schedule: &Schedule,
    protected_ids: &HashSet<TaskId>,
) -> usize {
    protected_ids
        .iter()
        .filter(|id| schedule.contains(**id))
        .count()
}

pub(super) fn count_unplaced_displaced(schedule: &Schedule, displaced: &HashSet<TaskId>) -> usize {
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
#[allow(clippy::too_many_arguments)]
pub(super) fn is_better_snapshot(
    new_protected: usize,
    new_unplaced_displaced: usize,
    current: &Schedule,
    best_protected: usize,
    best_unplaced: usize,
    best: &Schedule,
    problem: &SchedulingProblem,
    horizon_start: Time<MJD>,
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
    compare_schedules(current, best, problem, horizon_start) == std::cmp::Ordering::Less
}
