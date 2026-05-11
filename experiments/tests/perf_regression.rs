//! Performance regression tests for scheduling algorithms.
//!
//! These tests run the `experiments` binary against the archived local
//! datasets and compare `ScheduleMetrics` values against committed golden
//! baselines. A regression failure means a key metric moved outside the
//! recorded tolerance band.
//!
//! # Fast vs slow tests
//!
//! All tests use a **10-day horizon override** so each EST cell finishes
//! in well under a second. HAP cells are more expensive (~20-60 s each) and
//! are therefore annotated with `#[ignore]`; run them explicitly with:
//!
//! ```text
//! cargo test --test perf_regression -- --include-ignored
//! ```
//!
//! # Regenerating baselines
//!
//! When an algorithm change intentionally improves metric values, update
//! the golden files by setting `UPDATE_PERF_BASELINES=1`:
//!
//! ```text
//! UPDATE_PERF_BASELINES=1 cargo test --test perf_regression -- --include-ignored
//! ```
//!
//! This re-runs every cell and overwrites `tests/perf_fixtures/golden/*.json`.
//!
//! # Data availability
//!
//! Tests require `data/local/` datasets to be present (checked out via
//! Git LFS). If a dataset file is missing the test is skipped with a
//! descriptive message rather than failing, so CI without LFS does not break.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;
use tempfile::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_experiments");

// ─── Path helpers ──────────────────────────────────────────────────────────

/// Root of the PhD repository.
///
/// `CARGO_MANIFEST_DIR` is `experiments/`, so the repository root is one
/// level up.
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("experiments must be inside the repo root")
        .to_path_buf()
}

/// Location of the golden baselines and spec fixtures bundled with this test
/// crate.
fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("perf_fixtures")
}

fn golden_dir() -> PathBuf {
    fixtures_dir().join("golden")
}

// ─── Golden file types ─────────────────────────────────────────────────────

/// Tolerances stored alongside each golden metric snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Tolerances {
    /// Maximum absolute deviation for `scheduled_task_count`.
    scheduled_task_count_abs: i64,
    completion_ratio_rel: f64,
    priority_sum_rel: f64,
    priority_p50_rel: f64,
    priority_p90_rel: f64,
    utilization_rel: f64,
    fragmentation_index_rel: f64,
    composite_rank_score_rel: f64,
}

/// A golden baseline file (`tests/perf_fixtures/golden/<cell_id>.json`).
#[derive(Debug, Serialize, Deserialize)]
struct Golden {
    cell_id: String,
    metrics: Value,
    tolerances: Tolerances,
}

// ─── Spec split into fast (EST) and slow (HAP) ─────────────────────────────

/// Spec for the fast (EST-only, 10-day horizon) regression cells.
const FAST_SPEC_JSON: &str = r#"{
  "name": "perf_regression_fast",
  "output_dir": "__PLACEHOLDER__",
  "emit_trace": false,
  "max_parallel": 4,
  "datasets": [
    {
      "id": "lst_sh",
      "path": "__REPO__/data/local/LST-SH/scheduling_problem.json",
      "label": "LST single-hemisphere (10-day window)",
      "horizon_override": { "start_mjd": 60341.0, "end_mjd": 60351.0 }
    },
    {
      "id": "cta_n",
      "path": "__REPO__/data/local/CTA-N/scheduling_problem.json",
      "label": "CTA North (10-day window)",
      "horizon_override": { "start_mjd": 61771.0, "end_mjd": 61781.0 }
    },
    {
      "id": "cta_s",
      "path": "__REPO__/data/local/CTA-S/scheduling_problem.json",
      "label": "CTA South (10-day window)",
      "horizon_override": { "start_mjd": 61771.0, "end_mjd": 61781.0 }
    }
  ],
  "algorithms": [
    {
      "kind": "est",
      "axes": {
        "endangered_thresholds": [1],
        "k_beams": [1, 3],
        "branching_factors": [1, 2]
      }
    }
  ],
  "ranking": { "completion": 1.0, "priority": 1.0, "utilization": 1.0, "fragmentation": 1.0 }
}"#;

