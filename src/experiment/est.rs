use crate::prescheduler::{TaskPeriodMap, preschedule};
use crate::schedule::{Schedule, ScheduleOutput, SchedulingProblem};
use crate::scheduler::est::{EstConfig, EstFomKind, EstScheduler};
use crate::task::Task;
use crate::time::{MJD, Period, TaskId, Time};
use csv::Writer;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const RETIMED_TOLERANCE_DAYS: f64 = 1.0 / 86_400.0;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HorizonOverride {
    pub start_mjd: f64,
    pub end_mjd: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EstRunConfigOverride {
    #[serde(default)]
    pub fom: Option<EstFomKind>,
    #[serde(default)]
    pub endangered_threshold: Option<u32>,
    #[serde(default)]
    pub k_beams: Option<usize>,
    #[serde(default)]
    pub branching_factor: Option<usize>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EstSweepSpec {
    #[serde(default)]
    pub foms: Vec<EstFomKind>,
    #[serde(default)]
    pub endangered_thresholds: Vec<u32>,
    #[serde(default)]
    pub k_beams: Vec<usize>,
    #[serde(default)]
    pub branching_factors: Vec<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EstExperimentSpec {
    pub input_json: PathBuf,
    pub output_dir: PathBuf,
    #[serde(default)]
    pub horizon_override: Option<HorizonOverride>,
    #[serde(default)]
    pub baseline: EstRunConfigOverride,
    #[serde(default)]
    pub sweep: EstSweepSpec,
}

#[derive(Debug, Clone, Default)]
pub struct EstExperimentCliOverrides {
    pub input_json: Option<String>,
    pub output_dir: Option<PathBuf>,
    pub horizon_override: Option<HorizonOverride>,
    pub fom_values: Option<Vec<EstFomKind>>,
    pub endangered_threshold_values: Option<Vec<u32>>,
    pub k_beam_values: Option<Vec<usize>>,
    pub branching_factor_values: Option<Vec<usize>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct EstRunConfig {
    pub fom: EstFomKind,
    pub endangered_threshold: u32,
    pub k_beams: usize,
    pub branching_factor: usize,
}

impl Default for EstRunConfig {
    fn default() -> Self {
        Self {
            fom: EstFomKind::TaskCount,
            endangered_threshold: 1,
            k_beams: 1,
            branching_factor: 1,
        }
    }
}

impl EstRunConfig {
    pub const fn est_config(self) -> EstConfig {
        EstConfig {
            endangered_threshold: self.endangered_threshold,
            k_beams: self.k_beams,
            branching_factor: self.branching_factor,
        }
    }

    pub fn build_scheduler(self) -> Result<EstScheduler, String> {
        EstScheduler::with_kind(self.est_config(), self.fom)
            .map_err(|e| format!("invalid EST configuration for {}: {e}", self.slug()))
    }

    pub fn slug(self) -> String {
        format!(
            "fom-{}__e-{}__k-{}__b-{}",
            self.fom, self.endangered_threshold, self.k_beams, self.branching_factor
        )
    }

    fn apply_override(mut self, override_config: &EstRunConfigOverride) -> Self {
        if let Some(fom) = override_config.fom {
            self.fom = fom;
        }
        if let Some(endangered_threshold) = override_config.endangered_threshold {
            self.endangered_threshold = endangered_threshold;
        }
        if let Some(k_beams) = override_config.k_beams {
            self.k_beams = k_beams;
        }
        if let Some(branching_factor) = override_config.branching_factor {
            self.branching_factor = branching_factor;
        }
        self
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ResolvedEstExperiment {
    pub input_path: PathBuf,
    pub output_dir: PathBuf,
    pub horizon_override: Option<HorizonOverride>,
    pub baseline: EstRunConfig,
    pub runs: Vec<EstRunConfig>,
}

impl ResolvedEstExperiment {
    pub fn baseline_slug(&self) -> String {
        self.baseline.slug()
    }
}

#[derive(Debug, Clone)]
pub struct EstExperimentExecution {
    pub manifest_path: PathBuf,
    pub comparison_csv_path: PathBuf,
    pub schedule_paths: Vec<PathBuf>,
    pub run_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct EstExperimentManifest {
    pub input_json: String,
    pub output_dir: String,
    pub horizon_override: Option<HorizonOverride>,
    pub baseline_slug: String,
    pub baseline: EstRunConfig,
    pub comparison_csv: String,
    pub runs: Vec<ManifestRunEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ManifestRunEntry {
    pub slug: String,
    pub fom: EstFomKind,
    pub endangered_threshold: u32,
    pub k_beams: usize,
    pub branching_factor: usize,
    pub schedule_json: String,
}

#[derive(Debug)]
struct PreparedProblem {
    raw_json: Value,
    tasks: Vec<Task>,
    possible_periods: TaskPeriodMap,
    horizon: Period<MJD>,
    common_metrics: CommonMetrics,
    priority_by_task: HashMap<TaskId, f64>,
}

#[derive(Debug, Clone)]
struct CommonMetrics {
    input_json: String,
    total_blocks: usize,
    total_tasks: usize,
    horizon_start_mjd: f64,
    horizon_end_mjd: f64,
    horizon_hours: f64,
    total_requested_hours: f64,
    mean_requested_hours: f64,
    total_priority_all: f64,
    mean_priority_all: f64,
    median_priority_all: f64,
    preschedule_elapsed_sec: f64,
    feasible_tasks: usize,
    infeasible_tasks: usize,
    feasible_task_rate: f64,
    total_feasible_windows: usize,
    task_feasible_hours_total: f64,
    mean_feasible_windows_per_feasible_task: f64,
    mean_feasible_hours_per_feasible_task: f64,
}

#[derive(Debug, Clone)]
struct RunOutcome {
    config: EstRunConfig,
    schedule: Schedule,
    schedule_path: PathBuf,
    metrics: RunMetrics,
}

#[derive(Debug, Clone)]
struct RunMetrics {
    est_elapsed_sec: f64,
    total_elapsed_sec: f64,
    scheduled_tasks: usize,
    unscheduled_tasks: usize,
    feasible_unscheduled_tasks: usize,
    scheduled_task_rate: f64,
    scheduled_requested_hours: f64,
    unscheduled_requested_hours: f64,
    scheduled_requested_fraction: f64,
    scheduled_priority_total: f64,
    scheduled_priority_mean: f64,
    scheduled_priority_median: f64,
    unscheduled_priority_total: f64,
    unscheduled_priority_mean: f64,
    unscheduled_priority_median: f64,
    first_scheduled_start_mjd: Option<f64>,
    last_scheduled_end_mjd: Option<f64>,
    makespan_hours: f64,
    gap_count: usize,
    gap_total_hours: f64,
    gap_mean_hours: f64,
    gap_median_hours: f64,
    gap_std_dev_hours: f64,
    gap_p90_hours: f64,
    largest_gap_hours: f64,
}

#[derive(Debug, Clone, Serialize)]
struct ComparisonRow {
    run_slug: String,
    is_baseline: bool,
    baseline_slug: String,
    input_json: String,
    fom: EstFomKind,
    endangered_threshold: u32,
    k_beams: usize,
    branching_factor: usize,
    total_blocks: usize,
    total_tasks: usize,
    horizon_start_mjd: f64,
    horizon_end_mjd: f64,
    horizon_hours: f64,
    total_requested_hours: f64,
    mean_requested_hours: f64,
    total_priority_all: f64,
    mean_priority_all: f64,
    median_priority_all: f64,
    preschedule_elapsed_sec: f64,
    feasible_tasks: usize,
    infeasible_tasks: usize,
    feasible_task_rate: f64,
    total_feasible_windows: usize,
    task_feasible_hours_total: f64,
    mean_feasible_windows_per_feasible_task: f64,
    mean_feasible_hours_per_feasible_task: f64,
    est_elapsed_sec: f64,
    total_elapsed_sec: f64,
    scheduled_tasks: usize,
    unscheduled_tasks: usize,
    feasible_unscheduled_tasks: usize,
    scheduled_task_rate: f64,
    scheduled_requested_hours: f64,
    unscheduled_requested_hours: f64,
    scheduled_requested_fraction: f64,
    scheduled_priority_total: f64,
    scheduled_priority_mean: f64,
    scheduled_priority_median: f64,
    unscheduled_priority_total: f64,
    unscheduled_priority_mean: f64,
    unscheduled_priority_median: f64,
    first_scheduled_start_mjd: Option<f64>,
    last_scheduled_end_mjd: Option<f64>,
    makespan_hours: f64,
    gap_count: usize,
    gap_total_hours: f64,
    gap_mean_hours: f64,
    gap_median_hours: f64,
    gap_std_dev_hours: f64,
    gap_p90_hours: f64,
    largest_gap_hours: f64,
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

pub fn resolve_experiment(
    spec: Option<EstExperimentSpec>,
    cli: EstExperimentCliOverrides,
) -> Result<ResolvedEstExperiment, String> {
    let baseline = EstRunConfig::default().apply_override(
        &spec
            .as_ref()
            .map(|value| value.baseline.clone())
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
        .or_else(|| spec.as_ref().and_then(|value| value.horizon_override));

    let spec_sweep = spec.as_ref().map(|value| &value.sweep);

    let foms = resolve_axis(
        cli.fom_values,
        spec_sweep.map(|value| value.foms.clone()),
        baseline.fom,
    );
    let endangered_thresholds = resolve_axis(
        cli.endangered_threshold_values,
        spec_sweep.map(|value| value.endangered_thresholds.clone()),
        baseline.endangered_threshold,
    );
    let k_beams = resolve_axis(
        cli.k_beam_values,
        spec_sweep.map(|value| value.k_beams.clone()),
        baseline.k_beams,
    );
    let branching_factors = resolve_axis(
        cli.branching_factor_values,
        spec_sweep.map(|value| value.branching_factors.clone()),
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

pub fn run_experiment(
    experiment: &ResolvedEstExperiment,
) -> Result<EstExperimentExecution, String> {
    prepare_output_dir(&experiment.output_dir)?;

    let schedules_dir = experiment.output_dir.join("schedules");
    fs::create_dir_all(&schedules_dir).map_err(|e| {
        format!(
            "failed to create schedules directory {}: {e}",
            schedules_dir.display()
        )
    })?;

    let prepared_problem = prepare_problem(&experiment.input_path, experiment.horizon_override)?;
    let baseline_slug = experiment.baseline_slug();

    let mut outcomes = Vec::with_capacity(experiment.runs.len());
    for run in &experiment.runs {
        let schedule_path = schedules_dir.join(format!("{}.json", run.slug()));
        let outcome = execute_run(run, &prepared_problem, &schedule_path)?;
        outcomes.push(outcome);
    }

    let baseline_outcome = outcomes
        .first()
        .ok_or_else(|| "resolved experiment contained no runs".to_string())?;

    let rows: Vec<_> = outcomes
        .iter()
        .map(|outcome| {
            build_comparison_row(
                &baseline_slug,
                &prepared_problem.common_metrics,
                &baseline_outcome.schedule,
                &baseline_outcome.metrics,
                outcome,
            )
        })
        .collect();

    let comparison_csv_path = experiment.output_dir.join("comparison.csv");
    write_comparison_csv(&comparison_csv_path, &rows)?;

    let manifest_path = experiment.output_dir.join("manifest.json");
    let manifest = build_manifest(experiment, &comparison_csv_path, &outcomes);
    let manifest_text = serde_json::to_string_pretty(&manifest).map_err(|e| {
        format!(
            "failed to serialize manifest {}: {e}",
            manifest_path.display()
        )
    })?;
    fs::write(&manifest_path, manifest_text)
        .map_err(|e| format!("failed to write manifest {}: {e}", manifest_path.display()))?;

    Ok(EstExperimentExecution {
        manifest_path,
        comparison_csv_path,
        schedule_paths: outcomes
            .into_iter()
            .map(|outcome| outcome.schedule_path)
            .collect(),
        run_count: experiment.runs.len(),
    })
}

fn build_manifest(
    experiment: &ResolvedEstExperiment,
    comparison_csv_path: &Path,
    outcomes: &[RunOutcome],
) -> EstExperimentManifest {
    EstExperimentManifest {
        input_json: experiment.input_path.display().to_string(),
        output_dir: experiment.output_dir.display().to_string(),
        horizon_override: experiment.horizon_override,
        baseline_slug: experiment.baseline_slug(),
        baseline: experiment.baseline,
        comparison_csv: relative_to_output(&experiment.output_dir, comparison_csv_path),
        runs: outcomes
            .iter()
            .map(|outcome| ManifestRunEntry {
                slug: outcome.config.slug(),
                fom: outcome.config.fom,
                endangered_threshold: outcome.config.endangered_threshold,
                k_beams: outcome.config.k_beams,
                branching_factor: outcome.config.branching_factor,
                schedule_json: relative_to_output(&experiment.output_dir, &outcome.schedule_path),
            })
            .collect(),
    }
}

fn execute_run(
    run: &EstRunConfig,
    prepared_problem: &PreparedProblem,
    schedule_path: &Path,
) -> Result<RunOutcome, String> {
    let scheduler = run.build_scheduler()?;
    let est_start = Instant::now();
    let schedule = scheduler
        .run_scheduler(
            &prepared_problem.tasks,
            &prepared_problem.possible_periods,
            &prepared_problem.horizon,
        )
        .map_err(|e| format!("EST run {} failed: {e}", run.slug()))?;
    let est_elapsed = est_start.elapsed();

    let output = ScheduleOutput::new(prepared_problem.raw_json.clone(), &schedule);
    let output_text = serde_json::to_string_pretty(&output)
        .map_err(|e| format!("failed to serialize schedule output {}: {e}", run.slug()))?;
    fs::write(schedule_path, output_text).map_err(|e| {
        format!(
            "failed to write schedule output {}: {e}",
            schedule_path.display()
        )
    })?;

    let metrics = compute_run_metrics(
        &schedule,
        &prepared_problem.tasks,
        &prepared_problem.common_metrics,
        &prepared_problem.priority_by_task,
        est_elapsed,
    );

    Ok(RunOutcome {
        config: *run,
        schedule,
        schedule_path: schedule_path.to_path_buf(),
        metrics,
    })
}

fn prepare_problem(
    input_path: &Path,
    horizon_override: Option<HorizonOverride>,
) -> Result<PreparedProblem, String> {
    let text = fs::read_to_string(input_path)
        .map_err(|e| format!("failed to read {}: {e}", input_path.display()))?;
    let raw_json: Value = serde_json::from_str(&text)
        .map_err(|e| format!("failed to parse JSON {}: {e}", input_path.display()))?;
    let problem: SchedulingProblem = serde_json::from_str(&text)
        .map_err(|e| format!("failed to parse {}: {e}", input_path.display()))?;

    if problem.tasks.is_empty() {
        return Err("input JSON contains no tasks".to_string());
    }

    let priority_by_task = extract_task_priorities(&raw_json);

    let SchedulingProblem {
        tasks,
        blocks,
        detected_horizon,
        telescope,
    } = problem;

    let horizon = build_horizon(detected_horizon, horizon_override)?;
    let telescope = telescope.ok_or_else(|| {
        "missing observing site in input; expected resources[0] or legacy location".to_string()
    })?;
    let blocks: Vec<_> = blocks.into_values().collect();
    let total_blocks = blocks.len();

    let preschedule_start = Instant::now();
    let possible_periods = preschedule(&blocks, &tasks, &horizon, &telescope)
        .map_err(|e| format!("prescheduling failed: {e}"))?;
    let preschedule_elapsed = preschedule_start.elapsed();

    let mut tasks: Vec<_> = tasks.into_values().collect();
    tasks.sort_by_key(|task| task.id.0);

    let common_metrics = build_common_metrics(
        input_path,
        total_blocks,
        &tasks,
        &possible_periods,
        &priority_by_task,
        &horizon,
        preschedule_elapsed,
    );

    Ok(PreparedProblem {
        raw_json,
        tasks,
        possible_periods,
        horizon,
        common_metrics,
        priority_by_task,
    })
}

fn build_common_metrics(
    input_path: &Path,
    total_blocks: usize,
    tasks: &[Task],
    possible_periods: &TaskPeriodMap,
    priority_by_task: &HashMap<TaskId, f64>,
    horizon: &Period<MJD>,
    preschedule_elapsed: Duration,
) -> CommonMetrics {
    let total_tasks = tasks.len();
    let total_requested_hours: f64 = tasks
        .iter()
        .map(|task| task.duration.value() / 3600.0)
        .sum();
    let mean_requested_hours = safe_mean(total_requested_hours, total_tasks);

    let mut priorities: Vec<f64> = tasks
        .iter()
        .map(|task| *priority_by_task.get(&task.id).unwrap_or(&0.0))
        .collect();
    let total_priority_all: f64 = priorities.iter().sum();
    let mean_priority_all = safe_mean(total_priority_all, total_tasks);
    let median_priority_all = median(&mut priorities);

    let feasible_tasks = possible_periods
        .values()
        .filter(|periods| !periods.is_empty())
        .count();
    let infeasible_tasks = total_tasks.saturating_sub(feasible_tasks);
    let feasible_task_rate = safe_ratio(feasible_tasks as f64, total_tasks as f64);

    let total_feasible_windows: usize =
        possible_periods.values().map(|periods| periods.len()).sum();
    let task_feasible_hours_total: f64 = possible_periods
        .values()
        .flat_map(|periods| periods.iter())
        .map(period_hours)
        .sum();
    let mean_feasible_windows_per_feasible_task =
        safe_mean(total_feasible_windows as f64, feasible_tasks);
    let mean_feasible_hours_per_feasible_task =
        safe_mean(task_feasible_hours_total, feasible_tasks);

    CommonMetrics {
        input_json: input_path.display().to_string(),
        total_blocks,
        total_tasks,
        horizon_start_mjd: horizon.start.value(),
        horizon_end_mjd: horizon.end.value(),
        horizon_hours: days_to_hours(horizon.end.value() - horizon.start.value()),
        total_requested_hours,
        mean_requested_hours,
        total_priority_all,
        mean_priority_all,
        median_priority_all,
        preschedule_elapsed_sec: preschedule_elapsed.as_secs_f64(),
        feasible_tasks,
        infeasible_tasks,
        feasible_task_rate,
        total_feasible_windows,
        task_feasible_hours_total,
        mean_feasible_windows_per_feasible_task,
        mean_feasible_hours_per_feasible_task,
    }
}

fn compute_run_metrics(
    schedule: &Schedule,
    tasks: &[Task],
    common_metrics: &CommonMetrics,
    priority_by_task: &HashMap<TaskId, f64>,
    est_elapsed: Duration,
) -> RunMetrics {
    let scheduled_ids: HashSet<_> = schedule.placements.keys().copied().collect();
    let scheduled_tasks = scheduled_ids.len();
    let unscheduled_tasks = common_metrics.total_tasks.saturating_sub(scheduled_tasks);
    let feasible_unscheduled_tasks = common_metrics
        .feasible_tasks
        .saturating_sub(scheduled_tasks);
    let scheduled_task_rate = safe_ratio(scheduled_tasks as f64, common_metrics.total_tasks as f64);

    let mut scheduled_requested_hours = 0.0;
    let mut scheduled_priorities = Vec::new();
    let mut unscheduled_priorities = Vec::new();

    for task in tasks {
        let priority = *priority_by_task.get(&task.id).unwrap_or(&0.0);
        if scheduled_ids.contains(&task.id) {
            scheduled_priorities.push(priority);
        } else {
            unscheduled_priorities.push(priority);
        }
    }

    for placement in schedule.placements.values() {
        scheduled_requested_hours +=
            days_to_hours(placement.end.to::<MJD>().value() - placement.start.to::<MJD>().value());
    }
    let unscheduled_requested_hours =
        (common_metrics.total_requested_hours - scheduled_requested_hours).max(0.0);

    let scheduled_priority_total: f64 = scheduled_priorities.iter().sum();
    let scheduled_priority_mean = safe_mean(scheduled_priority_total, scheduled_priorities.len());
    let scheduled_priority_median = median(&mut scheduled_priorities);

    let unscheduled_priority_total: f64 = unscheduled_priorities.iter().sum();
    let unscheduled_priority_mean =
        safe_mean(unscheduled_priority_total, unscheduled_priorities.len());
    let unscheduled_priority_median = median(&mut unscheduled_priorities);

    let scheduled_requested_fraction = safe_ratio(
        scheduled_requested_hours,
        common_metrics.total_requested_hours,
    );

    let mut placements: Vec<_> = schedule.placements.values().collect();
    placements.sort_by(|left, right| {
        left.start
            .to::<MJD>()
            .value()
            .total_cmp(&right.start.to::<MJD>().value())
    });

    let first_scheduled_start_mjd = placements
        .first()
        .map(|placement| placement.start.to::<MJD>().value());
    let last_scheduled_end_mjd = placements
        .last()
        .map(|placement| placement.end.to::<MJD>().value());
    let makespan_hours = match (first_scheduled_start_mjd, last_scheduled_end_mjd) {
        (Some(first), Some(last)) => days_to_hours(last - first),
        _ => 0.0,
    };

    let gap_hours = compute_gap_hours(&placements);
    let gap_count = gap_hours.len();
    let gap_total_hours: f64 = gap_hours.iter().sum();
    let gap_mean_hours = safe_mean(gap_total_hours, gap_count);
    let gap_median_hours = median(&mut gap_hours.clone());
    let gap_std_dev_hours = sample_standard_deviation(&gap_hours);
    let gap_p90_hours = percentile_90(&gap_hours);
    let largest_gap_hours = gap_hours.iter().copied().fold(0.0, f64::max);

    RunMetrics {
        est_elapsed_sec: est_elapsed.as_secs_f64(),
        total_elapsed_sec: common_metrics.preschedule_elapsed_sec + est_elapsed.as_secs_f64(),
        scheduled_tasks,
        unscheduled_tasks,
        feasible_unscheduled_tasks,
        scheduled_task_rate,
        scheduled_requested_hours,
        unscheduled_requested_hours,
        scheduled_requested_fraction,
        scheduled_priority_total,
        scheduled_priority_mean,
        scheduled_priority_median,
        unscheduled_priority_total,
        unscheduled_priority_mean,
        unscheduled_priority_median,
        first_scheduled_start_mjd,
        last_scheduled_end_mjd,
        makespan_hours,
        gap_count,
        gap_total_hours,
        gap_mean_hours,
        gap_median_hours,
        gap_std_dev_hours,
        gap_p90_hours,
        largest_gap_hours,
    }
}

fn build_comparison_row(
    baseline_slug: &str,
    common_metrics: &CommonMetrics,
    baseline_schedule: &Schedule,
    baseline_metrics: &RunMetrics,
    outcome: &RunOutcome,
) -> ComparisonRow {
    let is_baseline = outcome.config.slug() == baseline_slug;
    let schedule_delta = compare_schedules(baseline_schedule, &outcome.schedule);

    ComparisonRow {
        run_slug: outcome.config.slug(),
        is_baseline,
        baseline_slug: baseline_slug.to_string(),
        input_json: common_metrics.input_json.clone(),
        fom: outcome.config.fom,
        endangered_threshold: outcome.config.endangered_threshold,
        k_beams: outcome.config.k_beams,
        branching_factor: outcome.config.branching_factor,
        total_blocks: common_metrics.total_blocks,
        total_tasks: common_metrics.total_tasks,
        horizon_start_mjd: common_metrics.horizon_start_mjd,
        horizon_end_mjd: common_metrics.horizon_end_mjd,
        horizon_hours: common_metrics.horizon_hours,
        total_requested_hours: common_metrics.total_requested_hours,
        mean_requested_hours: common_metrics.mean_requested_hours,
        total_priority_all: common_metrics.total_priority_all,
        mean_priority_all: common_metrics.mean_priority_all,
        median_priority_all: common_metrics.median_priority_all,
        preschedule_elapsed_sec: common_metrics.preschedule_elapsed_sec,
        feasible_tasks: common_metrics.feasible_tasks,
        infeasible_tasks: common_metrics.infeasible_tasks,
        feasible_task_rate: common_metrics.feasible_task_rate,
        total_feasible_windows: common_metrics.total_feasible_windows,
        task_feasible_hours_total: common_metrics.task_feasible_hours_total,
        mean_feasible_windows_per_feasible_task: common_metrics
            .mean_feasible_windows_per_feasible_task,
        mean_feasible_hours_per_feasible_task: common_metrics.mean_feasible_hours_per_feasible_task,
        est_elapsed_sec: outcome.metrics.est_elapsed_sec,
        total_elapsed_sec: outcome.metrics.total_elapsed_sec,
        scheduled_tasks: outcome.metrics.scheduled_tasks,
        unscheduled_tasks: outcome.metrics.unscheduled_tasks,
        feasible_unscheduled_tasks: outcome.metrics.feasible_unscheduled_tasks,
        scheduled_task_rate: outcome.metrics.scheduled_task_rate,
        scheduled_requested_hours: outcome.metrics.scheduled_requested_hours,
        unscheduled_requested_hours: outcome.metrics.unscheduled_requested_hours,
        scheduled_requested_fraction: outcome.metrics.scheduled_requested_fraction,
        scheduled_priority_total: outcome.metrics.scheduled_priority_total,
        scheduled_priority_mean: outcome.metrics.scheduled_priority_mean,
        scheduled_priority_median: outcome.metrics.scheduled_priority_median,
        unscheduled_priority_total: outcome.metrics.unscheduled_priority_total,
        unscheduled_priority_mean: outcome.metrics.unscheduled_priority_mean,
        unscheduled_priority_median: outcome.metrics.unscheduled_priority_median,
        first_scheduled_start_mjd: outcome.metrics.first_scheduled_start_mjd,
        last_scheduled_end_mjd: outcome.metrics.last_scheduled_end_mjd,
        makespan_hours: outcome.metrics.makespan_hours,
        gap_count: outcome.metrics.gap_count,
        gap_total_hours: outcome.metrics.gap_total_hours,
        gap_mean_hours: outcome.metrics.gap_mean_hours,
        gap_median_hours: outcome.metrics.gap_median_hours,
        gap_std_dev_hours: outcome.metrics.gap_std_dev_hours,
        gap_p90_hours: outcome.metrics.gap_p90_hours,
        largest_gap_hours: outcome.metrics.largest_gap_hours,
        delta_scheduled_tasks: outcome.metrics.scheduled_tasks as i64
            - baseline_metrics.scheduled_tasks as i64,
        delta_scheduled_task_rate: outcome.metrics.scheduled_task_rate
            - baseline_metrics.scheduled_task_rate,
        delta_scheduled_requested_hours: outcome.metrics.scheduled_requested_hours
            - baseline_metrics.scheduled_requested_hours,
        delta_scheduled_requested_fraction: outcome.metrics.scheduled_requested_fraction
            - baseline_metrics.scheduled_requested_fraction,
        delta_scheduled_priority_total: outcome.metrics.scheduled_priority_total
            - baseline_metrics.scheduled_priority_total,
        delta_gap_count: outcome.metrics.gap_count as i64 - baseline_metrics.gap_count as i64,
        delta_gap_total_hours: outcome.metrics.gap_total_hours - baseline_metrics.gap_total_hours,
        delta_gap_mean_hours: outcome.metrics.gap_mean_hours - baseline_metrics.gap_mean_hours,
        delta_largest_gap_hours: outcome.metrics.largest_gap_hours
            - baseline_metrics.largest_gap_hours,
        delta_est_elapsed_sec: outcome.metrics.est_elapsed_sec - baseline_metrics.est_elapsed_sec,
        delta_total_elapsed_sec: outcome.metrics.total_elapsed_sec
            - baseline_metrics.total_elapsed_sec,
        vs_baseline_newly_scheduled_count: schedule_delta.newly_scheduled_count,
        vs_baseline_newly_unscheduled_count: schedule_delta.newly_unscheduled_count,
        vs_baseline_retimed_count: schedule_delta.retimed_count,
        vs_baseline_mean_abs_start_shift_hours: schedule_delta.mean_abs_start_shift_hours,
    }
}

fn compare_schedules(baseline: &Schedule, current: &Schedule) -> ScheduleDelta {
    let baseline_ids: HashSet<_> = baseline.placements.keys().copied().collect();
    let current_ids: HashSet<_> = current.placements.keys().copied().collect();

    let newly_scheduled_count = current_ids.difference(&baseline_ids).count();
    let newly_unscheduled_count = baseline_ids.difference(&current_ids).count();

    let mut retimed_count = 0usize;
    let mut total_abs_start_shift_hours = 0.0;

    for task_id in baseline_ids.intersection(&current_ids) {
        let baseline_placement = &baseline.placements[task_id];
        let current_placement = &current.placements[task_id];

        let start_shift_days = current_placement.start.to::<MJD>().value()
            - baseline_placement.start.to::<MJD>().value();
        let end_shift_days =
            current_placement.end.to::<MJD>().value() - baseline_placement.end.to::<MJD>().value();

        if start_shift_days.abs() > RETIMED_TOLERANCE_DAYS
            || end_shift_days.abs() > RETIMED_TOLERANCE_DAYS
        {
            retimed_count += 1;
            total_abs_start_shift_hours += days_to_hours(start_shift_days.abs());
        }
    }

    ScheduleDelta {
        newly_scheduled_count,
        newly_unscheduled_count,
        retimed_count,
        mean_abs_start_shift_hours: safe_mean(total_abs_start_shift_hours, retimed_count),
    }
}

fn write_comparison_csv(path: &Path, rows: &[ComparisonRow]) -> Result<(), String> {
    let mut writer = Writer::from_path(path)
        .map_err(|e| format!("failed to create comparison CSV {}: {e}", path.display()))?;
    for row in rows {
        writer
            .serialize(row)
            .map_err(|e| format!("failed to write comparison row to {}: {e}", path.display()))?;
    }
    writer
        .flush()
        .map_err(|e| format!("failed to flush comparison CSV {}: {e}", path.display()))
}

fn prepare_output_dir(path: &Path) -> Result<(), String> {
    if path.exists() {
        if !path.is_dir() {
            return Err(format!(
                "output path {} exists and is not a directory",
                path.display()
            ));
        }

        let mut entries = fs::read_dir(path)
            .map_err(|e| format!("failed to read output directory {}: {e}", path.display()))?;
        if entries.next().is_some() {
            return Err(format!("output directory {} must be empty", path.display()));
        }
    } else {
        fs::create_dir_all(path)
            .map_err(|e| format!("failed to create output directory {}: {e}", path.display()))?;
    }

    Ok(())
}

fn build_horizon(
    detected: Option<Period<MJD>>,
    override_range: Option<HorizonOverride>,
) -> Result<Period<MJD>, String> {
    if let Some(override_range) = override_range {
        return period_from_mjd(override_range.start_mjd, override_range.end_mjd);
    }
    detected.ok_or_else(|| {
        "missing schedule_time_window in input and no horizon override was provided".to_string()
    })
}

fn period_from_mjd(start_mjd: f64, end_mjd: f64) -> Result<Period<MJD>, String> {
    if !start_mjd.is_finite() || !end_mjd.is_finite() {
        return Err("horizon bounds must be finite numbers".to_string());
    }
    if start_mjd >= end_mjd {
        return Err(format!(
            "invalid horizon: start ({start_mjd}) must be before end ({end_mjd})"
        ));
    }
    Ok(Period::new(
        Time::<MJD>::new(start_mjd),
        Time::<MJD>::new(end_mjd),
    ))
}

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

fn extract_task_priorities(json: &Value) -> HashMap<TaskId, f64> {
    let mut priorities = HashMap::new();

    if let Some(blocks) = json.get("scheduling_blocks").and_then(Value::as_array) {
        for block in blocks {
            collect_block_priorities(block, &mut priorities);
        }
    } else if let Some(blocks) = json.as_array() {
        for block in blocks {
            collect_block_priorities(block, &mut priorities);
        }
    }

    priorities
}

fn collect_block_priorities(block: &Value, priorities: &mut HashMap<TaskId, f64>) {
    let Some(tasks) = block.get("tasks").and_then(Value::as_array) else {
        return;
    };

    for task in tasks {
        let Some(id) = task.get("id").and_then(Value::as_u64) else {
            continue;
        };

        let priority = task
            .get("soft_constraints")
            .and_then(|soft| soft.get("priority"))
            .and_then(Value::as_f64)
            .filter(|value| value.is_finite())
            .unwrap_or(0.0);

        priorities.insert(TaskId(id), priority);
    }
}

fn compute_gap_hours(placements: &[&crate::schedule::TaskPlacement]) -> Vec<f64> {
    let mut gap_hours = Vec::new();
    for window in placements.windows(2) {
        let gap_days = window[1].start.to::<MJD>().value() - window[0].end.to::<MJD>().value();
        if gap_days > 0.0 {
            gap_hours.push(days_to_hours(gap_days));
        }
    }
    gap_hours
}

fn median(values: &mut [f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }

    values.sort_by(|left, right| left.total_cmp(right));
    let mid = values.len() / 2;
    if values.len().is_multiple_of(2) {
        (values[mid - 1] + values[mid]) / 2.0
    } else {
        values[mid]
    }
}

fn sample_standard_deviation(values: &[f64]) -> f64 {
    if values.len() <= 1 {
        return 0.0;
    }

    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let variance = values
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / (values.len() - 1) as f64;
    variance.sqrt()
}

fn percentile_90(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }

    if values.len() == 1 {
        return values[0];
    }

    let mut sorted = values.to_vec();
    sorted.sort_by(|left, right| left.total_cmp(right));

    let rank = 0.9 * (sorted.len() as f64 - 1.0);
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;

    if lower == upper {
        sorted[lower]
    } else {
        let fraction = rank - lower as f64;
        sorted[lower] + fraction * (sorted[upper] - sorted[lower])
    }
}

fn safe_ratio(numerator: f64, denominator: f64) -> f64 {
    if denominator > 0.0 {
        numerator / denominator
    } else {
        0.0
    }
}

fn safe_mean(total: f64, count: usize) -> f64 {
    if count > 0 { total / count as f64 } else { 0.0 }
}

fn period_hours(period: &Period<MJD>) -> f64 {
    days_to_hours(period.end.value() - period.start.value())
}

fn days_to_hours(days: f64) -> f64 {
    days * 24.0
}

fn relative_to_output(output_dir: &Path, path: &Path) -> String {
    path.strip_prefix(output_dir)
        .unwrap_or(path)
        .display()
        .to_string()
}

#[derive(Debug, Clone, Copy)]
struct ScheduleDelta {
    newly_scheduled_count: usize,
    newly_unscheduled_count: usize,
    retimed_count: usize,
    mean_abs_start_shift_hours: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constraints::{ConstraintExpr, PrioritySoftConstraint, SoftConstraintExpr};
    use crate::schedule::TaskPlacement;
    use crate::task::{IcrsTarget, Task};
    use qtty::{Degrees, Seconds};
    use siderust::coordinates::frames::ICRS;
    use siderust::coordinates::spherical::Direction;
    use tempfile::TempDir;

    fn target() -> IcrsTarget {
        Direction::<ICRS>::new_raw(Degrees::new(10.0), Degrees::new(20.0))
    }

    fn task(id: u64, hours: f64, priority: f64) -> Task {
        let soft = Some(SoftConstraintExpr::atom(PrioritySoftConstraint::new(
            priority,
        )));
        Task::new(
            TaskId(id),
            format!("task-{id}"),
            target(),
            Seconds::new(hours * 3600.0),
            ConstraintExpr::Intersection(vec![]),
            soft,
        )
        .expect("task should be valid")
    }

    fn schedule_with_slots(slots: &[(u64, f64, f64)]) -> Schedule {
        let mut schedule = Schedule::new();
        for (task_id, start, end) in slots {
            schedule.insert_placement(TaskPlacement {
                task_id: TaskId(*task_id),
                start: Time::<MJD>::new(*start).to::<crate::time::JD>(),
                end: Time::<MJD>::new(*end).to::<crate::time::JD>(),
                block_id: None,
            });
        }
        schedule
    }

    #[test]
    fn run_slug_encodes_all_configuration_axes() {
        let run = EstRunConfig {
            fom: EstFomKind::SoftConstraint,
            endangered_threshold: 2,
            k_beams: 5,
            branching_factor: 3,
        };

        assert_eq!(run.slug(), "fom-soft_constraint__e-2__k-5__b-3");
    }

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

    #[test]
    fn prepare_output_dir_rejects_non_empty_directories() {
        let temp_dir = TempDir::new().expect("temp dir should exist");
        fs::write(temp_dir.path().join("artifact.txt"), "not empty")
            .expect("artifact should be written");

        let error = prepare_output_dir(temp_dir.path()).expect_err("directory should be rejected");
        assert!(error.contains("must be empty"));
    }

    #[test]
    fn compute_run_metrics_reports_gap_statistics() {
        let schedule = schedule_with_slots(&[(1, 0.0, 0.1), (2, 0.2, 0.3), (3, 0.5, 0.6)]);
        let common = CommonMetrics {
            input_json: "input.json".to_string(),
            total_blocks: 3,
            total_tasks: 3,
            horizon_start_mjd: 0.0,
            horizon_end_mjd: 1.0,
            horizon_hours: 24.0,
            total_requested_hours: 7.2,
            mean_requested_hours: 2.4,
            total_priority_all: 9.0,
            mean_priority_all: 3.0,
            median_priority_all: 3.0,
            preschedule_elapsed_sec: 1.0,
            feasible_tasks: 3,
            infeasible_tasks: 0,
            feasible_task_rate: 1.0,
            total_feasible_windows: 3,
            task_feasible_hours_total: 8.0,
            mean_feasible_windows_per_feasible_task: 1.0,
            mean_feasible_hours_per_feasible_task: 8.0 / 3.0,
        };
        let priorities = HashMap::from([(TaskId(1), 5.0), (TaskId(2), 3.0), (TaskId(3), 1.0)]);

        let tasks = vec![task(1, 2.4, 5.0), task(2, 2.4, 3.0), task(3, 2.4, 1.0)];
        let metrics = compute_run_metrics(
            &schedule,
            &tasks,
            &common,
            &priorities,
            Duration::from_secs(2),
        );

        assert_eq!(metrics.scheduled_tasks, 3);
        assert_eq!(metrics.gap_count, 2);
        assert!((metrics.gap_total_hours - 7.2).abs() < 1e-6);
        assert!((metrics.largest_gap_hours - 4.8).abs() < 1e-6);
    }

    #[test]
    fn compare_schedules_tracks_status_changes_and_retiming() {
        let baseline = schedule_with_slots(&[(1, 0.0, 0.1), (2, 0.2, 0.3)]);
        let current = schedule_with_slots(&[(1, 0.0, 0.1), (2, 0.25, 0.35), (3, 0.4, 0.5)]);

        let delta = compare_schedules(&baseline, &current);

        assert_eq!(delta.newly_scheduled_count, 1);
        assert_eq!(delta.newly_unscheduled_count, 0);
        assert_eq!(delta.retimed_count, 1);
        assert!((delta.mean_abs_start_shift_hours - 1.2).abs() < 1e-6);
    }

    #[test]
    fn build_common_metrics_uses_missing_priorities_as_zero() {
        let tasks = vec![task(1, 1.0, 10.0), task(2, 2.0, 5.0)];
        let possible_periods = HashMap::from([
            (
                TaskId(1),
                crate::time::PeriodSet::from_periods(vec![Period::new(
                    Time::<MJD>::new(0.0),
                    Time::<MJD>::new(0.1),
                )]),
            ),
            (TaskId(2), crate::time::PeriodSet::new()),
        ]);
        let priorities = HashMap::from([(TaskId(1), 10.0)]);
        let horizon = Period::new(Time::<MJD>::new(0.0), Time::<MJD>::new(1.0));

        let metrics = build_common_metrics(
            Path::new("input.json"),
            2,
            &tasks,
            &possible_periods,
            &priorities,
            &horizon,
            Duration::from_secs(1),
        );

        assert_eq!(metrics.total_priority_all, 10.0);
        assert_eq!(metrics.mean_priority_all, 5.0);
        assert_eq!(metrics.median_priority_all, 5.0);
    }
}
