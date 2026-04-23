use scheduler::scheduler::est;
use scheduler::scheduler::hap;
use scheduler::telescope::Telescope;
use scheduler::time::{MJD, Period, Time};
use scheduler::{Schedule, ScheduleOutput, SchedulingProblem, preschedule};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Algorithm {
    Est,
    Hap,
}

#[derive(Debug, Clone)]
struct CliArgs {
    input_path: PathBuf,
    horizon_override: Option<(f64, f64)>,
    algorithm: Algorithm,
    est_config: est::Configuration,
    est_fom: est::EstFomKind,
    hap_config: hap::Configuration,
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
    let cli = match parse_cli_args(&args[0], &args[1..]) {
        Ok(cli) => cli,
        Err(error) if error == "help requested" => return Ok(()),
        Err(error) => return Err(error),
    };

    let input_path = cli.input_path;
    let horizon_override = cli.horizon_override;

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
    let total_tasks = tasks.len();

    let preschedule_start = Instant::now();
    let possible_periods = preschedule(&blocks, &tasks, &horizon, &telescope)
        .map_err(|e| format!("prescheduling failed: {e}"))?;
    let preschedule_elapsed = preschedule_start.elapsed();

    let feasible_tasks = possible_periods.values().filter(|p| !p.is_empty()).count();

    let (schedule, algo_elapsed) = match cli.algorithm {
        Algorithm::Est => {
            let mut tasks_vec: Vec<_> = tasks.into_values().collect();
            tasks_vec.sort_by_key(|task| task.id.0);
            let start = Instant::now();
            let scheduler = est::EstScheduler::with_fom(cli.est_config, cli.est_fom.into_fom())
                .map_err(|e| format!("invalid EST configuration: {e}"))?;
            let schedule = scheduler
                .run_with_problem(&tasks_vec, &possible_periods, &horizon, &blocks)
                .map_err(|e| format!("EST run failed: {e}"))?;
            (schedule, start.elapsed())
        }
        Algorithm::Hap => {
            let start = Instant::now();
            let scheduler = hap::HapScheduler::new(cli.hap_config);
            let schedule = scheduler
                .run(&tasks, &possible_periods, &horizon, &blocks)
                .map_err(|e| format!("HAP run failed: {e}"))?;
            (schedule, start.elapsed())
        }
    };

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
    match cli.algorithm {
        Algorithm::Est => {
            println!(
                "EST config: fom={}, endangered_threshold={}, k={}, b={}",
                cli.est_fom,
                cli.est_config.endangered_threshold,
                cli.est_config.k_beams,
                cli.est_config.branching_factor,
            );
            println!("EST elapsed: {:.3}s", algo_elapsed.as_secs_f64());
        }
        Algorithm::Hap => {
            println!(
                "HAP config: num_crus={}, cru_iterations={}, stochastic_range={}, seed={}, impatience_alpha={}",
                cli.hap_config.num_crus,
                cli.hap_config.cru_max_iterations,
                cli.hap_config.stochastic_range,
                cli.hap_config.random_seed,
                cli.hap_config.impatience_alpha,
            );
            println!("HAP elapsed: {:.3}s", algo_elapsed.as_secs_f64());
        }
    }
    println!(
        "Total scheduling elapsed: {:.3}s",
        (preschedule_elapsed + algo_elapsed).as_secs_f64()
    );
    print_schedule(&schedule, cli.algorithm);

    let output_path = derive_output_path(&input_path);
    let output = ScheduleOutput::new(raw_json, &schedule);
    let output_text = serde_json::to_string_pretty(&output)
        .map_err(|e| format!("failed to serialize schedule output: {e}"))?;
    fs::write(&output_path, &output_text)
        .map_err(|e| format!("failed to write {}: {e}", output_path.display()))?;
    println!("Schedule written to {}", output_path.display());

    Ok(())
}

