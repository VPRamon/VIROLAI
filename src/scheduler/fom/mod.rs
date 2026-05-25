pub mod composite;
pub mod future_flexibility;
pub mod kind;
pub mod soft_constraint;

pub use self::composite::CompositeFom;
pub use self::future_flexibility::FutureFlexibilityFom;
pub use self::kind::FomKind;
pub use self::soft_constraint::SoftConstraintFom;

use crate::prescheduler::TaskPeriodMap;
use crate::schedule::Schedule;
use crate::schedule::SchedulingProblem;
use crate::time::{MJD, Period, Time};
use std::sync::Arc;

/// Context passed to every [`ScheduleFom::evaluate`] call during EST beam search.
///
/// `SoftConstraintFom` ignores this context. `FutureFlexibilityFom` uses it to
/// analyse the residual scheduling capacity from `cursor` to `horizon.end`.
pub struct FomContext<'a> {
    /// Beam cursor — new placements must start at or after this instant.
    pub cursor: Time<MJD>,
    /// Full scheduling horizon.
    pub horizon: Period<MJD>,
    /// Pre-computed feasibility windows per task.
    ///
    /// `None` when the FOM is called outside a full problem context (e.g. unit
    /// tests that use the flat-tasks entry point). `FutureFlexibilityFom` treats
    /// all unplaced tasks as *not* recoverable when this is absent.
    pub possible_periods: Option<&'a TaskPeriodMap>,
}

/// Scores a schedule state. Higher values indicate better schedules.
///
/// The beam search keeps the K states with the *highest* FOM after each
/// expansion round.
pub trait ScheduleFom: std::fmt::Debug + Send + Sync {
    /// Return the scalar score used to rank one schedule state against another.
    fn evaluate(
        &self,
        schedule: &Schedule,
        problem: &SchedulingProblem,
        ctx: &FomContext<'_>,
    ) -> f64;
    /// Return a human-readable label for this FOM.
    fn label(&self) -> &'static str;
}

impl<T: ScheduleFom + ?Sized> ScheduleFom for Arc<T> {
    fn evaluate(
        &self,
        schedule: &Schedule,
        problem: &SchedulingProblem,
        ctx: &FomContext<'_>,
    ) -> f64 {
        (**self).evaluate(schedule, problem, ctx)
    }

    fn label(&self) -> &'static str {
        (**self).label()
    }
}
