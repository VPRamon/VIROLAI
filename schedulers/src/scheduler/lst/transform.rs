use crate::prescheduler::TaskPeriodMap;
use crate::schedule::{Schedule, TaskPlacement};
#[cfg(test)]
use crate::time::TaskId;
use crate::time::{MJD, Period, PeriodSet, Time};

/// Reflect a single time point across the horizon midpoint.
///
/// `mirror(t) = horizon.start + horizon.end - t`
pub fn mirror_time(t: Time<MJD>, horizon: &Period<MJD>) -> Time<MJD> {
    Time::<MJD>::new(horizon.start.value() + horizon.end.value() - t.value())
}

/// Reflect a half-open period `[start, end)` across the horizon.
///
/// The endpoints swap and are mirrored so the result is still a valid
/// half-open period ordered start ≤ end.
pub fn mirror_period(period: &Period<MJD>, horizon: &Period<MJD>) -> Period<MJD> {
    Period::new(
        mirror_time(period.end, horizon),
        mirror_time(period.start, horizon),
    )
}

/// Reflect every period in a set and return a normalised `PeriodSet`.
pub fn mirror_period_set(set: &PeriodSet<MJD>, horizon: &Period<MJD>) -> PeriodSet<MJD> {
    PeriodSet::from_periods(
        set.iter()
            .map(|period| mirror_period(period, horizon))
            .collect(),
    )
}

/// Reflect every task's feasibility windows across the horizon.
pub fn mirror_task_periods(
    possible_periods: &TaskPeriodMap,
    horizon: &Period<MJD>,
) -> TaskPeriodMap {
    possible_periods
        .iter()
        .map(|(&task_id, periods)| (task_id, mirror_period_set(periods, horizon)))
        .collect()
}

/// Convert a schedule produced on mirrored windows back to original time.
pub fn unmirror_schedule(mirrored: &Schedule, horizon: &Period<MJD>) -> Schedule {
    let mut out = Schedule::new();
    for placement in mirrored.placements() {
        let start = mirror_time(placement.end, horizon);
        let end = mirror_time(placement.start, horizon);
        out.insert_placement(TaskPlacement {
            task_id: placement.task_id,
            start,
            end,
        });
    }
    out
}

/// Unmirror every task's feasibility windows back to original time.
///
/// Since `mirror` is its own inverse, this is identical to
/// [`mirror_task_periods`] but is provided under a distinct name to make
/// call-sites self-documenting.
pub fn unmirror_task_periods(
    mirrored_periods: &TaskPeriodMap,
    horizon: &Period<MJD>,
) -> TaskPeriodMap {
    mirror_task_periods(mirrored_periods, horizon)
}

/// Return the IDs of all tasks whose mirrored windows are non-empty.
///
/// Used in tests to verify that mirroring preserves feasibility.
#[cfg(test)]
pub fn feasible_task_ids(periods: &TaskPeriodMap) -> Vec<TaskId> {
    let mut ids: Vec<TaskId> = periods
        .iter()
        .filter(|(_, p)| !p.is_empty())
        .map(|(&id, _)| id)
        .collect();
    ids.sort_by_key(|id| id.0);
    ids
}
