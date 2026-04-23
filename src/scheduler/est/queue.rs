use super::candidate::EstCandidate;
use super::ordering::{compare_by_cached, sort_candidates};
use crate::prescheduler::TaskPeriodMap;
use crate::task::Task;
use crate::time::{MJD, Period, SchedulingBlockId, TaskId, Time};
use std::collections::HashMap;

/// Refreshable EST candidate queue kept in EST sort order.
///
/// After every [`refresh`][CandidateQueue::refresh] (or [`build`][CandidateQueue::build]):
/// * All schedulable candidates occupy indices `[0, schedulable_count)`.
/// * All impossible candidates occupy indices `[schedulable_count, len)`.
#[derive(Debug, Clone)]
pub(super) struct CandidateQueue<'a> {
    candidates: Vec<EstCandidate<'a>>,
    /// Reusable buffer for collecting endangered ESTs; cleared on each refresh.
    scratch_endangered: Vec<Time<MJD>>,
    /// Cached count of schedulable candidates; updated after every sort.
    schedulable_count: usize,
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
        threshold: u32,
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

        sort_candidates(&mut candidates, horizon.start, threshold);

        let schedulable_count = candidates.iter().take_while(|c| !c.is_impossible()).count();

        log::debug!(
            "est: built candidate queue — {} candidate(s)",
            candidates.len(),
        );

        Self {
            candidates,
            scratch_endangered: Vec::new(),
            schedulable_count,
        }
    }

    /// Return the cached count of schedulable candidates.
    pub(super) fn count_schedulable(&self) -> usize {
        self.schedulable_count
    }

    /// Remove and return the candidate at sorted position `idx`.
    ///
    /// `idx` must be in `[0, schedulable_count)`.  Because all schedulable
    /// candidates are at the front of the sorted slice, this is a direct
    /// `Vec::remove` with no scan.
    pub(super) fn pop_at(&mut self, idx: usize) -> EstCandidate<'a> {
        debug_assert!(
            idx < self.schedulable_count,
            "pop_at: idx={idx} out of schedulable range (count={})",
            self.schedulable_count
        );
        self.schedulable_count -= 1;
        self.candidates.remove(idx)
    }

    /// Refresh every candidate against the new beam horizon and re-sort the queue.
    pub(super) fn refresh(&mut self, horizon: &Period<MJD>, threshold: u32) {
        log::trace!(
            "est: refreshing {} candidate(s) at cursor={:.4}",
            self.candidates.len(),
            horizon.start.value(),
        );

        let priority_at = horizon.start;

        for c in &mut self.candidates {
            c.refresh(horizon);
        }

        // Collect endangered ESTs into the scratch buffer.
        self.scratch_endangered.clear();
        for c in &self.candidates {
            if !c.is_impossible() && c.is_endangered(threshold)
                && let Some(est) = c.est
            {
                self.scratch_endangered.push(est);
            }
        }

        // Temporarily move scratch out to satisfy the borrow checker: we need
        // an immutable view of endangered ESTs while mutably iterating candidates.
        let endangered = std::mem::take(&mut self.scratch_endangered);
        for c in &mut self.candidates {
            c.priority_at_cursor = c.priority(priority_at);
            let is_endangered = !c.is_impossible() && c.is_endangered(threshold);
            c.is_endangered_cached = is_endangered;
            c.effective_est = if c.is_impossible() {
                None
            } else if is_endangered {
                c.est
            } else {
                c.est.map(|est| {
                    let est_days = est.value();
                    let dur = c.duration().value();
                    let mut eff = est;
                    for &e in &endangered {
                        let ed = e.value();
                        if est_days <= ed && est_days + dur > ed && ed > eff.value() {
                            eff = e;
                        }
                    }
                    eff
                })
            };
        }
        self.scratch_endangered = endangered;

        self.candidates.sort_by(compare_by_cached);

        self.schedulable_count = self
            .candidates
            .iter()
            .take_while(|c| !c.is_impossible())
            .count();
    }

    #[cfg(test)]
    pub(super) fn from_candidates_for_test(mut candidates: Vec<EstCandidate<'a>>) -> Self {
        // Maintain the sorted-prefix invariant: schedulable candidates first.
        candidates.sort_by(|a, b| match (a.is_impossible(), b.is_impossible()) {
            (false, true) => std::cmp::Ordering::Less,
            (true, false) => std::cmp::Ordering::Greater,
            _ => std::cmp::Ordering::Equal,
        });
        let schedulable_count = candidates.iter().take_while(|c| !c.is_impossible()).count();
        Self {
            candidates,
            scratch_endangered: Vec::new(),
            schedulable_count,
        }
    }
}
