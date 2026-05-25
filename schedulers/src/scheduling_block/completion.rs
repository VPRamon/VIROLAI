//! Block completion expression: which subsets of tasks satisfy a block.
//!
//! [`CompletionExpr`] is a small AST over [`TaskId`]s with `And`, `Or`, and
//! `Leaf` nodes. It is *orthogonal* to the [`SchedulingBlock`] dependency
//! DAG: dependencies constrain ordering inside an AND-branch, while the
//! completion expression decides which AND-branches actually count as a
//! completed block.
//!
//! For CRU's outer Block Scheduling Cycle the expression is rewritten to
//! disjunctive normal form via [`CompletionExpr::dnf_branches`] so the
//! algorithm can iterate over each completion alternative independently.
//!
//! [`SchedulingBlock`]: super::SchedulingBlock

use crate::schedule::Schedule;
use crate::time::TaskId;
use std::collections::HashSet;

/// Maximum number of DNF branches accepted by [`CompletionExpr::dnf_branches`].
///
/// Pathological OR-trees can produce exponential blow-up; this guard keeps
/// CRU bounded. Real workloads stay well below this limit.
pub const MAX_DNF_BRANCHES: usize = 1024;

/// Boolean expression over [`TaskId`]s describing valid completions of a block.
///
/// The leaf semantics is "this task is scheduled". The expression evaluates
/// against a [`Schedule`] via [`CompletionExpr::is_satisfied_by`]; CRU uses
/// [`CompletionExpr::dnf_branches`] to enumerate the disjoint completion
/// alternatives it must try.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionExpr {
    /// "task `id` is scheduled".
    Leaf(TaskId),
    /// All children must be satisfied. Empty `And` is trivially satisfied.
    And(Vec<CompletionExpr>),
    /// At least one child must be satisfied. Empty `Or` is unsatisfiable.
    Or(Vec<CompletionExpr>),
}

impl CompletionExpr {
    /// Build the trivial expression "every listed task is scheduled".
    pub fn all_of<I: IntoIterator<Item = TaskId>>(ids: I) -> Self {
        CompletionExpr::And(ids.into_iter().map(CompletionExpr::Leaf).collect())
    }

    /// Build "at least one of the listed tasks is scheduled".
    pub fn any_of<I: IntoIterator<Item = TaskId>>(ids: I) -> Self {
        CompletionExpr::Or(ids.into_iter().map(CompletionExpr::Leaf).collect())
    }

    /// Recursively check whether `schedule` satisfies the expression.
    pub fn is_satisfied_by(&self, schedule: &Schedule) -> bool {
        match self {
            CompletionExpr::Leaf(id) => schedule.contains(*id),
            CompletionExpr::And(children) => children.iter().all(|c| c.is_satisfied_by(schedule)),
            CompletionExpr::Or(children) => children.iter().any(|c| c.is_satisfied_by(schedule)),
        }
    }

    /// Recursively check that every leaf of `branch` is placed in `schedule`.
    pub fn branch_satisfied(branch: &[TaskId], schedule: &Schedule) -> bool {
        branch.iter().all(|id| schedule.contains(*id))
    }

    /// Collect every [`TaskId`] referenced in the expression (deduplicated).
    pub fn referenced_tasks(&self) -> HashSet<TaskId> {
        let mut out = HashSet::new();
        self.collect_into(&mut out);
        out
    }

    fn collect_into(&self, out: &mut HashSet<TaskId>) {
        match self {
            CompletionExpr::Leaf(id) => {
                out.insert(*id);
            }
            CompletionExpr::And(children) | CompletionExpr::Or(children) => {
                for c in children {
                    c.collect_into(out);
                }
            }
        }
    }

    /// Rewrite the expression in disjunctive normal form and return one
    /// branch (a flat list of `TaskId`s that must all be scheduled together)
    /// per disjunct.
    ///
    /// Each returned inner `Vec` is sorted and deduplicated for stable
    /// downstream behavior, and the outer list is deduplicated by branch
    /// content.
    ///
    /// Returns an empty vector when the expression is unsatisfiable
    /// (an empty `Or`). Returns at most [`MAX_DNF_BRANCHES`] branches; if
    /// the expansion would exceed that, returns `None`.
    pub fn dnf_branches(&self) -> Option<Vec<Vec<TaskId>>> {
        let raw = self.dnf_collect()?;
        let mut seen = HashSet::new();
        let mut out = Vec::with_capacity(raw.len());
        for branch in raw {
            let mut sorted: Vec<TaskId> = branch.into_iter().collect();
            sorted.sort_by_key(|t| t.0);
            sorted.dedup();
            if seen.insert(sorted.clone()) {
                out.push(sorted);
            }
        }
        Some(out)
    }

