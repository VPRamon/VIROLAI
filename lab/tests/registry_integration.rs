//! Integration tests for the SQLite run registry and DB-only runner.

use rusqlite::{Connection, params};
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
  "name": "registry-smoke",
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

// ── Run and skip tests ────────────────────────────────────────────────────────

/// First run executes the cell and inserts a record.
#[test]
fn first_run_executes_and_inserts() {
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

    assert!(db_path.exists(), "registry database should exist after run");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("1 completed"),
        "expected '1 completed' in stdout; got: {stdout}"
    );
    assert!(
        stdout.contains("0 skipped"),
        "expected '0 skipped' in stdout; got: {stdout}"
    );
}

/// Second run with the same spec reports skips and does NOT update stored
/// runtime or schedule_json.
#[test]
fn second_run_reports_skips_and_does_not_update_row() {
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

    // Capture last_seen_at from first run.
    let list1 = Command::new(BIN)
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
    let rows1: serde_json::Value = serde_json::from_slice(&list1.stdout).unwrap();
    let last_seen_at_1 = rows1[0]["last_seen_at"].as_str().unwrap().to_string();

    // Sleep to ensure timestamps differ if a write happens.
    std::thread::sleep(std::time::Duration::from_millis(1100));

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
        "second run failed:\n{}",
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

    // last_seen_at must not have changed (row was not re-written).
    let list2 = Command::new(BIN)
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
    let rows2: serde_json::Value = serde_json::from_slice(&list2.stdout).unwrap();
    let last_seen_at_2 = rows2[0]["last_seen_at"].as_str().unwrap().to_string();
    assert_eq!(
        last_seen_at_1, last_seen_at_2,
        "last_seen_at should not change on a skip"
    );
}

// ── `registry export` tests ───────────────────────────────────────────────────

/// `lab registry export --run <KEY> --out <FILE>` produces a valid schedule JSON.
#[test]
fn registry_export_single_run_produces_valid_schedule() {
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
            "--run-db",
            db_path.to_str().unwrap(),
        ])
        .status()
        .expect("run should succeed");
    assert!(status.success(), "run to populate registry failed");

    // Get run key via `registry list --format json`.
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
    assert!(list_output.status.success());

    let list_json: serde_json::Value =
        serde_json::from_slice(&list_output.stdout).expect("registry list must produce JSON");
    let run_key = list_json[0]["run_key"]
        .as_str()
        .expect("run_key must be a string");

    // Export the schedule.
    let out_file = tmp.path().join("exported.json");
    let export_status = Command::new(BIN)
        .args([
            "registry",
            "export",
            "--run",
            run_key,
            "--out",
            out_file.to_str().unwrap(),
            "--run-db",
            db_path.to_str().unwrap(),
        ])
        .status()
        .expect("registry export should succeed");
    assert!(
        export_status.success(),
        "registry export exited with failure"
    );

    assert!(out_file.exists(), "exported schedule file must exist");

    let content = fs::read_to_string(&out_file).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(&content).expect("exported output must be valid JSON");
    assert!(
        v.get("schedule_metadata").is_some(),
        "exported schedule must contain schedule_metadata"
    );
    assert!(
        v.get("schedule_metrics").is_some(),
        "exported schedule must contain schedule_metrics"
    );
}

/// `--force` overwrites an existing export file.
#[test]
fn registry_export_force_overwrites_existing_file() {
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
        .unwrap();
    assert!(status.success());

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
        .unwrap();
    let rows: serde_json::Value = serde_json::from_slice(&list_out.stdout).unwrap();
    let run_key = rows[0]["run_key"].as_str().unwrap();

    let out_file = tmp.path().join("sched.json");
    // Write a placeholder so it already exists.
    fs::write(&out_file, b"placeholder").unwrap();

    // Without --force it should fail.
    let s_no_force = Command::new(BIN)
        .args([
            "registry",
            "export",
            "--run",
            run_key,
            "--out",
            out_file.to_str().unwrap(),
            "--run-db",
            db_path.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(
        !s_no_force.success(),
        "export without --force must fail when file exists"
    );

    // With --force it should succeed and overwrite.
    let s_force = Command::new(BIN)
        .args([
            "registry",
            "export",
            "--run",
            run_key,
            "--out",
            out_file.to_str().unwrap(),
            "--run-db",
            db_path.to_str().unwrap(),
            "--force",
        ])
        .status()
        .unwrap();
    assert!(s_force.success(), "export with --force must succeed");

    let content = fs::read_to_string(&out_file).unwrap();
    assert_ne!(
        content, "placeholder",
        "file must be overwritten by --force"
    );
}

/// `registry export --out-dir` exports multiple schedules to a directory.
#[test]
fn registry_export_filtered_to_dir() {
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
        .unwrap();
    assert!(status.success());

    let out_dir = tmp.path().join("exported");
    let export_status = Command::new(BIN)
        .args([
            "registry",
            "export",
            "--out-dir",
            out_dir.to_str().unwrap(),
            "--run-db",
            db_path.to_str().unwrap(),
        ])
        .status()
        .expect("registry export --out-dir should succeed");
    assert!(
        export_status.success(),
        "registry export --out-dir exited with failure"
    );

    assert!(out_dir.is_dir(), "output directory must be created");
    let files: Vec<_> = fs::read_dir(&out_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|s| s.to_str())
                .map(|s| s == "json")
                .unwrap_or(false)
        })
        .collect();
    assert!(
        !files.is_empty(),
        "at least one schedule JSON must be exported"
    );

    for f in &files {
        let content = fs::read_to_string(f.path()).unwrap();
        let v: serde_json::Value = serde_json::from_str(&content)
            .unwrap_or_else(|_| panic!("exported file {} must be valid JSON", f.path().display()));
        assert!(
            v.get("schedule_metadata").is_some(),
            "exported schedule must have schedule_metadata"
        );
    }
}

