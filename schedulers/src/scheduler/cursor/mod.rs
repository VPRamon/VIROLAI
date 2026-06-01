//! Configurable multi-cursor scheduler.
//!
//! This module generalises the EST/LST beam search into a configurable model
//! with one or more *cursors*, each owning a territory of the scheduling
//! horizon. It implements both **Plan A** (multiple simultaneous cursors with
//! fixed territories) and **Plan B** (dynamic territories whose boundaries
//! follow the live position of another cursor). Territory shape is resolved in
//! one place — [`config::CursorTerritory`] plus
//! `CursorRuntime::schedule_active_period` — never in the beam-search engine.
//!
//! # Mental model
//!
//! * **EST** is a preconfigured single-forward cursor wrapper over the whole
//!   horizon. [`crate::scheduler::est::EstScheduler`] delegates its `run`
//!   method to this engine.
//! * **LST** is a preconfigured single-backward cursor wrapper over the whole
//!   horizon. [`crate::scheduler::lst::LstScheduler`] delegates its `run`
//!   method to this engine.
//!   The backward direction is handled inside the engine via
//!   `CursorFrame::Mirrored`; there is no separate LST-specific mirroring pass.
//! * **Plan A** runs several cursors with disjoint fixed territories that share
//!   one global schedule; a task scheduled by one cursor becomes unavailable to
//!   all others, and no placement may escape its cursor's territory.
//!
//! # Equivalence
//!
//! [`MultiCursorScheduler::single_forward`] reproduces
//! [`crate::scheduler::est::EstScheduler`] exactly and
//! [`MultiCursorScheduler::single_backward`] reproduces
//! [`crate::scheduler::lst::LstScheduler`] exactly (see the tests in this
//! module). Both wrappers delegate to the same generic engine; direction is
//! handled via the cursor frame, not via external mirroring.

mod action;
mod config;
mod engine;
mod frame;
mod state;

pub use config::{
    BoundaryRef, CursorAnchor, CursorConfig, CursorDirection, CursorId, CursorPolicy,
    CursorTerritory, MultiCursorConfig,
};

use crate::error::ScheduleError;
use crate::prescheduler::TaskPeriodMap;
use crate::schedule::{Schedule, SchedulingProblem};
use crate::scheduler::SchedulingAlgorithm;
use crate::scheduler::est::{Configuration, ScheduleFom, SoftConstraintFom};
use crate::time::{MJD, Period};
use std::sync::Arc;

/// Multi-cursor beam-search scheduler.
#[derive(Clone)]
pub struct MultiCursorScheduler {
    /// Cursor layout and beam parameters.
    pub config: MultiCursorConfig,
    /// Figure of merit used to rank and prune beam states.
    pub fom: Arc<dyn ScheduleFom>,
}

impl std::fmt::Debug for MultiCursorScheduler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MultiCursorScheduler")
            .field("config", &self.config)
            .field("fom", &self.fom.label())
            .finish()
    }
}

impl MultiCursorScheduler {
    /// Construct a scheduler from an explicit config and figure of merit.
    pub fn new(
        config: MultiCursorConfig,
        fom: Arc<dyn ScheduleFom>,
    ) -> Result<Self, ScheduleError> {
        let config = config.normalised()?;
        Ok(Self { config, fom })
    }

    /// Single forward cursor over the whole horizon — equivalent to
    /// [`EstScheduler`](crate::scheduler::est::EstScheduler).
    pub fn single_forward(
        est_config: Configuration,
        fom: Arc<dyn ScheduleFom>,
    ) -> Result<Self, ScheduleError> {
        let config = MultiCursorConfig::single_forward(
            est_config.k_beams,
            est_config.branching_factor,
            est_config.endangered_threshold,
        );
        Self::new(config, fom)
    }

    /// Single backward cursor over the whole horizon — equivalent to
    /// [`LstScheduler`](crate::scheduler::lst::LstScheduler).
    pub fn single_backward(
        est_config: Configuration,
        fom: Arc<dyn ScheduleFom>,
    ) -> Result<Self, ScheduleError> {
        let config = MultiCursorConfig::single_backward(
            est_config.k_beams,
            est_config.branching_factor,
            est_config.endangered_threshold,
        );
        Self::new(config, fom)
    }

    /// Run the scheduler on a full problem.
    ///
    /// All cursor configurations — including a lone backward cursor (the
    /// LST-equivalent case) — run through the same generic beam engine. There
    /// is no special-case execution path: reverse cursors are handled inside the
    /// engine via their mirrored `CursorFrame`.
    pub fn run(
        &self,
        problem: &SchedulingProblem,
        possible_periods: &TaskPeriodMap,
        horizon: &Period<MJD>,
    ) -> Result<Schedule, ScheduleError> {
        engine::run_multi_cursor(
            &self.config,
            self.fom.as_ref(),
            problem,
            possible_periods,
            horizon,
        )
    }
}

impl Default for MultiCursorScheduler {
    fn default() -> Self {
        Self {
            config: MultiCursorConfig::single_forward(1, 1, 1),
            fom: Arc::new(SoftConstraintFom),
        }
    }
}

impl SchedulingAlgorithm for MultiCursorScheduler {
    fn run_unchecked(
        &self,
        problem: &SchedulingProblem,
        possible_periods: &TaskPeriodMap,
        horizon: &Period<MJD>,
    ) -> Result<Schedule, ScheduleError> {
        MultiCursorScheduler::run(self, problem, possible_periods, horizon)
    }
}

/// Run the cursor engine with a borrowed figure of merit.
///
/// Used by [`EstScheduler`](crate::scheduler::est::EstScheduler) and
/// [`LstScheduler`](crate::scheduler::lst::LstScheduler) to delegate to the
/// cursor engine without requiring an `Arc` allocation.
pub(crate) fn run_with_config(
    config: &MultiCursorConfig,
    fom: &dyn ScheduleFom,
    problem: &SchedulingProblem,
    possible_periods: &TaskPeriodMap,
    horizon: &Period<MJD>,
) -> Result<Schedule, ScheduleError> {
    engine::run_multi_cursor(config, fom, problem, possible_periods, horizon)
}

#[cfg(test)]
mod tests;
