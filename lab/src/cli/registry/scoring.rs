//! Query-time score and Pareto helpers for registry commands.

use lab::registry::RunRow;

use super::format::{is_known_metric, metric_opt, parse_metrics};

/// Extracts a metric value for scoring, defaulting absent/null fields to 0.0.
///
/// Returns `Err` for composite metrics (use `registry rank` instead) and for
/// unknown metric names.  The actual JSON extraction delegates to
/// [`metric_opt`] so the lookup logic lives in one place.
pub(super) fn metric_value(mv: &serde_json::Value, metric: &str) -> Result<f64, String> {
    if matches!(metric, "composite_score" | "composite_rank_score") {
        return Err(
            "composite_rank_score is persisted only for backward-compatible schedule metrics; \
             define query-time weights with `registry rank` instead"
                .to_string(),
        );
    }
    if !is_known_metric(metric) {
        return Err(format!("unsupported metric '{metric}'"));
    }
    Ok(metric_opt(mv, metric).unwrap_or(0.0))
}

pub(super) fn parse_weights(raw: &[String]) -> Result<Vec<(String, f64)>, String> {
    raw.iter()
        .map(|entry| {
            let (metric, weight) = entry
                .split_once('=')
                .ok_or_else(|| format!("invalid weight '{entry}', expected metric=value"))?;
            let metric = metric.trim();
            if metric.is_empty() {
                return Err(format!("invalid weight '{entry}', metric cannot be empty"));
            }
            let weight = weight
                .trim()
                .parse::<f64>()
                .map_err(|e| format!("invalid weight in '{entry}': {e}"))?;
            metric_value(&serde_json::Value::Null, metric)?;
            Ok((metric.to_string(), weight))
        })
        .collect()
}

pub(super) fn parse_objectives(
    maximize: &[String],
    minimize: &[String],
) -> Result<Vec<(String, bool)>, String> {
    let mut objectives = Vec::new();
    if maximize.is_empty() && minimize.is_empty() {
        objectives.extend([
            ("scheduled_priority_ratio".to_string(), true),
            ("scheduled_task_ratio".to_string(), true),
            ("priority_density".to_string(), true),
            ("runtime_ms".to_string(), false),
        ]);
        return Ok(objectives);
    }
    for metric in maximize {
        metric_value(&serde_json::Value::Null, metric)?;
        objectives.push((metric.clone(), true));
    }
    for metric in minimize {
        metric_value(&serde_json::Value::Null, metric)?;
        objectives.push((metric.clone(), false));
    }
    Ok(objectives)
}

pub(super) fn dominates(a: &RunRow, b: &RunRow, objectives: &[(String, bool)]) -> bool {
    let a_metrics = parse_metrics(&a.metrics_json);
    let b_metrics = parse_metrics(&b.metrics_json);
    let mut strictly_better = false;
    for (metric, maximize) in objectives {
        let av = metric_value(&a_metrics, metric).unwrap_or(0.0);
        let bv = metric_value(&b_metrics, metric).unwrap_or(0.0);
        if *maximize {
            if av < bv {
                return false;
            }
            strictly_better |= av > bv;
        } else {
            if av > bv {
                return false;
            }
            strictly_better |= av < bv;
        }
    }
    strictly_better
}

pub(super) fn compare_rows_by_default_policy(a: &RunRow, b: &RunRow) -> std::cmp::Ordering {
    let am = parse_metrics(&a.metrics_json);
    let bm = parse_metrics(&b.metrics_json);
    metric_value(&bm, "scheduled_priority_ratio")
        .unwrap_or(0.0)
        .total_cmp(&metric_value(&am, "scheduled_priority_ratio").unwrap_or(0.0))
        .then_with(|| {
            metric_value(&bm, "scheduled_task_ratio")
                .unwrap_or(0.0)
                .total_cmp(&metric_value(&am, "scheduled_task_ratio").unwrap_or(0.0))
        })
        .then_with(|| {
            metric_value(&bm, "priority_density")
                .unwrap_or(0.0)
                .total_cmp(&metric_value(&am, "priority_density").unwrap_or(0.0))
        })
        .then_with(|| {
            metric_value(&am, "runtime_ms")
                .unwrap_or(0.0)
                .total_cmp(&metric_value(&bm, "runtime_ms").unwrap_or(0.0))
        })
        .then_with(|| a.run_key.cmp(&b.run_key))
}