/// Spec for the slow (HAP-only, 10-day horizon) regression cells.
const SLOW_SPEC_JSON: &str = r#"{
  "name": "perf_regression_slow",
  "output_dir": "__PLACEHOLDER__",
  "emit_trace": false,
  "max_parallel": 3,
  "datasets": [
    {
      "id": "lst_sh",
      "path": "__REPO__/data/local/LST-SH/scheduling_problem.json",
      "label": "LST single-hemisphere (10-day window)",
      "horizon_override": { "start_mjd": 60341.0, "end_mjd": 60351.0 }
    },
    {
      "id": "cta_n",
      "path": "__REPO__/data/local/CTA-N/scheduling_problem.json",
      "label": "CTA North (10-day window)",
      "horizon_override": { "start_mjd": 61771.0, "end_mjd": 61781.0 }
    },
    {
      "id": "cta_s",
      "path": "__REPO__/data/local/CTA-S/scheduling_problem.json",
      "label": "CTA South (10-day window)",
      "horizon_override": { "start_mjd": 61771.0, "end_mjd": 61781.0 }
    }
  ],
  "algorithms": [
    {
      "kind": "hap",
      "axes": {
        "iota_max_values": [64, 128],
        "rho_values": [3],
        "population_sizes": [4],
        "survivor_modes": ["elitist_top_k"],
        "survivor_caps": [4],
        "seeds": [0]
      }
    }
  ],
  "ranking": { "completion": 1.0, "priority": 1.0, "utilization": 1.0, "fragmentation": 1.0 }
}"#;

// ─── Shared run infrastructure ─────────────────────────────────────────────

/// Return value of a successfully-executed matrix run: metrics keyed by cell_id.
type MetricsMap = HashMap<String, Value>;

struct RunResult {
    _tmp: TempDir,
    metrics: MetricsMap,
}

fn run_spec(spec_template: &str) -> RunResult {
    let tmp = TempDir::new().expect("failed to create temp dir");
    let out_dir = tmp.path().to_str().unwrap().to_string();
    let repo = repo_root().to_str().unwrap().to_string();

    let spec_json = spec_template
        .replace("__PLACEHOLDER__", &out_dir)
        .replace("__REPO__", &repo);

    let spec_path = tmp.path().join("spec.json");
    std::fs::write(&spec_path, &spec_json).expect("failed to write spec");

    let output = Command::new(BIN)
        .args(["run", "--spec", spec_path.to_str().unwrap()])
        .env("RUST_LOG", "error")
        .output()
        .expect("failed to spawn experiments");

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!(
            "experiments failed (exit {:?}):\n{stderr}",
            output.status.code()
        );
    }

    let spec: Value = serde_json::from_str(&spec_json).unwrap();
    let exp_name = spec["name"].as_str().unwrap();
    let exp_dir = tmp.path().join(exp_name);
    let run_dirs: Vec<_> = std::fs::read_dir(&exp_dir)
        .expect("experiment output dir missing")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .collect();
    assert_eq!(run_dirs.len(), 1, "expected exactly one run dir");
    let run_dir = run_dirs[0].path();
    let metrics_dir = run_dir.join("metrics");

    let mut metrics = HashMap::new();
    for entry in std::fs::read_dir(&metrics_dir).expect("metrics dir missing") {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("json") {
            let cell_id = path.file_stem().unwrap().to_str().unwrap().to_string();
            let text = std::fs::read_to_string(&path).unwrap();
            let m: Value = serde_json::from_str(&text).unwrap();
            metrics.insert(cell_id, m);
        }
    }

    RunResult { _tmp: tmp, metrics }
}

// Global shared results — the binary is spawned once per group to avoid
// repeatedly loading and prescheduling the large dataset files.

static FAST_RESULT: OnceLock<MetricsMap> = OnceLock::new();
static SLOW_RESULT: OnceLock<MetricsMap> = OnceLock::new();

fn fast_metrics() -> &'static MetricsMap {
    FAST_RESULT.get_or_init(|| {
        let r = run_spec(FAST_SPEC_JSON);
        // Leak TempDir so the output directory outlives the tests.
        Box::leak(Box::new(r._tmp));
        r.metrics
    })
}

fn slow_metrics() -> &'static MetricsMap {
    SLOW_RESULT.get_or_init(|| {
        let r = run_spec(SLOW_SPEC_JSON);
        Box::leak(Box::new(r._tmp));
        r.metrics
    })
}

// ─── Comparison engine ─────────────────────────────────────────────────────

fn dataset_present(dataset_id: &str) -> bool {
    let rel = match dataset_id {
        "lst_sh" => "data/local/LST-SH/scheduling_problem.json",
        "cta_n" => "data/local/CTA-N/scheduling_problem.json",
        "cta_s" => "data/local/CTA-S/scheduling_problem.json",
        other => panic!("unknown dataset id: {other}"),
    };
    repo_root().join(rel).exists()
}

