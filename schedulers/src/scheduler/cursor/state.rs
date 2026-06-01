//! Runtime state for the multi-cursor beam search.
//!
//! [`CursorRuntime`] is a thin per-cursor candidate queue built entirely from
//! the public EST [`Candidate`] / [`sort_candidates`] building blocks, so its
//! ordering is identical to the single-cursor EST queue. [`MultiCursorState`]
//! is one live beam: the shared schedule plus every cursor's queue and the
//! cached figure-of-merit score.
//!
//! # Territory resolution
//!
//! A cursor's *active period* — the schedule-time region it may place into this
//! round — is resolved in [`CursorRuntime::schedule_active_period`]. This is the
//! **single** place territory bounds become a concrete region. Fixed (Plan A)
//! and dynamic (Plan B) territories both flow through it; dynamic boundaries are
//! resolved against the live positions of other cursors carried in a
//! [`CursorWorld`]. The beam engine never special-cases territory shape.

use super::config::{BoundarySide, CursorDirection, CursorId, CursorTerritory};
use super::frame::CursorFrame;
use crate::error::ScheduleError;
use crate::schedule::Schedule;
use crate::scheduler::est::{Candidate, sort_candidates};
use crate::time::{MJD, Period, TaskId, Time};

/// Snapshot of every cursor's live schedule-time frontier for one beam state.
///
/// Built immediately before each beam expansion so dynamic boundaries resolve
/// against the *current* state, never a stale or cross-beam view.
pub(super) struct CursorWorld {
    positions: Vec<(CursorId, Time<MJD>)>,
}

impl CursorWorld {
    /// Build a world snapshot from a state's cursors.
    pub(super) fn snapshot(cursors: &[CursorRuntime<'_>]) -> Self {
        Self {
            positions: cursors
                .iter()
                .map(|c| (c.id, c.schedule_position()))
                .collect(),
        }
    }

    /// Live schedule-time position of a referenced cursor.
    fn position(&self, id: CursorId) -> Result<Time<MJD>, ScheduleError> {
        self.positions
            .iter()
            .find(|(cid, _)| *cid == id)
            .map(|(_, t)| *t)
            .ok_or_else(|| {
                ScheduleError::InvalidConfiguration(format!(
                    "dynamic boundary references unknown cursor {}",
                    id.0
                ))
            })
    }
}

/// One cursor's live search state.
#[derive(Clone)]
pub(super) struct CursorRuntime<'a> {
    /// Stable id (deterministic tie-breaks, logging).
    pub(super) id: CursorId,
    /// Direction of travel (forward or backward).
    pub(super) direction: CursorDirection,
    /// Frame mapping between this cursor's local time and schedule time.
    pub(super) frame: CursorFrame,
    /// Territory definition (fixed or dynamic), resolved live each round.
    pub(super) territory: CursorTerritory,
    /// Maximal fixed extent of the territory in schedule time (frame axis and
    /// fallback bound).
    pub(super) extent: Period<MJD>,
    /// Current cursor position in frame time.
    pub(super) frame_cursor: Time<MJD>,
    /// Candidate queue in EST order (schedulable candidates first).
    pub(super) candidates: Vec<Candidate<'a>>,
    /// `true` when the cursor had no active region last refresh (exhausted or
    /// squeezed out by a neighbouring cursor). Skips destructive queue refresh.
    pub(super) exhausted: bool,
}

impl<'a> CursorRuntime<'a> {
    /// The cursor's live frontier in schedule time.
    pub(super) fn schedule_position(&self) -> Time<MJD> {
        self.frame.to_schedule_time(self.frame_cursor)
    }

    /// Resolve the cursor's active region in **schedule** time for this round.
    ///
    /// This is the single place a territory becomes a concrete schedulable
    /// region. Dynamic boundaries are resolved against `world`; the cursor's own
    /// advancing frontier clamps the near edge. Returns `None` when the region
    /// is empty (the cursor is exhausted or has been squeezed out).
    pub(super) fn schedule_active_period(
        &self,
        world: &CursorWorld,
        horizon: &Period<MJD>,
    ) -> Result<Option<Period<MJD>>, ScheduleError> {
        let (start_side, end_side) = self.territory.sides(horizon)?;
        let gap = self.territory.min_gap();

        let mut start = match start_side {
            BoundarySide::Fixed(t) => t,
            BoundarySide::Cursor(id) => Time::<MJD>::new(world.position(id)?.value() + gap),
        };
        let mut end = match end_side {
            BoundarySide::Fixed(t) => t,
            BoundarySide::Cursor(id) => Time::<MJD>::new(world.position(id)?.value() - gap),
        };

        let own = self.schedule_position();
        match self.direction {
            CursorDirection::Forward => start = max_time(start, own),
            CursorDirection::Backward => end = min_time(end, own),
        }

        if end.value() <= start.value() {
            Ok(None)
        } else {
            Ok(Some(Period::new(start, end)))
        }
    }

