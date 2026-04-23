//! Dependency graph between tasks.
//!
//! A [`SchedulingBlock`] groups related tasks and captures ordering
//! constraints between them as a directed acyclic graph (DAG).
//! Nodes carry [`TaskId`]s; edges carry [`Dependency`] labels that
//! express ordering as predecessor -> successor.

pub mod serde;
pub mod task;

use crate::error::ScheduleError;
use crate::time::{SchedulingBlockId, TaskId};
use petgraph::algo::toposort;
use petgraph::stable_graph::{NodeIndex, StableDiGraph};
use rayon::prelude::*;
use std::collections::HashMap;

/// The only dependency kind: strict ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dependency {
    DependsOn,
}

/// A directed acyclic graph (DAG) of [`TaskId`]s with ordering constraints.
///
/// [`SchedulingBlock`] groups related tasks and records which must be
/// scheduled before others.  The internal graph is a
/// [`StableDiGraph`](petgraph::stable_graph::StableDiGraph) whose nodes
/// carry [`TaskId`]s and whose edges carry [`Dependency`] labels.
#[derive(Debug)]
pub struct SchedulingBlock {
    /// Public identifier used as the map key in [`Schedule`](crate::schedule::Schedule).
    pub id: SchedulingBlockId,
    graph: StableDiGraph<TaskId, Dependency>,
    /// Fast `TaskId` -> graph node index lookup.
    node_map: HashMap<TaskId, NodeIndex>,
}

impl SchedulingBlock {
    /// Create a new, empty scheduling block.
    pub fn new(id: SchedulingBlockId) -> Self {
        Self {
            id,
            graph: StableDiGraph::new(),
            node_map: HashMap::new(),
        }
    }

    /// Register `task_id` as a node.  Idempotent: a second call with the
    /// same id is a no-op.
    pub fn add_task(&mut self, task_id: TaskId) {
        if !self.node_map.contains_key(&task_id) {
            let idx = self.graph.add_node(task_id);
            self.node_map.insert(task_id, idx);
        }
    }

    /// Add an ordering edge `from -> to`.
    ///
    /// The `from` task is the predecessor and must be scheduled before `to`.
    ///
    /// Both tasks are auto-registered if not already present.
    ///
    /// Returns `Err(DependencyCycle)` if the edge would introduce a cycle.
    pub fn add_dependency(
        &mut self,
        from: TaskId,
        to: TaskId,
        dep: Dependency,
    ) -> Result<(), ScheduleError> {
        self.add_task(from);
        self.add_task(to);
        let from_idx = self.node_map[&from];
        let to_idx = self.node_map[&to];
        self.graph.add_edge(from_idx, to_idx, dep);
        toposort(&self.graph, None)
            .map(|_| ())
            .map_err(|_| ScheduleError::DependencyCycle)
    }

    /// Returns `true` if `task_id` is a node in this block.
    pub fn contains_task(&self, task_id: TaskId) -> bool {
        self.node_map.contains_key(&task_id)
    }

    /// Iterate task IDs stored in this block.
    ///
    /// The iteration order follows the graph's internal node storage order.
    pub fn iter(&self) -> std::vec::IntoIter<TaskId> {
        self.graph
            .node_weights()
            .copied()
            .collect::<Vec<_>>()
            .into_iter()
    }

    /// Iterate task IDs in parallel.
    ///
    /// The produced items are the same as [`Self::iter`], but processed via
    /// Rayon parallel iteration.
    pub fn par_iter(&self) -> rayon::vec::IntoIter<TaskId> {
        self.graph
            .node_weights()
            .copied()
            .collect::<Vec<_>>()
            .into_par_iter()
    }

    /// Direct predecessor task IDs of `task_id` (tasks that must be
    /// scheduled before it).
    ///
    /// Returns an empty `Vec` if `task_id` is not a node in this block.
    pub fn predecessors(&self, task_id: TaskId) -> Vec<TaskId> {
        let Some(&node) = self.node_map.get(&task_id) else {
            return vec![];
        };
        self.graph
            .neighbors_directed(node, petgraph::Direction::Incoming)
            .map(|n| self.graph[n])
            .collect()
    }

    /// Direct successor task IDs of `task_id` (tasks that must be scheduled
    /// after it).
    ///
    /// Returns an empty `Vec` if `task_id` is not a node in this block.
    pub fn successors(&self, task_id: TaskId) -> Vec<TaskId> {
        let Some(&node) = self.node_map.get(&task_id) else {
            return vec![];
        };
        self.graph
            .neighbors_directed(node, petgraph::Direction::Outgoing)
            .map(|n| self.graph[n])
            .collect()
    }

    /// All transitive descendant task IDs (BFS over successors).
    ///
    /// Returns an empty set if `task_id` is not a node in this block.
    pub fn all_descendants(&self, task_id: TaskId) -> std::collections::HashSet<TaskId> {
        let mut visited = std::collections::HashSet::new();
        let mut queue = std::collections::VecDeque::new();
        for s in self.successors(task_id) {
            queue.push_back(s);
        }
        while let Some(current) = queue.pop_front() {
            if visited.insert(current) {
                for s in self.successors(current) {
                    queue.push_back(s);
                }
            }
        }
        visited
    }

