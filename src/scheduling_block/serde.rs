//! Serde support for [`SchedulingBlock`](super::SchedulingBlock).

use super::{CompletionExpr, Dependency, SchedulingBlock};
use crate::constraints::{
    AltitudeConstraint, AzimuthConstraint, ConstraintExpr, MoonAltitudeConstraint, NightConstraint,
    PrioritySoftConstraint, SoftConstraintExpr, TimeConstraint,
};
use crate::serde_repr::{
    HardConstraintsRepr, TimeWindowRepr, TwilightRepr, hard_constraint_blocks_from_repr,
};
use crate::task::Task;
use crate::time::{SchedulingBlockId, TaskId};
use ::serde::{Deserialize, Deserializer, Serialize, Serializer};
use petgraph::visit::{EdgeRef, IntoEdgeReferences};
use qtty::{Degrees, Seconds};
use siderust::coordinates::frames::ICRS;
use siderust::coordinates::spherical::Direction;
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SchedulingBlockRepr {
    id: u64,
    tasks: Vec<TaskObjectRepr>,
    dependencies: Vec<DependencyEdgeRepr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    completion: Option<CompletionExprRepr>,
}

/// Serde representation of a [`CompletionExpr`].
///
/// JSON shapes (mutually exclusive):
/// - `{"task": <id>}`
/// - `{"all_of": [..]}`  (AND)
/// - `{"any_of": [..]}`  (OR)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
enum CompletionExprRepr {
    Task(u64),
    AllOf(Vec<CompletionExprRepr>),
    AnyOf(Vec<CompletionExprRepr>),
}

impl From<&CompletionExpr> for CompletionExprRepr {
    fn from(expr: &CompletionExpr) -> Self {
        match expr {
            CompletionExpr::Leaf(id) => CompletionExprRepr::Task(id.0),
            CompletionExpr::And(children) => {
                CompletionExprRepr::AllOf(children.iter().map(Into::into).collect())
            }
            CompletionExpr::Or(children) => {
                CompletionExprRepr::AnyOf(children.iter().map(Into::into).collect())
            }
        }
    }
}