/// Assert that `actual` is within the golden tolerance of `expected`.
fn assert_within_golden(cell_id: &str, actual: &Value) {
    let golden_path = golden_dir().join(format!("{cell_id}.json"));

    if std::env::var("UPDATE_PERF_BASELINES").as_deref() == Ok("1") {
        let existing: Golden = serde_json::from_str(
            &std::fs::read_to_string(&golden_path)
                .unwrap_or_else(|_| panic!("golden file missing for {cell_id}")),
        )
        .unwrap_or_else(|_| panic!("invalid golden file for {cell_id}"));

        let updated = Golden {
            cell_id: cell_id.to_string(),
            metrics: actual.clone(),
            tolerances: existing.tolerances,
        };
        std::fs::write(
            &golden_path,
            serde_json::to_string_pretty(&updated).unwrap(),
        )
        .unwrap_or_else(|e| panic!("failed to write golden {cell_id}: {e}"));
        println!("  [bless] updated golden for {cell_id}");
        return;
    }

    let golden_text = std::fs::read_to_string(&golden_path)
        .unwrap_or_else(|_| panic!("golden file not found: {}", golden_path.display()));
    let golden: Golden =
        serde_json::from_str(&golden_text).unwrap_or_else(|e| panic!("bad golden JSON: {e}"));

    let g = &golden.metrics;
    let t = &golden.tolerances;
    let mut failures = Vec::new();

    check_metric_abs(
        "scheduled_task_count",
        get_i64(g, &["scheduled_task_count"]),
        get_i64(actual, &["scheduled_task_count"]),
        t.scheduled_task_count_abs,
        &mut failures,
    );
    check_metric_rel(
        "completion_ratio",
        get_f64(g, &["completion_ratio"]),
        get_f64(actual, &["completion_ratio"]),
        t.completion_ratio_rel,
        &mut failures,
    );
    check_metric_rel(
        "priority.sum",
        get_f64(g, &["priority", "sum"]),
        get_f64(actual, &["priority", "sum"]),
        t.priority_sum_rel,
        &mut failures,
    );
    check_metric_rel(
        "priority.p50",
        get_f64(g, &["priority", "p50"]),
        get_f64(actual, &["priority", "p50"]),
        t.priority_p50_rel,
        &mut failures,
    );
    check_metric_rel(
        "priority.p90",
        get_f64(g, &["priority", "p90"]),
        get_f64(actual, &["priority", "p90"]),
        t.priority_p90_rel,
        &mut failures,
    );
    check_metric_rel(
        "utilization",
        get_f64(g, &["utilization"]),
        get_f64(actual, &["utilization"]),
        t.utilization_rel,
        &mut failures,
    );
    check_metric_rel(
        "fragmentation_index",
        get_f64(g, &["fragmentation", "fragmentation_index"]),
        get_f64(actual, &["fragmentation", "fragmentation_index"]),
        t.fragmentation_index_rel,
        &mut failures,
    );
    check_metric_rel(
        "composite_rank_score",
        get_f64(g, &["composite_rank_score"]),
        get_f64(actual, &["composite_rank_score"]),
        t.composite_rank_score_rel,
        &mut failures,
    );

    if !failures.is_empty() {
        panic!(
            "regression detected for {cell_id}:\n{}",
            failures.join("\n")
        );
    }
}

// ─── Metric helpers ────────────────────────────────────────────────────────

fn check_metric_rel(label: &str, expected: f64, actual: f64, rel: f64, failures: &mut Vec<String>) {
    if expected == 0.0 {
        if actual.abs() > 1e-9 {
            failures.push(format!("  {label}: expected 0.0, got {actual:.6}"));
        }
        return;
    }
    let deviation = ((actual - expected) / expected).abs();
    if deviation > rel {
        failures.push(format!(
            "  {label}: expected {expected:.6}, got {actual:.6} (deviation {:.1}%, tolerance {:.1}%)",
            deviation * 100.0,
            rel * 100.0,
        ));
    }
}

fn check_metric_abs(label: &str, expected: i64, actual: i64, abs: i64, failures: &mut Vec<String>) {
    let deviation = (actual - expected).unsigned_abs() as i64;
    if deviation > abs {
        failures.push(format!(
            "  {label}: expected {expected}, got {actual} (|deviation| {deviation}, tolerance {abs})"
        ));
    }
}

fn get_f64(v: &Value, path: &[&str]) -> f64 {
    let mut cur = v;
    for &key in path {
        cur = &cur[key];
    }
    cur.as_f64()
        .unwrap_or_else(|| panic!("missing f64 at {path:?}"))
}

fn get_i64(v: &Value, path: &[&str]) -> i64 {
    let mut cur = v;
    for &key in path {
        cur = &cur[key];
    }
    cur.as_i64()
        .unwrap_or_else(|| panic!("missing i64 at {path:?}"))
}

// ─── Cell check helpers ────────────────────────────────────────────────────

