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

/// Live schedule-time position of one cursor at evaluation time.
///
/// Used to give multi-cursor-aware figures of merit a formal view of every
/// cursor's frontier. `id` is the cursor's stable identifier (matching
/// `cursor::CursorId`'s inner `usize`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CursorPosition {
    /// Stable cursor identifier.
    pub id: usize,
    /// The cursor's current frontier in schedule time.
    pub position: Time<MJD>,
}

/// Context passed to every [`ScheduleFom::evaluate`] call during beam search.
///
/// # Single-cursor schedulers (EST/LST)
///
/// `SoftConstraintFom` ignores this context. `FutureFlexibilityFom` uses
/// `cursor`, `horizon`, and `possible_periods` to analyse the residual
/// scheduling capacity from `cursor` to `horizon.end`. The multi-cursor fields
/// are `None` for the single-cursor schedulers, so their behaviour is unchanged.
///
/// # Multi-cursor scheduler
///
/// The multi-cursor engine populates the optional fields with a *formal*,
/// complete view of the search frontier:
/// * `cursor` carries the schedule-time end of the most recent placement (the
///   same single-frontier signal the single-cursor schedulers expose);
/// * `last_action_cursor` identifies which cursor made that placement;
/// * `cursor_positions` lists every cursor's live frontier in the child state;
/// * `active_periods` lists every cursor's active region for the round that
///   produced this child, index-parallel to `cursor_positions` (`None` for a
///   cursor with no active region that round).
///
/// These fields let cursor-aware figures of merit score multi-cursor states
/// precisely. Schedule *validity* never depends on the figure of merit: the
/// engine enforces no-overlap, no-duplicate, territory, and dependency
/// invariants independently of scoring.
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
    /// Live frontier of every cursor (multi-cursor only; `None` otherwise).
    pub cursor_positions: Option<&'a [CursorPosition]>,
    /// Live schedule-time active region of every cursor, index-parallel to
    /// `cursor_positions` (multi-cursor only; `None` otherwise). An entry is
    /// `None` when that cursor had no active region this round (exhausted or
    /// squeezed out by a neighbouring cursor).
    pub active_periods: Option<&'a [Option<Period<MJD>>]>,
    /// Identifier of the cursor that made the most recent placement
    /// (multi-cursor only; `None` otherwise).
    pub last_action_cursor: Option<usize>,
}

impl<'a> FomContext<'a> {
    /// Build a single-cursor context (EST/LST and unit tests).
    ///
    /// The multi-cursor fields are left empty, so figures of merit see exactly
    /// the same context they did before the multi-cursor fields were added.
    pub fn single_cursor(
        cursor: Time<MJD>,
        horizon: Period<MJD>,
        possible_periods: Option<&'a TaskPeriodMap>,
    ) -> Self {
        Self {
            cursor,
            horizon,
            possible_periods,
            cursor_positions: None,
            active_periods: None,
            last_action_cursor: None,
        }
    }
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
