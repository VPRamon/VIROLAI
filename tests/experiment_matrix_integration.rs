use csv::Reader;
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_experiment_matrix");

fn write_input(dir: &Path) {
    let json = r#"{
  "resources": [
    {
      "id": 0,
      "name": "test-site",
      "location": {
        "longitude_deg": -17.892,
        "latitude_deg": 28.762,
        "height_m": 2396.0
      }
    }
  ],
  "schedule_time_window": {
    "start_mjd_utc": 62000.0,
    "end_mjd_utc": 62001.0
  },
  "scheduling_blocks": [
    {
      "id": 1,
      "tasks": [
        {
          "id": 101,
          "name": "task-101",
          "requested_duration_sec": 1800.0,
          "target": { "ra_deg": 83.8, "dec_deg": 22.0 },
          "hard_constraints": {
            "time_window": {
              "start_mjd_utc": 62000.10,
              "end_mjd_utc": 62000.30
            }
          },
          "soft_constraints": { "priority": 10.0 }
        }
      ],
      "dependencies": []
    },
    {
      "id": 2,
      "tasks": [
        {
          "id": 102,
          "name": "task-102",
          "requested_duration_sec": 1800.0,
          "target": { "ra_deg": 84.8, "dec_deg": 21.0 },
          "hard_constraints": {
            "time_window": {
              "start_mjd_utc": 62000.35,
              "end_mjd_utc": 62000.55
            }
          },
          "soft_constraints": { "priority": 5.0 }
        }
      ],
      "dependencies": []
    }
  ]
}"#;
    fs::write(dir.join("input.json"), json).unwrap();
}

fn write_spec(base: &Path) -> PathBuf {
    let spec = r#"{
  "name": "matrix-smoke",
  "datasets": [
    { "id": "ds1", "path": "input.json" }
  ],
  "algorithms": [
    { "kind": "est", "axes": { "endangered_thresholds": [1], "k_beams": [1], "branching_factors": [1] } }
  ],
  "emit_trace": true,
  "max_parallel": 1,
  "output_dir": "out"
}"#;
    let p = base.join("spec.json");
    fs::write(&p, spec).unwrap();
    p
}

fn find_run_dir(out_root: &Path) -> PathBuf {
    let exp_dir = out_root.join("matrix-smoke");
    let entries: Vec<_> = fs::read_dir(&exp_dir)
        .expect("experiment dir should exist")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .collect();
    assert_eq!(entries.len(), 1, "expected exactly one run dir");
    entries[0].path()
}

