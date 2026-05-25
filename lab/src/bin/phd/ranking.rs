use serde_json::Value;
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct RunRecord {
    pub cell_id: String,
    pub dataset_id: String,
    pub algorithm_id: String,
    pub config_id: String,
    pub config_json: String,
    pub schedule_path: PathBuf,
    pub manifest_path: Option<PathBuf>,
    pub scheduled_task_count: u64,
    pub total_task_count: u64,
    pub scheduled_priority_ratio: f64,
    pub scheduled_task_ratio: f64,
    pub priority_density: f64,
    pub scheduler_runtime_ms: Option<f64>,
    pub scheduled_priority_sum: f64,
    pub total_priority_sum: f64,
    pub utilization: f64,
    pub fragmentation_index: f64,
}

#[derive(Debug, Clone)]
pub struct RankedRun {
    pub record: RunRecord,
    pub rank: usize,
    pub is_winner: bool,
}

#[derive(Debug, Clone)]
pub struct ConfigSummary {
    pub config_id: String,
    pub algorithm_id: String,
    pub config_json: String,
    pub runs: usize,
    pub datasets: usize,
    pub avg_rank: f64,
    pub median_rank: f64,
    pub wins: usize,
    pub avg_priority_ratio: f64,
    pub avg_task_ratio: f64,
    pub avg_runtime: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct AnalysisArtifacts {
    pub all_runs_csv: PathBuf,
    pub rankings_by_dataset_csv: PathBuf,
    pub summary_by_config_csv: PathBuf,
    pub pareto_front_csv: PathBuf,
    pub best_schedules_dir: PathBuf,
}

pub fn record_from_schedule(
    cell_id: &str,
    dataset_id: &str,
    algorithm_id: &str,
    config: &Value,
    schedule_path: &Path,
    manifest_path: Option<PathBuf>,
) -> Result<RunRecord, String> {
    let text = fs::read_to_string(schedule_path)
        .map_err(|e| format!("failed to read {}: {e}", schedule_path.display()))?;
    let doc: Value = serde_json::from_str(&text)
        .map_err(|e| format!("failed to parse {}: {e}", schedule_path.display()))?;
    let metrics = doc.get("schedule_metrics").ok_or_else(|| {
        format!(
            "schedule {} has no `schedule_metrics` field",
            schedule_path.display()
        )
    })?;

    let config_json =
        serde_json::to_string(config).map_err(|e| format!("failed to serialize config: {e}"))?;
    let config_id = format!("{algorithm_id}:{config_json}");
    let scheduler_runtime_ms = optional_finite_metric(metrics, "scheduler_runtime_ms")
        .or_else(|| optional_finite_metric(metrics, "scheduled_time_sec").map(|v| v * 1000.0));

    Ok(RunRecord {
        cell_id: cell_id.to_string(),
        dataset_id: dataset_id.to_string(),
        algorithm_id: algorithm_id.to_string(),
        config_id,
        config_json,
        schedule_path: schedule_path.to_path_buf(),
        manifest_path,
        scheduled_task_count: integer_metric(metrics, "scheduled_task_count"),
        total_task_count: integer_metric(metrics, "total_task_count"),
        scheduled_priority_ratio: finite_metric(metrics, "scheduled_priority_ratio"),
        scheduled_task_ratio: finite_metric(metrics, "scheduled_task_ratio")
            .max(finite_metric(metrics, "completion_ratio")),
        priority_density: finite_metric(metrics, "priority_density"),
        scheduler_runtime_ms,
        scheduled_priority_sum: finite_metric(metrics, "scheduled_priority_sum"),
        total_priority_sum: finite_metric(metrics, "total_priority_sum"),
        utilization: finite_metric(metrics, "utilization"),
        fragmentation_index: metrics
            .get("fragmentation")
            .map(|f| finite_metric(f, "fragmentation_index"))
            .unwrap_or(0.0),
    })
}

pub fn write_analysis_outputs(
    out_dir: &Path,
    records: &[RunRecord],
) -> Result<AnalysisArtifacts, String> {
    fs::create_dir_all(out_dir)
        .map_err(|e| format!("failed to create {}: {e}", out_dir.display()))?;

    let rankings = rank_by_dataset(records);
    let summaries = summarize_by_config(&rankings);
    let pareto = pareto_front(&summaries);

    let all_runs_csv = out_dir.join("all_runs.csv");
    let rankings_by_dataset_csv = out_dir.join("rankings_by_dataset.csv");
    let summary_by_config_csv = out_dir.join("summary_by_config.csv");
    let pareto_front_csv = out_dir.join("pareto_front.csv");
    let best_schedules_dir = out_dir.join("best_schedules");

    write_all_runs_csv(&all_runs_csv, records)?;
    write_rankings_csv(&rankings_by_dataset_csv, &rankings)?;
    write_summary_csv(&summary_by_config_csv, &summaries)?;
    write_summary_csv(&pareto_front_csv, &pareto)?;
    copy_best_schedules(&best_schedules_dir, &rankings)?;

    Ok(AnalysisArtifacts {
        all_runs_csv,
        rankings_by_dataset_csv,
        summary_by_config_csv,
        pareto_front_csv,
        best_schedules_dir,
    })
}

pub fn rank_by_dataset(records: &[RunRecord]) -> Vec<RankedRun> {
    let mut by_dataset: BTreeMap<&str, Vec<&RunRecord>> = BTreeMap::new();
    for record in records {
        by_dataset
            .entry(record.dataset_id.as_str())
            .or_default()
            .push(record);
    }

    let mut ranked = Vec::new();
    for (_, mut group) in by_dataset {
        group.sort_by(|a, b| compare_records(a, b));
        let mut previous: Option<&RunRecord> = None;
        let mut current_rank = 1usize;
        for (idx, record) in group.into_iter().enumerate() {
            if let Some(prev) = previous
                && !same_policy_key(prev, record)
            {
                current_rank = idx + 1;
            }
            ranked.push(RankedRun {
                record: record.clone(),
                rank: current_rank,
                is_winner: current_rank == 1,
            });
            previous = Some(record);
        }
    }
    ranked
}

pub fn summarize_by_config(rankings: &[RankedRun]) -> Vec<ConfigSummary> {
    let mut groups: BTreeMap<&str, Vec<&RankedRun>> = BTreeMap::new();
    for ranked in rankings {
        groups
            .entry(ranked.record.config_id.as_str())
            .or_default()
            .push(ranked);
    }

    let mut summaries = Vec::new();
    for (config_id, group) in groups {
        let mut ranks: Vec<usize> = group.iter().map(|r| r.rank).collect();
        let runtime_values: Vec<f64> = group
            .iter()
            .filter_map(|r| r.record.scheduler_runtime_ms)
            .filter(|v| v.is_finite())
            .collect();
        let mut datasets: Vec<&str> = group.iter().map(|r| r.record.dataset_id.as_str()).collect();
        datasets.sort_unstable();
        datasets.dedup();

        let first = &group[0].record;
        summaries.push(ConfigSummary {
            config_id: config_id.to_string(),
            algorithm_id: first.algorithm_id.clone(),
            config_json: first.config_json.clone(),
            runs: group.len(),
            datasets: datasets.len(),
            avg_rank: avg_usize(&ranks),
            median_rank: median_usize(&mut ranks),
            wins: group.iter().filter(|r| r.is_winner).count(),
            avg_priority_ratio: avg_f64(
                group
                    .iter()
                    .map(|r| r.record.scheduled_priority_ratio)
                    .collect::<Vec<_>>()
                    .as_slice(),
            ),
            avg_task_ratio: avg_f64(
                group
                    .iter()
                    .map(|r| r.record.scheduled_task_ratio)
                    .collect::<Vec<_>>()
                    .as_slice(),
            ),
            avg_runtime: if runtime_values.is_empty() {
                None
            } else {
                Some(avg_f64(&runtime_values))
            },
        });
    }
    summaries.sort_by(|a, b| {
        a.avg_rank
            .total_cmp(&b.avg_rank)
            .then_with(|| b.wins.cmp(&a.wins))
            .then_with(|| b.avg_priority_ratio.total_cmp(&a.avg_priority_ratio))
            .then_with(|| a.config_id.cmp(&b.config_id))
    });
    summaries
}

pub fn pareto_front(summaries: &[ConfigSummary]) -> Vec<ConfigSummary> {
    summaries
        .iter()
        .filter(|candidate| {
            !summaries
                .iter()
                .any(|other| other.config_id != candidate.config_id && dominates(other, candidate))
        })
        .cloned()
        .collect()
}

fn compare_records(a: &RunRecord, b: &RunRecord) -> Ordering {
    b.scheduled_priority_ratio
        .total_cmp(&a.scheduled_priority_ratio)
        .then_with(|| b.scheduled_task_ratio.total_cmp(&a.scheduled_task_ratio))
        .then_with(|| b.priority_density.total_cmp(&a.priority_density))
        .then_with(|| runtime_key(a).total_cmp(&runtime_key(b)))
        .then_with(|| a.cell_id.cmp(&b.cell_id))
}

fn same_policy_key(a: &RunRecord, b: &RunRecord) -> bool {
    a.scheduled_priority_ratio == b.scheduled_priority_ratio
        && a.scheduled_task_ratio == b.scheduled_task_ratio
        && a.priority_density == b.priority_density
        && runtime_key(a) == runtime_key(b)
}

fn runtime_key(record: &RunRecord) -> f64 {
    record
        .scheduler_runtime_ms
        .filter(|v| v.is_finite())
        .unwrap_or(f64::INFINITY)
}

fn dominates(a: &ConfigSummary, b: &ConfigSummary) -> bool {
    let a_runtime = a.avg_runtime.unwrap_or(f64::INFINITY);
    let b_runtime = b.avg_runtime.unwrap_or(f64::INFINITY);
    let no_worse = a.avg_priority_ratio >= b.avg_priority_ratio
        && a.avg_task_ratio >= b.avg_task_ratio
        && a.wins >= b.wins
        && a.avg_rank <= b.avg_rank
        && a_runtime <= b_runtime;
    let strictly_better = a.avg_priority_ratio > b.avg_priority_ratio
        || a.avg_task_ratio > b.avg_task_ratio
        || a.wins > b.wins
        || a.avg_rank < b.avg_rank
        || a_runtime < b_runtime;
    no_worse && strictly_better
}

fn write_all_runs_csv(path: &Path, records: &[RunRecord]) -> Result<(), String> {
    let mut writer = csv::Writer::from_path(path)
        .map_err(|e| format!("failed to create {}: {e}", path.display()))?;
    writer
        .write_record([
            "cell_id",
            "dataset_id",
            "algorithm_id",
            "config_id",
            "config_json",
            "schedule_path",
            "manifest_path",
            "scheduled_task_count",
            "total_task_count",
            "scheduled_priority_ratio",
            "scheduled_task_ratio",
            "priority_density",
            "scheduler_runtime_ms",
            "scheduled_priority_sum",
            "total_priority_sum",
            "utilization",
            "fragmentation_index",
        ])
        .map_err(|e| format!("failed to write {}: {e}", path.display()))?;
    let mut sorted = records.to_vec();
    sorted.sort_by(|a, b| a.cell_id.cmp(&b.cell_id));
    for record in sorted {
        writer
            .write_record([
                record.cell_id,
                record.dataset_id,
                record.algorithm_id,
                record.config_id,
                record.config_json,
                path_string(&record.schedule_path),
                record
                    .manifest_path
                    .as_ref()
                    .map(|p| path_string(p))
                    .unwrap_or_default(),
                record.scheduled_task_count.to_string(),
                record.total_task_count.to_string(),
                float_string(record.scheduled_priority_ratio),
                float_string(record.scheduled_task_ratio),
                float_string(record.priority_density),
                opt_float_string(record.scheduler_runtime_ms),
                float_string(record.scheduled_priority_sum),
                float_string(record.total_priority_sum),
                float_string(record.utilization),
                float_string(record.fragmentation_index),
            ])
            .map_err(|e| format!("failed to write {}: {e}", path.display()))?;
    }
    writer
        .flush()
        .map_err(|e| format!("failed to flush {}: {e}", path.display()))
}

fn write_rankings_csv(path: &Path, rankings: &[RankedRun]) -> Result<(), String> {
    let mut writer = csv::Writer::from_path(path)
        .map_err(|e| format!("failed to create {}: {e}", path.display()))?;
    writer
        .write_record([
            "dataset_id",
            "rank",
            "is_winner",
            "cell_id",
            "algorithm_id",
            "config_id",
            "config_json",
            "scheduled_priority_ratio",
            "scheduled_task_ratio",
            "priority_density",
            "scheduler_runtime_ms",
            "schedule_path",
            "manifest_path",
        ])
        .map_err(|e| format!("failed to write {}: {e}", path.display()))?;
    let mut sorted = rankings.to_vec();
    sorted.sort_by(|a, b| {
        a.record
            .dataset_id
            .cmp(&b.record.dataset_id)
            .then_with(|| a.rank.cmp(&b.rank))
            .then_with(|| a.record.cell_id.cmp(&b.record.cell_id))
    });
    for ranked in sorted {
        let record = ranked.record;
        writer
            .write_record([
                record.dataset_id,
                ranked.rank.to_string(),
                ranked.is_winner.to_string(),
                record.cell_id,
                record.algorithm_id,
                record.config_id,
                record.config_json,
                float_string(record.scheduled_priority_ratio),
                float_string(record.scheduled_task_ratio),
                float_string(record.priority_density),
                opt_float_string(record.scheduler_runtime_ms),
                path_string(&record.schedule_path),
                record
                    .manifest_path
                    .as_ref()
                    .map(|p| path_string(p))
                    .unwrap_or_default(),
            ])
            .map_err(|e| format!("failed to write {}: {e}", path.display()))?;
    }
    writer
        .flush()
        .map_err(|e| format!("failed to flush {}: {e}", path.display()))
}

fn write_summary_csv(path: &Path, summaries: &[ConfigSummary]) -> Result<(), String> {
    let mut writer = csv::Writer::from_path(path)
        .map_err(|e| format!("failed to create {}: {e}", path.display()))?;
    writer
        .write_record([
            "config_id",
            "algorithm_id",
            "config_json",
            "runs",
            "datasets",
            "avg_rank",
            "median_rank",
            "wins",
            "avg_priority_ratio",
            "avg_task_ratio",
            "avg_runtime",
        ])
        .map_err(|e| format!("failed to write {}: {e}", path.display()))?;
    for summary in summaries {
        writer
            .write_record([
                summary.config_id.clone(),
                summary.algorithm_id.clone(),
                summary.config_json.clone(),
                summary.runs.to_string(),
                summary.datasets.to_string(),
                float_string(summary.avg_rank),
                float_string(summary.median_rank),
                summary.wins.to_string(),
                float_string(summary.avg_priority_ratio),
                float_string(summary.avg_task_ratio),
                opt_float_string(summary.avg_runtime),
            ])
            .map_err(|e| format!("failed to write {}: {e}", path.display()))?;
    }
    writer
        .flush()
        .map_err(|e| format!("failed to flush {}: {e}", path.display()))
}

fn copy_best_schedules(best_dir: &Path, rankings: &[RankedRun]) -> Result<(), String> {
    if best_dir.exists() {
        fs::remove_dir_all(best_dir)
            .map_err(|e| format!("failed to clear {}: {e}", best_dir.display()))?;
    }
    fs::create_dir_all(best_dir)
        .map_err(|e| format!("failed to create {}: {e}", best_dir.display()))?;

    let mut winners: Vec<&RankedRun> = rankings.iter().filter(|r| r.is_winner).collect();
    winners.sort_by(|a, b| {
        a.record
            .dataset_id
            .cmp(&b.record.dataset_id)
            .then_with(|| a.record.cell_id.cmp(&b.record.cell_id))
    });
    for winner in winners {
        let schedule_name = winner.record.schedule_path.file_name().ok_or_else(|| {
            format!(
                "schedule path {} has no file name",
                winner.record.schedule_path.display()
            )
        })?;
        fs::copy(&winner.record.schedule_path, best_dir.join(schedule_name)).map_err(|e| {
            format!(
                "failed to copy {} into {}: {e}",
                winner.record.schedule_path.display(),
                best_dir.display()
            )
        })?;
        if let Some(manifest_path) = &winner.record.manifest_path
            && manifest_path.exists()
        {
            let manifest_name = manifest_path.file_name().ok_or_else(|| {
                format!("manifest path {} has no file name", manifest_path.display())
            })?;
            fs::copy(manifest_path, best_dir.join(manifest_name)).map_err(|e| {
                format!(
                    "failed to copy {} into {}: {e}",
                    manifest_path.display(),
                    best_dir.display()
                )
            })?;
        }
    }
    Ok(())
}

fn integer_metric(metrics: &Value, key: &str) -> u64 {
    metrics.get(key).and_then(Value::as_u64).unwrap_or(0)
}

fn finite_metric(metrics: &Value, key: &str) -> f64 {
    optional_finite_metric(metrics, key).unwrap_or(0.0)
}

fn optional_finite_metric(metrics: &Value, key: &str) -> Option<f64> {
    metrics
        .get(key)
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite())
}

