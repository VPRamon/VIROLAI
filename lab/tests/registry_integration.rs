//! Integration tests for the SQLite run registry and cache-mode runner.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_lab");

// ── Shared fixture helpers ────────────────────────────────────────────────────

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
    }
  ]
}"#;
    fs::write(dir.join("input.json"), json).unwrap();
}

fn write_spec(base: &Path) -> PathBuf {
    let spec = r#"{
  "name": "cache-smoke",
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
    let exp_dir = out_root.join("cache-smoke");
    let entries: Vec<_> = fs::read_dir(&exp_dir)
        .expect("experiment dir should exist")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .collect();
    assert_eq!(entries.len(), 1, "expected exactly one run dir");
    entries[0].path()
}

// ── Registry cache tests ──────────────────────────────────────────────────────

/// First cache-enabled run executes the cell and inserts a record.
#[test]
fn cache_first_run_executes_and_inserts() {
    let tmp = TempDir::new().unwrap();
    write_input(tmp.path());
    let spec_path = write_spec(tmp.path());
    let db_path = tmp.path().join("runs.sqlite");

    let output = Command::new(BIN)
        .args([
            "run",
            "--spec",
            spec_path.to_str().unwrap(),
            "--cache",
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

    // Registry file was created.
    assert!(db_path.exists(), "registry database should exist after run");

    // Schedule file was written.
    let run_dir = find_run_dir(&tmp.path().join("out"));
    let cell_id = "ds1__est__e1-k1-b1";
    assert!(
        run_dir
            .join("schedules")
            .join(format!("{cell_id}.json"))
            .is_file(),
        "schedule json missing after first cache run"
    );

    // Summary line reports 0 registry hits, 1 completed.
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("1 completed"),
        "expected 1 completed in stdout; got: {stdout}"
    );
    assert!(
        stdout.contains("0 registry hits"),
        "expected 0 registry hits in stdout; got: {stdout}"
    );
}

/// Second cache-enabled run with the same spec reports registry hits and does
/// not write new schedule files.
#[test]
fn cache_second_run_reports_hits_and_no_schedule_write() {
    let tmp = TempDir::new().unwrap();
    write_input(tmp.path());
    let spec_path = write_spec(tmp.path());
    let db_path = tmp.path().join("runs.sqlite");

    // First run — populates registry.
    let status = Command::new(BIN)
        .args([
            "run",
            "--spec",
            spec_path.to_str().unwrap(),
            "--cache",
            "--run-db",
            db_path.to_str().unwrap(),
        ])
        .status()
        .expect("first run should succeed");
    assert!(status.success(), "first cache run failed");

    let first_run_dir = find_run_dir(&tmp.path().join("out"));
    let cell_id = "ds1__est__e1-k1-b1";
    let schedule_path = first_run_dir
        .join("schedules")
        .join(format!("{cell_id}.json"));

    // Record mtime before second run.
    let mtime_before = fs::metadata(&schedule_path)
        .ok()
        .and_then(|m| m.modified().ok());

    // Second run against the same spec — should be a registry hit.
    // Use a new output dir to avoid resume-mode interfering.
    let spec2 = {
        let spec_text = fs::read_to_string(&spec_path).unwrap();
        let spec_text2 = spec_text.replace("\"out\"", "\"out2\"");
        let p = tmp.path().join("spec2.json");
        fs::write(&p, spec_text2).unwrap();
        p
    };

    let output2 = Command::new(BIN)
        .args([
            "run",
            "--spec",
            spec2.to_str().unwrap(),
            "--cache",
            "--run-db",
            db_path.to_str().unwrap(),
        ])
        .output()
        .expect("second cache run should complete");

    assert!(
        output2.status.success(),
        "second cache run failed:\n{}",
        String::from_utf8_lossy(&output2.stderr)
    );

    let stdout2 = String::from_utf8_lossy(&output2.stdout);
    assert!(
        stdout2.contains("1 registry hits"),
        "expected 1 registry hits in second run; got: {stdout2}"
    );

    // The original schedule file must not have been overwritten.
    let mtime_after = fs::metadata(&schedule_path)
        .ok()
        .and_then(|m| m.modified().ok());
    assert_eq!(
        mtime_before, mtime_after,
        "schedule file should not be touched by cache hit"
    );

    // Under the second output dir, no schedule json should be written for
    // the hit cell.
    let out2_exp = tmp.path().join("out2").join("cache-smoke");
    if out2_exp.exists() {
        let run_dirs: Vec<_> = fs::read_dir(&out2_exp)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .collect();
        for rd in run_dirs {
            let sched = rd.path().join("schedules").join(format!("{cell_id}.json"));
            assert!(
                !sched.exists(),
                "schedule should NOT be written for a registry cache hit"
            );
        }
    }
}

// ── Regeneration test ─────────────────────────────────────────────────────────

/// `lab registry regenerate` reconstructs a schedule JSON from stored identity
/// and the result contains `schedule_metadata` and `schedule_metrics`.
#[test]
fn registry_regenerate_produces_valid_schedule() {
    let tmp = TempDir::new().unwrap();
    write_input(tmp.path());
    let spec_path = write_spec(tmp.path());
    let db_path = tmp.path().join("runs.sqlite");

    // Populate the registry.
    let status = Command::new(BIN)
        .args([
            "run",
            "--spec",
            spec_path.to_str().unwrap(),
            "--cache",
            "--run-db",
            db_path.to_str().unwrap(),
        ])
        .status()
        .expect("cache run should succeed");
    assert!(status.success(), "cache run to populate registry failed");

    // Fetch the run key via `lab registry list --format json`.
    let list_output = Command::new(BIN)
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
        list_output.status.success(),
        "registry list failed:\n{}",
        String::from_utf8_lossy(&list_output.stderr)
    );

    let list_json: serde_json::Value =
        serde_json::from_slice(&list_output.stdout).expect("registry list should produce JSON");
    let run_key = list_json[0]["run_key"]
        .as_str()
        .expect("run_key must be a string");

    // Regenerate the schedule.
    let out_file = tmp.path().join("regenerated.json");
    let regen_status = Command::new(BIN)
        .args([
            "registry",
            "regenerate",
            "--run",
            run_key,
            "--out",
            out_file.to_str().unwrap(),
            "--run-db",
            db_path.to_str().unwrap(),
        ])
        .status()
        .expect("registry regenerate should succeed");
    assert!(
        regen_status.success(),
        "registry regenerate exited with failure"
    );

    assert!(out_file.exists(), "regenerated schedule file must exist");

    let content = fs::read_to_string(&out_file).unwrap();
    let v: serde_json::Value = serde_json::from_str(&content).expect("regenerated output is JSON");
    assert!(
        v.get("schedule_metadata").is_some(),
        "regenerated schedule must contain schedule_metadata"
    );
    assert!(
        v.get("schedule_metrics").is_some(),
        "regenerated schedule must contain schedule_metrics"
    );
}

