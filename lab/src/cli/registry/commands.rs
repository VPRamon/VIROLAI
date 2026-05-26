//! Implementations for each `lab registry` subcommand.

use lab::registry::{
    BestOpts, ListOpts, Registry, RunRow, SortKey, default_sort_keys, parse_sort_key, registry_path,
};

use super::format::{parse_metrics, print_rows, row_json};
use super::scoring::{
    compare_rows_by_default_policy, dominates, metric_value, parse_objectives, parse_weights,
};
use super::{
    RegistryBestArgs, RegistryExportArgs, RegistryInspectArgs, RegistryListArgs,
    RegistryParetoArgs, RegistryRankArgs, RegistrySortArgs,
};

pub(super) fn list(args: RegistryListArgs) -> Result<(), String> {
    let db_path = registry_path(args.run_db.as_deref());
    let reg = Registry::open(&db_path)?;
    let sort = parse_sort_keys(&args.sort)?;
    let opts = ListOpts {
        dataset: args.dataset,
        algorithm: args.algorithm,
        metric: args.metric,
        min: args.min,
        max: args.max,
        sort: sort.clone(),
        limit: args.limit,
    };
    let rows = reg.list(&opts)?;
    print_rows(&rows, &args.format, &sort)?;
    Ok(())
}

pub(super) fn sort(args: RegistrySortArgs) -> Result<(), String> {
    let db_path = registry_path(args.run_db.as_deref());
    let reg = Registry::open(&db_path)?;
    let sort = parse_sort_keys(&args.sort)?;
    print_sort_policy(&sort);
    let rows = reg.list(&ListOpts {
        dataset: args.dataset,
        algorithm: args.algorithm,
        sort: sort.clone(),
        limit: args.limit.or(Some(20)),
        ..Default::default()
    })?;
    print_rows(&rows, &args.format, &sort)
}

pub(super) fn best(args: RegistryBestArgs) -> Result<(), String> {
    let db_path = registry_path(args.run_db.as_deref());
    let reg = Registry::open(&db_path)?;
    let sort = parse_sort_keys(&args.sort)?;
    let opts = BestOpts {
        dataset_id: args.dataset,
        algorithm: args.algorithm,
        sort: sort.clone(),
        limit: args.limit,
    };
    let rows = reg.best(&opts)?;
    print_sort_policy(&sort);
    print_rows(&rows, &args.format, &sort)
}

pub(super) fn rank(args: RegistryRankArgs) -> Result<(), String> {
    let weights = parse_weights(&args.weight)?;
    if weights.is_empty() {
        return Err("registry rank requires at least one --weight metric=value".to_string());
    }

    let db_path = registry_path(args.run_db.as_deref());
    let reg = Registry::open(&db_path)?;
    let mut scored: Vec<(RunRow, f64)> = reg
        .list(&ListOpts {
            dataset: args.dataset,
            algorithm: args.algorithm,
            limit: Some(10_000_000),
            ..Default::default()
        })?
        .into_iter()
        .map(|row| {
            let metrics = parse_metrics(&row.metrics_json);
            let score = weights
                .iter()
                .map(|(metric, weight)| metric_value(&metrics, metric).unwrap_or(0.0) * weight)
                .sum();
            (row, score)
        })
        .collect();
    scored.sort_by(|a, b| {
        b.1.total_cmp(&a.1)
            .then_with(|| a.0.run_key.cmp(&b.0.run_key))
    });
    scored.truncate(args.limit.unwrap_or(20));

    if args.format == "json" {
        let values: Vec<_> = scored
            .iter()
            .map(|(row, score)| row_json(row, Some(*score)))
            .collect();
        println!("{}", serde_json::to_string_pretty(&values).unwrap());
    } else {
        println!(
            "{:<18}  {:<12}  {:<8}  {:<30}  score",
            "run_key (prefix)", "dataset", "algo", "config_slug"
        );
        println!("{}", "-".repeat(96));
        for (row, score) in &scored {
            println!(
                "{:<18}  {:<12}  {:<8}  {:<30}  {:.6}",
                &row.run_key[..row.run_key.len().min(16)],
                row.dataset_id,
                row.algorithm,
                row.config_slug,
                score,
            );
        }
        println!("({} rows)", scored.len());
    }
    Ok(())
}

pub(super) fn pareto(args: RegistryParetoArgs) -> Result<(), String> {
    let objectives = parse_objectives(&args.maximize, &args.minimize)?;
    let db_path = registry_path(args.run_db.as_deref());
    let reg = Registry::open(&db_path)?;
    let rows = reg.list(&ListOpts {
        dataset: args.dataset,
        algorithm: args.algorithm,
        limit: Some(10_000_000),
        ..Default::default()
    })?;

    let mut front: Vec<RunRow> = rows
        .iter()
        .filter(|candidate| {
            !rows.iter().any(|other| {
                other.run_key != candidate.run_key && dominates(other, candidate, &objectives)
            })
        })
        .cloned()
        .collect();
    front.sort_by(compare_rows_by_default_policy);
    front.truncate(args.limit.unwrap_or(front.len()));

    if args.format == "json" {
        let values: Vec<_> = front.iter().map(|row| row_json(row, None)).collect();
        println!("{}", serde_json::to_string_pretty(&values).unwrap());
    } else {
        print_rows(&front, &args.format, &[])?;
    }
    Ok(())
}

