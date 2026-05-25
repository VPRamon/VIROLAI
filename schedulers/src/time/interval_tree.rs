//! Augmented interval tree backed by a sorted `Vec<(Period<S>, TaskId)>`.
//!
//! ## Design
//!
//! Intervals are stored sorted by start time.  A companion
//! `suffix_max_end` array (monotonically non-increasing) enables overlap
//! queries to terminate as soon as no further match is possible.
//!
//! ## Complexity
//!
//! | Operation           | Time         |
//! |---------------------|--------------|
//! | `insert`            | O(n)         |
//! | `remove`            | O(n)         |
//! | `query_overlapping` | O(log n + k) |
//!
//! The `O(log n + k)` case for queries holds whenever intervals are
//! distributed without extreme overlap depth — typical for astronomical
//! scheduling.  The suffix max-end pruning guarantees that the walk
//! terminates immediately after the last overlapping entry is collected.
//!
//! ## Integration with `tempoch`
//!
//! Interval endpoints are typed [`Period<S>`](tempoch::Period) values,
//! making the tree generic over any [`TimeScale`](tempoch::TimeScale):
//! `JD`, `MJD`, and so on.  [`Schedule`](crate::schedule::Schedule) uses
//! `IntervalTree<MJD>`.

use crate::time::{Period, TaskId, TimeScale, interval};

/// Interval tree over [`Period<S>`] → [`TaskId`] mappings.
///
/// Invariants maintained at all times:
/// - `entries` is sorted ascending by `entry.0.start.value()`.
/// - `suffix_max_end[i]` equals `max(entries[j].0.end.value() for j ≥ i)`.
/// - `suffix_max_end.len() == entries.len()`.
#[derive(Debug, Clone)]
pub struct IntervalTree<S: TimeScale> {
    /// `(interval, task_id)` pairs sorted ascending by `interval.start`.
    entries: Vec<(Period<S>, TaskId)>,

    /// `suffix_max_end[i]` = max end value of all entries from index `i`
    /// to the end of `entries`.  Monotonically non-increasing, so the
    /// overlap scan can `break` as soon as this value drops to or below
    /// the query's start endpoint.
    suffix_max_end: Vec<f64>,
}

impl<S: TimeScale> Default for IntervalTree<S> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            suffix_max_end: Vec::new(),
        }
    }
}

impl<S: TimeScale> IntervalTree<S> {
    /// Create an empty interval tree.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of intervals currently stored.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// `true` if no intervals are stored.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Insert `interval` associated with `task_id`.
    ///
    /// The insertion position is determined by `interval.start` to maintain
    /// sorted order.  The suffix max-end array is rebuilt afterwards in O(n).
    pub fn insert(&mut self, interval: Period<S>, task_id: TaskId) {
        let start_val = interval.start.value();
        let pos = self
            .entries
            .partition_point(|e| e.0.start.value() < start_val);
        self.entries.insert(pos, (interval, task_id));
        self.rebuild_suffix_max_end();
    }

    /// Remove the entry associated with `task_id`.
    ///
    /// If `task_id` is not present this is a no-op.  The suffix max-end
    /// array is rebuilt afterwards in O(n).
    pub fn remove(&mut self, task_id: TaskId) {
        if let Some(pos) = self.entries.iter().position(|e| e.1 == task_id) {
            self.entries.remove(pos);
            self.rebuild_suffix_max_end();
        }
    }

    /// Return the IDs of all tasks whose stored interval overlaps `query`.
    ///
    /// Two intervals `[a, b)` and `[c, d)` overlap iff `a < d && c < b`.
    ///
    /// ### Algorithm
    ///
    /// 1. **Binary search** (O(log n)): find the exclusive upper bound `p`
    ///    of entries whose `start < query.end`.  All entries at `p` and
    ///    beyond have `start ≥ query.end` and cannot overlap.
    /// 2. **Suffix-max-pruned walk** (O(k) on average): iterate indices
    ///    `0..p` and collect entries whose `end > query.start`.  Stop as
    ///    soon as `suffix_max_end[i] ≤ query.start`, because all subsequent
    ///    entries also have `end ≤ query.start` (the suffix-max array is
    ///    monotonically non-increasing).
    pub fn query_overlapping(&self, query: &Period<S>) -> Vec<TaskId> {
        if self.entries.is_empty() {
            return Vec::new();
        }

        let q_start = query.start.value();
        let q_end = query.end.value();

        // All entries with start ≥ q_end cannot overlap — skip them.
        let upper = self.entries.partition_point(|e| e.0.start.value() < q_end);

        let mut result = Vec::new();
        for i in 0..upper {
            // Suffix-max pruning: every entry from here onward has end ≤ q_start.
            if self.suffix_max_end[i] <= q_start {
                break;
            }
            if interval::overlaps(&self.entries[i].0, query) {
                result.push(self.entries[i].1);
            }
        }
        result
    }

