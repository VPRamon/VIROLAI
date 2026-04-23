//! Dependency graph between tasks.
//!
//! A [`SchedulingBlock`] groups owned tasks and captures ordering constraints
//! between them as a directed acyclic graph (DAG). Nodes carry [`TaskId`]s;
//! edges carry [`Dependency`] labels that express ordering as predecessor ->
//! successor.

pub mod serde;
pub mod task;

use crate::error::ScheduleError;
use crate::task::Task;
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

/// A directed acyclic graph (DAG) of owned [`Task`]s with ordering constraints.
#[derive(Debug)]
pub struct SchedulingBlock {
    /// Stable public identifier for this block.
    pub id: SchedulingBlockId,
    tasks: Vec<Task>,
    task_index: HashMap<TaskId, usize>,
    graph: StableDiGraph<TaskId, Dependency>,
    /// Fast `TaskId` -> graph node index lookup.
    node_map: HashMap<TaskId, NodeIndex>,
}

impl SchedulingBlock {
    /// Create a new, empty scheduling block.
    pub fn new(id: SchedulingBlockId) -> Self {
        Self {
            id,
            tasks: Vec::new(),
            task_index: HashMap::new(),
            graph: StableDiGraph::new(),
            node_map: HashMap::new(),
        }
    }

    /// Create a block populated with the provided tasks in input order.
    pub fn from_tasks(id: SchedulingBlockId, tasks: Vec<Task>) -> Result<Self, ScheduleError> {
        let mut block = Self::new(id);
        for task in tasks {
            block.push_task(task)?;
        }
        Ok(block)
    }

    /// Append one owned task to the block.
    pub fn push_task(&mut self, task: Task) -> Result<(), ScheduleError> {
        if self.task_index.contains_key(&task.id) {
            return Err(ScheduleError::InvalidTask(format!(
                "duplicate task id {} in scheduling block {}",
                task.id.0, self.id.0
            )));
        }

        let task_id = task.id;
        let task_pos = self.tasks.len();
        let node = self.graph.add_node(task_id);
        self.tasks.push(task);
        self.task_index.insert(task_id, task_pos);
        self.node_map.insert(task_id, node);
        Ok(())
    }

    /// Borrow the owned tasks in their input order.
    pub fn tasks(&self) -> &[Task] {
        &self.tasks
    }

    /// Iterate tasks in their input order.
    pub fn iter_tasks(&self) -> impl Iterator<Item = &Task> {
        self.tasks.iter()
    }

    /// Iterate task IDs in their input order.
    pub fn iter(&self) -> impl Iterator<Item = TaskId> + '_ {
        self.tasks.iter().map(|task| task.id)
    }

    /// Iterate task IDs in parallel.
    pub fn par_iter(&self) -> impl ParallelIterator<Item = TaskId> + '_ {
        self.tasks.par_iter().map(|task| task.id)
    }

    /// Look up an owned task by stable identifier.
    pub fn task(&self, task_id: TaskId) -> Option<&Task> {
        self.task_index.get(&task_id).map(|&idx| &self.tasks[idx])
    }

    /// Returns `true` if `task_id` is a node in this block.
    pub fn contains_task(&self, task_id: TaskId) -> bool {
        self.task_index.contains_key(&task_id)
    }

    /// Add an ordering edge `from -> to`.
    ///
    /// The `from` task is the predecessor and must be scheduled before `to`.
    /// Both tasks must already belong to this block.
    pub fn add_dependency(
        &mut self,
        from: TaskId,
        to: TaskId,
        dep: Dependency,
    ) -> Result<(), ScheduleError> {
        let Some(&from_idx) = self.node_map.get(&from) else {
            return Err(ScheduleError::InvalidTask(format!(
                "block {} dependency references unknown task {}",
                self.id.0, from.0
            )));
        };
        let Some(&to_idx) = self.node_map.get(&to) else {
            return Err(ScheduleError::InvalidTask(format!(
                "block {} dependency references unknown task {}",
                self.id.0, to.0
            )));
        };

        self.graph.add_edge(from_idx, to_idx, dep);
        toposort(&self.graph, None)
            .map(|_| ())
            .map_err(|_| ScheduleError::DependencyCycle)
    }

    /// Direct predecessor task IDs of `task_id` (tasks that must be scheduled
    /// before it).
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
    pub fn all_descendants(&self, task_id: TaskId) -> std::collections::HashSet<TaskId> {
        let mut visited = std::collections::HashSet::new();
        let mut queue = std::collections::VecDeque::new();
        for successor in self.successors(task_id) {
            queue.push_back(successor);
        }
        while let Some(current) = queue.pop_front() {
            if visited.insert(current) {
                for successor in self.successors(current) {
                    queue.push_back(successor);
                }
            }
        }
        visited
    }

    /// Return task IDs in topological order (predecessors first).
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
        self.tasks
            .into_iter()
            .map(|task| task.id)
            .collect::<Vec<_>>()
            .into_iter()
    }
}

