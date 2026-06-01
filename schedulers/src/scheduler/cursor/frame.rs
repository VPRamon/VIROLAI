//! Coordinate frames that let a single forward beam engine express both
//! forward and backward cursors.
//!
//! Every cursor runs *forward* in its own frame. The frame maps between the
//! cursor's local "frame time" (in which the candidate queue computes EST) and
//! the shared "schedule time" used by the global [`Schedule`](crate::schedule::Schedule).
//!
//! * [`CursorFrame::Identity`] — forward cursors. Frame time == schedule time.
//! * [`CursorFrame::Mirrored`] — backward cursors. Frame time is the territory
//!   reflected about its own midpoint, so scheduling earliest-feasible in frame
//!   time places latest-feasible in schedule time.

use crate::schedule::TaskPlacement;
use crate::time::{MJD, Period, PeriodSet, Time};

/// Maps between a cursor's local frame time and shared schedule time.
#[derive(Debug, Clone, Copy)]
pub(super) enum CursorFrame {
    /// Forward cursor: frame time and schedule time coincide.
    Identity,
    /// Backward cursor: frame time is reflected about the territory midpoint.
    Mirrored {
        /// The cursor territory (schedule time) used as the mirror axis.
        territory: Period<MJD>,
    },
}

impl CursorFrame {
    /// Map a whole feasibility set into frame time.
    pub(super) fn to_frame_periods(self, set: &PeriodSet<MJD>) -> PeriodSet<MJD> {
        match self {
            Self::Identity => set.clone(),
            Self::Mirrored { territory } => {
                PeriodSet::from_periods(set.iter().map(|p| mirror_period(p, &territory)).collect())
            }
        }
    }

    /// Convert a placement computed in frame time back into schedule time.
    pub(super) fn to_schedule_placement(self, placement: TaskPlacement) -> TaskPlacement {
        match self {
            Self::Identity => placement,
            Self::Mirrored { territory } => TaskPlacement {
                task_id: placement.task_id,
                // Endpoints swap under reflection so the result stays ordered.
                start: mirror(placement.end, &territory),
                end: mirror(placement.start, &territory),
            },
        }
    }
}

/// Reflect a time point across the territory midpoint.
///
/// The territory endpoints are preserved exactly (`start <-> end`) so that a
/// placement landing on a territory boundary cannot drift one ULP outside the
/// territory and be wrongly rejected by territory validation.
fn mirror(t: Time<MJD>, territory: &Period<MJD>) -> Time<MJD> {
    let tv = t.value();
    if tv == territory.start.value() {
        return territory.end;
    }
    if tv == territory.end.value() {
        return territory.start;
    }
    Time::<MJD>::new(territory.start.value() + territory.end.value() - tv)
}

/// Reflect a half-open period across the territory midpoint.
fn mirror_period(period: &Period<MJD>, territory: &Period<MJD>) -> Period<MJD> {
    Period::new(
        mirror(period.end, territory),
        mirror(period.start, territory),
    )
}