    /// Refresh and re-sort the queue against a frame-time active region.
    ///
    /// `None` marks the cursor exhausted for this round without mutating the
    /// queue's window cursor (which must only advance monotonically).
    pub(super) fn refresh(&mut self, frame_active: Option<&Period<MJD>>, threshold: u32) {
        match frame_active {
            Some(active) => {
                self.exhausted = false;
                for candidate in &mut self.candidates {
                    candidate.refresh(active);
                }
                sort_candidates(&mut self.candidates, active.start, threshold);
            }
            None => self.exhausted = true,
        }
    }

    /// Number of schedulable candidates at the front of the queue.
    pub(super) fn count_schedulable(&self) -> usize {
        if self.exhausted {
            return 0;
        }
        self.candidates
            .iter()
            .take_while(|c| !c.is_impossible())
            .count()
    }

    /// Task id of the candidate at sorted position `idx`, if any.
    pub(super) fn task_at(&self, idx: usize) -> Option<TaskId> {
        self.candidates.get(idx).map(|c| c.task_id())
    }

    /// Replicates EST's dominance pruning: candidate `idx` is dominated when its
    /// raw EST falls at or beyond candidate 0's window end.
    pub(super) fn is_dominated_by_first(&self, idx: usize) -> bool {
        if idx == 0 {
            return false;
        }
        let Some(first) = self.candidates.first() else {
            return false;
        };
        if first.is_impossible() {
            return false;
        }
        let Some(first_est) = first.est else {
            return false;
        };
        let cutoff = first_est + first.duration();
        self.candidates
            .get(idx)
            .and_then(|c| c.est)
            .is_some_and(|est| est >= cutoff)
    }

    /// Remove and return the candidate at sorted position `idx`.
    pub(super) fn pop_at(&mut self, idx: usize) -> Candidate<'a> {
        self.candidates.remove(idx)
    }

    /// Remove a candidate by task id (used when another cursor schedules it).
    ///
    /// Returns `true` when a candidate was removed.
    pub(super) fn remove_task(&mut self, task_id: TaskId) -> bool {
        if let Some(pos) = self.candidates.iter().position(|c| c.task_id() == task_id) {
            self.candidates.remove(pos);
            true
        } else {
            false
        }
    }

    /// Advance the cursor to the end of a just-placed frame-time interval.
    ///
    /// `frame_end` is the end of the placement in frame time.
    pub(super) fn advance_to(&mut self, frame_end: Time<MJD>) {
        self.frame_cursor = frame_end;
    }
}

/// One live beam state for the multi-cursor search.
#[derive(Clone)]
pub(super) struct MultiCursorState<'a> {
    /// Placements chosen so far (schedule time).
    pub(super) schedule: Schedule,
    /// Per-cursor queues.
    pub(super) cursors: Vec<CursorRuntime<'a>>,
    /// Cached figure-of-merit score for this state.
    pub(super) score: f64,
    /// Schedule-time position of the cursor that last placed a task, used to
    /// build the figure-of-merit context. Starts at the horizon start.
    pub(super) last_cursor_schedule_time: Time<MJD>,
    /// Id of the cursor that last placed a task (figure-of-merit context).
    pub(super) last_action_cursor: Option<usize>,
}

impl MultiCursorState<'_> {
    /// Total schedulable candidates across all cursors.
    pub(super) fn total_schedulable(&self) -> usize {
        self.cursors.iter().map(|c| c.count_schedulable()).sum()
    }
}

/// Initial frame-time position of a cursor: the near edge of its extent.
///
/// Every cursor runs forward in frame time, starting at the near edge of its
/// extent. For a backward cursor that frame edge maps to the extent end in
/// schedule time (scheduling latest-feasible first).
pub(super) fn initial_cursor_time(extent: &Period<MJD>) -> Time<MJD> {
    extent.start
}

fn max_time(a: Time<MJD>, b: Time<MJD>) -> Time<MJD> {
    if a.value() >= b.value() { a } else { b }
}

fn min_time(a: Time<MJD>, b: Time<MJD>) -> Time<MJD> {
    if a.value() <= b.value() { a } else { b }
}
