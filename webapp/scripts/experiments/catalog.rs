//! Filesystem-backed catalog over `<root>/<slug>/run-*/` directories.

use rayon::prelude::*;
use scheduler::metrics::{RankingWeights, ScheduleMetrics};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::{Duration, Instant};

use crate::experiments::errors::{ExperimentError, ExperimentResult};
use crate::experiments::state_events::{
    self, CellStatus, StateEvent, count_statuses, latest_per_cell,
};

const SCHEDULES_DIR: &str = "schedules";
const METRICS_DIR: &str = "metrics";
const TRACES_DIR: &str = "traces";
const STATE_FILE: &str = "state.jsonl";
const SUMMARY_FILE: &str = "summary.csv";
const EXPERIMENT_FILE: &str = "experiment.json";

const INDEX_TTL: Duration = Duration::from_secs(5);

/// One entry in the catalog index, keyed by `(experiment_slug, run_id)`.
#[derive(Debug, Clone, Serialize)]
pub struct ExperimentIndexEntry {
    pub experiment_slug: String,
    pub run_id: String,
    pub experiment_name: String,
    pub output_dir: PathBuf,
    pub created_at: String,
    pub updated_at: String,
    pub total_cells: usize,
    pub completed_cells: usize,
    pub failed_cells: usize,
    pub running_cells: usize,
    pub status: RunStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Completed,
    Failed,
    /// No state.jsonl yet (the matrix has not produced any events).
    Pending,
}

