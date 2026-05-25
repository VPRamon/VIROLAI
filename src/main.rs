use scheduler::manifest::{
    AlgorithmRef, ArtifactRef, Artifacts, DatasetRef, Horizon as ManifestHorizon, Links,
    MANIFEST_SCHEMA_VERSION, Manifest, Producer, Provenance, RunInfo, RunKind, RunStatus,
    ValidationReport, ValidationStatus,
};
use scheduler::metrics::{MetricsContext, ScheduleMetrics};
use scheduler::scheduler::{SchedulingAlgorithm, est, hap, lst};
use scheduler::telescope::Telescope;
use scheduler::time::{MJD, Period, Time};
use scheduler::{
    LocationMeta, PeriodMeta, Schedule, ScheduleMetadata, ScheduleOutput, SchedulingProblem,
    preschedule,
};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq)]
enum Algorithm {
    Est,
    Hap,
    Lst,
}

impl Algorithm {
    fn label(self) -> &'static str {
        match self {
            Self::Est => "EST",
            Self::Hap => "HAP",
            Self::Lst => "LST",
        }
    }
}

#[derive(Debug, Clone)]
struct CliArgs {
    input_path: PathBuf,
    output_path: Option<PathBuf>,
    horizon_override: Option<(f64, f64)>,
    algorithm: Algorithm,
    est_config: est::Configuration,
    est_fom: est::FomKind,
    hap_config: HapCliConfig,
    write_manifest: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HapCliConfig {
    num_crus: usize,
    cru_max_iterations: usize,
    rho: usize,
    random_seed: u64,
}

impl Default for HapCliConfig {
    fn default() -> Self {
        let config = hap::default_planner_config();
        Self {
            num_crus: config.population_size,
            cru_max_iterations: config.cru.max_iter,
            rho: selector_rho(config.cru.selector).unwrap_or(3),
            random_seed: config.seed,
        }
    }
}

impl HapCliConfig {
    fn planner_config(self) -> hap::PlannerConfig {
        let num_crus = self.num_crus.max(1);
        hap::PlannerConfig::hap(
            self.cru_max_iterations,
            self.rho,
            num_crus,
            hap::SurvivorSelector::ElitistTopK { k: num_crus },
            self.random_seed,
        )
    }
}

fn selector_rho(selector: hap::Selector) -> Option<usize> {
    match selector {
        hap::Selector::Stochastic { rho } => Some(rho),
        hap::Selector::Deterministic | hap::Selector::Random => None,
    }
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

    let input_path = cli.input_path.clone();
    let horizon_override = cli.horizon_override;

    let text = fs::read_to_string(&input_path)
        .map_err(|e| format!("failed to read {}: {e}", input_path.display()))?;
    let raw_json: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("failed to parse JSON {}: {e}", input_path.display()))?;
    let problem: SchedulingProblem = serde_json::from_str(&text)
        .map_err(|e| format!("failed to parse {}: {e}", input_path.display()))?;

    if problem.task_count() == 0 {
        return Err("input JSON contains no tasks".to_string());
    }

    let horizon = build_horizon(problem.detected_horizon, horizon_override)?;
    let telescope = problem
        .telescope
        .as_ref()
        .ok_or_else(|| "missing observing site in input; expected resources[0]".to_string())?;
    let total_tasks = problem.task_count();

    let preschedule_start = Instant::now();
    let possible_periods = preschedule(&problem, &horizon, telescope)
        .map_err(|e| format!("prescheduling failed: {e}"))?;
    let preschedule_elapsed = preschedule_start.elapsed();

    let feasible_tasks = possible_periods.values().filter(|p| !p.is_empty()).count();

    let scheduler = build_scheduler(&cli)?;
    let algo_start = Instant::now();
    let schedule = scheduler
        .run(&problem, &possible_periods, &horizon)
        .map_err(|e| format!("{} run failed: {e}", cli.algorithm.label()))?;
    let algo_elapsed = algo_start.elapsed();

