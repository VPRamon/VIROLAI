mod config;
mod output;
mod problem;
mod run;

use config::{ExperimentSpec, HorizonOverride, RunConfig, SweepAxes};
use rayon::prelude::*;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    env_logger::init();
    if let Err(e) = run() {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let cli = match parse_cli(&args[0], &args[1..]) {
        Ok(cli) => cli,
        Err(e) if e == "help requested" => return Ok(()),
        Err(e) => return Err(e),
    };

    let spec = cli.spec_path.as_deref().map(load_spec).transpose()?;

    let input_path = resolve_input(cli.input_path, spec.as_ref().map(|s| &s.input_json))?;
    let output_dir = cli
        .output_dir
        .or_else(|| spec.as_ref().map(|s| s.output_dir.clone()))
        .ok_or("missing output_dir; provide --spec or --output-dir")?;
    let horizon_override = cli
        .horizon_override
        .or_else(|| spec.as_ref().and_then(|s| s.horizon_override));

    // Merge axes: CLI > spec > EST defaults
    let axes = merge_axes(cli.cli_axes, spec.as_ref().map(|s| &s.sweep));
    let runs = build_run_list(&axes)?;

    println!("Resolved {} EST runs", runs.len());
    for run in &runs {
        println!("  {}", run.slug());
    }

    let prepared = problem::prepare_problem(&input_path, horizon_override)?;

    let run_dir = output::prepare_output_dir(&output_dir)?;
    let schedules_dir = run_dir.join("schedules");
    fs::create_dir_all(&schedules_dir)
        .map_err(|e| format!("failed to create schedules directory: {e}"))?;

    let baseline_slug = runs[0].slug();

    let outcomes: Vec<_> = runs
        .par_iter()
        .map(|run| {
            let schedule_path = schedules_dir.join(format!("{}.json", run.schedule_file_stem()));
            run::execute_run(run, &prepared, &schedule_path)
        })
        .collect::<Result<Vec<_>, _>>()?;

    let rows: Vec<_> = outcomes
        .iter()
        .map(|o| output::build_comparison_row(&baseline_slug, o))
        .collect();

    let comparison_csv_path = run_dir.join("comparison.csv");
    output::write_comparison_csv(&comparison_csv_path, &rows)?;

    let manifest = output::build_manifest(
        &input_path,
        &run_dir,
        &comparison_csv_path,
        horizon_override,
        &baseline_slug,
        &outcomes,
    );
    let manifest_path = run_dir.join("manifest.json");
    let manifest_text = serde_json::to_string_pretty(&manifest)
        .map_err(|e| format!("failed to serialize manifest: {e}"))?;
    fs::write(&manifest_path, &manifest_text)
        .map_err(|e| format!("failed to write manifest: {e}"))?;

    println!("Artifacts written under {}", run_dir.display());
    println!("Manifest:       {}", manifest_path.display());
    println!("Comparison CSV: {}", comparison_csv_path.display());
    println!("Schedule files: {}", outcomes.len());

    Ok(())
}

// ── Spec loading ─────────────────────────────────────────────────────────────

fn load_spec(path: &Path) -> Result<ExperimentSpec, String> {
    let text = fs::read_to_string(path)
        .map_err(|e| format!("failed to read spec {}: {e}", path.display()))?;
    let mut spec: ExperimentSpec = serde_json::from_str(&text)
        .map_err(|e| format!("failed to parse spec {}: {e}", path.display()))?;
    let base_dir = path.parent().unwrap_or(Path::new("."));
    spec.input_json = resolve_relative(base_dir, &spec.input_json);
    spec.output_dir = resolve_relative(base_dir, &spec.output_dir);
    Ok(spec)
}

// ── Axis / run-list resolution ────────────────────────────────────────────────

/// Merges CLI axes with spec axes; CLI takes precedence.
fn merge_axes(cli: SweepAxes, spec: Option<&SweepAxes>) -> SweepAxes {
    SweepAxes {
        k_beams: pick_axis(cli.k_beams, spec.map(|s| &s.k_beams)),
        branching_factors: pick_axis(cli.branching_factors, spec.map(|s| &s.branching_factors)),
    }
}

/// Returns `cli` if non-empty, then `spec` if non-empty, otherwise `[]`.
fn pick_axis<T: Clone>(cli: Vec<T>, spec: Option<&Vec<T>>) -> Vec<T> {
    if !cli.is_empty() {
        return cli;
    }
    if let Some(v) = spec
        && !v.is_empty()
    {
        return v.clone();
    }
    vec![]
}

