//! Matrix cell expansion.
//!
//! [`resolve_cells`] takes an [`ExperimentSpec`] and returns the flat list of
//! [`MatrixCell`] values that represent the full Cartesian product of
//! `(dataset × algorithm × configuration)`.
//!
//! # Deduplication and ordering
//!
//! Within an algorithm's sweep, configurations are collected into a
//! [`BTreeSet`](std::collections::BTreeSet) so duplicates are silently dropped
//! and the order is deterministic across runs. Across algorithms the order
//! follows the spec's `algorithms` list. The outermost dimension is the
//! spec's `datasets` list order.
//!
//! # Cell ID format
//!
//! ```text
//! <dataset_id>__<algorithm>__<config_slug>
//! ```
//!
//! e.g. `ctao_n__est__e1-k4-b2` or `ctao_s__hap__hap-i128-r3-p4-elitist4-s0`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::config::{
    EstRunConfig, EstSweepAxes, HapRunConfig, HapSweepAxes, HorizonOverride, LstRunConfig,
    RunConfig,
};
use crate::spec::{AlgorithmSweep, DatasetEntry, ExperimentSpec};

/// One fully-resolved unit of work in the experiment matrix.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatrixCell {
    /// Deterministic, filesystem-safe slug that uniquely identifies this cell.
    pub cell_id: String,
    /// Dataset slug (matches `DatasetEntry::id`).
    pub dataset_id: String,
    /// Path to the scheduling-problem JSON for this cell.
    pub dataset_path: PathBuf,
    /// Optional human-readable dataset label embedded in schedule metadata.
    #[serde(default)]
    pub dataset_label: Option<String>,
    /// Optional horizon override inherited from the dataset entry.
    #[serde(default)]
    pub horizon_override: Option<HorizonOverride>,
    /// Algorithm name (`"est"` or `"hap"`).
    pub algorithm: String,
    /// Fully resolved scheduler configuration for this cell.
    pub run_config: RunConfig,
}

