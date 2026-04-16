//! Serde support for [`SchedulingBlock`](super::SchedulingBlock).

use super::{Dependency, SchedulingBlock};
use crate::time::{SchedulingBlockId, TaskId};
use ::serde::{Deserialize, Deserializer, Serialize, Serializer};
use petgraph::visit::{EdgeRef, IntoEdgeReferences};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SchedulingBlockRepr {
    id: u64,
    tasks: Vec<TaskRepr>,
    dependencies: Vec<DependencyEdgeRepr>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum TaskRepr {
    Id(u64),
    Object(TaskObjectRepr),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TargetRepr {
    ra_deg: f64,
    dec_deg: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct TimeWindowRepr {
    start_mjd_utc: f64,
    end_mjd_utc: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct HardConstraintsRepr {
    #[serde(skip_serializing_if = "Option::is_none")]
    altitude_min_deg: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    altitude_max_deg: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    azimuth_min_deg: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    azimuth_max_deg: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    time_window: Option<TimeWindowRepr>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SoftConstraintsRepr {
    #[serde(skip_serializing_if = "Option::is_none")]
    priority: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TaskObjectRepr {
    id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    requested_duration_sec: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    target: Option<TargetRepr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hard_constraints: Option<HardConstraintsRepr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    soft_constraints: Option<SoftConstraintsRepr>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DependencyTag {
    DependsOn,
}

impl From<Dependency> for DependencyTag {
    fn from(value: Dependency) -> Self {
        match value {
            Dependency::DependsOn => DependencyTag::DependsOn,
        }
    }
}

impl From<DependencyTag> for Dependency {
    fn from(value: DependencyTag) -> Self {
        match value {
            DependencyTag::DependsOn => Dependency::DependsOn,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DependencyEdgeRepr {
    from: u64,
    to: u64,
    #[serde(default = "default_dependency_tag")]
    dependency: DependencyTag,
}

const fn default_dependency_tag() -> DependencyTag {
    DependencyTag::DependsOn
}

fn to_repr(block: &SchedulingBlock) -> SchedulingBlockRepr {
    let tasks = block
        .graph
        .node_weights()
        .map(|task_id| TaskRepr::Id(task_id.0))
        .collect::<Vec<_>>();

    let dependencies = block
        .graph
        .edge_references()
        .map(|edge| DependencyEdgeRepr {
            from: block.graph[edge.source()].0,
            to: block.graph[edge.target()].0,
            dependency: (*edge.weight()).into(),
        })
        .collect::<Vec<_>>();

    SchedulingBlockRepr {
        id: block.id.0,
        tasks,
        dependencies,
    }
}

fn from_repr(repr: SchedulingBlockRepr) -> Result<SchedulingBlock, String> {
    let mut block = SchedulingBlock::new(SchedulingBlockId(repr.id));
    let mut seen = HashSet::new();

    for task in repr.tasks {
        let task_id = match task {
            TaskRepr::Id(task_id) => task_id,
            TaskRepr::Object(task) => task.id,
        };

        if !seen.insert(task_id) {
            return Err(format!(
                "duplicate task id {} in scheduling block payload",
                task_id
            ));
        }
        block.add_task(TaskId(task_id));
    }

    for dep in repr.dependencies {
        if !seen.contains(&dep.from) || !seen.contains(&dep.to) {
            return Err(format!(
                "dependency references unknown task id {} -> {}",
                dep.from, dep.to
            ));
        }

        block
            .add_dependency(TaskId(dep.from), TaskId(dep.to), dep.dependency.into())
            .map_err(|err| {
                format!(
                    "invalid scheduling block dependency {} -> {}: {}",
                    dep.from, dep.to, err
                )
            })?;
    }

    Ok(block)
}

impl Serialize for SchedulingBlock {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        to_repr(self).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SchedulingBlock {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let repr = SchedulingBlockRepr::deserialize(deserializer)?;
        from_repr(repr).map_err(::serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_roundtrip_preserves_block_constraints() {
        let mut block = SchedulingBlock::new(SchedulingBlockId(42));
        block.add_task(TaskId(10));
        block.add_task(TaskId(20));
        block.add_task(TaskId(30));
        block
            .add_dependency(TaskId(20), TaskId(10), Dependency::DependsOn)
            .unwrap();
        block
            .add_dependency(TaskId(30), TaskId(20), Dependency::DependsOn)
            .unwrap();

        let value = serde_json::to_value(&block).unwrap();
        assert_eq!(value["id"], serde_json::json!(42));
        assert_eq!(value["tasks"], serde_json::json!([10, 20, 30]));
        assert_eq!(value["dependencies"].as_array().unwrap().len(), 2);

        let decoded: SchedulingBlock = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(decoded.id, SchedulingBlockId(42));

        let roundtrip = serde_json::to_value(&decoded).unwrap();
        assert_eq!(roundtrip, value);
    }

    #[test]
    fn deserialize_rejects_dependency_cycle() {
        let payload = serde_json::json!({
            "id": 1,
            "tasks": [1, 2],
            "dependencies": [
                {"from": 1, "to": 2, "dependency": "depends_on"},
                {"from": 2, "to": 1, "dependency": "depends_on"}
            ]
        });

        let err = serde_json::from_value::<SchedulingBlock>(payload)
            .unwrap_err()
            .to_string();

        assert!(err.contains("dependency graph contains a cycle"));
    }

    #[test]
    fn deserialize_accepts_task_objects() {
        let payload = serde_json::json!({
            "id": 5,
            "tasks": [
                {
                    "id": 10,
                    "name": "task-10",
                    "requested_duration_sec": 1200,
                    "hard_constraints": {},
                    "soft_constraints": {"priority": 7.5}
                },
                {
                    "id": 20,
                    "name": "task-20",
                    "requested_duration_sec": 1800,
                    "hard_constraints": {},
                    "soft_constraints": {"priority": 3.25}
                }
            ],
            "dependencies": [
                {"from": 20, "to": 10}
            ]
        });

        let block: SchedulingBlock = serde_json::from_value(payload).unwrap();
        assert_eq!(block.id, SchedulingBlockId(5));
        assert!(block.contains_task(TaskId(10)));
        assert!(block.contains_task(TaskId(20)));
    }
}
