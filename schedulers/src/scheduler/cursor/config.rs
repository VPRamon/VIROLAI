//! Configuration types for the multi-cursor scheduler.
//!
//! These types describe *what* cursors exist and *where* they may schedule,
//! independently of the beam-search engine that executes them. The design
//! anticipates a future "Plan B" (dynamic cursor territories) by keeping
//! territory resolution behind [`CursorTerritory`] so the engine never needs to
//! special-case how a cursor's active region is computed.

use crate::error::ScheduleError;
use crate::time::{MJD, Period, Time};

/// Stable identifier for a cursor within a [`MultiCursorConfig`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CursorId(pub usize);

/// Direction a cursor advances through its territory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorDirection {
    /// Schedule earliest-feasible first, advancing from the territory start
    /// towards its end (EST semantics).
    Forward,
    /// Schedule latest-feasible first, advancing from the territory end towards
    /// its start (LST semantics, realised via a mirrored frame).
    Backward,
}

/// Where a cursor is anchored.
///
/// Anchors are primarily a Plan B concept. In Plan A the [`CursorTerritory`] is
/// authoritative; the anchor is retained so configurations remain descriptive
/// and forward-compatible.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CursorAnchor {
    /// Anchored at the start of the scheduling horizon.
    HorizonStart,
    /// Anchored at the end of the scheduling horizon.
    HorizonEnd,
    /// Anchored at a horizon-relative fraction in `[0, 1]`.
    Fraction(f64),
    /// Anchored at an absolute Modified Julian Date.
    Mjd(Time<MJD>),
}

/// A reference to a territory boundary.
///
/// Reserved for Plan B (dynamic territories). Fixed territories never use this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryRef {
    /// The start of the scheduling horizon.
    HorizonStart,
    /// The end of the scheduling horizon.
    HorizonEnd,
    /// The live position of another cursor.
    Cursor(CursorId),
}

/// The time region a cursor is allowed to schedule into.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CursorTerritory {
    /// A fixed, absolute `[start, end)` region (Plan A).
    Fixed {
        /// Inclusive start of the territory.
        start: Time<MJD>,
        /// Exclusive end of the territory.
        end: Time<MJD>,
    },
    /// A fixed region expressed as horizon-relative fractions in `[0, 1]`,
    /// resolved against the scheduling horizon at run time (Plan A).
    FractionRange {
        /// Start fraction (`0.0` == horizon start).
        start: f64,
        /// End fraction (`1.0` == horizon end).
        end: f64,
    },
    /// A dynamic region whose boundaries follow other cursors (Plan B).
    ///
    /// Not implemented yet — resolving such a territory returns
    /// [`ScheduleError::UnsupportedConfiguration`].
    Dynamic {
        /// Left boundary reference.
        left: BoundaryRef,
        /// Right boundary reference.
        right: BoundaryRef,
    },
}

impl CursorTerritory {
    /// Resolve this territory into a concrete `[start, end)` period.
    ///
    /// This is the single place territory bounds are computed for Plan A. Plan B
    /// will extend it (or the per-cursor active-period resolution) without
    /// touching the beam-search engine.
    pub fn resolve(&self, horizon: &Period<MJD>) -> Result<Period<MJD>, ScheduleError> {
        match *self {
            Self::Fixed { start, end } => clamp_period(start, end, horizon),
            Self::FractionRange { start, end } => {
                let span = horizon.end.value() - horizon.start.value();
                let s = Time::<MJD>::new(horizon.start.value() + span * start);
                let e = Time::<MJD>::new(horizon.start.value() + span * end);
                clamp_period(s, e, horizon)
            }
            Self::Dynamic { .. } => Err(ScheduleError::UnsupportedConfiguration(
                "dynamic cursor territories are not implemented yet".into(),
            )),
        }
    }
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

/// Per-cursor configuration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CursorConfig {
    /// Stable identifier for this cursor.
    pub id: CursorId,
    /// Where the cursor is anchored (descriptive in Plan A).
    pub anchor: CursorAnchor,
    /// Direction of travel.
    pub direction: CursorDirection,
    /// Region the cursor may schedule into.
    pub territory: CursorTerritory,
}

impl CursorConfig {
    /// Convenience constructor for a forward cursor over a fixed territory.
    pub const fn forward(id: usize, territory: CursorTerritory) -> Self {
        Self {
            id: CursorId(id),
            anchor: CursorAnchor::HorizonStart,
            direction: CursorDirection::Forward,
            territory,
        }
    }

    /// Convenience constructor for a backward cursor over a fixed territory.
    pub const fn backward(id: usize, territory: CursorTerritory) -> Self {
        Self {
            id: CursorId(id),
            anchor: CursorAnchor::HorizonEnd,
            direction: CursorDirection::Backward,
            territory,
        }
    }
}

/// Policy controlling how competing cursor actions are ranked each round.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CursorPolicy {
    /// Rank all candidate actions globally by their within-cursor rank, breaking
    /// ties by cursor id then task id. This is the only policy implemented.
    #[default]
    BestCandidateGlobal,
    /// Reserved: rotate placement opportunities between cursors.
    RoundRobin,
}

/// Full configuration for a multi-cursor scheduling run.
#[derive(Debug, Clone)]
pub struct MultiCursorConfig {
    /// Cursors participating in the search.
    pub cursors: Vec<CursorConfig>,
    /// Number of schedule states kept alive after each beam expansion round.
    pub k_beams: usize,
    /// Number of distinct actions explored per beam per round.
    pub branching_factor: usize,
    /// Endangered-task protection threshold (shared with EST semantics).
    pub endangered_threshold: u32,
    /// How competing cursor actions are ranked.
    pub cursor_policy: CursorPolicy,
}

impl MultiCursorConfig {
    /// Build a single forward cursor spanning the whole horizon (EST shape).
    pub fn single_forward(
        k_beams: usize,
        branching_factor: usize,
        endangered_threshold: u32,
    ) -> Self {
        Self {
            cursors: vec![CursorConfig::forward(
                0,
                CursorTerritory::FractionRange {
                    start: 0.0,
                    end: 1.0,
                },
            )],
            k_beams,
            branching_factor,
            endangered_threshold,
            cursor_policy: CursorPolicy::BestCandidateGlobal,
        }
    }

    /// Build a single backward cursor spanning the whole horizon (LST shape).
    pub fn single_backward(
        k_beams: usize,
        branching_factor: usize,
        endangered_threshold: u32,
    ) -> Self {
        Self {
            cursors: vec![CursorConfig::backward(
                0,
                CursorTerritory::FractionRange {
                    start: 0.0,
                    end: 1.0,
                },
            )],
            k_beams,
            branching_factor,
            endangered_threshold,
            cursor_policy: CursorPolicy::BestCandidateGlobal,
        }
    }

    /// Validate the configuration, normalising beam parameters.
    pub(crate) fn normalised(mut self) -> Result<Self, ScheduleError> {
        self.k_beams = self.k_beams.max(1);
        self.branching_factor = self.branching_factor.max(1);
        if self.cursors.is_empty() {
            return Err(ScheduleError::InvalidConfiguration(
                "multi-cursor config must declare at least one cursor".into(),
            ));
        }
        if self.cursor_policy == CursorPolicy::RoundRobin {
            return Err(ScheduleError::UnsupportedConfiguration(
                "round-robin cursor policy is not implemented yet".into(),
            ));
        }
        Ok(self)
    }
}
