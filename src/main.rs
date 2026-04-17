use scheduler::scheduler::est;
use scheduler::telescope::Telescope;
use scheduler::time::{MJD, Period, Time};
use scheduler::{Schedule, ScheduleOutput, SchedulingProblem, preschedule};
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

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
    let raw_json: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("failed to parse JSON {}: {e}", input_path.display()))?;
    let problem: SchedulingProblem = serde_json::from_str(&text)
        .map_err(|e| format!("failed to parse {}: {e}", input_path.display()))?;

    if problem.tasks.is_empty() {
        return Err("input JSON contains no tasks".to_string());
    }

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
    let total_tasks = tasks.len();

    let preschedule_start = Instant::now();
    let possible_periods = preschedule(&blocks, &tasks, &horizon, &telescope)
        .map_err(|e| format!("prescheduling failed: {e}"))?;
    let preschedule_elapsed = preschedule_start.elapsed();

    let feasible_tasks = possible_periods.values().filter(|p| !p.is_empty()).count();

    let est_start = Instant::now();
    let schedule = est::run_scheduler(tasks.into_values().collect(), &possible_periods, &horizon)
        .map_err(|e| format!("EST run failed: {e}"))?;
    let est_elapsed = est_start.elapsed();

    println!(
        "Loaded {} blocks and {} tasks from {}",
        blocks.len(),
        total_tasks,
        input_path.display()
    );
    print_telescope(&telescope);
    println!(
        "Horizon (MJD): [{:.5}, {:.5})",
        horizon.start.value(),
        horizon.end.value()
    );
    println!(
        "Prescheduler feasible tasks: {feasible_tasks}/{total_tasks} in {:.3}s",
        preschedule_elapsed.as_secs_f64()
    );
    println!("EST elapsed: {:.3}s", est_elapsed.as_secs_f64());
    println!(
        "Total scheduling elapsed: {:.3}s",
        (preschedule_elapsed + est_elapsed).as_secs_f64()
    );
    print_schedule(&schedule);

    let output_path = derive_output_path(&input_path);
    let output = ScheduleOutput::new(raw_json, &schedule);
    let output_text = serde_json::to_string_pretty(&output)
        .map_err(|e| format!("failed to serialize schedule output: {e}"))?;
    fs::write(&output_path, &output_text)
        .map_err(|e| format!("failed to write {}: {e}", output_path.display()))?;
    println!("Schedule written to {}", output_path.display());

    Ok(())
}

fn print_telescope(telescope: &Telescope) {
    println!(
        "Telescope: id={} name='{}' lon={:.4}° lat={:.4}° h={:.0}m",
        telescope.id,
        telescope.name,
        telescope.location.lon.value(),
        telescope.location.lat.value(),
        telescope.location.height.value(),
    );
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

fn derive_output_path(input: &PathBuf) -> PathBuf {
    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("schedule");
    let parent = input.parent().unwrap_or(std::path::Path::new("."));
    parent.join(format!("{stem}_schedule.json"))
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
