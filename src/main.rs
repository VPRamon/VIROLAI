use scheduler::scheduler::est;
use scheduler::time::{MJD, Period, Time};
use scheduler::{Schedule, SchedulingProblem, preschedule};
use siderust::coordinates::centers::Geodetic;
use siderust::coordinates::frames::ECEF;
use std::fs;
use std::path::PathBuf;

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

    let text = fs::read_to_string(&input_path)
        .map_err(|e| format!("failed to read {}: {e}", input_path.display()))?;
    let problem: SchedulingProblem = serde_json::from_str(&text)
        .map_err(|e| format!("failed to parse {}: {e}", input_path.display()))?;

    if problem.tasks.is_empty() {
        return Err("input JSON contains no tasks".to_string());
    }

    // Destructure to allow independent borrows of tasks and blocks.
    let SchedulingProblem {
        tasks,
        blocks,
        detected_horizon,
        location,
    } = problem;

    let horizon = build_horizon(detected_horizon, horizon_override)?;
    let site = build_site(location)?;
    let blocks: Vec<_> = blocks.into_values().collect();

    let possible_periods = preschedule(&blocks, &tasks, &horizon, &site)
        .map_err(|e| format!("prescheduling failed: {e}"))?;

    let total_tasks = tasks.len();
    let feasible_tasks = possible_periods.values().filter(|p| !p.is_empty()).count();

    let schedule = est::run_scheduler(tasks.into_values().collect(), &possible_periods, &horizon)
        .map_err(|e| format!("EST run failed: {e}"))?;

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

fn print_schedule(schedule: &Schedule) {
    println!("EST placed {} tasks", schedule.placements.len());

    let mut placements: Vec<_> = schedule.placements.values().collect();
    placements.sort_by(|a, b| {
        a.start
            .to::<MJD>()
            .value()
            .total_cmp(&b.start.to::<MJD>().value())
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

fn build_horizon(
    detected: Option<Period<MJD>>,
    override_range: Option<(f64, f64)>,
) -> Result<Period<MJD>, String> {
    if let Some((start, end)) = override_range {
        return period_from_mjd(start, end);
    }
    detected.ok_or_else(|| {
        "missing schedule_time_window in input and no horizon override was provided".to_string()
    })
}

fn build_site(location: Option<Geodetic<ECEF>>) -> Result<Geodetic<ECEF>, String> {
    location.ok_or_else(|| {
        "missing top-level location in input; expected geodetic coordinates".to_string()
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
