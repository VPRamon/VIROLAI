use super::queue::CandidateQueue;
use crate::schedule::Schedule;
use crate::time::{MJD, Time};

/// One live EST beam state.
///
/// A state consists of the already-built schedule, the beam cursor
/// immediately after the latest placement, and the candidate queue derived
/// from that cursor.
#[derive(Clone)]
pub struct ScheduleState<'a> {
    /// Current beam cursor: new placements must start no earlier than this.
    pub(super) cursor: Time<MJD>,
    /// Placements chosen so far for this beam.
    pub(super) schedule: Schedule,
    /// Candidate queue refreshed relative to `cursor`.
    pub(super) candidates: CandidateQueue<'a>,
}
