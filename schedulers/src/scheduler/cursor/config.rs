//! Configuration types for the multi-cursor scheduler.
//!
//! These types describe *what* cursors exist and *where* they may schedule,
//! independently of the beam-search engine that executes them. The scheduler
//! supports both implemented territory models:
//!
//! - **Plan A** — fixed territories
//! - **Plan B** — dynamic cursor-relative territories
//!
//! Territory resolution stays behind [`CursorTerritory`] so the engine never
//! needs to special-case how a cursor's active region is computed.

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
/// Anchors are mainly descriptive for fixed layouts and operational for dynamic
/// layouts. In Plan A the [`CursorTerritory`] is authoritative; in Plan B the
/// anchor helps describe where the cursor starts before live boundaries begin to
/// constrain it.
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
/// Fixed territories never use this; dynamic territories (Plan B) use it to make
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
    fn extreme(self, horizon: &Period<MJD>, cursor_extreme: Time<MJD>) -> Time<MJD> {
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
    /// A dynamic region whose boundaries may follow other cursors (Plan B).
    ///
    /// A boundary that references another cursor is recomputed before every
    /// beam-expansion round from that cursor's live frontier, so two cursors can
    /// advance towards each other until they meet.
    Dynamic {
        /// Left boundary reference.
        start: BoundaryRef,
        /// Right boundary reference.
        end: BoundaryRef,
        /// Optional buffer (in days) kept between a cursor-referenced boundary
        /// and the referenced cursor's frontier. `None` is treated as `0.0`.
        min_gap: Option<f64>,
    },
}

impl CursorTerritory {
    /// Resolve this territory's **maximal fixed extent** against the horizon.
    ///
    /// For [`Fixed`](Self::Fixed) / [`FractionRange`](Self::FractionRange) this
    /// is the territory itself. For [`Dynamic`](Self::Dynamic) it is the widest
    /// region the territory can ever occupy (cursor-referenced boundaries
    /// resolve to the horizon edge on their side). The extent is used as the
    /// fixed reflection axis for backward cursors and to pre-compute frame
    /// windows; the live region is resolved separately each round.
    pub fn extent(&self, horizon: &Period<MJD>) -> Result<Period<MJD>, ScheduleError> {
        match *self {
            Self::Fixed { start, end } => clamp_period(start, end, horizon),
            Self::FractionRange { start, end } => {
                let (s, e) = fraction_bounds(start, end, horizon);
                clamp_period(s, e, horizon)
            }
            Self::Dynamic { start, end, .. } => {
                let s = start.extreme(horizon, horizon.start);
                let e = end.extreme(horizon, horizon.end);
                clamp_period(s, e, horizon)
            }
        }
    }

    /// Optional buffer kept around cursor-referenced boundaries, in days.
    pub(super) fn min_gap(&self) -> f64 {
        match *self {
            Self::Dynamic { min_gap, .. } => min_gap.unwrap_or(0.0),
            _ => 0.0,
        }
    }

    /// The two boundary sides of this territory in schedule time.
    ///
    /// Fixed/fraction territories resolve both sides to fixed instants; dynamic
    /// territories keep cursor references so the engine can resolve them against
    /// live positions each round.
    pub(super) fn sides(
        &self,
        horizon: &Period<MJD>,
    ) -> Result<(BoundarySide, BoundarySide), ScheduleError> {
        match *self {
            Self::Fixed { start, end } => {
                let p = clamp_period(start, end, horizon)?;
                Ok((BoundarySide::Fixed(p.start), BoundarySide::Fixed(p.end)))
            }
            Self::FractionRange { start, end } => {
                let (s, e) = fraction_bounds(start, end, horizon);
                let p = clamp_period(s, e, horizon)?;
                Ok((BoundarySide::Fixed(p.start), BoundarySide::Fixed(p.end)))
            }
            Self::Dynamic { start, end, .. } => {
                Ok((boundary_side(start, horizon), boundary_side(end, horizon)))
            }
        }
    }
}