impl From<CompletionExprRepr> for CompletionExpr {
    fn from(repr: CompletionExprRepr) -> Self {
        match repr {
            CompletionExprRepr::Task(id) => CompletionExpr::Leaf(TaskId(id)),
            CompletionExprRepr::AllOf(children) => {
                CompletionExpr::And(children.into_iter().map(Into::into).collect())
            }
            CompletionExprRepr::AnyOf(children) => {
                CompletionExpr::Or(children.into_iter().map(Into::into).collect())
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TargetRepr {
    ra_deg: f64,
    dec_deg: f64,
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
    requested_duration_sec: f64,
    target: TargetRepr,
    #[serde(default)]
    hard_constraints: HardConstraintsRepr,
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

fn task_from_repr(repr: TaskObjectRepr) -> Result<Task, String> {
    let hard_constraints = hard_constraint_blocks_from_repr(&repr.hard_constraints)?;
    let soft_constraints = repr
        .soft_constraints
        .as_ref()
        .and_then(|soft| soft.priority)
        .map(|priority| SoftConstraintExpr::atom(PrioritySoftConstraint::new(priority)));

    let name = repr
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("task-{}", repr.id));

    let target = Direction::<ICRS>::new_raw(
        Degrees::new(repr.target.dec_deg),
        Degrees::new(repr.target.ra_deg),
    );

    Task::new(
        TaskId(repr.id),
        name,
        target,
        Seconds::new(repr.requested_duration_sec),
        hard_constraints,
        soft_constraints,
    )
    .map_err(|err| format!("invalid task {}: {err}", repr.id))
}

fn hard_constraints_to_repr(task: &Task) -> Result<HardConstraintsRepr, String> {
    fn encode(expr: &ConstraintExpr, out: &mut HardConstraintsRepr) -> Result<(), String> {
        match expr {
            ConstraintExpr::Intersection(children) => {
                for child in children {
                    encode(child, out)?;
                }
                Ok(())
            }
            ConstraintExpr::Atom(atom) => {
                if let Some(constraint) = atom.downcast_ref::<AltitudeConstraint>() {
                    out.altitude_min_deg = Some(constraint.min.value());
                    out.altitude_max_deg = Some(constraint.max.value());
                    return Ok(());
                }
                if let Some(constraint) = atom.downcast_ref::<AzimuthConstraint>() {
                    out.azimuth_min_deg = Some(constraint.min.value());
                    out.azimuth_max_deg = Some(constraint.max.value());
                    return Ok(());
                }
                if let Some(constraint) = atom.downcast_ref::<TimeConstraint>() {
                    out.time_window = Some(TimeWindowRepr {
                        start_mjd_utc: constraint.window.start.value(),
                        end_mjd_utc: constraint.window.end.value(),
                    });
                    return Ok(());
                }
                if let Some(constraint) = atom.downcast_ref::<NightConstraint>() {
                    out.night_time = Some(crate::serde_repr::NightTimeRepr {
                        twilight: TwilightRepr::from(constraint.twilight),
                    });
                    return Ok(());
                }
                if let Some(constraint) = atom.downcast_ref::<MoonAltitudeConstraint>() {
                    out.moon_altitude = Some(crate::serde_repr::MoonAltitudeRepr {
                        min_deg: constraint.min.value(),
                        max_deg: constraint.max.value(),
                    });
                    return Ok(());
                }
                Err("unsupported hard constraint type in SchedulingBlock serialization".to_string())
            }
            ConstraintExpr::Union(_) => Err(
                "unsupported hard constraint union in SchedulingBlock serialization".to_string(),
            ),
        }
    }

    let mut repr = HardConstraintsRepr::default();
    encode(&task.hard_constraints.hard_static, &mut repr)?;
    encode(&task.hard_constraints.hard_dynamic, &mut repr)?;
    Ok(repr)
}

fn soft_constraints_to_repr(task: &Task) -> Result<Option<SoftConstraintsRepr>, String> {
    let Some(expr) = task.soft_constraints.as_ref() else {
        return Ok(None);
    };

    match expr {
        SoftConstraintExpr::Atom(atom) => {
            if let Some(priority) = atom.downcast_ref::<PrioritySoftConstraint>() {
                Ok(Some(SoftConstraintsRepr {
                    priority: Some(priority.priority),
                }))
            } else {
                Err("unsupported soft constraint type in SchedulingBlock serialization".to_string())
            }
        }
        _ => Err(
            "unsupported soft constraint expression in SchedulingBlock serialization".to_string(),
        ),
    }
}

fn task_to_repr(task: &Task) -> Result<TaskObjectRepr, String> {
    Ok(TaskObjectRepr {
        id: task.id.0,
        name: Some(task.name.clone()),
        requested_duration_sec: task.duration.value(),
        target: TargetRepr {
            ra_deg: task.target.azimuth.value(),
            dec_deg: task.target.polar.value(),
        },
        hard_constraints: hard_constraints_to_repr(task)?,
        soft_constraints: soft_constraints_to_repr(task)?,
    })
}

fn to_repr(block: &SchedulingBlock) -> Result<SchedulingBlockRepr, String> {
    let tasks = block
        .iter_tasks()
        .map(task_to_repr)
        .collect::<Result<Vec<_>, _>>()?;

    let dependencies = block
        .graph
        .edge_references()
        .map(|edge| DependencyEdgeRepr {
            from: block.graph[edge.source()].0,
            to: block.graph[edge.target()].0,
            dependency: (*edge.weight()).into(),
        })
        .collect::<Vec<_>>();

    Ok(SchedulingBlockRepr {
        id: block.id.0,
        tasks,
        dependencies,
        completion: block.completion().map(CompletionExprRepr::from),
    })
}

fn from_repr(repr: SchedulingBlockRepr) -> Result<SchedulingBlock, String> {
    let mut block = SchedulingBlock::new(SchedulingBlockId(repr.id));
    let mut seen = HashSet::new();

    for task_repr in repr.tasks {
        let task_id = task_repr.id;
        if !seen.insert(task_id) {
            return Err(format!(
                "duplicate task id {} in scheduling block payload",
                task_id
            ));
        }
        let task = task_from_repr(task_repr)?;
        block.push_task(task).map_err(|err| err.to_string())?;
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

    if let Some(completion_repr) = repr.completion {
        let expr: CompletionExpr = completion_repr.into();
        block
            .set_completion(expr)
            .map_err(|err| format!("invalid completion expression: {err}"))?;
    }

    Ok(block)
}

impl Serialize for SchedulingBlock {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        to_repr(self)
            .map_err(::serde::ser::Error::custom)?
            .serialize(serializer)
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
    use crate::constraints::{ConstraintExpr, SoftConstraintExpr};
    use qtty::Degrees;

    fn sample_task(id: u64, priority: f64) -> Task {
        Task::new(
            TaskId(id),
            format!("task-{id}"),
            Direction::<ICRS>::new_raw(Degrees::new(-16.716), Degrees::new(101.287)),
            Seconds::new(1200.0),
            ConstraintExpr::Intersection(vec![]),
            Some(SoftConstraintExpr::atom(PrioritySoftConstraint::new(
                priority,
            ))),
        )
        .unwrap()
    }

    #[test]
    fn json_roundtrip_preserves_block_constraints() {
        let mut block = SchedulingBlock::new(SchedulingBlockId(42));
        block.push_task(sample_task(10, 7.5)).unwrap();
        block.push_task(sample_task(20, 3.25)).unwrap();
        block.push_task(sample_task(30, 1.0)).unwrap();
        block
            .add_dependency(TaskId(20), TaskId(10), Dependency::DependsOn)
            .unwrap();
        block
            .add_dependency(TaskId(30), TaskId(20), Dependency::DependsOn)
            .unwrap();

        let value = serde_json::to_value(&block).unwrap();
        assert_eq!(value["id"], serde_json::json!(42));
        assert_eq!(value["tasks"].as_array().unwrap().len(), 3);
        assert_eq!(value["tasks"][0]["id"], serde_json::json!(10));
        assert_eq!(
            value["tasks"][0]["requested_duration_sec"],
            serde_json::json!(1200.0)
        );
        assert_eq!(value["dependencies"].as_array().unwrap().len(), 2);

        let decoded: SchedulingBlock = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(decoded.id, SchedulingBlockId(42));
        assert_eq!(
            decoded.iter().collect::<Vec<_>>(),
            vec![TaskId(10), TaskId(20), TaskId(30)]
        );

        let roundtrip = serde_json::to_value(&decoded).unwrap();
        assert_eq!(roundtrip, value);
    }

    #[test]
    fn deserialize_rejects_dependency_cycle() {
        let payload = serde_json::json!({
            "id": 1,
            "tasks": [
                {
                    "id": 1,
                    "name": "task-1",
                    "requested_duration_sec": 1200.0,
                    "target": {"ra_deg": 101.287, "dec_deg": -16.716},
                    "hard_constraints": {}
                },
                {
                    "id": 2,
                    "name": "task-2",
                    "requested_duration_sec": 1200.0,
                    "target": {"ra_deg": 102.0, "dec_deg": -15.0},
                    "hard_constraints": {}
                }
            ],
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
    fn deserialize_requires_full_task_objects() {
        let payload = serde_json::json!({
            "id": 5,
            "tasks": [10, 20],
            "dependencies": []
        });

        assert!(serde_json::from_value::<SchedulingBlock>(payload).is_err());
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
                    "target": {"ra_deg": 101.287, "dec_deg": -16.716},
                    "hard_constraints": {},
                    "soft_constraints": {"priority": 7.5}
                },
                {
                    "id": 20,
                    "name": "task-20",
                    "requested_duration_sec": 1800,
                    "target": {"ra_deg": 102.0, "dec_deg": -15.0},
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