/// Check a single fast (EST) cell.
fn check_fast_cell(cell_id: &str, dataset_id: &str) {
    if !dataset_present(dataset_id) {
        println!("SKIP {cell_id}: dataset '{dataset_id}' not present (run `git lfs pull`)");
        return;
    }
    let metrics = fast_metrics();
    let actual = metrics
        .get(cell_id)
        .unwrap_or_else(|| panic!("cell '{cell_id}' not found in fast run output"));
    assert_within_golden(cell_id, actual);
}

/// Check a single slow (HAP) cell.
fn check_slow_cell(cell_id: &str, dataset_id: &str) {
    if !dataset_present(dataset_id) {
        println!("SKIP {cell_id}: dataset '{dataset_id}' not present (run `git lfs pull`)");
        return;
    }
    let metrics = slow_metrics();
    let actual = metrics
        .get(cell_id)
        .unwrap_or_else(|| panic!("cell '{cell_id}' not found in slow run output"));
    assert_within_golden(cell_id, actual);
}

// ─── Fast tests: EST (12 cells, always run) ───────────────────────────────

#[test]
fn perf_lst_sh_est_e1_k1_b1() {
    check_fast_cell("lst_sh__est__e1-k1-b1", "lst_sh");
}

#[test]
fn perf_lst_sh_est_e1_k1_b2() {
    check_fast_cell("lst_sh__est__e1-k1-b2", "lst_sh");
}

#[test]
fn perf_lst_sh_est_e1_k3_b1() {
    check_fast_cell("lst_sh__est__e1-k3-b1", "lst_sh");
}

#[test]
fn perf_lst_sh_est_e1_k3_b2() {
    check_fast_cell("lst_sh__est__e1-k3-b2", "lst_sh");
}

#[test]
fn perf_cta_n_est_e1_k1_b1() {
    check_fast_cell("cta_n__est__e1-k1-b1", "cta_n");
}

#[test]
fn perf_cta_n_est_e1_k1_b2() {
    check_fast_cell("cta_n__est__e1-k1-b2", "cta_n");
}

#[test]
fn perf_cta_n_est_e1_k3_b1() {
    check_fast_cell("cta_n__est__e1-k3-b1", "cta_n");
}

#[test]
fn perf_cta_n_est_e1_k3_b2() {
    check_fast_cell("cta_n__est__e1-k3-b2", "cta_n");
}

#[test]
fn perf_cta_s_est_e1_k1_b1() {
    check_fast_cell("cta_s__est__e1-k1-b1", "cta_s");
}

#[test]
fn perf_cta_s_est_e1_k1_b2() {
    check_fast_cell("cta_s__est__e1-k1-b2", "cta_s");
}

#[test]
fn perf_cta_s_est_e1_k3_b1() {
    check_fast_cell("cta_s__est__e1-k3-b1", "cta_s");
}

#[test]
fn perf_cta_s_est_e1_k3_b2() {
    check_fast_cell("cta_s__est__e1-k3-b2", "cta_s");
}

// ─── Slow tests: HAP (6 cells, #[ignore] by default) ──────────────────────

#[test]
#[ignore = "slow (~20-60 s per cell): run with `cargo test --test perf_regression -- --include-ignored`"]
fn perf_lst_sh_hap_i64() {
    check_slow_cell("lst_sh__hap__hap-i64-r3-p4-elitist4-s0", "lst_sh");
}

#[test]
#[ignore = "slow (~20-60 s per cell): run with `cargo test --test perf_regression -- --include-ignored`"]
fn perf_lst_sh_hap_i128() {
    check_slow_cell("lst_sh__hap__hap-i128-r3-p4-elitist4-s0", "lst_sh");
}

#[test]
#[ignore = "slow (~20-60 s per cell): run with `cargo test --test perf_regression -- --include-ignored`"]
fn perf_cta_n_hap_i64() {
    check_slow_cell("cta_n__hap__hap-i64-r3-p4-elitist4-s0", "cta_n");
}

#[test]
#[ignore = "slow (~20-60 s per cell): run with `cargo test --test perf_regression -- --include-ignored`"]
fn perf_cta_n_hap_i128() {
    check_slow_cell("cta_n__hap__hap-i128-r3-p4-elitist4-s0", "cta_n");
}

#[test]
#[ignore = "slow (~20-60 s per cell): run with `cargo test --test perf_regression -- --include-ignored`"]
fn perf_cta_s_hap_i64() {
    check_slow_cell("cta_s__hap__hap-i64-r3-p4-elitist4-s0", "cta_s");
}

#[test]
#[ignore = "slow (~20-60 s per cell): run with `cargo test --test perf_regression -- --include-ignored`"]
fn perf_cta_s_hap_i128() {
    check_slow_cell("cta_s__hap__hap-i128-r3-p4-elitist4-s0", "cta_s");
}