    /// Return task IDs in topological order (predecessors first).
    ///
    /// Returns `Err(DependencyCycle)` if the graph contains a cycle.
    pub fn topological_order(&self) -> Result<Vec<TaskId>, ScheduleError> {
        toposort(&self.graph, None)
            .map(|indices| indices.into_iter().map(|idx| self.graph[idx]).collect())
            .map_err(|_| ScheduleError::DependencyCycle)
    }
}

impl IntoIterator for SchedulingBlock {
    type Item = TaskId;
    type IntoIter = std::vec::IntoIter<TaskId>;

    fn into_iter(self) -> Self::IntoIter {
        self.graph
            .node_weights()
            .copied()
            .collect::<Vec<_>>()
            .into_iter()
    }
}

impl IntoIterator for &SchedulingBlock {
    type Item = TaskId;
    type IntoIter = std::vec::IntoIter<TaskId>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl IntoParallelIterator for SchedulingBlock {
    type Item = TaskId;
    type Iter = rayon::vec::IntoIter<TaskId>;

    fn into_par_iter(self) -> Self::Iter {
        self.graph
            .node_weights()
            .copied()
            .collect::<Vec<_>>()
            .into_par_iter()
    }
}

impl IntoParallelIterator for &SchedulingBlock {
    type Item = TaskId;
    type Iter = rayon::vec::IntoIter<TaskId>;

    fn into_par_iter(self) -> Self::Iter {
        self.par_iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn sample_block() -> SchedulingBlock {
        let mut block = SchedulingBlock::new(SchedulingBlockId(1));
        block.add_task(TaskId(10));
        block.add_task(TaskId(20));
        block.add_task(TaskId(30));
        block
    }

    #[test]
    fn iter_returns_all_task_ids() {
        let block = sample_block();
        let ids: HashSet<_> = block.iter().collect();
        let expected: HashSet<_> = [TaskId(10), TaskId(20), TaskId(30)].into_iter().collect();
        assert_eq!(ids, expected);
    }

    #[test]
    fn into_iter_ref_matches_iter() {
        let block = sample_block();
        let from_method: Vec<_> = block.iter().collect();
        let from_trait: Vec<_> = (&block).into_iter().collect();
        assert_eq!(from_method, from_trait);
    }

    #[test]
    fn par_iter_returns_all_task_ids() {
        let block = sample_block();
        let ids: HashSet<_> = block.par_iter().collect();
        let expected: HashSet<_> = [TaskId(10), TaskId(20), TaskId(30)].into_iter().collect();
        assert_eq!(ids, expected);
    }

    #[test]
    fn into_par_iter_ref_returns_all_task_ids() {
        let block = sample_block();
        let ids: HashSet<_> = (&block).into_par_iter().collect();
        let expected: HashSet<_> = [TaskId(10), TaskId(20), TaskId(30)].into_iter().collect();
        assert_eq!(ids, expected);
    }

    /// Build a chain: 10 → 20 → 30
    fn chain_block() -> SchedulingBlock {
        let mut block = SchedulingBlock::new(SchedulingBlockId(2));
        block
            .add_dependency(TaskId(10), TaskId(20), Dependency::DependsOn)
            .unwrap();
        block
            .add_dependency(TaskId(20), TaskId(30), Dependency::DependsOn)
            .unwrap();
        block
    }

    #[test]
    fn predecessors_returns_direct_predecessors() {
        let block = chain_block();
        let preds_20: HashSet<_> = block.predecessors(TaskId(20)).into_iter().collect();
        assert_eq!(preds_20, HashSet::from([TaskId(10)]));

        let preds_10: Vec<_> = block.predecessors(TaskId(10));
        assert!(preds_10.is_empty());
    }

    #[test]
    fn successors_returns_direct_successors() {
        let block = chain_block();
        let succs_20: HashSet<_> = block.successors(TaskId(20)).into_iter().collect();
        assert_eq!(succs_20, HashSet::from([TaskId(30)]));

        let succs_30: Vec<_> = block.successors(TaskId(30));
        assert!(succs_30.is_empty());
    }

    #[test]
    fn all_descendants_is_transitive() {
        let block = chain_block();
        let desc_10 = block.all_descendants(TaskId(10));
        assert_eq!(desc_10, HashSet::from([TaskId(20), TaskId(30)]));

        let desc_20 = block.all_descendants(TaskId(20));
        assert_eq!(desc_20, HashSet::from([TaskId(30)]));

        let desc_30 = block.all_descendants(TaskId(30));
        assert!(desc_30.is_empty());
    }

    #[test]
    fn predecessors_unknown_task_returns_empty() {
        let block = chain_block();
        assert!(block.predecessors(TaskId(999)).is_empty());
    }

    #[test]
    fn all_descendants_unknown_task_returns_empty() {
        let block = chain_block();
        assert!(block.all_descendants(TaskId(999)).is_empty());
    }
}
