use super::candidate::Candidate;
use super::ordering::{compare_by_cached, sort_candidates};
use crate::prescheduler::TaskPeriodMap;
use crate::task::Task;
use crate::time::{MJD, Period};

/// Refreshable EST candidate queue kept in EST sort order.
///
/// After every [`refresh`][CandidateQueue::refresh] (or [`build`][CandidateQueue::build]):
/// * All schedulable candidates occupy indices `[0, schedulable_count)`.
/// * All impossible candidates occupy indices `[schedulable_count, len)`.
#[derive(Debug, Clone)]
pub(super) struct CandidateQueue<'a> {
    candidates: Vec<Candidate<'a>>,
}

impl<'a> CandidateQueue<'a> {
    /// Build the initial candidate queue from validated tasks and feasible windows.
    ///
    pub(super) fn build(
        tasks: &'a [&Task],
        possible_periods: &'a TaskPeriodMap,
        horizon: &Period<MJD>,
        threshold: u32,
    ) -> Self {
        let mut candidates = tasks
            .iter()
            .map(|task| {
                let windows = possible_periods
                    .get(&task.id)
                    .expect("EST invariant violated: filtered task missing possible periods");
                Candidate::new(task, windows, horizon)
            })
            .collect::<Vec<_>>();

        sort_candidates(&mut candidates, horizon.start, threshold);

        log::debug!(
            "est: built candidate queue — {} candidate(s)",
            candidates.len(),
        );

        Self { candidates }
    }

    /// Return the number of schedulable candidates at the front of the queue.
    pub(super) fn count_schedulable(&self) -> usize {
        self.candidates
            .iter()
            .take_while(|c| !c.is_impossible())
            .count()
    }

    /// Remove and return the candidate at sorted position `idx`.
    ///
    /// `idx` must be in `[0, schedulable_count)`.  Because all schedulable
    /// candidates are at the front of the sorted slice, this is a direct
    /// `Vec::remove` with no scan.
    pub(super) fn pop_at(&mut self, idx: usize) -> Candidate<'a> {
        let schedulable_count = self.count_schedulable();
        debug_assert!(
            idx < schedulable_count,
            "pop_at: idx={idx} out of schedulable range (count={})",
            schedulable_count
        );
        self.candidates.remove(idx)
    }

    /// Refresh every candidate against the new beam horizon and re-sort the queue.
    pub(super) fn refresh(&mut self, horizon: &Period<MJD>, threshold: u32) {
        log::trace!(
            "est: refreshing {} candidate(s) at cursor={:.4}",
            self.candidates.len(),
            horizon.start.value(),
        );

        for c in &mut self.candidates {
            c.refresh(horizon);
        }

        let endangered: Vec<_> = self
            .candidates
            .iter()
            .filter(|c| !c.is_impossible() && c.is_endangered(threshold))
            .filter_map(|c| c.est)
            .collect();

        for c in &mut self.candidates {
            c.update_caches(horizon.start, threshold, &endangered);
        }

        self.candidates.sort_by(compare_by_cached);
    }

    #[cfg(test)]
    pub(super) fn from_candidates_for_test(mut candidates: Vec<Candidate<'a>>) -> Self {
        // Maintain the sorted-prefix invariant: schedulable candidates first.
        candidates.sort_by(|a, b| match (a.is_impossible(), b.is_impossible()) {
            (false, true) => std::cmp::Ordering::Less,
            (true, false) => std::cmp::Ordering::Greater,
            _ => std::cmp::Ordering::Equal,
        });
        Self { candidates }
    }
}
