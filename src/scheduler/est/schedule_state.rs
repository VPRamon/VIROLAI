use super::queue::CandidateQueue;
use crate::schedule::Schedule;
use crate::time::{MJD, Time};

pub struct ScheduleState<'a> {
    pub(super) cursor: Time<MJD>,
    pub(super) schedule: Schedule,
    pub(super) candidates: CandidateQueue<'a>,
}
