use serde::Serialize;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::config::{EstExperimentCliOverrides, EstExperimentSpec, EstRunConfig, HorizonOverride};

/// An experiment that has been fully resolved and is ready to execute.
///
/// All paths are absolute, all run configurations have been validated, and the baseline
/// is guaranteed to appear first in `runs`.
#[derive(Debug, Clone, Serialize)]
pub struct ResolvedEstExperiment {
    pub input_path: PathBuf,
    pub output_dir: PathBuf,
    pub horizon_override: Option<HorizonOverride>,
    /// The baseline configuration; always the first element of `runs`.
    pub baseline: EstRunConfig,
    /// All runs to execute, deduplicated; the baseline is always at index 0.
    pub runs: Vec<EstRunConfig>,
}

impl ResolvedEstExperiment {
    pub fn baseline_slug(&self) -> String {
        self.baseline.slug()
    }
}

/// Loads and deserializes an experiment spec from `path`.
///
/// Relative paths inside the spec are resolved relative to the spec file's directory.
pub fn load_experiment_spec(path: &Path) -> Result<EstExperimentSpec, String> {
    let text = fs::read_to_string(path)
        .map_err(|e| format!("failed to read spec {}: {e}", path.display()))?;
    let mut spec: EstExperimentSpec = serde_json::from_str(&text)
        .map_err(|e| format!("failed to parse spec {}: {e}", path.display()))?;
    let base_dir = path.parent().unwrap_or(Path::new("."));
    spec.input_json = resolve_input_path_from(base_dir, &spec.input_json);
    spec.output_dir = resolve_path_from(base_dir, &spec.output_dir);
    Ok(spec)
}

/// Merges a spec file and CLI overrides into a [`ResolvedEstExperiment`].
///
/// CLI values take precedence over spec values. Sweep axes are resolved independently
/// per parameter: CLI > spec > baseline singleton. The Cartesian product of all axes
/// is computed, deduplicated, and validated by attempting to construct each
/// [`EstScheduler`] before returning.
///
/// [`EstScheduler`]: crate::scheduler::est::EstScheduler
pub fn resolve_experiment(
    spec: Option<EstExperimentSpec>,
    cli: EstExperimentCliOverrides,
) -> Result<ResolvedEstExperiment, String> {
    let baseline = EstRunConfig::default().apply_override(
        &spec
            .as_ref()
            .map(|s| s.baseline.clone())
            .unwrap_or_default(),
    );

    let input_path = if let Some(input_json) = cli.input_json {
        resolve_input_path(&input_json)
    } else if let Some(spec) = &spec {
        spec.input_json.clone()
    } else {
        return Err("missing input_json; provide --spec or a positional <input_json>".to_string());
    };

    let output_dir = if let Some(output_dir) = cli.output_dir {
        output_dir
    } else if let Some(spec) = &spec {
        spec.output_dir.clone()
    } else {
        return Err("missing output_dir; provide --spec or --output-dir".to_string());
    };

    let horizon_override = cli
        .horizon_override
        .or_else(|| spec.as_ref().and_then(|s| s.horizon_override));

    let spec_sweep = spec.as_ref().map(|s| &s.sweep);

    let foms = resolve_axis(
        cli.fom_values,
        spec_sweep.map(|s| s.foms.clone()),
        baseline.fom,
    );
    let endangered_thresholds = resolve_axis(
        cli.endangered_threshold_values,
        spec_sweep.map(|s| s.endangered_thresholds.clone()),
        baseline.endangered_threshold,
    );
    let k_beams = resolve_axis(
        cli.k_beam_values,
        spec_sweep.map(|s| s.k_beams.clone()),
        baseline.k_beams,
    );
    let branching_factors = resolve_axis(
        cli.branching_factor_values,
        spec_sweep.map(|s| s.branching_factors.clone()),
        baseline.branching_factor,
    );

    let mut run_set = BTreeSet::new();
    run_set.insert(baseline);
    for fom in &foms {
        for endangered_threshold in &endangered_thresholds {
            for k_beams_value in &k_beams {
                for branching_factor in &branching_factors {
                    run_set.insert(EstRunConfig {
                        fom: *fom,
                        endangered_threshold: *endangered_threshold,
                        k_beams: *k_beams_value,
                        branching_factor: *branching_factor,
                    });
                }
            }
        }
    }

    let mut runs: Vec<_> = run_set.into_iter().collect();
    if let Some(index) = runs.iter().position(|run| *run == baseline) {
        let baseline_run = runs.remove(index);
        runs.insert(0, baseline_run);
    }

    for run in &runs {
        run.build_scheduler()?;
    }

    Ok(ResolvedEstExperiment {
        input_path,
        output_dir,
        horizon_override,
        baseline,
        runs,
    })
}