    fn dnf_collect(&self) -> Option<Vec<HashSet<TaskId>>> {
        match self {
            CompletionExpr::Leaf(id) => {
                let mut s = HashSet::new();
                s.insert(*id);
                Some(vec![s])
            }
            CompletionExpr::Or(children) => {
                let mut out: Vec<HashSet<TaskId>> = Vec::new();
                for child in children {
                    let child_branches = child.dnf_collect()?;
                    for b in child_branches {
                        if out.len() >= MAX_DNF_BRANCHES {
                            return None;
                        }
                        out.push(b);
                    }
                }
                Some(out)
            }
            CompletionExpr::And(children) => {
                let mut acc: Vec<HashSet<TaskId>> = vec![HashSet::new()];
                for child in children {
                    let child_branches = child.dnf_collect()?;
                    if child_branches.is_empty() {
                        // child is unsatisfiable -> AND collapses
                        return Some(Vec::new());
                    }
                    let mut next: Vec<HashSet<TaskId>> = Vec::new();
                    for a in &acc {
                        for b in &child_branches {
                            if next.len() >= MAX_DNF_BRANCHES {
                                return None;
                            }
                            let mut merged = a.clone();
                            merged.extend(b.iter().copied());
                            next.push(merged);
                        }
                    }
                    acc = next;
                }
                Some(acc)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schedule::{Schedule, TaskPlacement};
    use crate::time::{MJD, Time};

    fn t(id: u64) -> TaskId {
        TaskId(id)
    }

    fn schedule_with(ids: &[u64]) -> Schedule {
        let mut s = Schedule::new();
        for (i, id) in ids.iter().enumerate() {
            s.insert_placement(TaskPlacement {
                task_id: TaskId(*id),
                start: Time::<MJD>::new(i as f64),
                end: Time::<MJD>::new((i + 1) as f64),
            });
        }
        s
    }

    #[test]
    fn leaf_branches() {
        let expr = CompletionExpr::Leaf(t(1));
        assert_eq!(expr.dnf_branches().unwrap(), vec![vec![t(1)]]);
    }

    #[test]
    fn and_collapses_to_single_branch() {
        let expr = CompletionExpr::all_of([t(1), t(2)]);
        assert_eq!(expr.dnf_branches().unwrap(), vec![vec![t(1), t(2)]]);
    }

    #[test]
    fn or_yields_one_branch_per_disjunct() {
        let expr = CompletionExpr::any_of([t(1), t(2), t(3)]);
        let branches = expr.dnf_branches().unwrap();
        assert_eq!(branches, vec![vec![t(1)], vec![t(2)], vec![t(3)]]);
    }

    #[test]
    fn nested_and_or_distributes() {
        // (t1 AND t2) OR t3
        let expr = CompletionExpr::Or(vec![
            CompletionExpr::all_of([t(1), t(2)]),
            CompletionExpr::Leaf(t(3)),
        ]);
        let branches = expr.dnf_branches().unwrap();
        assert_eq!(branches, vec![vec![t(1), t(2)], vec![t(3)]]);
    }

    #[test]
    fn distributive_and_over_or() {
        // t1 AND (t2 OR t3) -> [{t1,t2}, {t1,t3}]
        let expr = CompletionExpr::And(vec![
            CompletionExpr::Leaf(t(1)),
            CompletionExpr::any_of([t(2), t(3)]),
        ]);
        let branches = expr.dnf_branches().unwrap();
        assert_eq!(branches, vec![vec![t(1), t(2)], vec![t(1), t(3)]]);
    }

    #[test]
    fn empty_or_is_unsatisfiable() {
        let expr = CompletionExpr::Or(vec![]);
        assert!(expr.dnf_branches().unwrap().is_empty());
    }

    #[test]
    fn empty_and_is_trivially_satisfied() {
        let expr = CompletionExpr::And(vec![]);
        let branches = expr.dnf_branches().unwrap();
        assert_eq!(branches, vec![Vec::<TaskId>::new()]);
    }

    #[test]
    fn duplicate_branches_are_collapsed() {
        // (t1) OR (t1) -> single branch
        let expr = CompletionExpr::Or(vec![CompletionExpr::Leaf(t(1)), CompletionExpr::Leaf(t(1))]);
        assert_eq!(expr.dnf_branches().unwrap(), vec![vec![t(1)]]);
    }

    #[test]
    fn is_satisfied_by_evaluates_recursively() {
        let expr = CompletionExpr::Or(vec![
            CompletionExpr::all_of([t(1), t(2)]),
            CompletionExpr::Leaf(t(3)),
        ]);
        assert!(!expr.is_satisfied_by(&schedule_with(&[])));
        assert!(!expr.is_satisfied_by(&schedule_with(&[1])));
        assert!(expr.is_satisfied_by(&schedule_with(&[1, 2])));
        assert!(expr.is_satisfied_by(&schedule_with(&[3])));
        assert!(expr.is_satisfied_by(&schedule_with(&[1, 3])));
    }

    #[test]
    fn dnf_branches_caps_blow_up() {
        // 11 ORs of 2 leaves each ANDed together would be 2^11 = 2048 branches,
        // exceeding MAX_DNF_BRANCHES (1024).
        let mut children = Vec::new();
        for i in 0..11 {
            children.push(CompletionExpr::any_of([TaskId(2 * i), TaskId(2 * i + 1)]));
        }
        let expr = CompletionExpr::And(children);
        assert!(expr.dnf_branches().is_none());
    }

    #[test]
    fn referenced_tasks_collects_deduped() {
        let expr = CompletionExpr::Or(vec![
            CompletionExpr::all_of([t(1), t(2)]),
            CompletionExpr::Leaf(t(2)),
        ]);
        let ids: Vec<u64> = {
            let mut v: Vec<TaskId> = expr.referenced_tasks().into_iter().collect();
            v.sort_by_key(|t| t.0);
            v.into_iter().map(|t| t.0).collect()
        };
        assert_eq!(ids, vec![1, 2]);
    }
}
