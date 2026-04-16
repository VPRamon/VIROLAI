use crate::schedule::TaskPlacement;
use crate::task::Task;
use crate::time::{JD, MJD, Period, PeriodSet, TaskId, Time};
use qtty::Day;

pub trait IntoTaskPlacement {
    fn into_task_placement(self, horizon_end: Time<MJD>) -> TaskPlacement;
}

/// Mutable EST work item used during candidate ordering and placement.
#[derive(Debug, Clone)]
pub struct EstCandidate<'a> {
    task: &'a Task,
    windows: &'a PeriodSet<MJD>,
    pub(crate) next_window_idx: usize,
    pub est: Option<Time<MJD>>,
    pub deadline: Option<Time<MJD>>,
    pub flexibility: f64,
}

impl<'a> EstCandidate<'a> {
    pub fn new(task: &'a Task, windows: &'a PeriodSet<MJD>, horizon: &Period<MJD>) -> Self {
        let mut candidate = Self {
            task,
            windows,
            next_window_idx: 0,
            est: None,
            deadline: None,
            flexibility: 0.0,
        };
        candidate.refresh(horizon);
        candidate
    }

    pub const fn task_id(&self) -> TaskId {
        self.task.id
    }

    pub fn duration(&self) -> qtty::Quantity<Day> {
        self.task.duration.to::<Day>()
    }

    pub fn priority(&self, at: Time<MJD>) -> f64 {
        Self::task_score(self.task, at)
    }

    pub fn refresh(&mut self, horizon: &Period<MJD>) {
        self.est = None;
        self.deadline = None;
        self.flexibility = 0.0;
        let windows = self.windows.as_slice();
        let duration = self.duration();
        let duration_days = duration.value();

        while self.next_window_idx < windows.len()
            && windows[self.next_window_idx].end <= horizon.start
        {
            self.next_window_idx += 1;
        }

        for window in &windows[self.next_window_idx..] {
            if window.start >= horizon.end {
                break;
            }

            let Some(overlap) = window.intersection(horizon) else {
                continue;
            };

            let overlap_duration_days = overlap.duration().to::<Day>().value();
            if overlap_duration_days < duration_days {
                continue;
            }

            self.est.get_or_insert(overlap.start);
            self.deadline = Some(overlap.end - duration);
            self.flexibility += overlap_duration_days / duration_days;
        }

        log::trace!(
            "est: candidate task={} refreshed — est={}, deadline={}, flexibility={:.2}, endangered={}",
            self.task.id.0,
            self.est
                .map_or("none".to_string(), |t| format!("{:.4}", t.value())),
            self.deadline
                .map_or("none".to_string(), |t| format!("{:.4}", t.value())),
            self.flexibility,
            self.is_endangered(1),
        );
    }

    #[inline]
    fn task_score(task: &Task, at: Time<MJD>) -> f64 {
        task.soft_constraints
            .as_ref()
            .map(|expr| expr.score(&at, None, Some(&task.target)))
            .unwrap_or(0.0)
    }

    pub const fn is_endangered(&self, endangered_threshold: u32) -> bool {
        self.flexibility < endangered_threshold as f64
    }

    pub const fn is_impossible(&self) -> bool {
        self.flexibility < 1.0
    }
}

impl IntoTaskPlacement for EstCandidate<'_> {
    fn into_task_placement(self, horizon_end: Time<MJD>) -> TaskPlacement {
        let start = self
            .est
            .expect("EST invariant violated: candidate has no start time");
        let end = start + self.duration();
        assert!(
            end <= horizon_end,
            "EST invariant violated: candidate placement exceeds horizon"
        );

        TaskPlacement {
            task_id: self.task_id(),
            start: start.to::<JD>(),
            end: end.to::<JD>(),
            block_id: None,
        }
    }
}