fn parse_cli_args(program: &str, args: &[String]) -> Result<CliArgs, String> {
    if args.is_empty() {
        print_usage(program);
        return Err("invalid CLI arguments".to_string());
    }

    let mut positionals: Vec<&str> = Vec::new();
    let mut algorithm = Algorithm::Est;
    let mut est_config = est::Configuration::default();
    let mut est_fom = est::EstFomKind::default();
    let mut hap_config = hap::Configuration::default();

    // Track which algorithm-specific flags were explicitly set
    let mut est_flags_set: Vec<&str> = Vec::new();
    let mut hap_flags_set: Vec<&str> = Vec::new();

    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--algorithm" => {
                let flag = args[i].as_str();
                let Some(value) = args.get(i + 1) else {
                    print_usage(program);
                    return Err(format!(
                        "missing value for {flag} (expected 'est' or 'hap')"
                    ));
                };
                algorithm = match value.as_str() {
                    "est" => Algorithm::Est,
                    "hap" => Algorithm::Hap,
                    _ => {
                        print_usage(program);
                        return Err(format!(
                            "invalid {flag} value '{value}': expected 'est' or 'hap'"
                        ));
                    }
                };
                i += 2;
            }
            "--est-fom" => {
                let flag = args[i].as_str();
                let Some(value) = args.get(i + 1) else {
                    print_usage(program);
                    return Err(format!(
                        "missing value for {flag} (expected 'soft_constraint')"
                    ));
                };
                est_fom = value
                    .parse::<est::EstFomKind>()
                    .map_err(|e| format!("invalid {flag} value '{value}': {e}"))?;
                est_flags_set.push("--est-fom");
                i += 2;
            }
            "--est-e" | "--est-endangered-threshold" => {
                let flag = args[i].as_str();
                let Some(value) = args.get(i + 1) else {
                    print_usage(program);
                    return Err(format!(
                        "missing value for {flag} (expected an unsigned integer)"
                    ));
                };
                est_config.endangered_threshold = value
                    .parse::<u32>()
                    .map_err(|e| format!("invalid {flag} value '{value}': {e}"))?;
                est_flags_set.push("--est-e");
                i += 2;
            }
            "--est-k" | "--est-schedule-states" => {
                let flag = args[i].as_str();
                let Some(value) = args.get(i + 1) else {
                    print_usage(program);
                    return Err(format!(
                        "missing value for {flag} (expected an unsigned integer)"
                    ));
                };
                est_config.k_beams = value
                    .parse::<usize>()
                    .map_err(|e| format!("invalid {flag} value '{value}': {e}"))?;
                est_flags_set.push("--est-k");
                i += 2;
            }
            "--est-b" | "--est-branching-factor" => {
                let flag = args[i].as_str();
                let Some(value) = args.get(i + 1) else {
                    print_usage(program);
                    return Err(format!(
                        "missing value for {flag} (expected an unsigned integer)"
                    ));
                };
                est_config.branching_factor = value
                    .parse::<usize>()
                    .map_err(|e| format!("invalid {flag} value '{value}': {e}"))?;
                est_flags_set.push("--est-b");
                i += 2;
            }
            "--hap-num-crus" => {
                let flag = args[i].as_str();
                let Some(value) = args.get(i + 1) else {
                    print_usage(program);
                    return Err(format!(
                        "missing value for {flag} (expected an unsigned integer)"
                    ));
                };
                hap_config.num_crus = value
                    .parse::<usize>()
                    .map_err(|e| format!("invalid {flag} value '{value}': {e}"))?;
                hap_flags_set.push("--hap-num-crus");
                i += 2;
            }
            "--hap-cru-iterations" => {
                let flag = args[i].as_str();
                let Some(value) = args.get(i + 1) else {
                    print_usage(program);
                    return Err(format!(
                        "missing value for {flag} (expected an unsigned integer)"
                    ));
                };
                hap_config.cru_max_iterations = value
                    .parse::<usize>()
                    .map_err(|e| format!("invalid {flag} value '{value}': {e}"))?;
                hap_flags_set.push("--hap-cru-iterations");
                i += 2;
            }
            "--hap-stochastic-range" => {
                let flag = args[i].as_str();
                let Some(value) = args.get(i + 1) else {
                    print_usage(program);
                    return Err(format!(
                        "missing value for {flag} (expected an unsigned integer)"
                    ));
                };
                hap_config.stochastic_range = value
                    .parse::<usize>()
                    .map_err(|e| format!("invalid {flag} value '{value}': {e}"))?;
                hap_flags_set.push("--hap-stochastic-range");
                i += 2;
            }
            "--hap-seed" => {
                let flag = args[i].as_str();
                let Some(value) = args.get(i + 1) else {
                    print_usage(program);
                    return Err(format!("missing value for {flag} (expected a u64)"));
                };
                hap_config.random_seed = value
                    .parse::<u64>()
                    .map_err(|e| format!("invalid {flag} value '{value}': {e}"))?;
                hap_flags_set.push("--hap-seed");
                i += 2;
            }
            "--hap-impatience-alpha" => {
                let flag = args[i].as_str();
                let Some(value) = args.get(i + 1) else {
                    print_usage(program);
                    return Err(format!(
                        "missing value for {flag} (expected a floating-point number)"
                    ));
                };
                hap_config.impatience_alpha = value
                    .parse::<f64>()
                    .map_err(|e| format!("invalid {flag} value '{value}': {e}"))?;
                hap_flags_set.push("--hap-impatience-alpha");
                i += 2;
            }
            "-h" | "--help" => {
                print_usage(program);
                return Err("help requested".to_string());
            }
            flag if flag.starts_with('-') => {
                print_usage(program);
                return Err(format!("unknown argument '{flag}'"));
            }
            value => {
                positionals.push(value);
                i += 1;
            }
        }
    }

    // Cross-flag validation
    if algorithm == Algorithm::Hap && !est_flags_set.is_empty() {
        print_usage(program);
        return Err(format!(
            "EST flags ({}) cannot be used with --algorithm hap",
            est_flags_set.join(", ")
        ));
    }
    if algorithm == Algorithm::Est && !hap_flags_set.is_empty() {
        print_usage(program);
        return Err(format!(
            "HAP flags ({}) cannot be used with --algorithm est",
            hap_flags_set.join(", ")
        ));
    }

    let (input_arg, horizon_override) = match positionals.as_slice() {
        [input] => (*input, None),
        [input, start, end] => (*input, parse_horizon_args(&[*start, *end])?),
        _ => {
            print_usage(program);
            return Err(
                "expected <input_json> and optional [horizon_start_mjd horizon_end_mjd]"
                    .to_string(),
            );
        }
    };

    Ok(CliArgs {
        input_path: resolve_input_path(input_arg),
        horizon_override,
        algorithm,
        est_config,
        est_fom,
        hap_config,
    })
}

