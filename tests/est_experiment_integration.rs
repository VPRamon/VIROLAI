use csv::Reader;
use serde::Deserialize;
use serde_json::Value;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

use scheduler::experiment::est::{
    EstExperimentCliOverrides, load_experiment_spec, resolve_experiment, run_experiment,
};

#[derive(Debug, Deserialize)]
struct ComparisonRow {
    run_slug: String,
    is_baseline: bool,
    baseline_slug: String,
    delta_scheduled_tasks: i64,
    delta_scheduled_task_rate: f64,
    delta_scheduled_requested_hours: f64,
    delta_scheduled_requested_fraction: f64,
    delta_scheduled_priority_total: f64,
    delta_gap_count: i64,
    delta_gap_total_hours: f64,
    delta_gap_mean_hours: f64,
    delta_largest_gap_hours: f64,
    delta_est_elapsed_sec: f64,
    delta_total_elapsed_sec: f64,
    vs_baseline_newly_scheduled_count: usize,
    vs_baseline_newly_unscheduled_count: usize,
    vs_baseline_retimed_count: usize,
    vs_baseline_mean_abs_start_shift_hours: f64,
}

#[test]
fn est_experiment_pipeline_writes_expected_artifacts() {
    let temp_dir = TempDir::new().expect("temp dir should exist");
    write_input_fixture(temp_dir.path());
    let spec_path = write_spec_fixture(temp_dir.path());

    let spec = load_experiment_spec(&spec_path).expect("spec should load");
    let resolved = resolve_experiment(Some(spec), EstExperimentCliOverrides::default())
        .expect("experiment should resolve");

    assert_eq!(resolved.runs.len(), 4);

    let execution = run_experiment(&resolved).expect("experiment should run");

    assert_eq!(execution.run_count, 4);
    assert_eq!(execution.schedule_paths.len(), 4);
    assert!(execution.manifest_path.exists());
    assert!(execution.comparison_csv_path.exists());
    for schedule_path in &execution.schedule_paths {
        assert!(
            schedule_path.exists(),
            "{} should exist",
            schedule_path.display()
        );
    }

    let manifest: Value = serde_json::from_str(
        &fs::read_to_string(&execution.manifest_path).expect("manifest should be readable"),
    )
    .expect("manifest JSON should parse");

    let baseline_slug = manifest
        .get("baseline_slug")
        .and_then(Value::as_str)
        .expect("manifest should contain baseline slug");
    assert_eq!(baseline_slug, "fom-task_count__e-1__k-1__b-1");

    let runs = manifest
        .get("runs")
        .and_then(Value::as_array)
        .expect("manifest should contain runs");
    assert_eq!(runs.len(), 4);

    let mut reader =
        Reader::from_path(&execution.comparison_csv_path).expect("comparison csv should load");
    let headers = reader.headers().expect("csv headers should exist").clone();
    assert!(
        headers
            .iter()
            .any(|header| header == "vs_baseline_retimed_count")
    );

    let rows: Vec<ComparisonRow> = reader
        .deserialize()
        .map(|row| row.expect("row should deserialize"))
        .collect();
    assert_eq!(rows.len(), 4);

    let baseline_row = rows
        .iter()
        .find(|row| row.is_baseline)
        .expect("baseline row should exist");

    assert_eq!(baseline_row.run_slug, baseline_slug);
    assert_eq!(baseline_row.baseline_slug, baseline_slug);
    assert_eq!(baseline_row.delta_scheduled_tasks, 0);
    assert_eq!(baseline_row.delta_gap_count, 0);
    assert_eq!(baseline_row.vs_baseline_newly_scheduled_count, 0);
    assert_eq!(baseline_row.vs_baseline_newly_unscheduled_count, 0);
    assert_eq!(baseline_row.vs_baseline_retimed_count, 0);
    assert!(baseline_row.delta_scheduled_task_rate.abs() < 1e-9);
    assert!(baseline_row.delta_scheduled_requested_hours.abs() < 1e-9);
    assert!(baseline_row.delta_scheduled_requested_fraction.abs() < 1e-9);
    assert!(baseline_row.delta_scheduled_priority_total.abs() < 1e-9);
    assert!(baseline_row.delta_gap_total_hours.abs() < 1e-9);
    assert!(baseline_row.delta_gap_mean_hours.abs() < 1e-9);
    assert!(baseline_row.delta_largest_gap_hours.abs() < 1e-9);
    assert!(baseline_row.delta_est_elapsed_sec.abs() < 1e-9);
    assert!(baseline_row.delta_total_elapsed_sec.abs() < 1e-9);
    assert!(baseline_row.vs_baseline_mean_abs_start_shift_hours.abs() < 1e-9);
}

fn write_input_fixture(base_dir: &Path) {
    let input_json = r#"{
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

    fs::write(base_dir.join("input.json"), input_json).expect("fixture input should be written");
}

fn write_spec_fixture(base_dir: &Path) -> std::path::PathBuf {
    let spec_json = r#"{
  "input_json": "input.json",
  "output_dir": "results",
  "sweep": {
    "foms": ["task_count", "soft_constraint"],
    "endangered_thresholds": [1, 2],
    "k_beams": [1],
    "branching_factors": [1]
  }
}"#;

    let spec_path = base_dir.join("experiment.json");
    fs::write(&spec_path, spec_json).expect("fixture spec should be written");
    spec_path
}