    println!(
        "Loaded {} blocks and {} tasks from {}",
        problem.block_count(),
        total_tasks,
        input_path.display()
    );
    print_telescope(telescope);
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
        Algorithm::Est | Algorithm::Lst => {
            let label = cli.algorithm.label();
            println!(
                "{label} config: fom={}, endangered_threshold={}, k={}, b={}",
                cli.est_fom,
                cli.est_config.endangered_threshold,
                cli.est_config.k_beams,
                cli.est_config.branching_factor,
            );
            println!("{label} elapsed: {:.3}s", algo_elapsed.as_secs_f64());
        }
        Algorithm::Hap => {
            println!(
                "HAP config: num_crus={}, cru_iterations={}, rho={}, seed={}",
                cli.hap_config.num_crus,
                cli.hap_config.cru_max_iterations,
                cli.hap_config.rho,
                cli.hap_config.random_seed,
            );
            println!("HAP elapsed: {:.3}s", algo_elapsed.as_secs_f64());
        }
    }
    println!(
        "Total scheduling elapsed: {:.3}s",
        (preschedule_elapsed + algo_elapsed).as_secs_f64()
    );
    print_schedule(&schedule, cli.algorithm);

    let output_path = cli
        .output_path
        .clone()
        .unwrap_or_else(|| default_output_path(&input_path));
    let metadata = build_schedule_metadata(telescope, &horizon, &cli);
    let metrics =
        ScheduleMetrics::compute(&schedule, &problem, &horizon, &MetricsContext::default());
    let metrics_value = serde_json::to_value(&metrics)
        .map_err(|e| format!("failed to serialize schedule metrics: {e}"))?;
    let output =
        ScheduleOutput::new(raw_json, &schedule, Some(metadata)).with_metrics(metrics_value);
    let output_text = serde_json::to_string_pretty(&output)
        .map_err(|e| format!("failed to serialize schedule output: {e}"))?;
    fs::write(&output_path, &output_text)
        .map_err(|e| format!("failed to write {}: {e}", output_path.display()))?;
    println!("Schedule written to {}", output_path.display());

    if cli.write_manifest {
        let manifest_path = PathBuf::from(format!("{}.manifest.json", output_path.display()));
        match write_manifest_sidecar(&output_path, &manifest_path, &input_path, &cli, &horizon) {
            Ok(()) => println!("Manifest written to {}", manifest_path.display()),
            Err(e) => eprintln!("Warning: failed to write manifest: {e}"),
        }
    }

    Ok(())
}

