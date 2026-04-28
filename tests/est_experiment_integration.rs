use csv::Reader;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

const EST_EXPERIMENT_BIN: &str = env!("CARGO_BIN_EXE_est_experiment");

#[derive(Debug, Deserialize)]
struct ComparisonRow {
    run_slug: String,
    is_baseline: bool,
    scheduled_task_count: usize,
    fitness_priority_sum: f64,
    scheduled_priority_p25: f64,
    scheduled_priority_p50: f64,
    scheduled_priority_p75: f64,
    scheduled_priority_p90: f64,
}

#[test]
fn est_experiment_pipeline_writes_expected_artifacts() {
    let temp_dir = TempDir::new().expect("temp dir should exist");
    write_input_fixture(temp_dir.path());
    let spec_path = write_spec_fixture(temp_dir.path());

    let status = Command::new(EST_EXPERIMENT_BIN)
        .args(["--spec", spec_path.to_str().unwrap()])
        .status()
        .expect("est_experiment binary should run");
    assert!(status.success(), "est_experiment exited with failure");

    let results_dir = temp_dir.path().join("results");
    assert!(results_dir.is_dir(), "results directory should exist");

    let run_dirs: Vec<_> = fs::read_dir(&results_dir)
        .expect("results dir should be readable")
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .collect();
    assert_eq!(
        run_dirs.len(),
        1,
        "exactly one timestamped run dir expected"
    );

    let run_dir = run_dirs[0].path();
    let run_dir_name = run_dir
        .file_name()
        .and_then(|n| n.to_str())
        .expect("run dir should have UTF-8 name");
    assert!(run_dir_name.starts_with("run-"));

    let manifest_path = run_dir.join("manifest.json");
    let comparison_csv_path = run_dir.join("comparison.csv");
    let schedules_dir = run_dir.join("schedules");

    assert!(manifest_path.exists(), "manifest.json should exist");
    assert!(comparison_csv_path.exists(), "comparison.csv should exist");
    assert!(schedules_dir.is_dir(), "schedules/ directory should exist");

    let expected_schedule_names = HashSet::from([
        "e1-k1-b1.json".to_string(),
        "e2-k1-b1.json".to_string(),
        "hap-i8-r2-p2-elitist2-s0.json".to_string(),
    ]);
    let actual_schedule_names: HashSet<String> = fs::read_dir(&schedules_dir)
        .expect("schedules dir should be readable")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|ext| ext.to_str()) == Some("json"))
        .map(|e| {
            e.file_name()
                .to_str()
                .expect("schedule file should have UTF-8 name")
                .to_string()
        })
        .collect();
    assert_eq!(actual_schedule_names, expected_schedule_names);

    for name in &expected_schedule_names {
        assert!(schedules_dir.join(name).exists(), "{name} should exist");
    }
    assert!(schedules_dir.join("e1-k1-b1.est_trace.jsonl").exists());
    assert!(schedules_dir.join("e2-k1-b1.est_trace.jsonl").exists());
    assert!(
        !schedules_dir
            .join("hap-i8-r2-p2-elitist2-s0.est_trace.jsonl")
            .exists()
    );

    let manifest: Value = serde_json::from_str(
        &fs::read_to_string(&manifest_path).expect("manifest should be readable"),
    )
    .expect("manifest JSON should parse");

    let manifest_output_dir = manifest
        .get("output_dir")
        .and_then(Value::as_str)
        .expect("manifest should contain output_dir");
    assert_eq!(Path::new(manifest_output_dir), run_dir.as_path());

    let baseline_slug = manifest
        .get("baseline_slug")
        .and_then(Value::as_str)
        .expect("manifest should contain baseline_slug");
    assert_eq!(baseline_slug, "e1-k1-b1");

    let runs = manifest
        .get("runs")
        .and_then(Value::as_array)
        .expect("manifest should contain runs");
    assert_eq!(runs.len(), 3);

    let baseline_run_entry = runs
        .iter()
        .find(|entry| {
            entry
                .get("slug")
                .and_then(Value::as_str)
                .is_some_and(|slug| slug == baseline_slug)
        })
        .expect("manifest should include baseline run entry");
    let baseline_schedule_json = baseline_run_entry
        .get("schedule_json")
        .and_then(Value::as_str)
        .expect("manifest run should contain schedule_json");
    assert_eq!(baseline_schedule_json, "schedules/e1-k1-b1.json");

    let hap_run_entry = runs
        .iter()
        .find(|entry| {
            entry
                .get("slug")
                .and_then(Value::as_str)
                .is_some_and(|slug| slug == "hap-i8-r2-p2-elitist2-s0")
        })
        .expect("manifest should include HAP run entry");
    assert_eq!(
        hap_run_entry.get("algorithm").and_then(Value::as_str),
        Some("hap")
    );
    assert_eq!(
        hap_run_entry.get("survivor_mode").and_then(Value::as_str),
        Some("elitist_top_k")
    );

    let mut reader = Reader::from_path(&comparison_csv_path).expect("comparison csv should load");
    let headers = reader.headers().expect("csv headers should exist").clone();
    assert_eq!(
        headers.iter().collect::<Vec<_>>(),
        vec![
            "run_slug",
            "is_baseline",
            "scheduled_task_count",
            "fitness_priority_sum",
            "scheduled_priority_p25",
            "scheduled_priority_p50",
            "scheduled_priority_p75",
            "scheduled_priority_p90",
        ]
    );

    let rows: Vec<ComparisonRow> = reader
        .deserialize()
        .map(|row| row.expect("row should deserialize"))
        .collect();
    assert_eq!(rows.len(), 3);

    let baseline_row = rows
        .iter()
        .find(|row| row.is_baseline)
        .expect("baseline row should exist");
    assert_eq!(baseline_row.run_slug, "e1-k1-b1");
    assert!(baseline_row.scheduled_task_count <= 2);
    assert!(baseline_row.fitness_priority_sum >= 0.0);
    assert!(baseline_row.fitness_priority_sum <= 15.0);

    for row in &rows {
        assert!(row.scheduled_priority_p25 <= row.scheduled_priority_p50);
        assert!(row.scheduled_priority_p50 <= row.scheduled_priority_p75);
        assert!(row.scheduled_priority_p75 <= row.scheduled_priority_p90);

        if row.scheduled_task_count == 0 {
            assert!(row.fitness_priority_sum.abs() < 1e-9);
            assert!(row.scheduled_priority_p25.abs() < 1e-9);
            assert!(row.scheduled_priority_p50.abs() < 1e-9);
            assert!(row.scheduled_priority_p75.abs() < 1e-9);
            assert!(row.scheduled_priority_p90.abs() < 1e-9);
        } else {
            assert!(row.fitness_priority_sum >= row.scheduled_priority_p90);
        }
    }
}

