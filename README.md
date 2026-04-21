# PhD Scheduler

This repository contains three main runnable pieces:

- `ctao_adapter`: converts CTA dataset directories into a scheduler-ready `scheduling_problem.json`
- `scheduler`: runs the Rust scheduler on a `scheduling_problem.json` input
- `webapp`: starts the adapted TSI web UI for uploading and analyzing schedules

## Prerequisites

- Rust with `cargo`
- Docker and Docker Compose for the webapp stack

## 1. Run the adapter

Use the adapter to convert one CTA dataset directory into a single scheduler input file.

```bash
cargo run --bin ctao_adapter -- <dataset_dir> [output_json]
```

Examples:

```bash
cargo run --bin ctao_adapter -- CTA-N
cargo run --bin ctao_adapter -- CTA-S
cargo run --bin ctao_adapter -- data/CTA-N data/CTA-N/scheduling_problem.json
```

Notes:

- `CTA-N` and `CTA-S` are resolved automatically under `data/`
- if `output_json` is omitted, the adapter writes `<dataset_dir>/scheduling_problem.json`

## 2. Run the scheduler

Run the scheduler on a `scheduling_problem.json` file:

```bash
cargo run --bin scheduler -- <input_json> [horizon_start_mjd horizon_end_mjd] [--est-fom <task_count|soft_constraint>] [--est-endangered-threshold <u32>] [--est-k <usize>] [--est-b <usize>]
```

Examples:

```bash
cargo run --bin scheduler --release -- data/CTA-N/scheduling_problem.json
cargo run --bin scheduler --release -- data/CTA-S/scheduling_problem.json
```

Optional horizon override:

```bash
cargo run --bin scheduler -- data/CTA-N/scheduling_problem.json 61710.0 61720.0
```

Optional EST configuration override:

```bash
cargo run --bin scheduler -- data/CTA-N/scheduling_problem.json --est-fom soft_constraint --est-endangered-threshold 2 --est-k 5 --est-b 3
```

The scheduler writes a result file next to the input using the pattern:

```text
<input_stem>_schedule.json
```

For example:

```text
data/CTA-N/scheduling_problem_schedule.json
```

## 2b. Run an EST experiment sweep

Run a whole EST configuration sweep and write one schedule per configuration plus a comparison CSV:

```bash
cargo run --bin est_experiment -- --spec experiments/ctao_n_est.json
```

You can also drive the sweep directly from CLI overrides:

```bash
cargo run --bin est_experiment -- data/CTA-N/scheduling_problem.json \
  --output-dir out/ctao_n_est \
  --est-fom-values task_count,soft_constraint \
  --est-e-values 1,2 \
  --est-k-values 1,4 \
  --est-b-values 1,2
```

Generated output layout:

```text
<output_dir>/
  run-<timestamp>/
    manifest.json
    comparison.csv
    schedules/
      e1-k1-b1-count.json
      ...
```

  `comparison.csv` is intentionally compact and contains only:

  - `run_slug`
  - `is_baseline`
  - `scheduled_task_count`
  - `fitness_priority_sum` (sum of priorities of scheduled tasks)
  - `scheduled_priority_p25`
  - `scheduled_priority_p50`
  - `scheduled_priority_p75`
  - `scheduled_priority_p90`

  `run_slug` uses the compact naming stem `e{endangered_threshold}-k{k_beams}-b{branching_factor}-{count|fitness}`.

Example experiment-spec JSON:

```json
{
  "input_json": "data/CTA-N/scheduling_problem.json",
  "output_dir": "out/ctao_n_est",
  "baseline": {
    "fom": "task_count",
    "endangered_threshold": 1,
    "k_beams": 1,
    "branching_factor": 1
  },
  "sweep": {
    "foms": ["task_count", "soft_constraint"],
    "endangered_thresholds": [1, 2],
    "k_beams": [1, 4],
    "branching_factors": [1, 2]
  }
}
```

## 3. Run the webapp

The simplest way to run the webapp is with Docker from the repository root:

```bash
./webapp/setup.sh
```

Useful variants:

```bash
./webapp/setup.sh -d
./webapp/teardown.sh
./webapp/teardown.sh --purge-db
```

Once started:

- frontend: `http://localhost:3000`
- backend health endpoint: `http://localhost:8080/health`

The webapp backend accepts this repository's `scheduling_problem.json` format directly, so you can upload the adapter output in the UI.

## Local backend only

If you only want to run the adapted backend without Docker:

```bash
cargo run --bin phd_tsi_server
```

This starts the HTTP server on `http://localhost:8080`.

## Recommended flow

```bash
cargo run --bin ctao_adapter -- CTA-N
cargo run --bin scheduler -- data/CTA-N/scheduling_problem.json
./webapp/setup.sh
```

Then open `http://localhost:3000` and upload the generated `scheduling_problem.json` or scheduler output as needed for inspection.

## QA

From the repository root:

```bash
./scripts/qa-pipeline.sh
```