fn parse_cli_args(program: &str, args: &[String]) -> Result<CliArgs, String> {
    if args.is_empty() {
        print_usage(program);
        return Err("invalid CLI arguments".to_string());
    }

    let mut positionals: Vec<&str> = Vec::new();
    let mut output_path: Option<PathBuf> = None;
    let mut algorithm = Algorithm::Est;
    let mut est_config = est::Configuration::default();
    let mut est_fom = est::FomKind::default();
    let mut hap_config = HapCliConfig::default();
    let mut write_manifest = false;

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
                        "missing value for {flag} (expected 'est', 'hap', or 'lst')"
                    ));
                };
                algorithm = match value.as_str() {
                    "est" => Algorithm::Est,
                    "hap" => Algorithm::Hap,
                    "lst" => Algorithm::Lst,
                    _ => {
                        print_usage(program);
                        return Err(format!(
                            "invalid {flag} value '{value}': expected 'est', 'hap', or 'lst'"
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
                        "missing value for {flag} (expected 'soft_constraint' or 'future_flexibility')"
                    ));
                };
                est_fom = value
                    .parse::<est::FomKind>()
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
            "--hap-rho" => {
                let flag = args[i].as_str();
                let Some(value) = args.get(i + 1) else {
                    print_usage(program);
                    return Err(format!(
                        "missing value for {flag} (expected an unsigned integer)"
                    ));
                };
                hap_config.rho = value
                    .parse::<usize>()
                    .map_err(|e| format!("invalid {flag} value '{value}': {e}"))?;
                hap_flags_set.push("--hap-rho");
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
                print_usage(program);
                return Err(
                    "--hap-impatience-alpha is not supported by the active HAP planner".to_string(),
                );
            }
            "-o" | "--output" => {
                let flag = args[i].as_str();
                let Some(value) = args.get(i + 1) else {
                    print_usage(program);
                    return Err(format!("missing value for {flag} (expected a file path)"));
                };
                output_path = Some(PathBuf::from(value));
                i += 2;
            }
            "--write-manifest" => {
                write_manifest = true;
                i += 1;
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
    if (algorithm == Algorithm::Est || algorithm == Algorithm::Lst) && !hap_flags_set.is_empty() {
        print_usage(program);
        return Err(format!(
            "HAP flags ({}) cannot be used with --algorithm {}",
            hap_flags_set.join(", "),
            cli_name(algorithm),
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
        output_path,
        horizon_override,
        algorithm,
        est_config,
        est_fom,
        hap_config,
        write_manifest,
    })
}

fn print_usage(program: &str) {
    eprintln!(
        "Usage: {program} <input_json> [horizon_start_mjd horizon_end_mjd] [-o <output_json>] [--write-manifest] [--algorithm est|hap|lst] [EST/LST options] [HAP options]\n\
         Output: [-o|--output <path>]  write schedule to this file (default: <input_stem>_schedule_<YYYYMMDD_HHMMSS>.json); --write-manifest also writes <output>.manifest.json\n\
         EST/LST options: [--est-fom <soft_constraint|future_flexibility>] [--est-e <u32>] [--est-k <usize>] [--est-b <usize>]\n\
         HAP options: [--hap-num-crus <usize>] [--hap-cru-iterations <usize>] [--hap-rho <usize>] [--hap-seed <u64>]\n\
         Aliases: --est-endangered-threshold <u32> for --est-e, --est-schedule-states <usize> for --est-k, --est-branching-factor <usize> for --est-b\n\
         Example: {program} data/ctao_n.json --algorithm est --est-fom soft_constraint --est-e 2 --est-k 5 --est-b 3\n\
         Example: {program} data/ctao_n.json --algorithm lst --est-fom soft_constraint --est-e 2 --est-k 5 --est-b 3\n\
         Example: {program} data/ctao_n.json --algorithm est --est-fom future_flexibility --est-k 5 --est-b 3\n\
         Example: {program} data/ctao_n.json -o out/my_schedule.json --algorithm hap --hap-num-crus 8 --hap-seed 42"
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
    println!("{} placed {} tasks", algorithm.label(), schedule.len());

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

fn default_output_path(input: &Path) -> PathBuf {
    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("schedule");
    let parent = input.parent().unwrap_or(Path::new("."));
    let ts = timestamp_str();
    parent.join(format!("{stem}_schedule_{ts}.json"))
}

fn timestamp_str() -> String {
    // Seconds since UNIX epoch → YYYYMMDD_HHMMSS (UTC).
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let s = secs % 60;
    let m = (secs / 60) % 60;
    let h = (secs / 3600) % 24;
    // Days since epoch (1970-01-01)
    let days = secs / 86400;
    let (y, mo, d) = days_to_ymd(days);
    format!("{y:04}{mo:02}{d:02}_{h:02}{m:02}{s:02}")
}

fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    // Gregorian proleptic calendar from days since 1970-01-01.
    let mut year = 1970u64;
    loop {
        let leap = is_leap(year);
        let days_in_year = if leap { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }
    let leap = is_leap(year);
    let month_days: [u64; 12] = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 1u64;
    for &md in &month_days {
        if days < md {
            break;
        }
        days -= md;
        month += 1;
    }
    (year, month, days + 1)
}

fn is_leap(y: u64) -> bool {
    (y.is_multiple_of(4) && !y.is_multiple_of(100)) || y.is_multiple_of(400)
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

fn build_schedule_metadata(
    telescope: &Telescope,
    horizon: &Period<MJD>,
    cli: &CliArgs,
) -> ScheduleMetadata {
    let location = LocationMeta {
        name: telescope.name.clone(),
        longitude_deg: telescope.location.lon.value(),
        latitude_deg: telescope.location.lat.value(),
        height_m: telescope.location.height.value(),
    };

    let period = PeriodMeta {
        start_mjd_utc: horizon.start.value(),
        end_mjd_utc: horizon.end.value(),
    };

    let (algorithm, algorithm_config) = match cli.algorithm {
        Algorithm::Est | Algorithm::Lst => {
            let config = serde_json::json!({
                "k_beams": cli.est_config.k_beams,
                "branching_factor": cli.est_config.branching_factor,
                "endangered_threshold": cli.est_config.endangered_threshold,
                "fom": cli.est_fom.to_string(),
            });
            (cli_name(cli.algorithm).to_string(), config)
        }
        Algorithm::Hap => {
            let config = serde_json::json!({
                "num_crus": cli.hap_config.num_crus,
                "cru_max_iterations": cli.hap_config.cru_max_iterations,
                "rho": cli.hap_config.rho,
                "random_seed": cli.hap_config.random_seed,
            });
            ("hap".to_string(), config)
        }
    };

    ScheduleMetadata {
        algorithm,
        algorithm_config,
        location: Some(location),
        period: Some(period),
        dataset_id: None,
        dataset_label: None,
    }
}

fn write_manifest_sidecar(
    schedule_path: &Path,
    manifest_path: &Path,
    input_path: &Path,
    cli: &CliArgs,
    horizon: &Period<MJD>,
) -> Result<(), String> {
    let schedule_text =
        fs::read_to_string(schedule_path).map_err(|e| format!("read schedule: {e}"))?;
    let schedule_doc: serde_json::Value =
        serde_json::from_str(&schedule_text).map_err(|e| format!("parse schedule: {e}"))?;

    let metrics: ScheduleMetrics = schedule_doc
        .get("schedule_metrics")
        .ok_or_else(|| "schedule has no schedule_metrics".to_string())
        .and_then(|v| {
            serde_json::from_value(v.clone()).map_err(|e| format!("parse metrics: {e}"))
        })?;

    let schedule_sha = sha256_file(schedule_path)?;
    let schedule_size = fs::metadata(schedule_path).map(|m| m.len()).unwrap_or(0);

    let dataset_id = input_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();
    let dataset_sha = sha256_file(input_path).unwrap_or_else(|_| "0".repeat(64));

    let algorithm_id = cli_name(cli.algorithm).to_string();
    let algorithm_config = match cli.algorithm {
        Algorithm::Est | Algorithm::Lst => serde_json::json!({
            "fom": cli.est_fom.to_string(),
            "endangered_threshold": cli.est_config.endangered_threshold,
            "k_beams": cli.est_config.k_beams,
            "branching_factor": cli.est_config.branching_factor,
        }),
        Algorithm::Hap => serde_json::json!({
            "num_crus": cli.hap_config.num_crus,
            "cru_max_iterations": cli.hap_config.cru_max_iterations,
            "rho": cli.hap_config.rho,
            "random_seed": cli.hap_config.random_seed,
        }),
    };

    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let horizon_manifest = ManifestHorizon {
        start_mjd_utc: horizon.start.value(),
        end_mjd_utc: horizon.end.value(),
    };

    let manifest = Manifest {
        manifest_schema_version: MANIFEST_SCHEMA_VERSION.to_string(),
        manifest_id: uuid::Uuid::new_v4().to_string(),
        created_at: now.clone(),
        producer: Producer {
            name: "scheduler".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            git_sha: option_env!("GIT_SHA").map(str::to_string),
            host: std::env::var("HOSTNAME").ok().filter(|s| !s.is_empty()),
        },
        dataset: DatasetRef {
            id: dataset_id.clone(),
            name: dataset_id,
            source_path: input_path.display().to_string(),
            sha256: dataset_sha,
            schema_version: "scheduling_problem/1".to_string(),
        },
        algorithm: AlgorithmRef {
            id: algorithm_id.clone(),
            label: algorithm_id.to_uppercase(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            config: algorithm_config,
        },
        run: RunInfo {
            run_id: "standalone".to_string(),
            kind: RunKind::Single,
            started_at: now.clone(),
            finished_at: now.clone(),
            status: RunStatus::Completed,
            exit_code: 0,
        },
        horizon: horizon_manifest,
        metrics,
        artifacts: Artifacts {
            schedule: Some(ArtifactRef {
                uri: format!(
                    "file://{}",
                    fs::canonicalize(schedule_path)
                        .unwrap_or_else(|_| schedule_path.to_path_buf())
                        .display()
                ),
                size_bytes: schedule_size,
                sha256: schedule_sha,
                media_type: "application/json".to_string(),
            }),
            trace: None,
            problem: None,
        },
        links: Links::default(),
        provenance: Provenance {
            matrix_run_id: None,
            cell_id: None,
            parent_manifest: None,
            repo_root: None,
            cli_args: std::env::args().collect(),
        },
        validation: ValidationReport {
            status: ValidationStatus::Valid,
            issues: Vec::new(),
        },
        extensions: serde_json::Value::Null,
    };

    let text =
        serde_json::to_string_pretty(&manifest).map_err(|e| format!("serialize manifest: {e}"))?;
    fs::write(manifest_path, &text).map_err(|e| format!("write manifest: {e}"))?;
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(format!("{:x}", hasher.finalize()))
}

fn build_scheduler(cli: &CliArgs) -> Result<Box<dyn SchedulingAlgorithm>, String> {
    match cli.algorithm {
        Algorithm::Est => {
            let scheduler = est::EstScheduler::from_parts(cli.est_config, cli.est_fom.into_fom())
                .map_err(|e| format!("invalid EST configuration: {e}"))?;
            Ok(Box::new(scheduler))
        }
        Algorithm::Lst => {
            let scheduler = lst::LstScheduler::with_fom(cli.est_config, cli.est_fom.into_fom())
                .map_err(|e| format!("invalid LST configuration: {e}"))?;
            Ok(Box::new(scheduler))
        }
        Algorithm::Hap => Ok(Box::new(hap::HapScheduler::new(
            cli.hap_config.planner_config(),
        ))),
    }
}

fn cli_name(algorithm: Algorithm) -> &'static str {
    match algorithm {
        Algorithm::Est => "est",
        Algorithm::Hap => "hap",
        Algorithm::Lst => "lst",
    }
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