#[test]
fn est_experiment_range_syntax_produces_correct_run_count() {
    let temp_dir = TempDir::new().expect("temp dir should exist");
    write_input_fixture(temp_dir.path());
    let output_dir = temp_dir.path().join("out");

    let status = Command::new(EST_EXPERIMENT_BIN)
        .args([
            temp_dir.path().join("input.json").to_str().unwrap(),
            "--output-dir",
            output_dir.to_str().unwrap(),
            "--est-e-values",
            "1-3",
            "--est-k-values",
            "1,5",
            "--est-b-values",
            "1",
        ])
        .status()
        .expect("est_experiment binary should run");
    assert!(status.success(), "est_experiment exited with failure");

    let run_dirs: Vec<_> = fs::read_dir(&output_dir)
        .expect("output dir should be readable")
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .collect();
    assert_eq!(run_dirs.len(), 1);

    let schedules_dir = run_dirs[0].path().join("schedules");
    let schedule_count = fs::read_dir(&schedules_dir)
        .expect("schedules dir should be readable")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|ext| ext.to_str()) == Some("json"))
        .count();
    // e=[1,2,3] × k=[1,5] × b=[1] = 6 runs
    assert_eq!(schedule_count, 6);
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
  "emit_trace": true,
  "sweep": {
    "est": {
      "endangered_thresholds": [1, 2],
      "k_beams": [1],
      "branching_factors": [1]
    },
    "hap": {
      "iota_max_values": [8],
      "rho_values": [2],
      "population_sizes": [2],
      "survivor_modes": ["elitist_top_k"],
      "survivor_caps": [2],
      "seeds": [0]
    }
  }
}"#;

    let spec_path = base_dir.join("experiment.json");
    fs::write(&spec_path, spec_json).expect("fixture spec should be written");
    spec_path
}
