<div align="center">

# PhD Scheduler

Rust tooling for astronomical observation scheduling:
CTAO dataset adaptation, EST- and HAP-based scheduling, experiment sweeps, and TSI-backed schedule inspection.

[Quick Start](#quick-start) | [Components](#components) | [Data Model](#data-model) | [Web App](#web-app) | [QA](#qa)

</div>

## Overview

This repository bundles the main pieces used in the current scheduling workflow:

- convert CTAO dataset directories into a scheduler-ready `scheduling_problem.json`
- compute schedules with the Rust scheduler (EST or HAP engine)
- run repeatable EST parameter sweeps for experiments
- inspect problems and generated schedules in an adapted TSI web application

The broader research context is astronomical observation scheduling. A useful reference is [A hybrid multi-start metaheuristic scheduler for astronomical observations](https://doi.org/10.1016/j.engappai.2023.106856). The runnable CLI can execute either the EST scheduler or the HAP planner; treat the paper as research background rather than a one-to-one description of every shipped implementation detail.

## Components

| Component | Entry point | Purpose |
| --- | --- | --- |
| Scheduler library | [`src/lib.rs`](src/lib.rs) | Core scheduling types, prescheduler, EST scheduler, time handling, and constraints |
| `scheduler` | [`src/main.rs`](src/main.rs) | Reads a scheduling problem, computes feasible windows, runs EST or HAP, and writes an annotated schedule |
| `ctao_adapter` | [`scripts/ctao_adapter.rs`](scripts/ctao_adapter.rs) | Converts CTAO `*_internalSDC.json` directories into `scheduling_problem.json` |
| `est_experiment` | [`scripts/est_experiment/main.rs`](scripts/est_experiment/main.rs) | Sweeps EST configurations and writes schedules, a manifest, and a comparison CSV |
| `experiment_matrix` | [`scripts/experiment_matrix/main.rs`](scripts/experiment_matrix/main.rs) | Cross-algorithm test matrix (datasets × algorithms × sweeps) with rich metrics — see [docs/evaluation-environment.md](docs/evaluation-environment.md) |
| `phd_tsi_server` | [`webapp/scripts/phd_tsi_server.rs`](webapp/scripts/phd_tsi_server.rs) | Runs the adapted TSI backend locally |
| Docker web app | [`webapp/setup.sh`](webapp/setup.sh) | Starts the adapted frontend, backend, and PostgreSQL stack |

## Quick Start

### Prerequisites

- Rust toolchain with `cargo`
- Docker with Compose support for the web application
- CTAO datasets under `data/CTA-N/` or `data/CTA-S/` if you want to use the adapter shortcuts

### Common Flow

1. Convert a CTAO dataset into `scheduling_problem.json`.
2. Run the scheduler on that JSON file.
3. Optionally open the web app to inspect the problem or the produced schedule.

```bash
cargo run --bin ctao_adapter -- CTA-N
cargo run --bin scheduler --release -- data/CTA-N/scheduling_problem.json
./webapp/setup.sh
```

Once the stack is running:

- frontend: `http://localhost:3000`
- backend health: `http://localhost:8080/health`

## CLI Workflows

### 1. Convert CTAO Input

```bash
cargo run --bin ctao_adapter -- <dataset_dir> [output_json]
```

Examples:

```bash
cargo run --bin ctao_adapter -- CTA-N
cargo run --bin ctao_adapter -- CTA-S
cargo run --bin ctao_adapter -- data/CTA-N data/CTA-N/scheduling_problem.json
```

What the adapter does:

- resolves shorthand dataset names `CTA-N` and `CTA-S` under `data/`
- writes `<dataset_dir>/scheduling_problem.json` when `output_json` is omitted
- emits the envelope validated by [`schemas/scheduling_problem.schema.json`](schemas/scheduling_problem.schema.json)
- converts each CTAO scheduling block into one scheduler block containing one task
- infers the observing site from the dataset and fills telescope-level hard constraints
- sets `night_time.twilight = "Nautical"` and `moon_altitude = [-90, 0]` on the generated resource
- uses a default schedule window of `[2028-01-01T00:00:00Z, 2029-01-01T00:00:00Z)` expressed in MJD UTC

### 2. Run the Scheduler

```bash
cargo run --bin scheduler -- <input_json> [horizon_start_mjd horizon_end_mjd] \
  [--algorithm est|hap] \
  [EST options] \
  [HAP options]
```

#### EST

Run with the default EST algorithm and default EST settings:

```bash
cargo run --bin scheduler --release -- data/CTA-N/scheduling_problem.json
cargo run --bin scheduler --release -- data/CTA-S/scheduling_problem.json
```

Override the scheduling horizon:

```bash
cargo run --bin scheduler -- data/CTA-N/scheduling_problem.json 61710.0 61720.0
```

Override EST parameters:

```bash
cargo run --bin scheduler -- data/CTA-N/scheduling_problem.json \
  --est-fom soft_constraint \
  --est-e 2 \
  --est-k 5 \
  --est-b 3
```

#### HAP

Run with the HAP algorithm using default settings:

```bash
cargo run --bin scheduler --release -- data/CTA-N/scheduling_problem.json \
  --algorithm hap
```

Run with custom HAP parameters:

```bash
cargo run --bin scheduler --release -- data/CTA-N/scheduling_problem.json \
  --algorithm hap \
  --hap-num-crus 8 \
  --hap-cru-iterations 256 \
  --hap-stochastic-range 5 \
  --hap-seed 42
```

HAP options:

| Flag | Default | Description |
| --- | --- | --- |
| `--hap-num-crus` | `4` | Number of CRU attempts per block and survivor schedules kept between rounds |
| `--hap-cru-iterations` | `128` | Maximum repair iterations per CRU run |
| `--hap-stochastic-range` | `3` | Number of lowest-cost candidate windows to choose from stochastically |
| `--hap-seed` | `0` | Master seed for deterministic per-CRU RNG derivation |

HAP notes:

- each `SchedulingBlock` is scheduled as one unit; block priority is the sum of its member-task soft-constraint priorities
- HAP uses stochastic CRU-S attempts over the active survivor set; `--hap-num-crus` controls both the number of attempts and the survivor cap
- survivors are ranked by completion fitness (fraction of scheduling blocks fully placed, weighted by priority), then by total science time, then by deterministic tie-breakers
- `--hap-seed` guarantees reproducible results across runs with the same input and configuration
- EST-specific flags (`--est-*`) and HAP-specific flags (`--hap-*`) are mutually exclusive; mixing them is an error

Scheduler notes:

- the preferred input format is the `scheduling_problem.json` envelope with `resources`, optional `schedule_time_window`, and `scheduling_blocks`
- legacy top-level arrays of scheduling blocks are still accepted
- if `schedule_time_window` is absent, the scheduler falls back to the union of task `time_window` constraints when available
- the default algorithm is `--algorithm est` with EST configuration `--est-fom soft_constraint --est-e 1 --est-k 1 --est-b 1`
- output is written next to the input as `<input_stem>_schedule.json`
- each task in the output is annotated with:
  - `scheduled`
  - `scheduled_start_mjd_utc`
  - `scheduled_end_mjd_utc`

Example output path:

```text
data/CTA-N/scheduling_problem_schedule.json
```

### 3. Run an EST Experiment Sweep

Run a full EST sweep from a JSON spec:

```bash
cargo run --bin est_experiment -- --spec experiments/ctao_n_est.json
```

Or drive the sweep from CLI overrides:

```bash
cargo run --bin est_experiment -- data/CTA-N/scheduling_problem.json \
  --output-dir out/ctao_n_est \
  --est-e-values 1,2 \
  --est-k-values 1,4 \
  --est-b-values 1,2
```

Generated artifact layout:

```text
<output_dir>/
  run-<timestamp>/
    manifest.json
    comparison.csv
    schedules/
      e1-k1-b1.json
      ...
```

The comparison CSV contains compact per-run metrics:

- `run_slug`
- `is_baseline`
- `scheduled_task_count`
- `fitness_priority_sum`
- `scheduled_priority_p25`
- `scheduled_priority_p50`
- `scheduled_priority_p75`
- `scheduled_priority_p90`

Minimal experiment spec:

```json
{
  "input_json": "data/CTA-N/scheduling_problem.json",
  "output_dir": "out/ctao_n_est",
  "sweep": {
    "endangered_thresholds": [1, 2],
    "k_beams": [1, 4],
    "branching_factors": [1, 2]
  }
}
```

For the full CLI and sweep details, see [`scripts/README.md`](scripts/README.md).

## Web App

### Docker Stack

The simplest way to run the adapted TSI stack is from the repository root:

```bash
./webapp/setup.sh
```

Useful variants:

```bash
./webapp/setup.sh -d
./webapp/teardown.sh
./webapp/teardown.sh --purge-db
```

The Docker setup runs:

- frontend on `http://localhost:3000`
- adapted backend on `http://localhost:8080`
- PostgreSQL with a persistent Docker volume

The UI can upload this repository's `scheduling_problem.json` inputs directly, as well as the annotated schedule outputs produced by the `scheduler` binary.

### Local Backend Only

If you only want the adapted backend without Docker:

```bash
cargo run --bin phd_tsi_server
```

By default the server listens on `http://localhost:8080`. The bind address can be adjusted with `HOST` and `PORT`.

For Docker-specific details and troubleshooting, see [`webapp/docker/README.md`](webapp/docker/README.md).

## Data Model

The preferred on-disk format is the top-level envelope described by [`schemas/scheduling_problem.schema.json`](schemas/scheduling_problem.schema.json):

```json
{
  "resources": [
    {
      "id": 0,
      "name": "CTA-N",
      "location": {
        "longitude_deg": -17.89,
        "latitude_deg": 28.76,
        "height_m": 2396.0
      },
      "hard_constraints": {
        "night_time": { "twilight": "Nautical" },
        "moon_altitude": { "min_deg": -90.0, "max_deg": 0.0 }
      }
    }
  ],
  "schedule_time_window": {
    "start_mjd_utc": 61710.0,
    "end_mjd_utc": 62076.0
  },
  "scheduling_blocks": [
    {
      "id": 1,
      "tasks": [
        {
          "id": 1,
          "name": "target-1",
          "requested_duration_sec": 1200.0,
          "target": { "ra_deg": 83.8, "dec_deg": 22.0 },
          "hard_constraints": {},
          "soft_constraints": { "priority": 5.0 }
        }
      ],
      "dependencies": []
    }
  ]
}
```

Key schema files:

- [`schemas/scheduling_problem.schema.json`](schemas/scheduling_problem.schema.json): top-level envelope
- [`schemas/scheduling_blocks.schema.json`](schemas/scheduling_blocks.schema.json): block list and intra-block dependencies
- [`schemas/task.schema.json`](schemas/task.schema.json): task representation
- [`schemas/hard_constraints.schema.json`](schemas/hard_constraints.schema.json): task and resource hard constraints
- [`schemas/soft_constraints.schema.json`](schemas/soft_constraints.schema.json): soft-constraint payloads

The scheduler output preserves the original structure and adds scheduling annotations to each task instead of writing a separate result schema.

## Repository Layout

| Path | Contents |
| --- | --- |
| [`src/`](src/) | Scheduler library and `scheduler` CLI |
| [`scripts/`](scripts/) | CLI utilities, including the CTAO adapter, EST sweep runner, and QA pipeline |
| [`schemas/`](schemas/) | JSON schemas for problems, blocks, tasks, and constraints |
| [`data/`](data/) | Example datasets and convenience JSON files |
| [`webapp/`](webapp/) | Adapted TSI integration, Docker stack, and helper scripts |
| [`webapp/TSI/`](webapp/TSI/) | TSI submodule used by the web app |
| [`siderust/`](siderust/) | Local astronomy, time, and coordinate utilities crate |

## QA

From the repository root, the required checks are:

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
cargo test --all-features
```

Equivalent helper:

```bash
./scripts/qa-pipeline.sh
```

If formatting fails, run:

```bash
cargo fmt --all
```
