//! Dependency graph between tasks.
//!
//! A [`SchedulingBlock`] groups related [`Task`]s and captures ordering
//! constraints between them as a directed acyclic graph (DAG).
//! Nodes are [`TaskId`]s; edges represent the [`Dependency::DependsOn`]
//! relation ("A depends on B" means B must be placed before A).

pub mod task;

use crate::error::ScheduleError;
use self::task::Task;
use petgraph::algo::toposort;
use petgraph::stable_graph::StableDiGraph;
use std::collections::HashMap;

/// The only dependency kind in v1: strict ordering.
///
/// "A `DependsOn` B" is read as "task A must be scheduled *after* task B".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dependency {
    DependsOn,
}

/// A directed acyclic graph of [`Task`](crate::task::Task) IDs.
///
/// Helper methods keep the internal `petgraph` DAG and the reverse-lookup
/// map consistent.
pub type SchedulingBlock = StableDiGraph<Task, Dependency>;
