//! Territory models for the multi-cursor scheduler.
//!
//! This module implements the two primary territory strategies:
//!
//! - **Static Partitioning** — fixed territories (absolute or fractional).
//! - **Dynamic Frontiering** — dynamic cursor-relative territories.

use crate::error::ScheduleError;
use crate::time::{MJD, Period, Time};
use super::config::CursorId;

/// A reference to a territory boundary.
///
/// Static territories never use this; dynamic territories use it to make
/// a boundary follow the live position of another cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryRef {
    /// The start of the scheduling horizon.
    HorizonStart,
    /// The end of the scheduling horizon.
    HorizonEnd,
    /// The live position of another cursor.
    Cursor(CursorId),
}

impl BoundaryRef {
    /// The cursor this boundary follows, if any.
    pub const fn cursor(self) -> Option<CursorId> {
        match self {
            Self::Cursor(id) => Some(id),
            _ => None,
        }
    }

    /// Resolve to a fixed time using the horizon extremes, ignoring live cursor
    /// positions. `cursor_extreme` is the horizon edge used for a cursor
    /// reference (the most permissive bound for that side).
    pub(crate) fn extreme(self, horizon: &Period<MJD>, cursor_extreme: Time<MJD>) -> Time<MJD> {
        match self {
            Self::HorizonStart => horizon.start,
            Self::HorizonEnd => horizon.end,
            Self::Cursor(_) => cursor_extreme,
        }
    }
}

/// One side of a resolved territory: either a fixed instant or a live cursor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum BoundarySide {
    /// A fixed schedule-time instant.
    Fixed(Time<MJD>),
    /// Follows another cursor's live position.
    Cursor(CursorId),
}

/// Map a [`BoundaryRef`] onto a [`BoundarySide`]. Horizon edges are fixed; a
/// cursor reference stays live.
pub(super) fn boundary_side(r: BoundaryRef, horizon: &Period<MJD>) -> BoundarySide {
    match r {
        BoundaryRef::HorizonStart => BoundarySide::Fixed(horizon.start),
        BoundaryRef::HorizonEnd => BoundarySide::Fixed(horizon.end),
        BoundaryRef::Cursor(id) => BoundarySide::Cursor(id),
    }
}

/// Static Partitioning: A fixed, absolute or fractional territory.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StaticPartitioning {
    /// A fixed, absolute `[start, end)` region.
    Fixed {
        /// Inclusive start of the territory.
        start: Time<MJD>,
        /// Exclusive end of the territory.
        end: Time<MJD>,
    },
    /// A fixed region expressed as horizon-relative fractions in `[0, 1]`,
    /// resolved against the scheduling horizon at run time.
    FractionRange {
        /// Start fraction (`0.0` == horizon start).
        start: f64,
        /// End fraction (`1.0` == horizon end).
        end: f64,
    },
}

impl StaticPartitioning {
    pub(crate) fn extent(&self, horizon: &Period<MJD>) -> Result<Period<MJD>, ScheduleError> {
        match *self {
            Self::Fixed { start, end } => clamp_period(start, end, horizon),
            Self::FractionRange { start, end } => {
                let (s, e) = fraction_bounds(start, end, horizon);
                clamp_period(s, e, horizon)
            }
        }
    }

    pub(crate) fn sides(
        &self,
        horizon: &Period<MJD>,
    ) -> Result<(BoundarySide, BoundarySide), ScheduleError> {
        let p = self.extent(horizon)?;
        Ok((BoundarySide::Fixed(p.start), BoundarySide::Fixed(p.end)))
    }
}

/// Dynamic Frontiering: A dynamic region whose boundaries may follow other cursors.
///
/// A boundary that references another cursor is recomputed before every
/// beam-expansion round from that cursor's live frontier, so two cursors can
/// advance towards each other until they meet.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DynamicFrontiering {
    /// Left boundary reference.
    pub start: BoundaryRef,
    /// Right boundary reference.
    pub end: BoundaryRef,
    /// Optional buffer (in days) kept between a cursor-referenced boundary
    /// and the referenced cursor's frontier. `None` is treated as `0.0`.
    pub min_gap: Option<f64>,
}

impl DynamicFrontiering {
    pub(crate) fn extent(&self, horizon: &Period<MJD>) -> Result<Period<MJD>, ScheduleError> {
        let s = self.start.extreme(horizon, horizon.start);
        let e = self.end.extreme(horizon, horizon.end);
        clamp_period(s, e, horizon)
    }

    pub(crate) fn sides(
        &self,
        horizon: &Period<MJD>,
    ) -> Result<(BoundarySide, BoundarySide), ScheduleError> {
        Ok((
            boundary_side(self.start, horizon),
            boundary_side(self.end, horizon),
        ))
    }

    pub(crate) fn min_gap(&self) -> f64 {
        self.min_gap.unwrap_or(0.0)
    }
}

/// Convert fraction bounds into absolute schedule-time instants.
fn fraction_bounds(start: f64, end: f64, horizon: &Period<MJD>) -> (Time<MJD>, Time<MJD>) {
    let span = horizon.end.value() - horizon.start.value();
    (
        Time::<MJD>::new(horizon.start.value() + span * start),
        Time::<MJD>::new(horizon.start.value() + span * end),
    )
}

/// Clamp `[start, end)` to the horizon and reject degenerate ranges.
fn clamp_period(
    start: Time<MJD>,
    end: Time<MJD>,
    horizon: &Period<MJD>,
) -> Result<Period<MJD>, ScheduleError> {
    let s = start.value().max(horizon.start.value());
    let e = end.value().min(horizon.end.value());
    if e <= s {
        return Err(ScheduleError::InvalidConfiguration(format!(
            "cursor territory [{start_v:.4}, {end_v:.4}) is empty after clamping to horizon [{hs:.4}, {he:.4})",
            start_v = start.value(),
            end_v = end.value(),
            hs = horizon.start.value(),
            he = horizon.end.value(),
        )));
    }
    Ok(Period::new(Time::<MJD>::new(s), Time::<MJD>::new(e)))
}