/// Resolves a sweep axis. CLI > spec > baseline singleton.
fn resolve_axis<T: Clone>(cli: Option<Vec<T>>, spec: Option<Vec<T>>, baseline: T) -> Vec<T> {
    if let Some(values) = cli {
        return if values.is_empty() {
            vec![baseline]
        } else {
            values
        };
    }
    if let Some(values) = spec {
        return if values.is_empty() {
            vec![baseline]
        } else {
            values
        };
    }
    vec![baseline]
}

fn resolve_path_from(base_dir: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_dir.join(path)
    }
}

/// Resolves an input path from a CLI string, trying `data/<arg>` as a fallback.
fn resolve_input_path(arg: &str) -> PathBuf {
    let direct = PathBuf::from(arg);
    if direct.exists() {
        return direct;
    }
    let under_data = PathBuf::from("data").join(arg);
    if under_data.exists() {
        return under_data;
    }
    direct
}

/// Resolves a path relative to a spec file's directory, with fallbacks.
fn resolve_input_path_from(base_dir: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    let relative = base_dir.join(path);
    if relative.exists() {
        return relative;
    }
    if path.exists() {
        return path.to_path_buf();
    }
    let under_data = PathBuf::from("data").join(path);
    if under_data.exists() {
        return under_data;
    }
    relative
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::experiment::est::config::{
        EstExperimentCliOverrides, EstExperimentSpec, EstRunConfig, EstRunConfigOverride,
        EstSweepSpec,
    };
    use crate::scheduler::est::EstFomKind;

    #[test]
    fn resolve_experiment_uses_baseline_singleton_when_sweep_is_empty() {
        let resolved = resolve_experiment(
            Some(EstExperimentSpec {
                input_json: PathBuf::from("data/input.json"),
                output_dir: PathBuf::from("out"),
                horizon_override: None,
                baseline: EstRunConfigOverride::default(),
                sweep: EstSweepSpec::default(),
            }),
            EstExperimentCliOverrides::default(),
        )
        .expect("resolution should succeed");

        assert_eq!(resolved.runs, vec![EstRunConfig::default()]);
        assert_eq!(resolved.baseline, EstRunConfig::default());
    }

    #[test]
    fn resolve_experiment_cli_axes_override_spec_axes() {
        let resolved = resolve_experiment(
            Some(EstExperimentSpec {
                input_json: PathBuf::from("data/input.json"),
                output_dir: PathBuf::from("out"),
                horizon_override: None,
                baseline: EstRunConfigOverride::default(),
                sweep: EstSweepSpec {
                    foms: vec![EstFomKind::SoftConstraint],
                    endangered_thresholds: vec![4],
                    k_beams: vec![8],
                    branching_factors: vec![6],
                },
            }),
            EstExperimentCliOverrides {
                fom_values: Some(vec![EstFomKind::TaskCount]),
                endangered_threshold_values: Some(vec![2]),
                k_beam_values: Some(vec![3]),
                branching_factor_values: Some(vec![4]),
                ..EstExperimentCliOverrides::default()
            },
        )
        .expect("resolution should succeed");

        assert_eq!(
            resolved.runs,
            vec![
                EstRunConfig::default(),
                EstRunConfig {
                    fom: EstFomKind::TaskCount,
                    endangered_threshold: 2,
                    k_beams: 3,
                    branching_factor: 4,
                },
            ]
        );
    }

    #[test]
    fn resolve_experiment_includes_baseline_and_deduplicates() {
        let resolved = resolve_experiment(
            Some(EstExperimentSpec {
                input_json: PathBuf::from("data/input.json"),
                output_dir: PathBuf::from("out"),
                horizon_override: None,
                baseline: EstRunConfigOverride {
                    fom: Some(EstFomKind::TaskCount),
                    endangered_threshold: Some(2),
                    k_beams: Some(2),
                    branching_factor: Some(2),
                },
                sweep: EstSweepSpec {
                    foms: vec![EstFomKind::TaskCount],
                    endangered_thresholds: vec![2],
                    k_beams: vec![2],
                    branching_factors: vec![2],
                },
            }),
            EstExperimentCliOverrides::default(),
        )
        .expect("resolution should succeed");

        assert_eq!(
            resolved.runs,
            vec![EstRunConfig {
                fom: EstFomKind::TaskCount,
                endangered_threshold: 2,
                k_beams: 2,
                branching_factor: 2,
            }]
        );
    }
}