fn avg_usize(values: &[usize]) -> f64 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<usize>() as f64 / values.len() as f64
    }
}

fn avg_f64(values: &[f64]) -> f64 {
    let finite: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
    if finite.is_empty() {
        0.0
    } else {
        finite.iter().sum::<f64>() / finite.len() as f64
    }
}

fn median_usize(values: &mut [usize]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_unstable();
    let mid = values.len() / 2;
    if values.len().is_multiple_of(2) {
        (values[mid - 1] + values[mid]) as f64 / 2.0
    } else {
        values[mid] as f64
    }
}

fn float_string(value: f64) -> String {
    if value.is_finite() {
        value.to_string()
    } else {
        String::new()
    }
}

fn opt_float_string(value: Option<f64>) -> String {
    value
        .filter(|v| v.is_finite())
        .map_or_else(String::new, |v| v.to_string())
}

fn path_string(path: &Path) -> String {
    path.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn record(
        cell_id: &str,
        dataset_id: &str,
        config_json: &str,
        priority: f64,
        task: f64,
        density: f64,
        runtime: f64,
    ) -> RunRecord {
        RunRecord {
            cell_id: cell_id.to_string(),
            dataset_id: dataset_id.to_string(),
            algorithm_id: "est".to_string(),
            config_id: format!("est:{config_json}"),
            config_json: config_json.to_string(),
            schedule_path: PathBuf::from(format!("{cell_id}.json")),
            manifest_path: None,
            scheduled_task_count: 0,
            total_task_count: 0,
            scheduled_priority_ratio: priority,
            scheduled_task_ratio: task,
            priority_density: density,
            scheduler_runtime_ms: Some(runtime),
            scheduled_priority_sum: 0.0,
            total_priority_sum: 0.0,
            utilization: 0.0,
            fragmentation_index: 0.0,
        }
    }

    #[test]
    fn ranks_by_priority_ratio_first() {
        let records = vec![
            record("b", "d", "{}", 0.8, 1.0, 1.0, 1.0),
            record("a", "d", "{}", 0.9, 0.1, 0.1, 100.0),
        ];
        let ranked = rank_by_dataset(&records);
        assert_eq!(ranked[0].record.cell_id, "a");
        assert_eq!(ranked[0].rank, 1);
    }

    #[test]
    fn task_ratio_breaks_priority_ties() {
        let records = vec![
            record("a", "d", "{}", 0.8, 0.4, 2.0, 1.0),
            record("b", "d", "{}", 0.8, 0.6, 1.0, 1.0),
        ];
        assert_eq!(rank_by_dataset(&records)[0].record.cell_id, "b");
    }

    #[test]
    fn density_breaks_next_tie() {
        let records = vec![
            record("a", "d", "{}", 0.8, 0.6, 1.0, 1.0),
            record("b", "d", "{}", 0.8, 0.6, 2.0, 1.0),
        ];
        assert_eq!(rank_by_dataset(&records)[0].record.cell_id, "b");
    }

    #[test]
    fn runtime_breaks_final_metric_tie() {
        let records = vec![
            record("a", "d", "{}", 0.8, 0.6, 2.0, 20.0),
            record("b", "d", "{}", 0.8, 0.6, 2.0, 10.0),
        ];
        assert_eq!(rank_by_dataset(&records)[0].record.cell_id, "b");
    }

    #[test]
    fn cell_id_breaks_complete_ties_without_changing_rank() {
        let records = vec![
            record("b", "d", "{}", 0.8, 0.6, 2.0, 10.0),
            record("a", "d", "{}", 0.8, 0.6, 2.0, 10.0),
        ];
        let ranked = rank_by_dataset(&records);
        assert_eq!(ranked[0].record.cell_id, "a");
        assert_eq!(ranked[0].rank, 1);
        assert_eq!(ranked[1].rank, 1);
    }

    #[test]
    fn exact_config_grouping_includes_seed() {
        let rankings = vec![
            RankedRun {
                record: record("a", "d1", r#"{"seed":0}"#, 1.0, 1.0, 1.0, 1.0),
                rank: 1,
                is_winner: true,
            },
            RankedRun {
                record: record("b", "d1", r#"{"seed":1}"#, 1.0, 1.0, 1.0, 1.0),
                rank: 1,
                is_winner: true,
            },
        ];
        assert_eq!(summarize_by_config(&rankings).len(), 2);
    }

    #[test]
    fn pareto_front_excludes_dominated_configs() {
        let strong = ConfigSummary {
            config_id: "strong".to_string(),
            algorithm_id: "est".to_string(),
            config_json: "{}".to_string(),
            runs: 1,
            datasets: 1,
            avg_rank: 1.0,
            median_rank: 1.0,
            wins: 1,
            avg_priority_ratio: 0.9,
            avg_task_ratio: 0.8,
            avg_runtime: Some(10.0),
        };
        let weak = ConfigSummary {
            config_id: "weak".to_string(),
            algorithm_id: "est".to_string(),
            config_json: "{}".to_string(),
            runs: 1,
            datasets: 1,
            avg_rank: 2.0,
            median_rank: 2.0,
            wins: 0,
            avg_priority_ratio: 0.8,
            avg_task_ratio: 0.7,
            avg_runtime: Some(20.0),
        };
        let front = pareto_front(&[strong, weak]);
        assert_eq!(front.len(), 1);
        assert_eq!(front[0].config_id, "strong");
    }

    #[test]
    fn writes_csvs_and_copies_best_schedules() {
        let tmp = TempDir::new().unwrap();
        let out = tmp.path();
        let sched_a = out.join("a.json");
        let sched_b = out.join("b.json");
        let manifest_a = out.join("a.manifest.json");
        fs::write(&sched_a, "{}").unwrap();
        fs::write(&sched_b, "{}").unwrap();
        fs::write(&manifest_a, "{}").unwrap();

        let mut a = record("a", "d1", r#"{"seed":0}"#, 1.0, 1.0, 1.0, 5.0);
        a.schedule_path = sched_a;
        a.manifest_path = Some(manifest_a);
        let mut b = record("b", "d2", r#"{"seed":1}"#, 1.0, 1.0, 1.0, 5.0);
        b.schedule_path = sched_b;

        let artifacts = write_analysis_outputs(out, &[a, b]).unwrap();
        assert!(artifacts.all_runs_csv.is_file());
        assert!(artifacts.rankings_by_dataset_csv.is_file());
        assert!(artifacts.summary_by_config_csv.is_file());
        assert!(artifacts.pareto_front_csv.is_file());
        assert!(artifacts.best_schedules_dir.join("a.json").is_file());
        assert!(
            artifacts
                .best_schedules_dir
                .join("a.manifest.json")
                .is_file()
        );
        assert!(artifacts.best_schedules_dir.join("b.json").is_file());
        assert!(
            !artifacts
                .best_schedules_dir
                .join("b.manifest.json")
                .exists()
        );
    }

    #[test]
    fn record_from_schedule_reads_metrics_and_config() {
        let tmp = TempDir::new().unwrap();
        let schedule = tmp.path().join("run.json");
        fs::write(
            &schedule,
            serde_json::to_string(&json!({
                "schedule_metrics": {
                    "scheduled_task_count": 2,
                    "total_task_count": 4,
                    "scheduled_task_ratio": 0.5,
                    "scheduled_priority_ratio": 0.75,
                    "priority_density": 1.5,
                    "scheduler_runtime_ms": 12.0,
                    "scheduled_priority_sum": 3.0,
                    "total_priority_sum": 4.0,
                    "scheduled_time_sec": 120.0,
                    "utilization": 0.1,
                    "fragmentation": {"fragmentation_index": 0.2}
                }
            }))
            .unwrap(),
        )
        .unwrap();
        let record = record_from_schedule(
            "cell",
            "dataset",
            "hap",
            &json!({"seed": 1}),
            &schedule,
            None,
        )
        .unwrap();
        assert_eq!(record.config_id, r#"hap:{"seed":1}"#);
        assert_eq!(record.scheduler_runtime_ms, Some(12.0));
    }
}
