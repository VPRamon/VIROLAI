use scheduler::experiment::est::{
    EstExperimentCliOverrides, HorizonOverride, load_experiment_spec, resolve_experiment,
    run_experiment,
};
use scheduler::scheduler::est::EstFomKind;
use std::path::PathBuf;

#[derive(Debug, Clone)]
struct CliArgs {
    spec_path: Option<PathBuf>,
    overrides: EstExperimentCliOverrides,
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

    let spec = cli
        .spec_path
        .as_deref()
        .map(load_experiment_spec)
        .transpose()?;

    let resolved = resolve_experiment(spec, cli.overrides)?;
    println!(
        "Resolved {} EST runs (baseline: {})",
        resolved.runs.len(),
        resolved.baseline_slug()
    );

    for run in &resolved.runs {
        println!("  {}", run.slug());
    }

    let execution = run_experiment(&resolved)?;
    println!("Artifacts written under {}", execution.output_dir.display());
    println!("Manifest written to {}", execution.manifest_path.display());
    println!(
        "Comparison CSV written to {}",
        execution.comparison_csv_path.display()
    );
    println!(
        "Generated {} schedule files",
        execution.schedule_paths.len()
    );

    Ok(())
}

fn parse_cli_args(program: &str, args: &[String]) -> Result<CliArgs, String> {
    let mut spec_path = None;
    let mut output_dir = None;
    let mut fom_values = None;
    let mut endangered_threshold_values = None;
    let mut k_beam_values = None;
    let mut branching_factor_values = None;
    let mut positionals: Vec<&str> = Vec::new();

    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--spec" => {
                let Some(value) = args.get(i + 1) else {
                    print_usage(program);
                    return Err("missing value for --spec".to_string());
                };
                spec_path = Some(PathBuf::from(value));
                i += 2;
            }
            "--output-dir" => {
                let Some(value) = args.get(i + 1) else {
                    print_usage(program);
                    return Err("missing value for --output-dir".to_string());
                };
                output_dir = Some(PathBuf::from(value));
                i += 2;
            }
            "--est-fom-values" => {
                let Some(value) = args.get(i + 1) else {
                    print_usage(program);
                    return Err("missing value for --est-fom-values".to_string());
                };
                fom_values = Some(parse_fom_values(value)?);
                i += 2;
            }
            "--est-e-values" => {
                let Some(value) = args.get(i + 1) else {
                    print_usage(program);
                    return Err("missing value for --est-e-values".to_string());
                };
                endangered_threshold_values = Some(parse_list::<u32>(value, "--est-e-values")?);
                i += 2;
            }
            "--est-k-values" => {
                let Some(value) = args.get(i + 1) else {
                    print_usage(program);
                    return Err("missing value for --est-k-values".to_string());
                };
                k_beam_values = Some(parse_list::<usize>(value, "--est-k-values")?);
                i += 2;
            }
            "--est-b-values" => {
                let Some(value) = args.get(i + 1) else {
                    print_usage(program);
                    return Err("missing value for --est-b-values".to_string());
                };
                branching_factor_values = Some(parse_list::<usize>(value, "--est-b-values")?);
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

    let (input_json, horizon_override) = match positionals.as_slice() {
        [] => (None, None),
        [input] => (Some((*input).to_string()), None),
        [input, start, end] => (
            Some((*input).to_string()),
            Some(HorizonOverride {
                start_mjd: parse_f64("horizon_start_mjd", start)?,
                end_mjd: parse_f64("horizon_end_mjd", end)?,
            }),
        ),
        _ => {
            print_usage(program);
            return Err(
                "expected optional positional <input_json> [horizon_start_mjd horizon_end_mjd]"
                    .to_string(),
            );
        }
    };

    Ok(CliArgs {
        spec_path,
        overrides: EstExperimentCliOverrides {
            input_json,
            output_dir,
            horizon_override,
            fom_values,
            endangered_threshold_values,
            k_beam_values,
            branching_factor_values,
        },
    })
}

fn parse_fom_values(value: &str) -> Result<Vec<EstFomKind>, String> {
    if value.trim().is_empty() {
        return Err("empty value list for --est-fom-values".to_string());
    }

    value
        .split(',')
        .map(|entry| entry.trim().parse::<EstFomKind>())
        .collect()
}

fn parse_list<T>(value: &str, flag: &str) -> Result<Vec<T>, String>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    if value.trim().is_empty() {
        return Err(format!("empty value list for {flag}"));
    }

    value
        .split(',')
        .map(|entry| {
            entry
                .trim()
                .parse::<T>()
                .map_err(|error| format!("invalid value '{entry}' for {flag}: {error}"))
        })
        .collect()
}

fn parse_f64(label: &str, value: &str) -> Result<f64, String> {
    value
        .parse::<f64>()
        .map_err(|error| format!("invalid {label} value '{value}': {error}"))
}

fn print_usage(program: &str) {
    eprintln!(
        "Usage: {program} [--spec <spec.json>] [<input_json> [horizon_start_mjd horizon_end_mjd]] [--output-dir <dir>] [--est-fom-values <task_count,soft_constraint>] [--est-e-values <u32,...>] [--est-k-values <usize,...>] [--est-b-values <usize,...>]\n\
         Example: {program} --spec experiments/est.json\n\
         Example: {program} data/ctao_n.json --output-dir out/ctao_n --est-fom-values task_count,soft_constraint --est-e-values 1,2 --est-k-values 1,4 --est-b-values 1,2"
    );
}