#[test]
fn experiment_matrix_pipeline_writes_expected_artifacts() {
    let tmp = TempDir::new().unwrap();
    write_input(tmp.path());
    let spec_path = write_spec(tmp.path());

    let status = Command::new(BIN)
        .args(["--spec", spec_path.to_str().unwrap()])
        .status()
        .expect("binary should run");
    assert!(status.success(), "experiment_matrix exited with failure");

    let run_dir = find_run_dir(&tmp.path().join("out"));

    // Every expected artefact exists.
    assert!(run_dir.join("experiment.json").is_file());
    assert!(run_dir.join("state.jsonl").is_file());
    assert!(run_dir.join("summary.csv").is_file());
    let cell_id = "ds1__est__e1-k1-b1";
    assert!(
        run_dir
            .join("schedules")
            .join(format!("{cell_id}.json"))
            .is_file(),
        "schedule json missing"
    );
    assert!(
        run_dir
            .join("metrics")
            .join(format!("{cell_id}.json"))
            .is_file(),
        "metrics json missing"
    );

    // Summary CSV header + one row.
    let mut reader = Reader::from_path(run_dir.join("summary.csv")).unwrap();
    let headers: Vec<String> = reader
        .headers()
        .unwrap()
        .iter()
        .map(str::to_string)
        .collect();
    let expected_headers: Vec<&str> = vec![
        "cell_id",
        "dataset_id",
        "algorithm",
        "config_slug",
        "scheduled_task_count",
        "total_task_count",
        "completion_ratio",
        "priority_sum",
        "priority_min",
        "priority_max",
        "priority_mean",
        "priority_std",
        "priority_p25",
        "priority_p50",
        "priority_p75",
        "priority_p90",
        "fragmentation_gap_count",
        "fragmentation_gap_total_sec",
        "fragmentation_largest_gap_sec",
        "fragmentation_index",
        "total_horizon_sec",
        "available_time_sec",
        "scheduled_time_sec",
        "utilization",
        "composite_rank_score",
    ];
    assert_eq!(headers, expected_headers);
    let rows: Vec<_> = reader.records().map(|r| r.unwrap()).collect();
    assert_eq!(rows.len(), 1, "summary.csv should have one data row");
    assert_eq!(rows[0].get(0), Some(cell_id));

    // experiment.json round-trips and lists the cell.
    let manifest: Value =
        serde_json::from_str(&fs::read_to_string(run_dir.join("experiment.json")).unwrap())
            .unwrap();
    assert_eq!(
        manifest
            .get("spec")
            .and_then(|s| s.get("name"))
            .and_then(Value::as_str),
        Some("matrix-smoke")
    );
    let cells = manifest.get("cells").and_then(Value::as_array).unwrap();
    assert_eq!(cells.len(), 1);
    assert_eq!(
        cells[0].get("cell_id").and_then(Value::as_str),
        Some(cell_id)
    );
}

#[test]
fn experiment_matrix_resume_skips_completed_cells() {
    let tmp = TempDir::new().unwrap();
    write_input(tmp.path());
    let spec_path = write_spec(tmp.path());

    // First run: populate everything.
    let status = Command::new(BIN)
        .args(["--spec", spec_path.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(status.success());
    let run_dir = find_run_dir(&tmp.path().join("out"));

    let cell_id = "ds1__est__e1-k1-b1";
    let schedule_file = run_dir.join("schedules").join(format!("{cell_id}.json"));
    let mtime_before = fs::metadata(&schedule_file).unwrap().modified().unwrap();

    // Sleep a bit so any rewrite would change mtime.
    std::thread::sleep(std::time::Duration::from_millis(1100));

    // Resume into the same run dir; nothing should be re-scheduled.
    let status = Command::new(BIN)
        .args([
            "--spec",
            spec_path.to_str().unwrap(),
            "--resume",
            run_dir.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let mtime_after = fs::metadata(&schedule_file).unwrap().modified().unwrap();
    assert_eq!(
        mtime_before, mtime_after,
        "schedule file should not have been rewritten on resume"
    );

    // state.jsonl should still contain a single completed event for the cell
    // (no second `started`/`completed` pair appended).
    let state_text = fs::read_to_string(run_dir.join("state.jsonl")).unwrap();
    let lines: Vec<_> = state_text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect();
    let completed: HashSet<&str> = lines
        .iter()
        .filter(|l| l.contains("\"status\":\"completed\""))
        .copied()
        .collect();
    assert_eq!(
        completed.len(),
        1,
        "exactly one completed event expected after resume; got {lines:?}"
    );
}

#[test]
fn experiment_matrix_dry_run_emits_manifest_only() {
    let tmp = TempDir::new().unwrap();
    write_input(tmp.path());
    let spec_path = write_spec(tmp.path());

    let status = Command::new(BIN)
        .args(["--spec", spec_path.to_str().unwrap(), "--dry-run"])
        .status()
        .unwrap();
    assert!(status.success());

    let run_dir = find_run_dir(&tmp.path().join("out"));
    assert!(run_dir.join("experiment.json").is_file());
    assert!(!run_dir.join("summary.csv").exists());
    let schedules = run_dir.join("schedules");
    if schedules.exists() {
        assert_eq!(fs::read_dir(&schedules).unwrap().count(), 0);
    }
}
