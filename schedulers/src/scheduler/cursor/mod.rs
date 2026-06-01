//! Configurable multi-cursor scheduler.
//!
//! This module generalises the EST/LST beam search into a configurable model
//! with one or more *cursors*, each owning a fixed territory of the scheduling
//! horizon. It implements **Plan A** (multiple simultaneous cursors with fixed
//! territories) and is structured so that **Plan B** (dynamic territories) can
//! be added later by changing only [`config::CursorTerritory::resolve`] and
//! [`state::CursorRuntime::active_period`] — not the beam-search engine.
//!
//! # Mental model
//!
//! * **EST** is a single forward cursor over the whole horizon.
//! * **LST** is a single backward cursor over the whole horizon (realised by
//!   mirroring the horizon and running EST, exactly as [`LstScheduler`] does).
//! * **Plan A** runs several cursors with disjoint fixed territories that share
//!   one global schedule; a task scheduled by one cursor becomes unavailable to
//!   all others, and no placement may escape its cursor's territory.
//!
//! # Equivalence
//!
//! [`MultiCursorScheduler::single_forward`] reproduces [`EstScheduler`] exactly
//! and [`MultiCursorScheduler::single_backward`] reproduces [`LstScheduler`]
//! exactly (see the tests in this module). The single-forward path runs the
//! generic engine with one identity-frame cursor; the single-backward path
//! mirrors the problem and runs that same engine, then unmirrors the result.

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
use crate::scheduler::lst::MirroredFom;
use crate::scheduler::lst::transform::{mirror_task_periods, unmirror_schedule};
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
    pub fn run(
        &self,
        problem: &SchedulingProblem,
        possible_periods: &TaskPeriodMap,
        horizon: &Period<MJD>,
    ) -> Result<Schedule, ScheduleError> {
        if let Some(schedule) = self.try_run_single_backward(problem, possible_periods, horizon)? {
            return Ok(schedule);
        }
        engine::run_multi_cursor(
            &self.config,
            self.fom.as_ref(),
            problem,
            possible_periods,
            horizon,
        )
    }

    /// LST-equivalent fast path for a lone backward cursor.
    ///
    /// A single backward cursor is executed by mirroring the problem about the
    /// cursor's territory, running the forward engine in mirrored space with a
    /// [`MirroredFom`] wrapper, then unmirroring the result. This reproduces
    /// [`LstScheduler`] exactly when the territory spans the full horizon.
    fn try_run_single_backward(
        &self,
        problem: &SchedulingProblem,
        possible_periods: &TaskPeriodMap,
        horizon: &Period<MJD>,
    ) -> Result<Option<Schedule>, ScheduleError> {
        if self.config.cursors.len() != 1 {
            return Ok(None);
        }
        let cursor = &self.config.cursors[0];
        if cursor.direction != CursorDirection::Backward {
            return Ok(None);
        }

        let territory = cursor.territory.resolve(horizon)?;
        let mirrored_periods = mirror_task_periods(possible_periods, &territory);
        let mirrored_fom: Arc<dyn ScheduleFom> =
            Arc::new(MirroredFom::new(Arc::clone(&self.fom), territory));

        // One forward identity cursor over `territory`, in mirrored space.
        let forward = MultiCursorConfig {
            cursors: vec![CursorConfig::forward(
                cursor.id.0,
                CursorTerritory::Fixed {
                    start: territory.start,
                    end: territory.end,
                },
            )],
            k_beams: self.config.k_beams,
            branching_factor: self.config.branching_factor,
            endangered_threshold: self.config.endangered_threshold,
            cursor_policy: self.config.cursor_policy,
        };

        let mirrored_schedule = engine::run_multi_cursor(
            &forward,
            mirrored_fom.as_ref(),
            problem,
            &mirrored_periods,
            &territory,
        )?;

        Ok(Some(unmirror_schedule(&mirrored_schedule, &territory)))
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

#[cfg(test)]
mod tests;