pub(super) fn inspect(args: RegistryInspectArgs) -> Result<(), String> {
    let db_path = registry_path(args.run_db.as_deref());
    let reg = Registry::open(&db_path)?;
    let key = if args.run.len() == 64 {
        args.run.clone()
    } else {
        reg.resolve_prefix(&args.run)?
    };
    let row = reg
        .get_row(&key)?
        .ok_or_else(|| format!("no run found for key '{key}'"))?;
    println!("run_key:       {}", row.run_key);
    println!("dataset_id:    {}", row.dataset_id);
    println!("dataset_path:  {}", row.dataset_path);
    println!("algorithm:     {}", row.algorithm);
    println!("config_slug:   {}", row.config_slug);
    println!("created_at:    {}", row.created_at);
    println!("last_seen_at:  {}", row.last_seen_at);
    if let Some(cell) = &row.source_cell_id {
        println!("source_cell:   {cell}");
    }
    println!("\n--- identity ---");
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&row.identity_json) {
        println!("{}", serde_json::to_string_pretty(&v).unwrap());
    } else {
        println!("{}", row.identity_json);
    }
    println!("\n--- metrics ---");
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&row.metrics_json) {
        println!("{}", serde_json::to_string_pretty(&v).unwrap());
    } else {
        println!("{}", row.metrics_json);
    }
    Ok(())
}

pub(super) fn export(args: RegistryExportArgs) -> Result<(), String> {
    match (&args.run, &args.out, &args.out_dir) {
        (Some(key), Some(out), None) => {
            if out.exists() && !args.force {
                return Err(format!(
                    "output file '{}' already exists; use --force to overwrite",
                    out.display()
                ));
            }
            let db_path = registry_path(args.run_db.as_deref());
            let reg = Registry::open(&db_path)?;
            let full_key = if key.len() == 64 {
                key.clone()
            } else {
                reg.resolve_prefix(key)?
            };
            let row = reg
                .get_row(&full_key)?
                .ok_or_else(|| format!("no run found for key '{full_key}'"))?;
            let json = row.schedule_json.ok_or_else(|| {
                format!(
                    "run '{}' has no stored schedule JSON; rerun with `lab run --override` to regenerate",
                    &full_key[..full_key.len().min(16)]
                )
            })?;
            std::fs::write(out, json)
                .map_err(|e| format!("failed to write {}: {e}", out.display()))?;
            println!("exported -> {}", out.display());
            Ok(())
        }
        (None, None, Some(out_dir)) => {
            if !out_dir.exists() {
                std::fs::create_dir_all(out_dir)
                    .map_err(|e| format!("failed to create output dir: {e}"))?;
            }
            let db_path = registry_path(args.run_db.as_deref());
            let reg = Registry::open(&db_path)?;
            let sort = parse_sort_keys(&args.sort)?;
            let rows = reg.list(&ListOpts {
                dataset: args.dataset,
                algorithm: args.algorithm,
                metric: args.metric,
                min: args.min,
                max: args.max,
                sort,
                limit: args.limit.or(Some(100)),
            })?;
            let mut exported = 0usize;
            let mut unavailable = 0usize;
            let mut used_names = std::collections::HashSet::new();
            for row in &rows {
                let Some(json) = &row.schedule_json else {
                    eprintln!(
                        "  skip (no schedule_json): {} - rerun with `lab run --override`",
                        &row.run_key[..row.run_key.len().min(16)]
                    );
                    unavailable += 1;
                    continue;
                };
                let base_name = format!(
                    "{}__{}__{}.json",
                    row.dataset_id, row.algorithm, row.config_slug
                );
                let filename = if used_names.contains(&base_name) {
                    let key_prefix = &row.run_key[..row.run_key.len().min(8)];
                    format!(
                        "{}__{}__{}__{}.json",
                        row.dataset_id, row.algorithm, row.config_slug, key_prefix
                    )
                } else {
                    base_name.clone()
                };
                used_names.insert(filename.clone());
                used_names.insert(base_name);
                let dest = out_dir.join(&filename);
                if dest.exists() && !args.force {
                    eprintln!("  skip (exists, use --force): {}", dest.display());
                    continue;
                }
                std::fs::write(&dest, json)
                    .map_err(|e| format!("failed to write {}: {e}", dest.display()))?;
                exported += 1;
            }
            println!(
                "exported {exported} schedule(s) to {}{}",
                out_dir.display(),
                if unavailable > 0 {
                    format!(" ({unavailable} skipped: no schedule_json)")
                } else {
                    String::new()
                }
            );
            Ok(())
        }
        (Some(_), None, _) => Err("--run requires --out for single-run export".to_string()),
        (None, Some(_), _) => Err("--out requires --run for single-run export".to_string()),
        (Some(_), Some(_), Some(_)) => {
            Err("--out and --out-dir are mutually exclusive".to_string())
        }
        (None, None, None) => Err(
            "registry export requires either --run + --out (single) or --out-dir (filtered)"
                .to_string(),
        ),
    }
}

fn parse_sort_keys(raw: &[String]) -> Result<Vec<SortKey>, String> {
    raw.iter().map(|s| parse_sort_key(s)).collect()
}

fn print_sort_policy(sort: &[SortKey]) {
    let keys = if sort.is_empty() {
        default_sort_keys()
    } else {
        sort.to_vec()
    };
    let policy = keys
        .iter()
        .map(|k| format!("{}:{}", k.metric, k.direction.as_str()))
        .collect::<Vec<_>>()
        .join(", ");
    eprintln!("registry query sort: {policy}");
}