impl IntoIterator for &SchedulingBlock {
    type Item = TaskId;
    type IntoIter = std::vec::IntoIter<TaskId>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter().collect::<Vec<_>>().into_iter()
    }
}

impl IntoParallelIterator for SchedulingBlock {
    type Item = TaskId;
    type Iter = rayon::vec::IntoIter<TaskId>;

    fn into_par_iter(self) -> Self::Iter {
        self.tasks
            .into_iter()
            .map(|task| task.id)
            .collect::<Vec<_>>()
            .into_par_iter()
    }
}

impl IntoParallelIterator for &SchedulingBlock {
    type Item = TaskId;
    type Iter = rayon::vec::IntoIter<TaskId>;

    fn into_par_iter(self) -> Self::Iter {
        self.iter().collect::<Vec<_>>().into_par_iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constraints::ConstraintExpr;
    use qtty::{Degrees, Seconds};
    use siderust::coordinates::frames::ICRS;
    use siderust::coordinates::spherical::Direction;
    use std::collections::HashSet;

    fn make_task(id: u64) -> Task {
        Task::new(
            TaskId(id),
            format!("task-{id}"),
            Direction::<ICRS>::new_raw(Degrees::new(10.0), Degrees::new(20.0)),
            Seconds::new(600.0),
            ConstraintExpr::Intersection(vec![]),
            None,
        )
        .unwrap()
    }

    fn sample_block() -> SchedulingBlock {
        SchedulingBlock::from_tasks(
            SchedulingBlockId(1),
            vec![make_task(10), make_task(20), make_task(30)],
        )
        .unwrap()
    }

    #[test]
    fn iter_returns_all_task_ids() {
        let block = sample_block();
        let ids: HashSet<_> = block.iter().collect();
        let expected: HashSet<_> = [TaskId(10), TaskId(20), TaskId(30)].into_iter().collect();
        assert_eq!(ids, expected);
    }

    #[test]
    fn iter_tasks_preserves_input_order() {
        let block = sample_block();
        let ids: Vec<_> = block.iter_tasks().map(|task| task.id).collect();
        assert_eq!(ids, vec![TaskId(10), TaskId(20), TaskId(30)]);
    }

    #[test]
    fn task_lookup_returns_owned_task() {
        let block = sample_block();
        assert_eq!(
            block.task(TaskId(20)).map(|task| task.name.as_str()),
            Some("task-20")
        );
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

    fn chain_block() -> SchedulingBlock {
        let mut block = SchedulingBlock::from_tasks(
            SchedulingBlockId(2),
            vec![make_task(10), make_task(20), make_task(30)],
        )
        .unwrap();
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
    fn add_dependency_rejects_unknown_task() {
        let mut block = sample_block();
        let err = block
            .add_dependency(TaskId(10), TaskId(999), Dependency::DependsOn)
            .unwrap_err();
        assert!(err.to_string().contains("unknown task"));
    }
}
