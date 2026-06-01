//! Candidate actions proposed by cursors during a beam round.

use super::config::CursorId;

/// Rank of a candidate action within its originating cursor's queue.
///
/// Lower ranks are better (rank 0 is the cursor's most-preferred candidate).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ActionRank(pub usize);

/// A single placement opportunity: "cursor `cursor_id` could place its
/// `candidate_idx`-th schedulable candidate".
#[derive(Debug, Clone, Copy)]
pub(super) struct CursorAction {
    /// Index of the cursor within the state's cursor list.
    pub(super) cursor_pos: usize,
    /// Stable id of that cursor (used for deterministic tie-breaking).
    pub(super) cursor_id: CursorId,
    /// Position of the chosen candidate inside the cursor's queue.
    pub(super) candidate_idx: usize,
    /// Within-cursor rank of the candidate.
    pub(super) rank: ActionRank,
}