impl MatrixCell {
    /// Returns the configuration slug portion of the cell ID.
    #[allow(dead_code)]
    pub fn config_slug(&self) -> String {
        self.run_config.slug()
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Resolves the full Cartesian product of `(dataset × algorithm × config)`.
///
/// Returns an error if:
/// - `spec.datasets` is empty
/// - `spec.algorithms` is empty
/// - A dataset ID is invalid (not alphanumeric / `_` / `-`)
/// - An algorithm sweep produces zero configurations
/// - Two different cells would map to the same `cell_id`
pub fn resolve_cells(spec: &ExperimentSpec) -> Result<Vec<MatrixCell>, String> {
    if spec.datasets.is_empty() {
        return Err("experiment spec must declare at least one dataset".to_string());
    }
    if spec.algorithms.is_empty() {
        return Err("experiment spec must declare at least one algorithm".to_string());
    }

    let mut cells = Vec::new();
    for dataset in &spec.datasets {
        validate_dataset(dataset)?;
        for sweep in &spec.algorithms {
            let configs = resolve_configs(sweep)?;
            for config in configs {
                let cell_id = format!("{}__{}__{}", dataset.id, sweep.algorithm(), config.slug());
                cells.push(MatrixCell {
                    cell_id,
                    dataset_id: dataset.id.clone(),
                    dataset_path: dataset.path.clone(),
                    dataset_label: dataset.label.clone(),
                    horizon_override: dataset.horizon_override,
                    algorithm: sweep.algorithm().to_string(),
                    run_config: config,
                });
            }
        }
    }

    // Detect cell ID collisions (should only occur on pathological specs).
    let mut seen = std::collections::HashSet::new();
    for cell in &cells {
        if !seen.insert(cell.cell_id.clone()) {
            return Err(format!(
                "duplicate cell_id '{}' (dataset/algorithm/config slug collision)",
                cell.cell_id
            ));
        }
    }

    Ok(cells)
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn validate_dataset(d: &DatasetEntry) -> Result<(), String> {
    if d.id.is_empty() {
        return Err("dataset entry has empty id".to_string());
    }
    if !d
        .id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(format!(
            "dataset id '{}' must be alphanumeric, '_' or '-' (used as filesystem path)",
            d.id
        ));
    }
    Ok(())
}

fn resolve_configs(sweep: &AlgorithmSweep) -> Result<Vec<RunConfig>, String> {
    let mut set: BTreeSet<RunConfig> = BTreeSet::new();
    match sweep {
        AlgorithmSweep::Est { axes } => insert_est_configs(axes, &mut set),
        AlgorithmSweep::Hap { axes } => insert_hap_configs(axes, &mut set),
        AlgorithmSweep::Lst { axes } => insert_lst_configs(axes, &mut set),
    }
    let configs: Vec<_> = set.into_iter().collect();
    if configs.is_empty() {
        return Err(format!(
            "algorithm sweep '{}' resolved to zero configurations",
            sweep.algorithm()
        ));
    }
    for cfg in &configs {
        validate_config(*cfg)?;
    }
    Ok(configs)
}

fn insert_est_configs(axes: &EstSweepAxes, set: &mut BTreeSet<RunConfig>) {
    let def = EstRunConfig::default();
    let endangered = pick_or_default(&axes.endangered_thresholds, def.endangered_threshold);
    let k_beams = pick_or_default(&axes.k_beams, def.k_beams);
    let branching = pick_or_default(&axes.branching_factors, def.branching_factor);
    let foms = if axes.foms.is_empty() {
        vec![def.fom]
    } else {
        axes.foms.clone()
    };

    for &e in &endangered {
        for &k in &k_beams {
            for &b in &branching {
                for &fom in &foms {
                    set.insert(RunConfig::Est(EstRunConfig {
                        fom,
                        endangered_threshold: e,
                        k_beams: k,
                        branching_factor: b,
                    }));
                }
            }
        }
    }
}

fn insert_hap_configs(axes: &HapSweepAxes, set: &mut BTreeSet<RunConfig>) {
    let def = HapRunConfig::default();
    let iota = pick_or_default(&axes.iota_max_values, def.iota_max);
    let rho = pick_or_default(&axes.rho_values, def.rho);
    let pop = pick_or_default(&axes.population_sizes, def.population_size);
    let modes = pick_or_default(&axes.survivor_modes, def.survivor_mode);
    let caps = pick_or_default(&axes.survivor_caps, def.survivor_cap);
    let seeds = pick_or_default(&axes.seeds, def.seed);

    for &i in &iota {
        for &r in &rho {
            for &p in &pop {
                for &m in &modes {
                    for &c in &caps {
                        for &s in &seeds {
                            set.insert(RunConfig::Hap(HapRunConfig {
                                iota_max: i,
                                rho: r,
                                population_size: p,
                                survivor_mode: m,
                                survivor_cap: c,
                                seed: s,
                            }));
                        }
                    }
                }
            }
        }
    }
}

fn insert_lst_configs(axes: &EstSweepAxes, set: &mut BTreeSet<RunConfig>) {
    let def = LstRunConfig::default();
    let endangered = pick_or_default(&axes.endangered_thresholds, def.endangered_threshold);
    let k_beams = pick_or_default(&axes.k_beams, def.k_beams);
    let branching = pick_or_default(&axes.branching_factors, def.branching_factor);
    let foms = if axes.foms.is_empty() {
        vec![def.fom]
    } else {
        axes.foms.clone()
    };

    for &e in &endangered {
        for &k in &k_beams {
            for &b in &branching {
                for &fom in &foms {
                    set.insert(RunConfig::Lst(LstRunConfig {
                        fom,
                        endangered_threshold: e,
                        k_beams: k,
                        branching_factor: b,
                    }));
                }
            }
        }
    }
}

/// Returns `values` when non-empty, otherwise a single-element vec containing
/// `default`.
fn pick_or_default<T: Copy>(values: &[T], default: T) -> Vec<T> {
    if values.is_empty() {
        vec![default]
    } else {
        values.to_vec()
    }
}

fn validate_config(cfg: RunConfig) -> Result<(), String> {
    match cfg {
        RunConfig::Est(c) => c.build_scheduler().map(|_| ()),
        RunConfig::Hap(c) => c.build_scheduler().map(|_| ()),
        RunConfig::Lst(c) => c.build_scheduler().map(|_| ()),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::HapSurvivorMode;
    use crate::spec::ExperimentSpec;
    use std::path::PathBuf;

    fn spec_two_datasets_two_algorithms() -> ExperimentSpec {
        ExperimentSpec {
            name: "x".into(),
            datasets: vec![
                DatasetEntry {
                    id: "d1".into(),
                    path: PathBuf::from("d1.json"),
                    label: None,
                    horizon_override: None,
                },
                DatasetEntry {
                    id: "d2".into(),
                    path: PathBuf::from("d2.json"),
                    label: None,
                    horizon_override: None,
                },
            ],
            algorithms: vec![
                AlgorithmSweep::Est {
                    axes: EstSweepAxes {
                        endangered_thresholds: vec![1, 2],
                        k_beams: vec![1, 2],
                        branching_factors: vec![1],
                        foms: vec![],
                    },
                },
                AlgorithmSweep::Hap {
                    axes: HapSweepAxes {
                        iota_max_values: vec![64],
                        rho_values: vec![2],
                        population_sizes: vec![4, 8],
                        survivor_modes: vec![HapSurvivorMode::ElitistTopK],
                        survivor_caps: vec![4],
                        seeds: vec![0],
                    },
                },
            ],
            ranking: None,
            max_parallel: None,
            output_dir: PathBuf::from("out"),
        }
    }

    #[test]
    fn cartesian_product_size_matches_axes() {
        let cells = resolve_cells(&spec_two_datasets_two_algorithms()).unwrap();
        // 2 datasets × (4 EST + 2 HAP) = 12 cells
        assert_eq!(cells.len(), 12);
    }

    #[test]
    fn empty_axes_fall_back_to_singleton() {
        let mut spec = spec_two_datasets_two_algorithms();
        spec.algorithms = vec![AlgorithmSweep::Est {
            axes: EstSweepAxes::default(),
        }];
        let cells = resolve_cells(&spec).unwrap();
        // 2 datasets × 1 default est cell
        assert_eq!(cells.len(), 2);
    }

    #[test]
    fn cell_ids_are_deterministic_and_unique() {
        let spec = spec_two_datasets_two_algorithms();
        let cells_a = resolve_cells(&spec).unwrap();
        let cells_b = resolve_cells(&spec).unwrap();
        assert_eq!(
            cells_a.iter().map(|c| &c.cell_id).collect::<Vec<_>>(),
            cells_b.iter().map(|c| &c.cell_id).collect::<Vec<_>>()
        );
        let mut set = std::collections::HashSet::new();
        for c in &cells_a {
            assert!(set.insert(c.cell_id.clone()), "duplicate {}", c.cell_id);
        }
        let first = &cells_a[0];
        assert!(
            first
                .cell_id
                .starts_with(&format!("{}__", first.dataset_id))
        );
        assert!(first.cell_id.contains("__est__"));
    }

    #[test]
    fn empty_datasets_is_rejected() {
        let mut spec = spec_two_datasets_two_algorithms();
        spec.datasets.clear();
        assert!(resolve_cells(&spec).is_err());
    }

    #[test]
    fn empty_algorithms_is_rejected() {
        let mut spec = spec_two_datasets_two_algorithms();
        spec.algorithms.clear();
        assert!(resolve_cells(&spec).is_err());
    }

    #[test]
    fn lst_cells_use_lst_in_cell_id() {
        let spec = ExperimentSpec {
            name: "x".into(),
            datasets: vec![DatasetEntry {
                id: "isdc_n".into(),
                path: PathBuf::from("data/isdc_n.json"),
                label: None,
                horizon_override: None,
            }],
            algorithms: vec![AlgorithmSweep::Lst {
                axes: EstSweepAxes {
                    endangered_thresholds: vec![1],
                    k_beams: vec![4],
                    branching_factors: vec![2],
                    foms: vec![],
                },
            }],
            ranking: None,
            max_parallel: None,
            output_dir: PathBuf::from("out"),
        };
        let cells = resolve_cells(&spec).unwrap();
        assert_eq!(cells.len(), 1);
        assert!(
            cells[0].cell_id.contains("__lst__"),
            "cell_id: {}",
            cells[0].cell_id
        );
        assert_eq!(cells[0].cell_id, "isdc_n__lst__e1-k4-b2");
        assert_eq!(cells[0].algorithm, "lst");
    }

    #[test]
    fn lst_cells_include_fom_suffix_when_non_default() {
        let spec = ExperimentSpec {
            name: "x".into(),
            datasets: vec![DatasetEntry {
                id: "d1".into(),
                path: PathBuf::from("d1.json"),
                label: None,
                horizon_override: None,
            }],
            algorithms: vec![AlgorithmSweep::Lst {
                axes: EstSweepAxes {
                    endangered_thresholds: vec![1],
                    k_beams: vec![4],
                    branching_factors: vec![2],
                    foms: vec![schedulers::scheduler::est::FomKind::FutureFlexibility],
                },
            }],
            ranking: None,
            max_parallel: None,
            output_dir: PathBuf::from("out"),
        };
        let cells = resolve_cells(&spec).unwrap();
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0].cell_id, "d1__lst__e1-k4-b2-future_flexibility");
    }

    #[test]
    fn lst_and_est_do_not_collide() {
        let spec = ExperimentSpec {
            name: "x".into(),
            datasets: vec![DatasetEntry {
                id: "d1".into(),
                path: PathBuf::from("d1.json"),
                label: None,
                horizon_override: None,
            }],
            algorithms: vec![
                AlgorithmSweep::Est {
                    axes: EstSweepAxes {
                        endangered_thresholds: vec![1],
                        k_beams: vec![1],
                        branching_factors: vec![1],
                        foms: vec![],
                    },
                },
                AlgorithmSweep::Lst {
                    axes: EstSweepAxes {
                        endangered_thresholds: vec![1],
                        k_beams: vec![1],
                        branching_factors: vec![1],
                        foms: vec![],
                    },
                },
            ],
            ranking: None,
            max_parallel: None,
            output_dir: PathBuf::from("out"),
        };
        let cells = resolve_cells(&spec).unwrap();
        assert_eq!(cells.len(), 2);
        let ids: Vec<_> = cells.iter().map(|c| c.cell_id.as_str()).collect();
        assert!(ids.contains(&"d1__est__e1-k1-b1"));
        assert!(ids.contains(&"d1__lst__e1-k1-b1"));
    }

    #[test]
    fn invalid_dataset_id_is_rejected() {
        let mut spec = spec_two_datasets_two_algorithms();
        spec.datasets[0].id = "with space".into();
        assert!(resolve_cells(&spec).is_err());
    }
}
