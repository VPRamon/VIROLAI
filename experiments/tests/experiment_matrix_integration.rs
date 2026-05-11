use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_experiments");

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
        .args(["run", "--spec", spec_path.to_str().unwrap()])
        .status()
        .expect("binary should run");
    assert!(status.success(), "experiments exited with failure");

    let run_dir = find_run_dir(&tmp.path().join("out"));

    // Every expected artefact exists.
    assert!(run_dir.join("experiment.json").is_file());
    assert!(run_dir.join("state.jsonl").is_file());
    let cell_id = "ds1__est__e1-k1-b1";
    assert!(
        run_dir
            .join("schedules")
            .join(format!("{cell_id}.json"))
            .is_file(),
        "schedule json missing"
    );
    // Metrics are embedded in the schedule JSON; no separate metrics/ dir.
    assert!(
        !run_dir.join("metrics").exists(),
        "metrics dir should not exist"
    );
    assert!(
        !run_dir.join("summary.csv").exists(),
        "summary.csv should not be written"
    );
    assert!(
        !run_dir.join("traces").exists(),
        "traces dir should not exist"
    );

    // The schedule JSON carries embedded schedule_metrics.
    let schedule_val: Value = serde_json::from_str(
        &fs::read_to_string(
            run_dir
                .join("schedules")
                .join(format!("{cell_id}.json")),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(
        schedule_val.get("schedule_metrics").is_some(),
        "schedule JSON must carry embedded schedule_metrics"
    );

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
        .args(["run", "--spec", spec_path.to_str().unwrap()])
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
            "run",
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
        .args(["run", "--spec", spec_path.to_str().unwrap(), "--dry-run"])
        .status()
        .unwrap();
    assert!(status.success());

    let run_dir = find_run_dir(&tmp.path().join("out"));
    assert!(run_dir.join("experiment.json").is_file());
    assert!(
        !run_dir.join("state.jsonl").exists(),
        "dry-run should not write state.jsonl"
    );
    let schedules = run_dir.join("schedules");
    if schedules.exists() {
        assert_eq!(fs::read_dir(&schedules).unwrap().count(), 0);
    }
}

#[test]
fn experiment_matrix_no_state_skips_state_file_and_emits_progress() {
    let tmp = TempDir::new().unwrap();
    write_input(tmp.path());
    let spec_path = write_spec(tmp.path());

    let output = Command::new(BIN)
        .args(["run", "--spec", spec_path.to_str().unwrap(), "--no-state"])
        .output()
        .expect("binary should run");
    assert!(output.status.success(), "experiments exited with failure");

    let run_dir = find_run_dir(&tmp.path().join("out"));

    // With --no-state no state.jsonl is written.
    assert!(
        !run_dir.join("state.jsonl").exists(),
        "state.jsonl must not be written when --no-state is set"
    );

    // Schedules are still produced.
    let cell_id = "ds1__est__e1-k1-b1";
    assert!(
        run_dir
            .join("schedules")
            .join(format!("{cell_id}.json"))
            .is_file(),
        "schedule json missing under --no-state"
    );

    // Progress lines appear on stderr.
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains('✓') || stderr.contains("✗") || stderr.contains('▶'),
        "expected progress characters on stderr; got: {stderr}"
    );
}

#[test]
fn experiment_matrix_no_state_and_resume_is_an_error() {
    let tmp = TempDir::new().unwrap();
    write_input(tmp.path());
    let spec_path = write_spec(tmp.path());

    // First run to create a run_dir.
    let status = Command::new(BIN)
        .args(["run", "--spec", spec_path.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(status.success());
    let run_dir = find_run_dir(&tmp.path().join("out"));

    // --no-state + --resume must fail.
    let status = Command::new(BIN)
        .args([
            "run",
            "--spec",
            spec_path.to_str().unwrap(),
            "--resume",
            run_dir.to_str().unwrap(),
            "--no-state",
        ])
        .status()
        .unwrap();
    assert!(
        !status.success(),
        "--no-state and --resume together should exit non-zero"
    );
}