/// Computes the Cartesian product of all axes, filling empty axes with EST defaults.
///
/// The resulting list is sorted and deduplicated. Every [`RunConfig`] is validated
/// by attempting to construct its scheduler before returning.
fn build_run_list(axes: &SweepAxes) -> Result<Vec<RunConfig>, String> {
    let def = RunConfig::default();
    let k_beams = if axes.k_beams.is_empty() {
        vec![def.k_beams]
    } else {
        axes.k_beams.clone()
    };
    let branching_factors = if axes.branching_factors.is_empty() {
        vec![def.branching_factor]
    } else {
        axes.branching_factors.clone()
    };

    let mut run_set = BTreeSet::new();
    for &k in &k_beams {
        for &b in &branching_factors {
            run_set.insert(RunConfig {
                fom: def.fom,
                k_beams: k,
                branching_factor: b,
            });
        }
    }

    let runs: Vec<_> = run_set.into_iter().collect();
    for run in &runs {
        run.build_scheduler()?;
    }
    Ok(runs)
}

// ── Path resolution ───────────────────────────────────────────────────────────

fn resolve_input(
    cli_input: Option<String>,
    spec_input: Option<&PathBuf>,
) -> Result<PathBuf, String> {
    if let Some(arg) = cli_input {
        return Ok(resolve_data_path(&arg));
    }
    spec_input.cloned().ok_or_else(|| {
        "missing input_json; provide --spec or a positional <input_json>".to_string()
    })
}

fn resolve_data_path(arg: &str) -> PathBuf {
    let p = PathBuf::from(arg);
    if p.exists() {
        return p;
    }
    let under_data = PathBuf::from("data").join(arg);
    if under_data.exists() {
        return under_data;
    }
    p
}

fn resolve_relative(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    let relative = base.join(path);
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

// ── CLI parsing ───────────────────────────────────────────────────────────────

struct CliArgs {
    spec_path: Option<PathBuf>,
    input_path: Option<String>,
    output_dir: Option<PathBuf>,
    horizon_override: Option<HorizonOverride>,
    cli_axes: SweepAxes,
}

fn parse_cli(program: &str, args: &[String]) -> Result<CliArgs, String> {
    let mut spec_path: Option<PathBuf> = None;
    let mut output_dir: Option<PathBuf> = None;
    let mut cli_axes = SweepAxes::default();
    let mut positionals: Vec<&str> = Vec::new();

    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--spec" => spec_path = Some(PathBuf::from(flag_arg(args, &mut i, "--spec")?)),
            "--output-dir" => {
                output_dir = Some(PathBuf::from(flag_arg(args, &mut i, "--output-dir")?));
            }
            "--est-k-values" => {
                cli_axes.k_beams =
                    parse_range_list(flag_arg(args, &mut i, "--est-k-values")?, "--est-k-values")?;
            }
            "--est-b-values" => {
                cli_axes.branching_factors =
                    parse_range_list(flag_arg(args, &mut i, "--est-b-values")?, "--est-b-values")?;
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

    let (input_path, horizon_override) = match positionals.as_slice() {
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
        input_path,
        output_dir,
        horizon_override,
        cli_axes,
    })
}

/// Advances `*i` by 2 and returns the next argument value, or errors if absent.
fn flag_arg<'a>(args: &'a [String], i: &mut usize, flag: &str) -> Result<&'a str, String> {
    match args.get(*i + 1) {
        Some(v) => {
            *i += 2;
            Ok(v.as_str())
        }
        None => Err(format!("missing value for {flag}")),
    }
}

