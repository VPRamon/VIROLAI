# Lab

`lab` contains VIROLAI's experiment runner, SQLite result registry, dataset adapters, and user-facing workflow CLI.

The scheduler core is independent of any single dataset source. Experiment specs point to generic scheduling problem JSON files, while adapters translate external formats into that common model.

## Binaries

| Binary | Purpose |
| --- | --- |
| `virolai` | User-facing CLI for runs, sweeps, dataset adaptation, and publishing |
| `lab` | Lower-level matrix runner and registry query CLI |
| `lab-ctao-adapter` | Optional adapter for supported CTAO dataset files |
| `lab-migrate-schedule-dedup` | Registry migration utility |

## Run a sweep

```bash
cargo run -p lab --bin virolai -- sweep \
  --spec lab/sweep-fast.json \
  --run-db .lab/runs.sqlite
```

Useful options:

| Flag | Description |
| --- | --- |
| `--spec <FILE>` | Experiment specification JSON |
| `--run-db <PATH>` | Registry database path; defaults to `.lab/runs.sqlite` |
| `--parallel <N>` | Override `max_parallel` from the spec |
| `--override` | Re-execute cells already present in the registry |

The same matrix can be run directly with `lab`:

```bash
cargo run -p lab --bin lab -- run \
  --spec lab/sweep-fast.json \
  --run-db .lab/runs.sqlite
```

Both workflows write results to the registry. They do not create schedule files during execution.

## Experiment specification

An experiment specification defines datasets, algorithms, and parameter axes. The runner evaluates their Cartesian product.

```json
{
  "name": "fast-comparison",
  "max_parallel": 16,
  "datasets": [
    {
      "id": "sample",
      "path": "datasets/isdc_n.json",
      "label": "Sample dataset"
    }
  ],
  "algorithms": [
    {
      "kind": "est",
      "axes": {
        "endangered_thresholds": [0, 1],
        "k_beams": [1, 2],
        "branching_factors": [1, 2],
        "foms": ["soft_constraint"]
      }
    }
  ]
}
```

Top-level fields:

| Field | Description |
| --- | --- |
| `name` | Human-readable experiment name |
| `max_parallel` | Optional worker limit |
| `datasets` | Input scheduling problems |
| `algorithms` | Algorithm sweep definitions |
| `output_dir` | Legacy field retained for compatibility; ignored by the DB-only runner |

Dataset entries may include `id`, `path`, `label`, and an optional `horizon_override`.

## Algorithm axes

EST and LST support:

- `endangered_thresholds`
- `k_beams`
- `branching_factors`
- `foms`

Multi-cursor adds `layouts` and uses the same beam-search axes.

HAP supports:

- `iota_max_values`
- `rho_values`
- `population_sizes`
- `survivor_modes`
- `survivor_caps`
- `seeds`

See [`../docs/algorithms/sweep-configuration.md`](../docs/algorithms/sweep-configuration.md) for examples.

## Registry

The SQLite registry stores one row per run identity and deduplicates semantically identical schedule bodies.

Common commands:

```bash
lab registry list
lab registry inspect --run <KEY|PREFIX>
lab registry best --dataset <ID>
lab registry sort --sort priority_density:desc
lab registry pareto --dataset <ID>
lab registry export --out-dir out/results
lab registry doctor
```

A run row stores identity, configuration, metrics, metadata, and a `schedule_hash` reference. The `schedules` table stores the invariant schedule body keyed by that hash.

### Export schedules

Export one run:

```bash
lab registry export \
  --run b50d151629d65018 \
  --out schedule.json
```

Export a filtered set:

```bash
lab registry export \
  --dataset sample \
  --sort priority_density:desc \
  --limit 20 \
  --out-dir out/top20
```

Export reconstructs a self-contained schedule by combining the shared schedule body with the selected run's metadata and metrics.

## Publish results

```bash
cargo run -p lab --bin virolai -- publish \
  --workspace paper \
  --dir out/top20 \
  --create-workspace
```

`virolai publish` uses `VIROLAI_WEBAPP_URL` and `VIROLAI_WEBAPP_TOKEN` when present. Legacy `PHD_WEBAPP_URL` and `PHD_WEBAPP_TOKEN` variables remain supported.

## Dataset adapters

Dataset adapters are integration utilities, not scheduler requirements. The current repository includes a CTAO adapter:

```bash
cargo run -p lab --bin virolai -- dataset adapt CTA-N
```

It converts supported CTAO input files to the same `scheduling_problem.json` schema consumed by the generic scheduler.

## Build and QA

Build the relevant binaries:

```bash
cargo build -p lab --bin lab --bin virolai --bin lab-ctao-adapter
```

Run the repository checks from the workspace root:

```bash
./scripts/qa-pipeline.sh
```
