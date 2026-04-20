use super::queue::CandidateQueue;
use crate::schedule::Schedule;
use crate::time::{Time, MJD};

pub struct ScheduleState<'a> {
    pub cursor: Time<MJD>,
    pub schedule: Schedule,
    pub candidates: CandidateQueue<'a>,
}