/// Map a [`BoundaryRef`] onto a [`BoundarySide`]. Horizon edges are fixed; a
/// cursor reference stays live.
fn boundary_side(r: BoundaryRef, horizon: &Period<MJD>) -> BoundarySide {
    match r {
        BoundaryRef::HorizonStart => BoundarySide::Fixed(horizon.start),
        BoundaryRef::HorizonEnd => BoundarySide::Fixed(horizon.end),
        BoundaryRef::Cursor(id) => BoundarySide::Cursor(id),
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

    /// Four forward cursors over contiguous quarter-horizon territories
    /// `[0, 0.25)`, `[0.25, 0.5)`, `[0.5, 0.75)`, and `[0.75, 1.0)` (Plan A).
    pub fn four_quarter_forward(
        k_beams: usize,
        branching_factor: usize,
        endangered_threshold: u32,
    ) -> Self {
        Self {
            cursors: vec![
                CursorConfig::forward(
                    0,
                    CursorTerritory::FractionRange {
                        start: 0.0,
                        end: 0.25,
                    },
                ),
                CursorConfig::forward(
                    1,
                    CursorTerritory::FractionRange {
                        start: 0.25,
                        end: 0.5,
                    },
                ),
                CursorConfig::forward(
                    2,
                    CursorTerritory::FractionRange {
                        start: 0.5,
                        end: 0.75,
                    },
                ),
                CursorConfig::forward(
                    3,
                    CursorTerritory::FractionRange {
                        start: 0.75,
                        end: 1.0,
                    },
                ),
            ],
            k_beams,
            branching_factor,
            endangered_threshold,
            cursor_policy: CursorPolicy::BestCandidateGlobal,
        }
    }

    /// Two cursors advancing from both horizon ends until they meet (Plan B).
    ///
    /// * cursor 0 — forward from the horizon start; its dynamic end follows
    ///   cursor 1's live position.
    /// * cursor 1 — backward from the horizon end; its dynamic start follows
    ///   cursor 0's live position.
    ///
    /// The two cursors share the whole horizon and split it wherever they meet.
    pub fn dynamic_est_lst_meet(
        k_beams: usize,
        branching_factor: usize,
        endangered_threshold: u32,
    ) -> Self {
        Self {
            cursors: vec![
                CursorConfig::forward(
                    0,
                    CursorTerritory::Dynamic {
                        start: BoundaryRef::HorizonStart,
                        end: BoundaryRef::Cursor(CursorId(1)),
                        min_gap: None,
                    },
                ),
                CursorConfig::backward(
                    1,
                    CursorTerritory::Dynamic {
                        start: BoundaryRef::Cursor(CursorId(0)),
                        end: BoundaryRef::HorizonEnd,
                        min_gap: None,
                    },
                ),
            ],
            k_beams,
            branching_factor,
            endangered_threshold,
            cursor_policy: CursorPolicy::BestCandidateGlobal,
        }
    }

    /// Two forward cursors, the second anchored at the horizon midpoint, where
    /// the first cursor's dynamic end follows the second cursor's live position
    /// (Plan B).
    ///
    /// * cursor 0 — forward from the horizon start; its dynamic end follows
    ///   cursor 1's live position.
    /// * cursor 1 — forward over the fixed second half `[0.5, 1.0)`; it advances
    ///   from the midpoint and never yields ground to cursor 0.
    pub fn dynamic_start_mid_forward(
        k_beams: usize,
        branching_factor: usize,
        endangered_threshold: u32,
    ) -> Self {
        Self {
            cursors: vec![
                CursorConfig::forward(
                    0,
                    CursorTerritory::Dynamic {
                        start: BoundaryRef::HorizonStart,
                        end: BoundaryRef::Cursor(CursorId(1)),
                        min_gap: None,
                    },
                ),
                CursorConfig::forward(
                    1,
                    CursorTerritory::FractionRange {
                        start: 0.5,
                        end: 1.0,
                    },
                ),
            ],
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

        let ids: Vec<CursorId> = self.cursors.iter().map(|c| c.id).collect();
        for (i, id) in ids.iter().enumerate() {
            if ids[..i].contains(id) {
                return Err(ScheduleError::InvalidConfiguration(format!(
                    "duplicate cursor id {}",
                    id.0
                )));
            }
        }

        for cursor in &self.cursors {
            if let CursorTerritory::Dynamic {
                start,
                end,
                min_gap,
            } = cursor.territory
            {
                if min_gap.is_some_and(|g| g < 0.0 || !g.is_finite()) {
                    return Err(ScheduleError::InvalidConfiguration(format!(
                        "cursor {} has a negative or non-finite min_gap",
                        cursor.id.0
                    )));
                }
                for boundary in [start, end] {
                    if let Some(ref_id) = boundary.cursor() {
                        if ref_id == cursor.id {
                            return Err(ScheduleError::InvalidConfiguration(format!(
                                "cursor {} dynamic boundary references itself",
                                cursor.id.0
                            )));
                        }
                        if !ids.contains(&ref_id) {
                            return Err(ScheduleError::InvalidConfiguration(format!(
                                "cursor {} dynamic boundary references unknown cursor {}",
                                cursor.id.0, ref_id.0
                            )));
                        }
                    }
                }
            }
        }

        Ok(self)
    }
}