#[test]
fn run_stores_unique_schedule_and_run_hash_reference() {
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
        .expect("run should succeed");
    assert!(status.success());

    let conn = Connection::open(&db_path).unwrap();
    let run_schedule_hash: Option<String> = conn
        .query_row("SELECT schedule_hash FROM runs LIMIT 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert!(
        run_schedule_hash.is_some(),
        "run row must reference a schedule hash"
    );
    let schedule_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM schedules", [], |row| row.get(0))
        .unwrap();
    assert_eq!(schedule_count, 1);
}

#[test]
fn registry_export_prefers_stored_schedule_json() {
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
        .expect("run should succeed");
    assert!(status.success());

    let conn = Connection::open(&db_path).unwrap();
    let (run_key, schedule_hash): (String, String) = conn
        .query_row(
            "SELECT run_key, schedule_hash FROM runs LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let sentinel = r#"{"stored":true}"#;
    conn.execute(
        "UPDATE schedules SET schedule_json = ?2 WHERE schedule_hash = ?1",
        params![schedule_hash, sentinel],
    )
    .unwrap();

    let out_file = tmp.path().join("exported.json");
    let export_status = Command::new(BIN)
        .args([
            "registry",
            "export",
            "--run",
            &run_key,
            "--out",
            out_file.to_str().unwrap(),
            "--run-db",
            db_path.to_str().unwrap(),
        ])
        .status()
        .expect("registry export should succeed");
    assert!(export_status.success());

    assert_eq!(fs::read_to_string(out_file).unwrap(), sentinel);
}

#[test]
fn registry_export_errors_when_schedule_hash_missing() {
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
        .expect("run should succeed");
    assert!(status.success());

    let conn = Connection::open(&db_path).unwrap();
    let run_key: String = conn
        .query_row("SELECT run_key FROM runs LIMIT 1", [], |row| row.get(0))
        .unwrap();
    conn.execute("UPDATE runs SET schedule_hash = NULL", [])
        .unwrap();

    let out_file = tmp.path().join("exported-missing.json");
    let export_status = Command::new(BIN)
        .args([
            "registry",
            "export",
            "--run",
            &run_key,
            "--out",
            out_file.to_str().unwrap(),
            "--run-db",
            db_path.to_str().unwrap(),
        ])
        .status()
        .expect("registry export should run");
    assert!(
        !export_status.success(),
        "registry export should fail when schedule_hash is missing"
    );
    assert!(!out_file.exists());
}

#[test]
fn registry_export_errors_when_schedule_row_missing() {
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
        .expect("run should succeed");
    assert!(status.success());

    let conn = Connection::open(&db_path).unwrap();
    let run_key: String = conn
        .query_row("SELECT run_key FROM runs LIMIT 1", [], |row| row.get(0))
        .unwrap();
    conn.execute("DELETE FROM schedules", []).unwrap();

    let out_file = tmp.path().join("exported-missing-row.json");
    let export_status = Command::new(BIN)
        .args([
            "registry",
            "export",
            "--run",
            &run_key,
            "--out",
            out_file.to_str().unwrap(),
            "--run-db",
            db_path.to_str().unwrap(),
        ])
        .status()
        .expect("registry export should run");
    assert!(
        !export_status.success(),
        "registry export should fail when the schedules row is missing"
    );
    assert!(!out_file.exists());
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
            "--run-db",
            db_path.to_str().unwrap(),
        ])
        .status()
        .expect("run should succeed");
    assert!(status.success());

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