/// Parses a comma-separated list of integer values or inclusive ranges.
///
/// Each segment is either `N` (a single value) or `N-M` (all integers from N to M inclusive).
/// Example: `"1,3-5,8"` → `[1, 3, 4, 5, 8]`.
fn parse_range_list<T>(value: &str, flag: &str) -> Result<Vec<T>, String>
where
    T: std::str::FromStr + TryFrom<u64>,
    T::Err: std::fmt::Display,
    <T as TryFrom<u64>>::Error: std::fmt::Display,
{
    if value.trim().is_empty() {
        return Err(format!("empty value list for {flag}"));
    }
    let mut result = Vec::new();
    for segment in value.split(',') {
        let segment = segment.trim();
        if let Some((lo_str, hi_str)) = segment.split_once('-') {
            let lo: u64 = lo_str
                .trim()
                .parse()
                .map_err(|e| format!("invalid {flag} range start '{lo_str}': {e}"))?;
            let hi: u64 = hi_str
                .trim()
                .parse()
                .map_err(|e| format!("invalid {flag} range end '{hi_str}': {e}"))?;
            if lo > hi {
                return Err(format!(
                    "invalid {flag} range '{segment}': start must be <= end"
                ));
            }
            for n in lo..=hi {
                result.push(
                    T::try_from(n)
                        .map_err(|e| format!("value {n} out of range for {flag}: {e}"))?,
                );
            }
        } else {
            let n: T = segment
                .parse()
                .map_err(|e| format!("invalid {flag} value '{segment}': {e}"))?;
            result.push(n);
        }
    }
    if result.is_empty() {
        return Err(format!("empty value list for {flag}"));
    }
    Ok(result)
}

fn parse_f64(label: &str, value: &str) -> Result<f64, String> {
    value
        .parse::<f64>()
        .map_err(|e| format!("invalid {label} '{value}': {e}"))
}

fn print_usage(program: &str) {
    eprintln!(
        "Usage: {program} [--spec <spec.json>] [<input_json> [horizon_start_mjd horizon_end_mjd]]\n\
         \x20  [--output-dir <dir>]\n\
         \x20  [--est-k-values <ranges>] [--est-b-values <ranges>]\n\
         \n\
         Ranges: comma-separated values or inclusive integer ranges, e.g. 1-5 or 1,3-5,8\n\
         \n\
         Examples:\n\
         \x20  {program} --spec experiments/est.json\n\
         \x20  {program} data/ctao_n.json --output-dir out/ --est-k-values 1,10 --est-b-values 1-10"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_range_list_handles_single_values() {
        let result: Vec<u32> = parse_range_list("1,2,3", "--test").unwrap();
        assert_eq!(result, vec![1, 2, 3]);
    }

    #[test]
    fn parse_range_list_expands_ranges() {
        let result: Vec<u32> = parse_range_list("1-5", "--test").unwrap();
        assert_eq!(result, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn parse_range_list_handles_mixed_segments() {
        let result: Vec<usize> = parse_range_list("1,3-5,8", "--test").unwrap();
        assert_eq!(result, vec![1, 3, 4, 5, 8]);
    }

    #[test]
    fn parse_range_list_rejects_inverted_range() {
        assert!(parse_range_list::<u32>("5-1", "--test").is_err());
    }

    #[test]
    fn build_run_list_uses_defaults_when_axes_empty() {
        let axes = SweepAxes::default();
        let runs = build_run_list(&axes).unwrap();
        assert_eq!(runs, vec![RunConfig::default()]);
    }

    #[test]
    fn build_run_list_computes_cartesian_product() {
        let axes = SweepAxes {
            k_beams: vec![1, 2],
            branching_factors: vec![1],
        };
        let runs = build_run_list(&axes).unwrap();
        assert_eq!(runs.len(), 2);
        assert!(runs.iter().any(|r| r.k_beams == 1));
        assert!(runs.iter().any(|r| r.k_beams == 2));
    }

    #[test]
    fn build_run_list_deduplicates() {
        let axes = SweepAxes {
            k_beams: vec![1, 1],
            branching_factors: vec![1],
        };
        let runs = build_run_list(&axes).unwrap();
        assert_eq!(runs.len(), 1);
    }

    #[test]
    fn merge_axes_cli_wins_over_spec() {
        let cli = SweepAxes {
            k_beams: vec![5, 10],
            ..SweepAxes::default()
        };
        let spec = SweepAxes {
            k_beams: vec![1, 2, 3],
            ..SweepAxes::default()
        };
        let merged = merge_axes(cli, Some(&spec));
        assert_eq!(merged.k_beams, vec![5, 10]);
    }

    #[test]
    fn merge_axes_falls_back_to_spec_when_cli_empty() {
        let cli = SweepAxes::default();
        let spec = SweepAxes {
            k_beams: vec![1, 2, 3],
            ..SweepAxes::default()
        };
        let merged = merge_axes(cli, Some(&spec));
        assert_eq!(merged.k_beams, vec![1, 2, 3]);
    }
}
