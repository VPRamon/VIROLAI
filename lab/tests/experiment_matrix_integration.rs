//! Integration tests for the DB-only `lab run` pipeline.
//!
//! Every run stores results in SQLite — no filesystem artifacts are produced
//! (`experiment.json`, `state.jsonl`, `schedules/` are all gone).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_lab");

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
  "max_parallel": 1
}"#;
    let p = base.join("spec.json");
    fs::write(&p, spec).unwrap();
    p
}

/// First run executes the cell and inserts a DB row with schedule_json and metrics.
#[test]
fn run_creates_db_row_with_metrics_and_schedule_json() {
    let tmp = TempDir::new().unwrap();
    write_input(tmp.path());
    let spec_path = write_spec(tmp.path());
    let db_path = tmp.path().join("runs.sqlite");

    let output = Command::new(BIN)
        .args([
            "run",
            "--spec",
            spec_path.to_str().unwrap(),
            "--run-db",
            db_path.to_str().unwrap(),
        ])
        .output()
        .expect("lab binary should run");

    assert!(
        output.status.success(),
        "lab exited with failure:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    // SQLite registry was created.
    assert!(db_path.exists(), "registry database must exist after run");

    // Summary line reports 1 completed.
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("1 completed"),
        "expected '1 completed' in stdout; got: {stdout}"
    );
    assert!(
        stdout.contains("0 failed"),
        "expected '0 failed' in stdout; got: {stdout}"
    );

    // The DB row has schedule_json — verify via `registry list --format json`.
    let list_out = Command::new(BIN)
        .args([
            "registry",
            "list",
            "--run-db",
            db_path.to_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .expect("registry list should succeed");
    assert!(
        list_out.status.success(),
        "registry list failed: {}",
        String::from_utf8_lossy(&list_out.stderr)
    );

    let rows: serde_json::Value =
        serde_json::from_slice(&list_out.stdout).expect("registry list must produce JSON");
    assert_eq!(rows.as_array().map(|a| a.len()).unwrap_or(0), 1);
    let row = &rows[0];
    assert_eq!(row["dataset_id"].as_str(), Some("ds1"));
    assert_eq!(row["algorithm"].as_str(), Some("est"));

    // Verify schedule_json was stored by exporting the run.
    let run_key = row["run_key"].as_str().expect("run_key must be a string");
    let export_file = tmp.path().join("verify.json");
    let export_status = Command::new(BIN)
        .args([
            "registry",
            "export",
            "--run",
            run_key,
            "--out",
            export_file.to_str().unwrap(),
            "--run-db",
            db_path.to_str().unwrap(),
        ])
        .status()
        .expect("registry export should succeed");
    assert!(
        export_status.success(),
        "registry export failed — schedule_json was not stored"
    );
    assert!(export_file.exists(), "exported schedule file must exist");
    let exported_val: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&export_file).unwrap())
            .expect("exported file must be valid JSON");
    assert!(
        exported_val.get("schedule_metadata").is_some(),
        "exported schedule must contain schedule_metadata"
    );
}

