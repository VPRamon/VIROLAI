use super::candidate::EstCandidate;
use super::ordering::sort_candidates;
use crate::prescheduler::TaskPeriodMap;
use crate::task::Task;
use crate::time::{MJD, Period};

#[derive(Debug, Clone)]
pub(super) struct CandidateQueue<'a> {
    candidates: Vec<EstCandidate<'a>>,
}

impl<'a> CandidateQueue<'a> {
    pub(super) fn build(
        tasks: &'a [Task],
        possible_periods: &'a TaskPeriodMap,
        horizon: &Period<MJD>,
        endangered_threshold: u32,
    ) -> Self {
        let mut candidates = tasks
            .iter()
            .map(|task| {
                let windows = possible_periods
                    .get(&task.id)
                    .expect("EST invariant violated: filtered task missing possible periods");
                EstCandidate::new(task, windows, horizon)
            })
            .collect::<Vec<_>>();

        sort_candidates(&mut candidates, endangered_threshold, horizon.start);

        log::debug!(
            "est: built candidate queue — {} candidate(s), threshold={}",
            candidates.len(),
            endangered_threshold,
        );

        Self { candidates }
    }

    pub(super) fn pop_next(&mut self) -> Option<EstCandidate<'a>> {
        self.candidates
            .first()
            .filter(|candidate| !candidate.is_impossible())?;
        Some(self.candidates.remove(0))
    }

    pub(super) fn refresh(&mut self, horizon: &Period<MJD>, endangered_threshold: u32) {
        log::trace!(
            "est: refreshing {} candidate(s) at cursor={:.4}",
            self.candidates.len(),
            horizon.start.value(),
        );

        self.candidates
            .iter_mut()
            .for_each(|candidate| candidate.refresh(horizon));

        sort_candidates(&mut self.candidates, endangered_threshold, horizon.start);
    }
}
