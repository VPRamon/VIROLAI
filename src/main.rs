use qtty::{Degrees, Meter, Quantity, Seconds};
use scheduler::constraints::{
    AltitudeConstraint, AzimuthConstraint, ConstraintExpr, PrioritySoftConstraint,
    SoftConstraintExpr, TimeConstraint,
};
use scheduler::preschedule;
use scheduler::scheduler::est;
use scheduler::scheduling_block::{Dependency, SchedulingBlock};
use scheduler::task::Task;
use scheduler::time::{MJD, Period, SchedulingBlockId, TaskId, Time};
use serde::Deserialize;
use siderust::coordinates::centers::Geodetic;
use siderust::coordinates::frames::{ECEF, ICRS};
use siderust::coordinates::spherical::Direction;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

const DEFAULT_HORIZON_START_MJD: f64 = 62000.0;
const DEFAULT_HORIZON_END_MJD: f64 = 62030.0;

#[derive(Debug, Deserialize)]
struct SchedulingBlockInput {
    id: u64,
    tasks: Vec<TaskInput>,
    #[serde(default)]
    dependencies: Vec<DependencyInput>,
}

#[derive(Debug, Deserialize)]
struct TaskInput {
    id: u64,
    #[serde(default)]
    name: Option<String>,
    requested_duration_sec: f64,
    target: TargetInput,
    #[serde(default)]
    hard_constraints: HardConstraintsInput,
    #[serde(default)]
    soft_constraints: Option<SoftConstraintsInput>,
}

#[derive(Debug, Deserialize)]
struct TargetInput {
    ra_deg: f64,
    dec_deg: f64,
}

