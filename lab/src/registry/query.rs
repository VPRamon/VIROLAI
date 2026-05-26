//! Query options and metric ordering helpers for registry reads.

use std::path::{Path, PathBuf};

/// Default path of the SQLite registry file, relative to the working dir.
pub const DEFAULT_RUN_DB: &str = ".lab/runs.sqlite";

/// Options for [`crate::registry::Registry::list`].
#[derive(Debug, Default)]
pub struct ListOpts {
    pub dataset: Option<String>,
    pub algorithm: Option<String>,
    pub metric: Option<String>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub sort: Vec<SortKey>,
    pub limit: Option<usize>,
}

/// Options for [`crate::registry::Registry::best`].
#[derive(Debug)]
pub struct BestOpts {
    pub dataset_id: String,
    pub algorithm: Option<String>,
    pub sort: Vec<SortKey>,
    pub limit: Option<usize>,
}

/// Direction for a query-time sort key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    Asc,
    Desc,
}

impl SortDirection {
    pub const fn as_sql(self) -> &'static str {
        match self {
            Self::Asc => "ASC",
            Self::Desc => "DESC",
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Asc => "asc",
            Self::Desc => "desc",
        }
    }
}

/// One metric/direction pair used for query-time ordering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SortKey {
    pub metric: String,
    pub direction: SortDirection,
}

/// Parse `metric:asc` / `metric:desc`. If direction is omitted, the metric's
/// conventional objective direction is used.
pub fn parse_sort_key(input: &str) -> Result<SortKey, String> {
    let (metric, dir) = input
        .split_once(':')
        .map_or((input, None), |(m, d)| (m, Some(d)));
    let metric = metric.trim();
    if metric.is_empty() {
        return Err("sort key metric cannot be empty".to_string());
    }
    let direction = match dir.map(str::trim) {
        Some("asc") => SortDirection::Asc,
        Some("desc") => SortDirection::Desc,
        Some(other) => {
            return Err(format!(
                "invalid sort direction '{other}' in '{input}', expected asc or desc"
            ));
        }
        None => default_metric_direction(metric)?,
    };
    metric_col(metric)?;
    Ok(SortKey {
        metric: metric.to_string(),
        direction,
    })
}

/// Default query-time policy used only when the user does not provide sort
/// keys. It is deliberately expressed as objective metric ordering, not as a
/// persisted composite score.
pub fn default_sort_keys() -> Vec<SortKey> {
    vec![
        SortKey {
            metric: "scheduled_priority_ratio".to_string(),
            direction: SortDirection::Desc,
        },
        SortKey {
            metric: "scheduled_task_ratio".to_string(),
            direction: SortDirection::Desc,
        },
        SortKey {
            metric: "priority_density".to_string(),
            direction: SortDirection::Desc,
        },
        SortKey {
            metric: "runtime_ms".to_string(),
            direction: SortDirection::Asc,
        },
    ]
}

pub fn metric_col(metric: &str) -> Result<&'static str, String> {
    match metric {
        "task_ratio" | "scheduled_task_ratio" => Ok("task_ratio"),
        "priority_ratio" | "scheduled_priority_ratio" => Ok("priority_ratio"),
        "priority_density" => Ok("priority_density"),
        "utilization" => Ok("utilization"),
        "fragmentation_index" => Ok("fragmentation_index"),
        "runtime_ms" | "scheduler_runtime_ms" => Ok("runtime_ms"),
        "requested_time_sec" => Ok("requested_time_sec"),
        "scheduled_time_sec" => Ok("scheduled_time_sec"),
        "scheduled_time_ratio" => Ok("scheduled_time_ratio"),
        "composite_score" | "composite_rank_score" => Err(
            "composite_rank_score is not a registry sort/filter metric; use `registry rank --weight ...` to compute a query-time score"
                .to_string(),
        ),
        _ => Err(format!("unsupported registry metric '{metric}'")),
    }
}

fn default_metric_direction(metric: &str) -> Result<SortDirection, String> {
    Ok(match metric_col(metric)? {
        "fragmentation_index" | "runtime_ms" => SortDirection::Asc,
        _ => SortDirection::Desc,
    })
}

pub(super) fn sort_expr(keys: &[SortKey]) -> Result<String, String> {
    let mut parts = Vec::with_capacity(keys.len() + 1);
    for key in keys {
        let col = metric_col(&key.metric)?;
        parts.push(format!("{col} {}", key.direction.as_sql()));
    }
    parts.push("run_key ASC".to_string());
    Ok(parts.join(", "))
}

/// Returns the registry path from an optional CLI override.
pub fn registry_path(run_db: Option<&Path>) -> PathBuf {
    run_db
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_RUN_DB))
}