#[derive(Debug, Clone, Serialize)]
pub struct CellSummary {
    pub cell_id: String,
    pub dataset_id: Option<String>,
    pub algorithm: Option<String>,
    pub config_slug: Option<String>,
    pub status: Option<CellStatus>,
    pub error: Option<String>,
    pub schedule_path: Option<String>,
    pub metrics_path: Option<String>,
    pub trace_path: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CellDetail {
    pub cell_id: String,
    pub dataset_id: Option<String>,
    pub algorithm: Option<String>,
    pub config_slug: Option<String>,
    pub status: Option<CellStatus>,
    pub metrics: Option<ScheduleMetrics>,
    pub schedule_path: Option<String>,
    pub trace_path: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExperimentDetail {
    #[serde(flatten)]
    pub index: ExperimentIndexEntry,
    pub spec: Value,
    pub cells: Vec<CellSummary>,
}

#[derive(Debug, Default, Deserialize)]
pub struct CellListFilter {
    pub status: Option<String>,
    pub dataset_id: Option<String>,
    pub algorithm: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ParetoPoint {
    pub cell_id: String,
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RankingEntry {
    pub key: String,
    pub mean_score: f64,
    pub mean_completion: f64,
    pub mean_priority_sum: f64,
    pub mean_utilization: f64,
    pub mean_fragmentation_index: f64,
    pub n: usize,
}

/// Cached on-disk index. Refreshed lazily on TTL and explicitly on POST.
struct IndexCache {
    entries: BTreeMap<(String, String), ExperimentIndexEntry>,
    refreshed_at: Instant,
}

impl IndexCache {
    fn empty() -> Self {
        Self {
            entries: BTreeMap::new(),
            // Far in the past — first read triggers a refresh.
            refreshed_at: Instant::now()
                .checked_sub(INDEX_TTL * 10)
                .unwrap_or_else(Instant::now),
        }
    }
}

pub struct Catalog {
    root: PathBuf,
    cache: RwLock<IndexCache>,
}

impl Catalog {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            cache: RwLock::new(IndexCache::empty()),
        }
    }

    #[allow(dead_code)]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Force a rescan and return a snapshot of the index.
    pub fn refresh(&self) -> ExperimentResult<Vec<ExperimentIndexEntry>> {
        let entries = scan_root(&self.root)?;
        let mut guard = self
            .cache
            .write()
            .map_err(|_| ExperimentError::Internal("catalog cache lock poisoned".to_string()))?;
        guard.entries = entries
            .iter()
            .map(|e| ((e.experiment_slug.clone(), e.run_id.clone()), e.clone()))
            .collect();
        guard.refreshed_at = Instant::now();
        Ok(entries)
    }

    fn ensure_fresh(&self) -> ExperimentResult<()> {
        let stale = {
            let guard = self.cache.read().map_err(|_| {
                ExperimentError::Internal("catalog cache lock poisoned".to_string())
            })?;
            guard.refreshed_at.elapsed() >= INDEX_TTL
        };
        if stale {
            self.refresh()?;
        }
        Ok(())
    }

    /// List all experiment runs (across slugs).
    pub fn list_experiments(&self) -> ExperimentResult<Vec<ExperimentIndexEntry>> {
        self.ensure_fresh()?;
        let guard = self
            .cache
            .read()
            .map_err(|_| ExperimentError::Internal("catalog cache lock poisoned".to_string()))?;
        Ok(guard.entries.values().cloned().collect())
    }

    pub fn get_experiment_index(
        &self,
        slug: &str,
        run_id: &str,
    ) -> ExperimentResult<ExperimentIndexEntry> {
        self.ensure_fresh()?;
        // First try the cache; if missing, rescan once (the entry might be
        // freshly created since the last refresh).
        {
            let guard = self.cache.read().map_err(|_| {
                ExperimentError::Internal("catalog cache lock poisoned".to_string())
            })?;
            if let Some(entry) = guard.entries.get(&(slug.to_string(), run_id.to_string())) {
                return Ok(entry.clone());
            }
        }
        self.refresh()?;
        let guard = self
            .cache
            .read()
            .map_err(|_| ExperimentError::Internal("catalog cache lock poisoned".to_string()))?;
        guard
            .entries
            .get(&(slug.to_string(), run_id.to_string()))
            .cloned()
            .ok_or_else(|| {
                ExperimentError::NotFound(format!("experiment {slug}/{run_id} not found"))
            })
    }

    /// Full experiment detail: spec manifest + per-cell summaries.
    pub fn get_experiment(&self, slug: &str, run_id: &str) -> ExperimentResult<ExperimentDetail> {
        let index = self.get_experiment_index(slug, run_id)?;
        let run_dir = self.run_dir(slug, run_id);
        let manifest = read_manifest(&run_dir).unwrap_or(Value::Null);
        let spec = manifest.get("spec").cloned().unwrap_or(manifest.clone());

        let cells = self.list_cells(slug, run_id, &CellListFilter::default())?;
        Ok(ExperimentDetail { index, spec, cells })
    }

    pub fn list_cells(
        &self,
        slug: &str,
        run_id: &str,
        filter: &CellListFilter,
    ) -> ExperimentResult<Vec<CellSummary>> {
        let run_dir = self.run_dir(slug, run_id);
        if !run_dir.exists() {
            return Err(ExperimentError::NotFound(format!(
                "run {slug}/{run_id} not found"
            )));
        }

        // Source of truth for cell list = experiment.json's `cells` array.
        // Fall back to state.jsonl event ids when manifest is missing.
        let manifest = read_manifest(&run_dir).unwrap_or(Value::Null);
        let manifest_cells = manifest
            .get("cells")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let events = state_events::read_events(&run_dir.join(STATE_FILE))?;
        let latest = latest_per_cell(&events);

        let mut summaries: Vec<CellSummary> = if !manifest_cells.is_empty() {
            manifest_cells
                .iter()
                .map(|c| cell_summary_from_manifest(c, &latest))
                .collect()
        } else {
            // No manifest — derive cells from event stream.
            let mut keys: Vec<_> = latest.keys().cloned().collect();
            keys.sort();
            keys.into_iter()
                .map(|cell_id| {
                    let ev = latest.get(&cell_id).copied();
                    CellSummary {
                        cell_id: cell_id.clone(),
                        dataset_id: None,
                        algorithm: None,
                        config_slug: None,
                        status: ev.map(|e| e.status),
                        error: ev.and_then(|e| e.error.clone()),
                        schedule_path: ev.and_then(|e| e.schedule_path.clone()),
                        metrics_path: ev.and_then(|e| e.metrics_path.clone()),
                        trace_path: ev.and_then(|e| e.trace_path.clone()),
                        started_at: ev.map(|e| e.started_at.clone()),
                        finished_at: ev.and_then(|e| e.finished_at.clone()),
                    }
                })
                .collect()
        };

        if let Some(s) = filter.status.as_deref() {
            let want = match s {
                "started" => Some(CellStatus::Started),
                "completed" => Some(CellStatus::Completed),
                "failed" => Some(CellStatus::Failed),
                _ => {
                    return Err(ExperimentError::BadRequest(format!(
                        "unknown status filter `{s}`"
                    )));
                }
            };
            summaries.retain(|c| c.status == want);
        }
        if let Some(d) = filter.dataset_id.as_deref() {
            summaries.retain(|c| c.dataset_id.as_deref() == Some(d));
        }
        if let Some(a) = filter.algorithm.as_deref() {
            summaries.retain(|c| c.algorithm.as_deref() == Some(a));
        }
        let offset = filter.offset.unwrap_or(0);
        if offset > 0 {
            if offset >= summaries.len() {
                summaries.clear();
            } else {
                summaries.drain(..offset);
            }
        }
        if let Some(limit) = filter.limit {
            summaries.truncate(limit);
        }
        Ok(summaries)
    }

    #[allow(dead_code)]
    pub fn get_cell_metrics(
        &self,
        slug: &str,
        run_id: &str,
        cell_id: &str,
    ) -> ExperimentResult<ScheduleMetrics> {
        let path = self
            .run_dir(slug, run_id)
            .join(METRICS_DIR)
            .join(format!("{cell_id}.json"));
        read_metrics_file(&path)
    }

    pub fn get_cell_detail(
        &self,
        slug: &str,
        run_id: &str,
        cell_id: &str,
    ) -> ExperimentResult<CellDetail> {
        let cells = self.list_cells(slug, run_id, &CellListFilter::default())?;
        let summary = cells
            .into_iter()
            .find(|c| c.cell_id == cell_id)
            .ok_or_else(|| {
                ExperimentError::NotFound(format!("cell {cell_id} not found in {slug}/{run_id}"))
            })?;

        let metrics_path = self
            .run_dir(slug, run_id)
            .join(METRICS_DIR)
            .join(format!("{cell_id}.json"));
        let metrics = if metrics_path.exists() {
            Some(read_metrics_file(&metrics_path)?)
        } else {
            None
        };

        Ok(CellDetail {
            cell_id: summary.cell_id,
            dataset_id: summary.dataset_id,
            algorithm: summary.algorithm,
            config_slug: summary.config_slug,
            status: summary.status,
            metrics,
            schedule_path: summary.schedule_path,
            trace_path: summary.trace_path,
            error: summary.error,
        })
    }

    pub fn get_cell_schedule_path(
        &self,
        slug: &str,
        run_id: &str,
        cell_id: &str,
    ) -> ExperimentResult<PathBuf> {
        let p = self
            .run_dir(slug, run_id)
            .join(SCHEDULES_DIR)
            .join(format!("{cell_id}.json"));
        if !p.exists() {
            return Err(ExperimentError::NotFound(format!(
                "schedule for {cell_id} not found"
            )));
        }
        Ok(p)
    }

    pub fn get_cell_trace_path(
        &self,
        slug: &str,
        run_id: &str,
        cell_id: &str,
    ) -> ExperimentResult<PathBuf> {
        let p = self
            .run_dir(slug, run_id)
            .join(TRACES_DIR)
            .join(format!("{cell_id}.jsonl"));
        if !p.exists() {
            return Err(ExperimentError::NotFound(format!(
                "trace for {cell_id} not found"
            )));
        }
        Ok(p)
    }

    /// Stream a trace file into `out` (used by the HTTP handler to avoid
    /// loading whole files into memory).
    #[allow(dead_code)]
    pub fn copy_cell_trace<W: Write>(
        &self,
        slug: &str,
        run_id: &str,
        cell_id: &str,
        out: &mut W,
    ) -> ExperimentResult<u64> {
        let p = self.get_cell_trace_path(slug, run_id, cell_id)?;
        let mut file = fs::File::open(&p)?;
        Ok(std::io::copy(&mut file, out)?)
    }

    pub fn get_summary_csv_path(&self, slug: &str, run_id: &str) -> ExperimentResult<PathBuf> {
        let p = self.run_dir(slug, run_id).join(SUMMARY_FILE);
        if !p.exists() {
            return Err(ExperimentError::NotFound(format!(
                "summary.csv missing for {slug}/{run_id}"
            )));
        }
        Ok(p)
    }

    /// Bulk metric fetch (parallelized via rayon) — designed to solve the
    /// "webapp slow when uploading many schedules" pain point.
    pub fn bulk_metrics(
        &self,
        slug: &str,
        run_id: &str,
        cell_ids: &[String],
    ) -> ExperimentResult<Vec<(String, ExperimentResult<ScheduleMetrics>)>> {
        let metrics_dir = self.run_dir(slug, run_id).join(METRICS_DIR);
        if !metrics_dir.exists() {
            return Err(ExperimentError::NotFound(format!(
                "run {slug}/{run_id} not found"
            )));
        }
        let results: Vec<(String, ExperimentResult<ScheduleMetrics>)> = cell_ids
            .par_iter()
            .map(|id| {
                let p = metrics_dir.join(format!("{id}.json"));
                (id.clone(), read_metrics_file(&p))
            })
            .collect();
        Ok(results)
    }

    /// Pareto front over (x_field, y_field). `maximize_*` flips the
    /// preferred direction.
    pub fn pareto_front(
        &self,
        slug: &str,
        run_id: &str,
        x_field: &str,
        y_field: &str,
        maximize_x: bool,
        maximize_y: bool,
    ) -> ExperimentResult<Vec<ParetoPoint>> {
        let points = self.collect_xy(slug, run_id, x_field, y_field)?;
        Ok(compute_pareto(&points, maximize_x, maximize_y))
    }

    fn collect_xy(
        &self,
        slug: &str,
        run_id: &str,
        x_field: &str,
        y_field: &str,
    ) -> ExperimentResult<Vec<ParetoPoint>> {
        let metrics_dir = self.run_dir(slug, run_id).join(METRICS_DIR);
        if !metrics_dir.exists() {
            return Err(ExperimentError::NotFound(format!(
                "run {slug}/{run_id} not found"
            )));
        }
        let entries: Vec<_> = fs::read_dir(&metrics_dir)?
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .collect();
        let points: Vec<ParetoPoint> = entries
            .par_iter()
            .filter_map(|e| {
                let path = e.path();
                let cell_id = path.file_stem()?.to_string_lossy().to_string();
                let metrics = read_metrics_file(&path).ok()?;
                let x = extract_field(&metrics, x_field)?;
                let y = extract_field(&metrics, y_field)?;
                Some(ParetoPoint { cell_id, x, y })
            })
            .collect();
        Ok(points)
    }

    pub fn dataset_ranking(
        &self,
        slug: &str,
        run_id: &str,
        weights: Option<RankingWeights>,
    ) -> ExperimentResult<Vec<RankingEntry>> {
        self.ranking_by(slug, run_id, weights, RankBy::Dataset)
    }

    pub fn algorithm_ranking(
        &self,
        slug: &str,
        run_id: &str,
        weights: Option<RankingWeights>,
    ) -> ExperimentResult<Vec<RankingEntry>> {
        self.ranking_by(slug, run_id, weights, RankBy::Algorithm)
    }

    fn ranking_by(
        &self,
        slug: &str,
        run_id: &str,
        weights: Option<RankingWeights>,
        by: RankBy,
    ) -> ExperimentResult<Vec<RankingEntry>> {
        let cells = self.list_cells(slug, run_id, &CellListFilter::default())?;
        let metrics_dir = self.run_dir(slug, run_id).join(METRICS_DIR);

        // Read all metrics in parallel.
        let loaded: Vec<(CellSummary, ScheduleMetrics)> = cells
            .par_iter()
            .filter_map(|c| {
                let p = metrics_dir.join(format!("{}.json", c.cell_id));
                read_metrics_file(&p).ok().map(|m| (c.clone(), m))
            })
            .collect();

        let mut groups: HashMap<String, Vec<ScheduleMetrics>> = HashMap::new();
        for (cell, metrics) in loaded {
            let key = match by {
                RankBy::Dataset => cell.dataset_id.unwrap_or_else(|| "<unknown>".into()),
                RankBy::Algorithm => cell.algorithm.unwrap_or_else(|| "<unknown>".into()),
            };
            groups.entry(key).or_default().push(metrics);
        }

        let mut out: Vec<RankingEntry> = groups
            .into_iter()
            .map(|(key, items)| {
                let n = items.len() as f64;
                let mean = |f: fn(&ScheduleMetrics) -> f64| -> f64 {
                    if items.is_empty() {
                        0.0
                    } else {
                        items.iter().map(f).sum::<f64>() / n
                    }
                };
                let score = if let Some(w) = weights {
                    items.iter().map(|m| weighted_score(m, w)).sum::<f64>() / n.max(1.0)
                } else {
                    mean(|m| m.composite_rank_score)
                };
                RankingEntry {
                    key,
                    mean_score: score,
                    mean_completion: mean(|m| m.completion_ratio),
                    mean_priority_sum: mean(|m| m.priority.sum),
                    mean_utilization: mean(|m| m.utilization),
                    mean_fragmentation_index: mean(|m| m.fragmentation.fragmentation_index),
                    n: items.len(),
                }
            })
            .collect();
        out.sort_by(|a, b| {
            b.mean_score
                .partial_cmp(&a.mean_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(out)
    }

    pub fn run_dir(&self, slug: &str, run_id: &str) -> PathBuf {
        self.root.join(slug).join(run_id)
    }

    pub fn state_path(&self, slug: &str, run_id: &str) -> PathBuf {
        self.run_dir(slug, run_id).join(STATE_FILE)
    }
}

#[derive(Debug, Clone, Copy)]
enum RankBy {
    Dataset,
    Algorithm,
}

// ── helpers ─────────────────────────────────────────────────────────────────

fn read_manifest(run_dir: &Path) -> ExperimentResult<Value> {
    let p = run_dir.join(EXPERIMENT_FILE);
    if !p.exists() {
        return Err(ExperimentError::NotFound(format!(
            "manifest missing at {}",
            p.display()
        )));
    }
    let text = fs::read_to_string(&p)?;
    Ok(serde_json::from_str(&text)?)
}

fn read_metrics_file(path: &Path) -> ExperimentResult<ScheduleMetrics> {
    if !path.exists() {
        return Err(ExperimentError::NotFound(format!(
            "metrics file missing: {}",
            path.display()
        )));
    }
    let text = fs::read_to_string(path)?;
    let m: ScheduleMetrics = serde_json::from_str(&text)?;
    Ok(m)
}

fn cell_summary_from_manifest(cell: &Value, latest: &HashMap<String, &StateEvent>) -> CellSummary {
    let cell_id = cell
        .get("cell_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let dataset_id = cell
        .get("dataset_id")
        .and_then(|v| v.as_str())
        .map(String::from);
    let algorithm = cell
        .get("algorithm")
        .and_then(|v| v.as_str())
        .map(String::from);
    // run_config slug isn't in the manifest as a flat field; fall back to
    // splitting the cell_id at the last `__` delimiter.
    let config_slug = cell_id.rsplit_once("__").map(|(_, slug)| slug.to_string());

    let ev = latest.get(&cell_id).copied();
    CellSummary {
        cell_id,
        dataset_id,
        algorithm,
        config_slug,
        status: ev.map(|e| e.status),
        error: ev.and_then(|e| e.error.clone()),
        schedule_path: ev.and_then(|e| e.schedule_path.clone()),
        metrics_path: ev.and_then(|e| e.metrics_path.clone()),
        trace_path: ev.and_then(|e| e.trace_path.clone()),
        started_at: ev.map(|e| e.started_at.clone()),
        finished_at: ev.and_then(|e| e.finished_at.clone()),
    }
}

fn extract_field(metrics: &ScheduleMetrics, field: &str) -> Option<f64> {
    Some(match field {
        "scheduled_task_count" => metrics.scheduled_task_count as f64,
        "total_task_count" => metrics.total_task_count as f64,
        "completion_ratio" => metrics.completion_ratio,
        "priority_sum" => metrics.priority.sum,
        "priority_mean" => metrics.priority.mean,
        "priority_p50" => metrics.priority.p50,
        "priority_p90" => metrics.priority.p90,
        "fragmentation_index" => metrics.fragmentation.fragmentation_index,
        "fragmentation_gap_total_sec" => metrics.fragmentation.gap_total_sec,
        "fragmentation_largest_gap_sec" => metrics.fragmentation.largest_gap_sec,
        "fragmentation_gap_count" => metrics.fragmentation.gap_count as f64,
        "available_time_sec" => metrics.available_time_sec,
        "scheduled_time_sec" => metrics.scheduled_time_sec,
        "utilization" => metrics.utilization,
        "composite_rank_score" => metrics.composite_rank_score,
        _ => return None,
    })
}

fn weighted_score(m: &ScheduleMetrics, w: RankingWeights) -> f64 {
    // Same shape as scheduler::metrics composite (1 - fragmentation for
    // "lower is better"), normalized by total weight. Used for cross-cell
    // re-ranking inside an aggregate where local normalization isn't
    // available, so we simply combine the raw values.
    let total = w.total().max(1.0);
    (w.completion * m.completion_ratio
        + w.priority * m.priority.sum
        + w.utilization * m.utilization
        + w.fragmentation * (1.0 - m.fragmentation.fragmentation_index))
        / total
}

fn compute_pareto(points: &[ParetoPoint], max_x: bool, max_y: bool) -> Vec<ParetoPoint> {
    let dominates = |a: &ParetoPoint, b: &ParetoPoint| -> bool {
        let x_better = if max_x { a.x >= b.x } else { a.x <= b.x };
        let y_better = if max_y { a.y >= b.y } else { a.y <= b.y };
        let x_strict = if max_x { a.x > b.x } else { a.x < b.x };
        let y_strict = if max_y { a.y > b.y } else { a.y < b.y };
        x_better && y_better && (x_strict || y_strict)
    };
    let mut front = Vec::new();
    for p in points {
        if !points.iter().any(|q| dominates(q, p)) {
            front.push(p.clone());
        }
    }
    front.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));
    front
}

fn scan_root(root: &Path) -> ExperimentResult<Vec<ExperimentIndexEntry>> {
    let mut out = Vec::new();
    if !root.exists() {
        return Ok(out);
    }
    for slug_entry in fs::read_dir(root)? {
        let slug_entry = match slug_entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let slug_path = slug_entry.path();
        if !slug_path.is_dir() {
            continue;
        }
        let slug = slug_entry.file_name().to_string_lossy().to_string();
        if slug.starts_with('.') {
            continue;
        }
        for run_entry in fs::read_dir(&slug_path)? {
            let run_entry = match run_entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let run_path = run_entry.path();
            if !run_path.is_dir() {
                continue;
            }
            let run_id = run_entry.file_name().to_string_lossy().to_string();
            if !run_id.starts_with("run-") {
                continue;
            }
            // Only index dirs that look like a real experiment_matrix run
            // (manifest OR state stream present).
            let has_manifest = run_path.join(EXPERIMENT_FILE).exists();
            let has_state = run_path.join(STATE_FILE).exists();
            if !has_manifest && !has_state {
                continue;
            }
            let entry = build_index_entry(&slug, &run_id, &run_path);
            out.push(entry);
        }
    }
    out.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    Ok(out)
}

fn build_index_entry(slug: &str, run_id: &str, run_dir: &Path) -> ExperimentIndexEntry {
    let manifest = read_manifest(run_dir).ok();
    let experiment_name = manifest
        .as_ref()
        .and_then(|m| m.get("spec"))
        .and_then(|s| s.get("name"))
        .and_then(|n| n.as_str())
        .map(String::from)
        .unwrap_or_else(|| slug.to_string());

    let total_cells_from_manifest = manifest
        .as_ref()
        .and_then(|m| m.get("cells"))
        .and_then(|c| c.as_array())
        .map(|a| a.len());

    let events = state_events::read_events(&run_dir.join(STATE_FILE)).unwrap_or_default();
    let (started, completed, failed) = count_statuses(&events);
    let total_cells = total_cells_from_manifest.unwrap_or(started + completed + failed);

    let created_at = run_dir
        .metadata()
        .and_then(|m| m.created())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| format!("{}", d.as_secs()))
        .unwrap_or_else(|| run_id.to_string());
    let updated_at = run_dir
        .join(STATE_FILE)
        .metadata()
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| format!("{}", d.as_secs()))
        .unwrap_or_else(|| created_at.clone());

    let status = derive_status(total_cells, started, completed, failed, run_dir);

    ExperimentIndexEntry {
        experiment_slug: slug.to_string(),
        run_id: run_id.to_string(),
        experiment_name,
        output_dir: run_dir.to_path_buf(),
        created_at,
        updated_at,
        total_cells,
        completed_cells: completed,
        failed_cells: failed,
        running_cells: started,
        status,
    }
}

fn derive_status(
    total: usize,
    started: usize,
    completed: usize,
    failed: usize,
    run_dir: &Path,
) -> RunStatus {
    if total == 0 && started == 0 && completed == 0 && failed == 0 {
        return RunStatus::Pending;
    }
    let terminal = completed + failed;
    if started > 0 && terminal < total {
        return RunStatus::Running;
    }
    // No "running" cells. Decide done vs failed.
    if total > 0 && terminal >= total {
        if failed > 0 {
            return RunStatus::Failed;
        }
        return RunStatus::Completed;
    }
    // Heuristic fallback: presence of summary.csv ⇒ matrix finished.
    if run_dir.join(SUMMARY_FILE).exists() {
        if failed > 0 {
            RunStatus::Failed
        } else {
            RunStatus::Completed
        }
    } else {
        RunStatus::Running
    }
}

/// Standalone helper for unit tests / external callers that want to read
/// `summary.csv` raw rows without going through axum.
#[allow(dead_code)]
pub fn read_summary_csv_text(path: &Path) -> ExperimentResult<String> {
    let mut s = String::new();
    fs::File::open(path)?.read_to_string(&mut s)?;
    Ok(s)
}

/// Stream a file's content line-by-line (used by tests).
#[allow(dead_code)]
pub fn read_lines(path: &Path) -> ExperimentResult<Vec<String>> {
    let f = fs::File::open(path)?;
    let mut out = Vec::new();
    for line in BufReader::new(f).lines() {
        out.push(line?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pareto_minimize_y_maximize_x() {
        // For max_x + min_y, a Pareto-optimal point is one where no other
        // point has both larger-or-equal x AND smaller-or-equal y (with at
        // least one strict). The dataset below forms a real trade-off:
        // a, b, c sit on the front; d is dominated by a; e is dominated by b.
        let pts = vec![
            ParetoPoint {
                cell_id: "a".into(),
                x: 1.0,
                y: 1.0,
            },
            ParetoPoint {
                cell_id: "b".into(),
                x: 2.0,
                y: 2.0,
            },
            ParetoPoint {
                cell_id: "c".into(),
                x: 3.0,
                y: 4.0,
            },
            ParetoPoint {
                cell_id: "d".into(),
                x: 0.5,
                y: 3.0,
            },
            ParetoPoint {
                cell_id: "e".into(),
                x: 1.5,
                y: 2.5,
            },
        ];
        let front = compute_pareto(&pts, true, false);
        let ids: Vec<_> = front.iter().map(|p| p.cell_id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
    }

    #[test]
    fn pareto_drops_strictly_dominated() {
        let pts = vec![
            ParetoPoint {
                cell_id: "a".into(),
                x: 0.0,
                y: 0.0,
            },
            ParetoPoint {
                cell_id: "b".into(),
                x: 1.0,
                y: 1.0,
            },
        ];
        let front = compute_pareto(&pts, true, true);
        assert_eq!(front.len(), 1);
        assert_eq!(front[0].cell_id, "b");
    }
}