    /// Rebuild `suffix_max_end` from scratch.  Called after every mutation.
    fn rebuild_suffix_max_end(&mut self) {
        let n = self.entries.len();
        self.suffix_max_end = vec![f64::NEG_INFINITY; n];
        if n == 0 {
            return;
        }
        self.suffix_max_end[n - 1] = self.entries[n - 1].0.end.value();
        for i in (0..n - 1).rev() {
            self.suffix_max_end[i] = self.entries[i]
                .0
                .end
                .value()
                .max(self.suffix_max_end[i + 1]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::{MJD, Time};

    fn p(start: f64, end: f64) -> Period<MJD> {
        Period::new(Time::<MJD>::new(start), Time::<MJD>::new(end))
    }

    fn tid(n: u64) -> TaskId {
        TaskId(n)
    }

    #[test]
    fn empty_tree_returns_no_overlaps() {
        let tree = IntervalTree::<MJD>::new();
        assert!(tree.query_overlapping(&p(0.0, 10.0)).is_empty());
    }

    #[test]
    fn single_interval_overlaps_with_itself() {
        let mut tree = IntervalTree::<MJD>::new();
        tree.insert(p(5.0, 10.0), tid(1));
        assert_eq!(tree.query_overlapping(&p(5.0, 10.0)), vec![tid(1)]);
    }

    #[test]
    fn non_overlapping_intervals() {
        let mut tree = IntervalTree::<MJD>::new();
        tree.insert(p(0.0, 5.0), tid(1));
        tree.insert(p(10.0, 15.0), tid(2));
        // [6, 9) touches neither [0,5) nor [10,15)
        assert!(tree.query_overlapping(&p(6.0, 9.0)).is_empty());
    }

    #[test]
    fn overlapping_several_intervals() {
        let mut tree = IntervalTree::<MJD>::new();
        tree.insert(p(0.0, 10.0), tid(1));
        tree.insert(p(5.0, 15.0), tid(2));
        tree.insert(p(20.0, 30.0), tid(3));

        let mut hits = tree.query_overlapping(&p(8.0, 12.0));
        hits.sort_by_key(|t| t.0);
        assert_eq!(hits, vec![tid(1), tid(2)]);
    }

    #[test]
    fn half_open_boundary_is_exclusive() {
        let mut tree = IntervalTree::<MJD>::new();
        tree.insert(p(0.0, 5.0), tid(1));
        // [5, 10) starts exactly where [0,5) ends — no overlap
        assert!(tree.query_overlapping(&p(5.0, 10.0)).is_empty());
    }

    #[test]
    fn remove_makes_interval_invisible() {
        let mut tree = IntervalTree::<MJD>::new();
        tree.insert(p(0.0, 10.0), tid(1));
        tree.remove(tid(1));
        assert!(tree.query_overlapping(&p(0.0, 10.0)).is_empty());
    }

    #[test]
    fn remove_nonexistent_is_noop() {
        let mut tree = IntervalTree::<MJD>::new();
        tree.insert(p(0.0, 10.0), tid(1));
        tree.remove(tid(99));
        assert_eq!(tree.len(), 1);
    }

    #[test]
    fn suffix_max_end_is_monotone_non_increasing() {
        let mut tree = IntervalTree::<MJD>::new();
        tree.insert(p(0.0, 8.0), tid(1));
        tree.insert(p(1.0, 6.0), tid(2));
        tree.insert(p(2.0, 10.0), tid(3));
        tree.insert(p(3.0, 4.0), tid(4));

        for i in 1..tree.suffix_max_end.len() {
            assert!(
                tree.suffix_max_end[i - 1] >= tree.suffix_max_end[i],
                "suffix_max_end must be non-increasing"
            );
        }
    }

    #[test]
    fn insert_maintains_sorted_order() {
        let mut tree = IntervalTree::<MJD>::new();
        tree.insert(p(10.0, 20.0), tid(3));
        tree.insert(p(0.0, 5.0), tid(1));
        tree.insert(p(5.0, 15.0), tid(2));

        let starts: Vec<f64> = tree.entries.iter().map(|e| e.0.start.value()).collect();
        assert_eq!(starts, vec![0.0, 5.0, 10.0]);
    }

    #[test]
    fn len_and_is_empty_track_mutations() {
        let mut tree = IntervalTree::<MJD>::new();
        assert!(tree.is_empty());
        assert_eq!(tree.len(), 0);

        tree.insert(p(0.0, 1.0), tid(1));
        assert!(!tree.is_empty());
        assert_eq!(tree.len(), 1);

        tree.remove(tid(1));
        assert!(tree.is_empty());
        assert_eq!(tree.len(), 0);
    }

    #[test]
    fn query_with_end_before_first_start_returns_empty() {
        let mut tree = IntervalTree::<MJD>::new();
        tree.insert(p(10.0, 20.0), tid(1));
        tree.insert(p(30.0, 40.0), tid(2));

        let hits = tree.query_overlapping(&p(0.0, 5.0));
        assert!(hits.is_empty());
    }
}