fn print_usage(program: &str) {
    eprintln!(
        "Usage: {program} <input_json> [horizon_start_mjd horizon_end_mjd] [--algorithm est|hap] [EST options] [HAP options]\n\
         EST options: [--est-fom <soft_constraint>] [--est-e <u32>] [--est-k <usize>] [--est-b <usize>]\n\
         HAP options: [--hap-num-crus <usize>] [--hap-cru-iterations <usize>] [--hap-stochastic-range <usize>] [--hap-seed <u64>] [--hap-impatience-alpha <f64>]\n\
         Aliases: --est-endangered-threshold <u32> for --est-e, --est-schedule-states <usize> for --est-k, --est-branching-factor <usize> for --est-b\n\
         Example: {program} data/ctao_n.json --algorithm est --est-fom soft_constraint --est-e 2 --est-k 5 --est-b 3\n\
         Example: {program} data/ctao_n.json --algorithm hap --hap-num-crus 8 --hap-seed 42"
    );
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

fn print_schedule(schedule: &Schedule, algorithm: Algorithm) {
    let label = match algorithm {
        Algorithm::Est => "EST",
        Algorithm::Hap => "HAP",
    };
    println!("{label} placed {} tasks", schedule.len());

    let mut placements: Vec<_> = schedule.placements().collect();
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

fn parse_horizon_args(args: &[&str]) -> Result<Option<(f64, f64)>, String> {
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

fn derive_output_path(input: &Path) -> PathBuf {
    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("schedule");
    let parent = input.parent().unwrap_or(Path::new("."));
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
