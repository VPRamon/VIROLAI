mod config;
mod output;
mod problem;
mod resolve;
mod run;
mod stats;

use rayon::prelude::*;
use std::fs;

pub use config::{
    EstExperimentCliOverrides, EstExperimentSpec, EstRunConfig, EstRunConfigOverride, EstSweepSpec,
    HorizonOverride,
};
pub use output::{EstExperimentExecution, EstExperimentManifest, ManifestRunEntry};
pub use resolve::{ResolvedEstExperiment, load_experiment_spec, resolve_experiment};

pub fn run_experiment(
    experiment: &ResolvedEstExperiment,
) -> Result<EstExperimentExecution, String> {
    let output_dir = output::prepare_output_dir(&experiment.output_dir)?;

    let schedules_dir = output_dir.join("schedules");
    fs::create_dir_all(&schedules_dir).map_err(|e| {
        format!(
            "failed to create schedules directory {}: {e}",
            schedules_dir.display()
        )
    })?;

    let prepared_problem =
        problem::prepare_problem(&experiment.input_path, experiment.horizon_override)?;
    let baseline_slug = experiment.baseline_slug();

    // Every EST run is independent once the shared problem is prepared.
    let outcomes: Vec<_> = experiment
        .runs
        .par_iter()
        .map(|run_config| {
            let schedule_path =
                schedules_dir.join(format!("{}.json", run_config.schedule_file_stem()));
            run::execute_run(run_config, &prepared_problem, &schedule_path)
        })
        .collect::<Result<Vec<_>, _>>()?;

    let rows: Vec<_> = outcomes
        .iter()
        .map(|outcome| output::build_comparison_row(&baseline_slug, outcome))
        .collect();

    let comparison_csv_path = output_dir.join("comparison.csv");
    output::write_comparison_csv(&comparison_csv_path, &rows)?;

    let manifest_path = output_dir.join("manifest.json");
    let manifest = output::build_manifest(experiment, &output_dir, &comparison_csv_path, &outcomes);
    let manifest_text = serde_json::to_string_pretty(&manifest).map_err(|e| {
        format!(
            "failed to serialize manifest {}: {e}",
            manifest_path.display()
        )
    })?;
    fs::write(&manifest_path, manifest_text)
        .map_err(|e| format!("failed to write manifest {}: {e}", manifest_path.display()))?;

    Ok(EstExperimentExecution {
        output_dir,
        manifest_path,
        comparison_csv_path,
        schedule_paths: outcomes
            .into_iter()
            .map(|outcome| outcome.schedule_path)
            .collect(),
        run_count: experiment.runs.len(),
    })
}