#[derive(Debug, Default, Deserialize)]
struct HardConstraintsInput {
    #[serde(default)]
    altitude_min_deg: Option<f64>,
    #[serde(default)]
    altitude_max_deg: Option<f64>,
    #[serde(default)]
    azimuth_min_deg: Option<f64>,
    #[serde(default)]
    azimuth_max_deg: Option<f64>,
    #[serde(default)]
    time_window: Option<TimeWindowInput>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
struct TimeWindowInput {
    start_mjd_utc: f64,
    end_mjd_utc: f64,
}

#[derive(Debug, Default, Deserialize)]
struct SoftConstraintsInput {
    #[serde(default)]
    priority: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct DependencyInput {
    from: u64,
    to: u64,
}

fn main() {
    env_logger::init();

    if let Err(error) = run() {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 || args.len() > 4 {
        eprintln!(
            "Usage: {} <input_json> [horizon_start_mjd horizon_end_mjd]\n\
             Example: {} data/ctao_n.json",
            args[0], args[0]
        );
        return Err("invalid CLI arguments".to_string());
    }

    let input_path = resolve_input_path(&args[1]);
    let horizon_override = parse_horizon_args(&args[2..])?;

    let payload_text = fs::read_to_string(&input_path)
        .map_err(|error| format!("failed to read {}: {error}", input_path.display()))?;
    let payload: Vec<SchedulingBlockInput> = serde_json::from_str(&payload_text)
        .map_err(|error| format!("failed to parse {}: {error}", input_path.display()))?;

    if payload.is_empty() {
        return Err("input JSON contains no scheduling blocks".to_string());
    }

    let (tasks_by_id, blocks) = build_domain(&payload)?;
    let horizon = build_horizon(&payload, horizon_override)?;
    let site = roque_site();

    let possible_periods = preschedule(&blocks, &tasks_by_id, &horizon, &site)
        .map_err(|error| format!("prescheduling failed: {error}"))?;
    let feasible_tasks = possible_periods
        .values()
        .filter(|p| !p.is_empty())
        .count();

    let total_tasks = tasks_by_id.len();
    let schedule = est::run_scheduler(
        tasks_by_id.into_values().collect(),
        &possible_periods,
        &horizon,
    )
    .map_err(|error| format!("EST run failed: {error}"))?;

    println!(
        "Loaded {} blocks and {} tasks from {}",
        blocks.len(),
        total_tasks,
        input_path.display()
    );
    println!(
        "Horizon (MJD): [{:.5}, {:.5})",
        horizon.start.value(),
        horizon.end.value()
    );
    println!("Prescheduler feasible tasks: {feasible_tasks}/{total_tasks}");
    print_schedule(&schedule);

    Ok(())
}

fn print_schedule(schedule: &scheduler::Schedule) {
    println!("EST placed {} tasks", schedule.placements.len());

    let mut placements: Vec<_> = schedule.placements.values().collect();
    placements.sort_by(|a, b| {
        a.start.to::<MJD>().value().total_cmp(&b.start.to::<MJD>().value())
    });

    for placement in placements.iter().take(20) {
        println!(
            "  task {}: [{:.5}, {:.5}) MJD",
            placement.task_id.0,
            placement.start.to::<MJD>().value(),
            placement.end.to::<MJD>().value()
        );
    }

    if placements.len() > 20 {
        println!("  ... {} more placements", placements.len() - 20);
    }
}

fn parse_horizon_args(args: &[String]) -> Result<Option<(f64, f64)>, String> {
    match args {
        [] => Ok(None),
        [start, end] => {
            let start = start
                .parse::<f64>()
                .map_err(|e| format!("invalid horizon_start_mjd '{start}': {e}"))?;
            let end = end
                .parse::<f64>()
                .map_err(|e| format!("invalid horizon_end_mjd '{end}': {e}"))?;
            Ok(Some((start, end)))
        }
        _ => Err("horizon override requires both start and end MJD values".to_string()),
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

fn build_domain(
    payload: &[SchedulingBlockInput],
) -> Result<(HashMap<TaskId, Task>, Vec<SchedulingBlock>), String> {
    let mut tasks_by_id = HashMap::new();
    let mut blocks = Vec::with_capacity(payload.len());

    for block_input in payload {
        let mut block = SchedulingBlock::new(SchedulingBlockId(block_input.id));

        for task_input in &block_input.tasks {
            let task = build_task(task_input)?;
            if tasks_by_id.insert(task.id, task).is_some() {
                return Err(format!(
                    "duplicate task id {} in input payload",
                    task_input.id
                ));
            }
            block.add_task(TaskId(task_input.id));
        }

        for dep in &block_input.dependencies {
            let (from, to) = (TaskId(dep.from), TaskId(dep.to));
            if !block.contains_task(from) || !block.contains_task(to) {
                return Err(format!(
                    "block {} dependency references unknown task id {} -> {}",
                    block_input.id, dep.from, dep.to
                ));
            }
            block
                .add_dependency(from, to, Dependency::DependsOn)
                .map_err(|e| {
                    format!(
                        "block {} dependency {} -> {} is invalid: {e}",
                        block_input.id, dep.from, dep.to
                    )
                })?;
        }

        blocks.push(block);
    }

    Ok((tasks_by_id, blocks))
}

fn build_task(input: &TaskInput) -> Result<Task, String> {
    let hard_constraints = build_hard_constraints(&input.hard_constraints)?;
    let soft_constraints = input
        .soft_constraints
        .as_ref()
        .and_then(|s| s.priority)
        .map(|p| SoftConstraintExpr::atom(PrioritySoftConstraint::new(p)));

    let name = input
        .name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("task-{}", input.id));

    let target = Direction::<ICRS>::new_raw(
        Degrees::new(input.target.dec_deg),
        Degrees::new(input.target.ra_deg),
    );

    Task::new(
        TaskId(input.id),
        name,
        target,
        Seconds::new(input.requested_duration_sec),
        hard_constraints,
        soft_constraints,
    )
    .map_err(|e| format!("invalid task {}: {e}", input.id))
}

fn build_hard_constraints(input: &HardConstraintsInput) -> Result<ConstraintExpr, String> {
    let mut constraints = Vec::new();

    if input.altitude_min_deg.is_some() || input.altitude_max_deg.is_some() {
        let min = input.altitude_min_deg.unwrap_or(0.0);
        let max = input.altitude_max_deg.unwrap_or(90.0);
        if min > max {
            return Err(format!(
                "invalid altitude bounds: min {min} is greater than max {max}"
            ));
        }
        constraints.push(ConstraintExpr::atom(AltitudeConstraint {
            min: Degrees::new(min),
            max: Degrees::new(max),
        }));
    }

    if input.azimuth_min_deg.is_some() || input.azimuth_max_deg.is_some() {
        let min = input.azimuth_min_deg.unwrap_or(0.0);
        let max = input.azimuth_max_deg.unwrap_or(360.0);
        constraints.push(ConstraintExpr::atom(AzimuthConstraint {
            min: Degrees::new(min),
            max: Degrees::new(max),
        }));
    }

    if let Some(tw) = input.time_window {
        let window = period_from_mjd(tw.start_mjd_utc, tw.end_mjd_utc)?;
        constraints.push(ConstraintExpr::atom(TimeConstraint { window }));
    }

    Ok(ConstraintExpr::Intersection(constraints))
}

fn build_horizon(
    payload: &[SchedulingBlockInput],
    override_range: Option<(f64, f64)>,
) -> Result<Period<MJD>, String> {
    if let Some((start, end)) = override_range {
        return period_from_mjd(start, end);
    }

    let windows = payload.iter().flat_map(|b| &b.tasks).filter_map(|t| t.hard_constraints.time_window);
    let (min_start, max_end) = windows.fold(
        (f64::INFINITY, f64::NEG_INFINITY),
        |(lo, hi), w| (lo.min(w.start_mjd_utc), hi.max(w.end_mjd_utc)),
    );

    if min_start.is_finite() && max_end.is_finite() {
        return period_from_mjd(min_start, max_end);
    }

    period_from_mjd(DEFAULT_HORIZON_START_MJD, DEFAULT_HORIZON_END_MJD)
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

fn roque_site() -> Geodetic<ECEF> {
    Geodetic::<ECEF>::new(
        Degrees::new(-17.892),
        Degrees::new(28.762),
        Quantity::<Meter>::new(2396.0),
    )
}
