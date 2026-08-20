# VIROLAI

**Versatile Infrastructure for Resource Optimization Leveraging Artificial Intelligence**

VIROLAI is a Rust infrastructure for resource scheduling and optimization. It provides scheduling algorithms, experiment sweeps, a result registry, dataset adapters, and a web interface for comparing runs.

The scheduling engine consumes a generic `scheduling_problem.json` model. Astronomy datasets, including CTAO datasets, are bundled integrations and evaluation cases; they are not assumptions of the scheduler architecture.

## Quick start

Build the workspace:

```bash
cargo build --release
```

Run an experiment sweep and store the results in SQLite:

```bash
cargo run -p lab --bin virolai --release -- sweep \
  --spec lab/est_sweep.json \
  --run-db .lab/runs.sqlite
```

Inspect or export selected runs:

```bash
cargo run -p lab --bin lab --release -- registry list \
  --run-db .lab/runs.sqlite

cargo run -p lab --bin lab --release -- registry export \
  --out-dir out/my-sweep \
  --run-db .lab/runs.sqlite
```

Start the web application:

```bash
./webapp/setup.sh -d
```

Publish exported schedules to a workspace:

```bash
cargo run -p lab --bin virolai --release -- publish \
  --workspace my-sweep \
  --create-workspace \
  --dir out/my-sweep
```

## Repository layout

| Path | Purpose |
| --- | --- |
| `schedulers/` | Scheduling library and standalone scheduler binary |
| `lab/` | Experiment runner, registry, dataset adapters, and VIROLAI CLI |
| `schemas/` | JSON schemas for problems, schedules, metrics, and manifests |
| `docs/algorithms/` | Algorithm reference |
| `webapp/` | Result inspection and TSI integration |
| `scripts/` | QA and operational helpers |
| `siderust/` | Astronomy, time, and coordinate utilities used by current integrations |

## CLI

`virolai` is the user-facing workflow CLI:

```text
virolai <COMMAND> [OPTIONS]
```

| Command | Purpose |
| --- | --- |
| `run` | Run one scheduling problem through the `schedulers` binary |
| `sweep` | Execute an experiment matrix and store results in SQLite |
| `matrix` | Forward lower-level matrix commands to `lab` |
| `dataset adapt` | Run an available external-dataset adapter |
| `publish` | Upload exported schedules to a webapp workspace |

Registry queries and exports remain under `lab registry`.

### Single run

```bash
cargo run -p lab --bin virolai -- run \
  data/isdc_n.json \
  --algorithm est \
  --est-k 4 \
  --est-b 2
```

### Sweep

```bash
virolai sweep --spec <FILE> [--run-db <PATH>] [--parallel <N>] [--override]
```

Sweeps are DB-only. Successful runs are written to the SQLite registry, and schedule files are created only when requested through `lab registry export`.

The registry separates run-specific metadata from deduplicated schedule bodies. Runs that produce the same placement set can share one schedule body while keeping independent configuration and metrics.

## Scheduling model

The scheduler operates on a problem description containing resources, a scheduling horizon, scheduling blocks, tasks, dependencies, hard constraints, and soft constraints. The engine does not require CTAO-specific identifiers or dataset formats.

A simplified problem has this shape:

```json
{
  "resources": [
    {
      "id": 0,
      "name": "resource-0",
      "location": {
        "longitude_deg": -17.89,
        "latitude_deg": 28.76,
        "height_m": 2396.0
      },
      "hard_constraints": {}
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
          "name": "task-1",
          "requested_duration_sec": 1200.0,
          "hard_constraints": {},
          "soft_constraints": { "priority": 5.0 }
        }
      ],
      "dependencies": []
    }
  ]
}
```

The complete schema is in [`schemas/scheduling_problem/scheduling_problem.schema.json`](schemas/scheduling_problem/scheduling_problem.schema.json).

## Algorithms

VIROLAI currently includes two scheduler families:

- Cursor-engine algorithms: EST, LST, and multi-cursor layouts.
- HAP: a separate adaptive/metaheuristic planner.

EST and LST are single-cursor configurations of the shared cursor engine. Multi-cursor experiments run several coordinated cursors over one schedule. HAP has its own planner pipeline.

See [`docs/algorithms/README.md`](docs/algorithms/README.md) for the algorithm reference and [`docs/algorithms/sweep-configuration.md`](docs/algorithms/sweep-configuration.md) for experiment configuration.

## Experiment specifications

A sweep specification describes datasets, algorithms, and parameter axes. For example:

```json
{
  "name": "my-experiment",
  "max_parallel": 8,
  "datasets": [
    {
      "id": "sample",
      "path": "data/isdc_n.json",
      "label": "Sample dataset"
    }
  ],
  "algorithms": [
    {
      "kind": "est",
      "axes": {
        "endangered_thresholds": [1, 2],
        "k_beams": [1, 4],
        "branching_factors": [1, 2]
      }
    }
  ]
}
```

The Cartesian product of the configured axes defines the experiment cells. Each successful cell is recorded in the registry with its configuration, metrics, and schedule reference.

## CTAO integration

CTAO support is an optional dataset integration. The scheduler itself works with the generic scheduling problem schema described above.

The repository includes `lab-ctao-adapter` to convert supported CTAO `*_internalSDC.json` datasets into `scheduling_problem.json`:

```bash
cargo run -p lab --bin virolai -- dataset adapt CTA-N
cargo run -p lab --bin virolai -- dataset adapt CTA-S
```

The astronomy datasets shipped with the repository are useful for experiments and regression testing, but they do not define the scheduler's scope.

## Web application

The webapp provides result storage, comparison, and schedule inspection. Start the Docker stack with:

```bash
./webapp/setup.sh -d
```

Default endpoints:

| Service | Address |
| --- | --- |
| Frontend | `http://localhost:3000` |
| Backend | `http://localhost:8080` |
| PostgreSQL | `localhost:5432` |

Useful environment variables include `BACKEND_PORT`, `FRONTEND_PORT`, `POSTGRES_PORT`, `RUST_LOG`, and `VIROLAI_WORKSPACES_DIR`. The legacy `PHD_WORKSPACES_DIR` name remains accepted for compatibility.

`virolai publish` reads `VIROLAI_WEBAPP_URL` and `VIROLAI_WEBAPP_TOKEN`. The previous `PHD_WEBAPP_URL` and `PHD_WEBAPP_TOKEN` names remain accepted as fallbacks.

## Quality checks

Run the repository QA pipeline before merging changes:

```bash
./scripts/qa-pipeline.sh
```

Equivalent commands:

```bash
cargo clippy --workspace --exclude tsi-rust --all-targets -- -D warnings
cargo fmt --all -- --check
cargo test --workspace --exclude tsi-rust --all-features
```
