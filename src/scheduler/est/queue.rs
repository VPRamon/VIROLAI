use super::candidate::EstCandidate;
use super::ordering::sort_candidates;
use crate::prescheduler::TaskPeriodMap;
use crate::task::Task;
use crate::time::{MJD, Period, SchedulingBlockId, TaskId};
use std::collections::HashMap;

/// Refreshable EST candidate queue kept in EST sort order.
#[derive(Debug, Clone)]
pub(super) struct CandidateQueue<'a> {
    candidates: Vec<EstCandidate<'a>>,
}

impl<'a> CandidateQueue<'a> {
    /// Build the initial candidate queue from validated tasks and feasible windows.
    ///
    /// When `task_block_map` is provided, each candidate's `block_id` is
    /// populated so that the scheduler can enforce dependency ordering.
    pub(super) fn build(
        tasks: &'a [&Task],
        possible_periods: &'a TaskPeriodMap,
        horizon: &Period<MJD>,
        task_block_map: Option<&HashMap<TaskId, SchedulingBlockId>>,
    ) -> Self {
        let mut candidates = tasks
            .iter()
            .map(|task| {
                let windows = possible_periods
                    .get(&task.id)
                    .expect("EST invariant violated: filtered task missing possible periods");
                let mut c = EstCandidate::new(task, windows, horizon);
                if let Some(map) = task_block_map {
                    c.block_id = map.get(&task.id).copied();
                }
                c
            })
            .collect::<Vec<_>>();

        sort_candidates(&mut candidates, horizon.start);

        log::debug!(
            "est: built candidate queue — {} candidate(s)",
            candidates.len(),
        );

        Self { candidates }
    }

    /// Count all currently schedulable candidates in queue order.
    pub(super) fn count_schedulable(&self) -> usize {
        self.candidates
            .iter()
            .filter(|c| !c.is_impossible())
            .count()
    }

    /// Remove and return the candidate at position `idx`.
    ///
    /// `idx` refers to the `idx`-th schedulable candidate in queue order. The
    /// remaining candidates keep their relative order — no re-sort is needed
    /// since `refresh()` has already sorted them.
    pub(super) fn pop_at(&mut self, idx: usize) -> EstCandidate<'a> {
        let raw_idx = self
            .candidates
            .iter()
            .enumerate()
            .filter(|(_, candidate)| !candidate.is_impossible())
            .nth(idx)
            .map(|(raw_idx, _)| raw_idx)
            .expect("pop_at: idx points past the end of the schedulable queue");

        debug_assert!(
            !self.candidates[raw_idx].is_impossible(),
            "pop_at: idx={idx} resolved to an impossible candidate at raw index {raw_idx}"
        );
        self.candidates.remove(raw_idx)
    }

    /// Refresh every candidate against the new beam horizon and re-sort the queue.
    pub(super) fn refresh(&mut self, horizon: &Period<MJD>) {
        log::trace!(
            "est: refreshing {} candidate(s) at cursor={:.4}",
            self.candidates.len(),
            horizon.start.value(),
        );

        self.candidates
            .iter_mut()
            .for_each(|candidate| candidate.refresh(horizon));

        // Candidate ordering is derived entirely from the refreshed EST
        // metadata, so the queue must be fully re-sorted after every refresh.
        sort_candidates(&mut self.candidates, horizon.start);
    }

    #[cfg(test)]
    pub(super) fn from_candidates_for_test(candidates: Vec<EstCandidate<'a>>) -> Self {
        Self { candidates }
    }
}
