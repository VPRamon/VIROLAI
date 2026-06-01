//! Runtime state for the multi-cursor beam search.
//!
//! [`CursorRuntime`] is a thin per-cursor candidate queue built entirely from
//! the public EST [`Candidate`] / [`sort_candidates`] building blocks, so its
//! ordering is identical to the single-cursor EST queue. [`MultiCursorState`]
//! is one live beam: the shared schedule plus every cursor's queue and the
//! cached figure-of-merit score.

use super::config::{CursorDirection, CursorId};
use super::frame::CursorFrame;
use crate::schedule::Schedule;
use crate::scheduler::est::{Candidate, sort_candidates};
use crate::time::{MJD, Period, TaskId, Time};

/// One cursor's live search state.
#[derive(Clone)]
pub(super) struct CursorRuntime<'a> {
    /// Stable id (deterministic tie-breaks, logging).
    pub(super) id: CursorId,
    /// Frame mapping between this cursor's local time and schedule time.
    pub(super) frame: CursorFrame,
    /// Territory in schedule time. The frame-space territory shares the same
    /// `[start, end)` bounds because the mirror axis is the territory itself.
    pub(super) territory: Period<MJD>,
    /// Current cursor position in frame time.
    pub(super) frame_cursor: Time<MJD>,
    /// Candidate queue in EST order (schedulable candidates first).
    pub(super) candidates: Vec<Candidate<'a>>,
}

impl<'a> CursorRuntime<'a> {
    /// Resolve the cursor's active region in frame time.
    ///
    /// This is the **single** place a cursor's territory is turned into the
    /// region it may schedule into. Plan B (dynamic territories) only needs to
    /// change this method, not the engine.
    pub(super) fn active_period(&self) -> Period<MJD> {
        Period::new(self.frame_cursor, self.territory.end)
    }

    /// Refresh and re-sort the queue against the current active region.
    pub(super) fn refresh(&mut self, threshold: u32) {
        let active = self.active_period();
        for candidate in &mut self.candidates {
            candidate.refresh(&active);
        }
        sort_candidates(&mut self.candidates, active.start, threshold);
    }

    /// Number of schedulable candidates at the front of the queue.
    pub(super) fn count_schedulable(&self) -> usize {
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
}

impl MultiCursorState<'_> {
    /// Total schedulable candidates across all cursors.
    pub(super) fn total_schedulable(&self) -> usize {
        self.cursors.iter().map(|c| c.count_schedulable()).sum()
    }
}

/// Convenience used by the engine when seeding the initial state.
pub(super) fn initial_cursor_time(
    _direction: CursorDirection,
    territory: &Period<MJD>,
) -> Time<MJD> {
    // Every cursor runs forward in frame time, starting at the near edge of its
    // territory. For a backward cursor that frame edge maps to the territory end
    // in schedule time (scheduling latest-feasible first).
    territory.start
}
