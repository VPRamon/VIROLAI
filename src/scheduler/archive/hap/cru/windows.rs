//! Candidate window generation helpers for CRU.

use crate::schedule::Schedule;
use crate::task::Task;
use crate::time::{MJD, Period, PeriodSet, TaskId, Time};
use qtty::Day;
use std::collections::HashSet;

/// Generate candidate start times for `task` within its feasible windows.
///
/// For each feasible period `[ws, we]`:
/// - Tries `start = max(ws, pred_end)` if the task fits.
/// - Tries every placed-task end `E` that falls within the window and
///   satisfies `E >= pred_end` and `E + duration <= we`.
///
/// The returned list is sorted ascending and deduplicated.
pub(super) fn generate_candidate_starts(
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

/// Returns `true` if placing `task` starting at `start` would overlap any
/// already-placed protected task.
pub(super) fn would_evict_protected(
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