/// No filesystem artifacts are produced by `lab run`.
#[test]
fn run_produces_no_filesystem_artifacts() {
    let tmp = TempDir::new().unwrap();
    write_input(tmp.path());
    let spec_path = write_spec(tmp.path());
    let db_path = tmp.path().join("runs.sqlite");

    let status = Command::new(BIN)
        .args([
            "run",
            "--spec",
            spec_path.to_str().unwrap(),
            "--run-db",
            db_path.to_str().unwrap(),
        ])
        .status()
        .expect("lab binary should run");
    assert!(status.success());

    // None of the old filesystem artifacts should exist.
    let entries: Vec<_> = fs::read_dir(tmp.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();

    assert!(
        !entries.iter().any(|n| n.ends_with(".jsonl")),
        "state.jsonl must not be written; found: {entries:?}"
    );
    assert!(
        !entries.contains(&"schedules".to_string()),
        "schedules/ directory must not be created; found: {entries:?}"
    );
    assert!(
        !entries.iter().any(|n| n == "experiment.json"),
        "experiment.json must not be created; found: {entries:?}"
    );
}

/// Second run with the same spec skips already-present DB rows.
#[test]
fn run_second_time_skips_existing_rows() {
    let tmp = TempDir::new().unwrap();
    write_input(tmp.path());
    let spec_path = write_spec(tmp.path());
    let db_path = tmp.path().join("runs.sqlite");

    // First run — populates registry.
    let s1 = Command::new(BIN)
        .args([
            "run",
            "--spec",
            spec_path.to_str().unwrap(),
            "--run-db",
            db_path.to_str().unwrap(),
        ])
        .status()
        .expect("first run should succeed");
    assert!(s1.success(), "first run failed");

    // Second run — should skip.
    let out2 = Command::new(BIN)
        .args([
            "run",
            "--spec",
            spec_path.to_str().unwrap(),
            "--run-db",
            db_path.to_str().unwrap(),
        ])
        .output()
        .expect("second run should succeed");
    assert!(
        out2.status.success(),
        "second run failed: {}",
        String::from_utf8_lossy(&out2.stderr)
    );

    let stdout2 = String::from_utf8_lossy(&out2.stdout);
    assert!(
        stdout2.contains("1 skipped"),
        "expected '1 skipped' on second run; got: {stdout2}"
    );
    assert!(
        stdout2.contains("0 completed"),
        "expected '0 completed' on second run; got: {stdout2}"
    );
}

/// `--override` re-executes cells that are already in the DB and updates them.
#[test]
fn run_override_reruns_and_updates_row() {
    let tmp = TempDir::new().unwrap();
    write_input(tmp.path());
    let spec_path = write_spec(tmp.path());
    let db_path = tmp.path().join("runs.sqlite");

    // First run.
    let s1 = Command::new(BIN)
        .args([
            "run",
            "--spec",
            spec_path.to_str().unwrap(),
            "--run-db",
            db_path.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(s1.success(), "first run failed");

    // Sleep a bit to ensure last_seen_at changes.
    std::thread::sleep(std::time::Duration::from_millis(1100));

    // Second run with --override.
    let out2 = Command::new(BIN)
        .args([
            "run",
            "--spec",
            spec_path.to_str().unwrap(),
            "--run-db",
            db_path.to_str().unwrap(),
            "--override",
        ])
        .output()
        .expect("override run should succeed");
    assert!(
        out2.status.success(),
        "override run failed: {}",
        String::from_utf8_lossy(&out2.stderr)
    );

    let stdout2 = String::from_utf8_lossy(&out2.stdout);
    assert!(
        stdout2.contains("1 overridden") || stdout2.contains("1 completed"),
        "expected overridden or completed count > 0 on --override run; got: {stdout2}"
    );
    assert!(
        stdout2.contains("0 skipped"),
        "expected '0 skipped' on --override run; got: {stdout2}"
    );
}

/// Spec that still carries an `output_dir` field parses without error.
#[test]
fn spec_with_output_dir_field_is_accepted() {
    let tmp = TempDir::new().unwrap();
    write_input(tmp.path());
    let db_path = tmp.path().join("runs.sqlite");

    // Old-style spec with output_dir still present.
    let spec = r#"{
  "name": "compat-smoke",
  "datasets": [
    { "id": "ds1", "path": "input.json" }
  ],
  "algorithms": [
    { "kind": "est", "axes": { "endangered_thresholds": [1], "k_beams": [1], "branching_factors": [1] } }
  ],
  "max_parallel": 1,
  "output_dir": "some_old_dir"
}"#;
    let spec_path = tmp.path().join("spec_compat.json");
    fs::write(&spec_path, spec).unwrap();

    let status = Command::new(BIN)
        .args([
            "run",
            "--spec",
            spec_path.to_str().unwrap(),
            "--run-db",
            db_path.to_str().unwrap(),
        ])
        .status()
        .expect("lab binary should run");
    assert!(
        status.success(),
        "spec with output_dir field must still be accepted"
    );
}