// ── `lab registry inspect` ────────────────────────────────────────────────────

#[test]
fn registry_inspect_shows_identity_and_metrics() {
    let tmp = TempDir::new().unwrap();
    write_input(tmp.path());
    let spec_path = write_spec(tmp.path());
    let db_path = tmp.path().join("runs.sqlite");

    let status = Command::new(BIN)
        .args([
            "run",
            "--spec",
            spec_path.to_str().unwrap(),
            "--cache",
            "--run-db",
            db_path.to_str().unwrap(),
        ])
        .status()
        .expect("cache run should succeed");
    assert!(status.success());

    // Get key from JSON list.
    let list_output = Command::new(BIN)
        .args([
            "registry",
            "list",
            "--run-db",
            db_path.to_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    let list_json: serde_json::Value = serde_json::from_slice(&list_output.stdout).unwrap();
    let run_key = list_json[0]["run_key"].as_str().unwrap();
    let prefix = &run_key[..12];

    let inspect_output = Command::new(BIN)
        .args([
            "registry",
            "inspect",
            "--run",
            prefix,
            "--run-db",
            db_path.to_str().unwrap(),
        ])
        .output()
        .expect("registry inspect should succeed");

    assert!(
        inspect_output.status.success(),
        "registry inspect failed:\n{}",
        String::from_utf8_lossy(&inspect_output.stderr)
    );

    let stdout = String::from_utf8_lossy(&inspect_output.stdout);
    assert!(
        stdout.contains("dataset_id:"),
        "inspect must show dataset_id"
    );
    assert!(stdout.contains("algorithm:"), "inspect must show algorithm");
    assert!(
        stdout.contains("--- identity ---"),
        "inspect must show identity section"
    );
    assert!(
        stdout.contains("--- metrics ---"),
        "inspect must show metrics section"
    );
}
