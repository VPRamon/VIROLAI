<div align="center">

# PhD Scheduler

Rust tooling for astronomical observation scheduling — CTAO dataset adaptation, EST- and
HAP-based scheduling, parameter sweep experiments, and an adapted TSI web application for
interactive result inspection.

</div>

[Quick Start](#quick-start) | [The `phd` CLI](#the-phd-cli) | [Sweep Workflow](#sweep-workflow-end-to-end) | [Web App](#web-app) | [Data Model](#data-model) | [QA](#qa)

---

## Quick Start

```bash
# 1. Build everything
cargo build --release

# 2. Run a sweep experiment (multiple datasets × algorithm configurations).
#    Results are stored in a SQLite registry (DB-only workflow).
cargo run -p lab --bin phd --release -- sweep \
  --spec lab/est_sweep.json \
  --run-db .lab/runs.sqlite

# 3. Export the schedules you want from the registry
cargo run -p lab --bin lab --release -- registry export \
  --out-dir out/my-sweep \
  --run-db .lab/runs.sqlite

# 4. Start the web app
./webapp/setup.sh -d          # Docker (frontend + backend + postgres)

# 5. Publish the exported schedules to a workspace
cargo run -p lab --bin phd --release -- publish \
  --workspace my-sweep --create-workspace --dir out/my-sweep
```

---

## Prerequisites

- **Rust toolchain** (`cargo`) — see [rustup.rs](https://rustup.rs)
- **Docker with Compose** — required for the full web stack
- **Dataset files** under `data/` — the repository ships example files; see
  [Datasets](#datasets)

---

## Repository Layout

| Path | Contents |
|---|---|
| `schedulers/src/` | Scheduler library and `schedulers` binary |
| `lab/src/bin/phd/` | The `phd` unified CLI |
| `lab/src/bin/lab_ctao_adapter/` | CTAO -> `scheduling_problem.json` adapter |
| `schemas/` | Modular JSON schemas (problem, block, algorithm, metrics, schedule, manifest) |
| `data/` | Example datasets (`isdc_n.json`, `lst_2024.json`, …) |
| `lab/` | Ready-to-run sweep specs |
| `webapp/` | TSI integration: Docker stack and PhD adapter server |
| `siderust/` | Local astronomy / time / coordinate utilities crate |

---

## The `phd` CLI

`phd` is the single entry point for all research workflows.

```
cargo run -p lab --bin phd -- <COMMAND> [OPTIONS]
```

| Command | Purpose |
|---|---|
| `sweep` | Run a parameter sweep and store results in the SQLite registry (primary workflow) |
| `matrix` | Lower-level alias — delegates directly to the `lab` binary |
| `run` | Run a single scheduling problem (delegates to `schedulers`) |
| `dataset adapt` | Convert a CTAO dataset directory into `scheduling_problem.json` |
| `publish` | Upload schedule JSONs from a directory to a webapp workspace |

Registry inspection and schedule export are provided by the `lab` binary under
`lab registry …` (see [The run registry](#the-run-registry)).

---

### `phd sweep`

```
phd sweep --spec <FILE> [--run-db <PATH>] [--parallel <N>] [--override]
```

Runs the full experiment matrix described in `<FILE>` and stores every
successful run in a SQLite registry (default `.lab/runs.sqlite`). This is a
**DB-only** workflow: no schedule files, manifests, or run directories are
written. Schedules are materialised on demand with
[`lab registry export`](#exporting-schedules).

> Note: `phd sweep` invokes the sibling `lab` binary from
> `target/debug/lab` or `target/release/lab`. If you encounter
> `failed to spawn lab: No such file or directory`, build it first with:
>
> ```bash
> cargo build -p lab --bin lab
> ```

| Flag | Required | Description |
|---|---|---|
| `--spec <FILE>` | ✅ | Path to the experiment spec JSON (see [Spec Format](#spec-format)) |
| `--run-db <PATH>` | — | Registry SQLite path (default `.lab/runs.sqlite`) |
| `--parallel <N>` | — | Override worker threads (defaults to the spec's `max_parallel` or the CPU count) |
| `--override` | — | Re-execute cells already present in the registry and refresh their rows |

Each stored run keeps its own identity, configuration, metrics, and schedule
metadata. Semantically identical schedules produced by different configurations
are **deduplicated**: the invariant schedule body is stored once, while each run
keeps its own metadata and metrics. See [The run registry](#the-run-registry).

---

### The run registry

The registry is a SQLite database (`.lab/runs.sqlite` by default) with two
tables:

- `runs` — one row per unique run identity. Holds `identity_json`,
  `config_json`, `metrics_json`, the run-specific `metadata_json`
  (`schedule_metadata` body), `source_cell_id`, indexed metric columns, and a
  `schedule_hash` foreign key.
- `schedules` — one row per **semantically unique** schedule, keyed by
  `schedule_hash` (a content hash of the placements only). Stores just the
  *invariant* schedule body (the problem annotated with placements). Run-specific
  `schedule_metadata` / `schedule_metrics` are **not** stored here, so multiple
  runs can safely share a single schedule body.

`lab registry` exposes read-only queries over this database:

```
lab registry list     [--dataset <ID>] [--algorithm <NAME>] [--sort <metric:dir>] [--format json|table]
lab registry inspect  --run <KEY|PREFIX>
lab registry best     --dataset <ID> [--algorithm <NAME>]
lab registry export   …            # see below
lab registry doctor                # referential-integrity check
```

`lab registry doctor` reports runs with a `NULL` schedule hash, runs pointing to
a missing schedule, orphan schedules, and legacy rows lacking stored metadata; it
exits non-zero when a real inconsistency is found.

#### Exporting schedules

`registry export` reconstructs a complete, run-specific schedule artifact by
recombining the shared invariant body with that run's own `schedule_metadata`
and `schedule_metrics`:

```bash
# Single run -> single file
lab registry export --run <KEY|PREFIX> --out out/best.json [--force] [--run-db <PATH>]

# Filtered set -> directory (one file per run)
lab registry export --out-dir out/my-sweep \
  [--dataset <ID>] [--algorithm <NAME>] [--sort <metric:dir>] [--limit <N>] \
  [--force] [--run-db <PATH>]
```

Because the body is deduplicated but metadata/metrics live on the run row, two
runs that share a schedule still export their **own** metadata and metrics — no
export ever inherits them from another run.

> Migrating an old database: registries created before schedule deduplication
> store run-specific fields inside the shared body and lack `metadata_json`.
> Upgrade such a file once with the standalone tool
> (`cargo run -p lab --bin lab-migrate-schedule-dedup -- .lab/runs.sqlite`), or
> simply re-run the sweep with `--override`.

### `phd dataset adapt`

```
phd dataset adapt <dataset_dir> [output_json]
```

Converts a CTAO `*_internalSDC.json` directory into `scheduling_problem.json`.

Shorthand names `CTA-N` and `CTA-S` are resolved automatically under `data/`:

```bash
cargo run -p lab --bin phd -- dataset adapt CTA-N
cargo run -p lab --bin phd -- dataset adapt CTA-S
cargo run -p lab --bin phd -- dataset adapt data/my_site data/my_site/scheduling_problem.json
```

---

### `phd run` (single run)

Delegates directly to the `schedulers` binary. Useful for one-off experiments.

```bash
cargo run -p lab --bin phd -- run data/isdc_n.json --algorithm est --est-k 4 --est-b 2
```

See `phd run --help` or the [scheduler options](#scheduler-options) section for full
flag details.

---

## Spec Format

A sweep spec is a JSON file that defines the experiment: which datasets to use,
which algorithms and parameter combinations to run, and where to write output.

### Minimal spec

```json
{
  "name": "my-experiment",
  "datasets": [
    {
      "id": "isdc_n",
      "path": "data/isdc_n.json",
      "label": "SDC North"
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

This produces `2 datasets × (2 × 2 × 2 EST cells) = 16` schedule files.

### Full spec reference

```json
{
  "name":         "Experiment display name",
  "output_dir":   "out/my-experiment",
  "max_parallel": 8,

  "datasets": [
    {
      "id":    "isdc_n",
      "path":  "data/isdc_n.json",
      "label": "SDC North",
      "horizon_override": {
        "start_mjd": 61771.0,
        "end_mjd":   61781.0
      }
    }
  ],

  "algorithms": [
    {
      "kind": "est",
      "axes": {
        "endangered_thresholds": [0, 1, 2, 4, 8],
        "k_beams":               [1, 2, 4, 8],
        "branching_factors":     [1, 2, 4]
      }
    },
    {
      "kind": "hap",
      "axes": {
        "iota_max_values":  [64, 128],
        "rho_values":       [3, 5],
        "population_sizes": [4, 8],
        "survivor_modes":   ["elitist_top_k"],
        "survivor_caps":    [4],
        "seeds":            [0, 1, 2]
      }
    }
  ]
}
```

| Field | Type | Default | Description |
|---|---|---|---|
| `name` | string | — | Human-readable name (used in manifest `producer` metadata) |
| `output_dir` | string | — | Legacy field used by `phd matrix` run directories; ignored by the DB-only `phd sweep` workflow |
| `max_parallel` | int | CPU count | Number of parallel worker threads |
| `datasets[].id` | string | — | Short identifier used in cell filenames |
| `datasets[].path` | string | — | Path to `scheduling_problem.json` |
| `datasets[].label` | string | `id` | Human-readable label embedded in manifests |
| `datasets[].horizon_override` | object | null | Override the problem's scheduling window (MJD UTC) |

#### EST algorithm axes

Each EST cell is the cartesian product of all three axes.

| Axis | Key | Description |
|---|---|---|
| Endangered threshold | `endangered_thresholds` | Minimum remaining scheduling blocks before a task is considered "endangered" (`--est-e`) |
| K-beams | `k_beams` | Beam width — number of partial schedules kept at each step (`--est-k`) |
| Branching factor | `branching_factors` | Candidates considered per block per beam (`--est-b`) |

#### HAP algorithm axes

| Axis | Key | Description |
|---|---|---|
| Iota max | `iota_max_values` | Max repair iterations per CRU run (`--hap-cru-iterations`) |
| Rho | `rho_values` | Stochastic window range (`--hap-rho`) |
| Population size | `population_sizes` | Number of CRU attempts and survivor cap (`--hap-num-crus`) |
| Survivor mode | `survivor_modes` | `"elitist_top_k"` or `"pareto_front"` |
| Survivor cap | `survivor_caps` | Hard cap on the survivor pool between rounds |
| Seeds | `seeds` | RNG seeds for reproducible stochastic runs |

#### Multi-cursor algorithm axes

The `multi_cursor` algorithm generalises EST/LST into several cursors that share
one schedule, each owning a fixed sub-region ("territory") of the horizon
(**Plan A**). See [Scheduling model](#scheduling-model) for the conceptual
background.

```json
{
  "kind": "multi_cursor",
  "axes": {
    "layouts":               ["est_lst_split", "start_mid_forward"],
    "endangered_thresholds": [1],
    "k_beams":               [4],
    "branching_factors":     [2]
  }
}
```

| Axis | Key | Description |
|---|---|---|
| Layouts | `layouts` | Cursor arrangement: `"est_lst_split"` or `"start_mid_forward"` |
| Endangered threshold | `endangered_thresholds` | Same semantics as EST |
| K-beams | `k_beams` | Beam width |
| Branching factor | `branching_factors` | Candidates explored per beam per round |
| FOMs | `foms` | Figure-of-merit variants (default `soft_constraint`) |

Each multi-cursor cell is the cartesian product of all axes. The cell slug
encodes the layout, e.g. `est_lst_split-e1-k4-b2`.

> Plain single-cursor EST and LST keep their dedicated `est` / `lst` algorithm
> kinds; `multi_cursor` is only for genuinely multi-cursor layouts. Cursor-aware
> figures of merit (e.g. `future_flexibility`) are best used with the single
> `est`/`lst` kinds; under multi-cursor layouts they only affect beam ranking,
> never schedule validity.

---

## Scheduling model

All beam-search schedulers in this workspace share one underlying model: a
*cursor* sweeps the horizon placing tasks, ordered by earliest feasible start
with endangered-task protection.

- **EST** (`est`) is a **single forward cursor** over the full horizon: it
  schedules the earliest-feasible task first.
- **LST** (`lst`) is a **single backward cursor** over the full horizon. It is
  realised by mirroring the horizon, running EST in mirrored time, then
  unmirroring the schedule — so latest-feasible tasks are placed first.
- **Multi-cursor** (`multi_cursor`, **Plan A**) runs several cursors with
  disjoint **fixed territories** that share one global schedule. A task placed
  by one cursor becomes unavailable to all others, no placement may escape its
  cursor's territory, and placements never overlap. Built-in layouts:
  - `est_lst_split` — forward cursor over `[0, 0.5)` + backward cursor over
    `[0.5, 1.0)` (fractions of the horizon).
  - `start_mid_forward` — forward cursor over `[0, 0.5)` + forward cursor over
    `[0.5, 1.0)`.
- **Plan B** (dynamic territories whose boundaries follow other cursors) is
  *not yet implemented*; configuring it returns an unsupported-configuration
  error. The engine is structured so Plan B only needs changes to per-cursor
  active-region resolution, not to the beam-search core.

Programmatically, `MultiCursorScheduler::single_forward(...)` is exactly
equivalent to `EstScheduler` and `MultiCursorScheduler::single_backward(...)` is
exactly equivalent to `LstScheduler` (proven by equivalence tests).

---

## Scheduler Options

For one-off runs via `phd run` or the raw `schedulers` binary:

```
schedulers <input_json> [horizon_start_mjd horizon_end_mjd]
          [--algorithm est|hap]
          [--output <path>]
          [EST options]
          [HAP options]
```

#### EST options

| Flag | Default | Description |
|---|---|---|
| `--est-fom soft_constraint` | `soft_constraint` | Figure-of-merit function |
| `--est-e <u32>` | `1` | Endangered threshold |
| `--est-k <usize>` | `1` | K-beams (beam width) |
| `--est-b <usize>` | `1` | Branching factor |

Aliases: `--est-endangered-threshold`, `--est-schedule-states`, `--est-branching-factor`.
Short flags `-e`, `-k`, and `-b` are not supported; use `--est-e`, `--est-k`, and `--est-b`.

#### HAP options

| Flag | Default | Description |
|---|---|---|
| `--hap-num-crus <usize>` | `4` | CRU attempts and survivor cap |
| `--hap-cru-iterations <usize>` | `128` | Max repair iterations per CRU |
| `--hap-rho <usize>` | `3` | Candidate window pool size |
| `--hap-seed <u64>` | `0` | Master RNG seed |

---

## Sweep Workflow (End-to-End)

This section walks through the complete workflow: **run many configurations into
the registry → export the schedules you want → upload to the webapp**.

### Step 1 — Prepare your experiment spec

Copy one of the bundled examples from `lab/` and edit it:

```bash
cp lab/est_sweep.json lab/my_sweep.json
```

Edit `my_sweep.json` to point at your datasets and set the parameter ranges.

### Step 2 — Run the sweep (DB-only)

```bash
cargo run -p lab --bin phd --release -- sweep \
  --spec lab/my_sweep.json \
  --run-db .lab/runs.sqlite
```

- `--release` is recommended for large sweeps (significantly faster).
- Every successful run is stored in the SQLite registry; no files are written.
- Re-running is cheap: cells already present are skipped (use `--override` to
  recompute and refresh their rows).
- Progress is logged to stderr; each cell runs in parallel.

### Step 3 — Inspect and export schedules from the registry

```bash
# Browse stored runs
cargo run -p lab --bin lab -- registry list --run-db .lab/runs.sqlite

# Export a filtered set to a directory (one self-contained JSON per run)
cargo run -p lab --bin lab -- registry export \
  --out-dir out/my-sweep \
  --run-db .lab/runs.sqlite

# Or export a single run by key/prefix
cargo run -p lab --bin lab -- registry export \
  --run 9f3a7c --out out/best.json \
  --run-db .lab/runs.sqlite
```

Each exported JSON is self-contained: the invariant schedule body recombined
with that run's own `schedule_metadata` and `schedule_metrics`.

### Step 4 — Start the web app

Using Docker (recommended):

```bash
./webapp/setup.sh -d
```

Or locally (without Docker):

```bash
PHD_WORKSPACES_DIR=./workspaces cargo run -p webapp --bin webapp
```

Wait for the backend health endpoint to respond:

```bash
curl http://localhost:8080/health
```

### Step 5 — Create a result workspace

Open `http://localhost:3000/workspace` in your browser.

Scroll to the **Algorithm Results** section at the bottom of the page and click
**+ New result workspace**. Give it a name (e.g. `EST k/b sweep — SDC North`) and
press **Create**.

### Step 6 — Upload results to the workspace

Inside the newly created workspace card a drop zone appears. You can:

- **Drag and drop** any number of `.manifest.json` or self-contained schedule `.json`
  files directly onto it.
- Click **browse files** to open a file picker (multi-select supported).
- Click **browse folder** to select an entire directory — all `.json` files in the
  directory tree are picked up automatically.

The drop zone **auto-detects the file type**:

| File type | How detected | Backend route |
|---|---|---|
| Manifest JSON | `manifest_schema_version` field present | `POST /v1/workspaces/{id}/manifests` |
| Schedule JSON | No `manifest_schema_version` field | `POST /v1/workspaces/{id}/schedules` (server builds the manifest) |

For each file the status updates inline:

| Status | Meaning |
|---|---|
| ⏳ uploading | Upload in progress |
| ✅ created | New result stored |
| ♻️ duplicate | Already present (idempotent — safe to re-upload) |
| ❌ error | Upload failed — hover to see the error message |

After uploading, click **Clear uploaded** to hide the completed entries.

> **Tip:** A fast workflow is `lab registry export --out-dir out/my-sweep`
> followed by drag-and-drop of the entire `out/my-sweep/` folder onto the drop
> zone. Self-contained schedule JSONs are converted to manifests server-side.

---

## Web App

### Docker Stack

```bash
./webapp/setup.sh          # foreground (logs in terminal)
./webapp/setup.sh -d       # detached (runs in background)
./webapp/teardown.sh       # stop all services
./webapp/teardown.sh --purge-db   # stop and delete the database volume
```

Services:

| Service | URL | Notes |
|---|---|---|
| Frontend | `http://localhost:3000` | React UI |
| Backend | `http://localhost:8080` | PhD adapter + TSI API |
| PostgreSQL | `localhost:5432` | Schedule/environment storage |

#### Optional environment variables

| Variable | Default | Description |
|---|---|---|
| `BACKEND_PORT` | `8080` | Backend listen port |
| `FRONTEND_PORT` | `3000` | Frontend listen port |
| `POSTGRES_PORT` | `5432` | PostgreSQL port |
| `POSTGRES_USER` | `tsi` | Database user |
| `POSTGRES_PASSWORD` | `tsi` | Database password |
| `POSTGRES_DB` | `tsi` | Database name |
| `RUST_LOG` | `info` | Tracing filter for the backend |
| `PHD_WORKSPACES_DIR` | `./workspaces` | Directory for manifest/workspace storage (local backend) |

### Local Backend (without Docker)

Run only the backend server:

```bash
PHD_WORKSPACES_DIR=./workspaces \
cargo run -p webapp --bin webapp
```

Listens on `http://localhost:8080` by default. Adjust with `HOST` and `PORT`
environment variables.

### Webapp Sections

| Section | Location | Purpose |
|---|---|---|
| Schedule Management | `/schedules` | Browse and manage all uploaded schedules |
| Workspace | `/workspace` | Group runs by cohort, compare manifests, drill down into schedules |

**Workspace page** is the single home for comparable runs. Each workspace
contains uploaded **manifests** (lightweight, canonical exchange format)
and optionally the **full schedules** they reference. Results are grouped
by cohort — `(dataset, observatory, period, block_pool_hash)` — so the
summary table works on manifest metrics alone, while the per-block table
appears only when at least one schedule has been persisted.

The upload area accepts mixed batches of `manifest.json` and self-contained
`schedule.json` files (drag-and-drop a folder). Standalone
`schedule_metrics.json` files are rejected; metrics live inside the
manifest envelope.

---

## Data Model

### Scheduling Problem (`scheduling_problem.json`)

Described by [`schemas/scheduling_problem/scheduling_problem.schema.json`](schemas/scheduling_problem/scheduling_problem.schema.json).

```json
{
  "resources": [
    {
      "id": 0,
      "name": "CTA-N",
      "location": { "longitude_deg": -17.89, "latitude_deg": 28.76, "height_m": 2396.0 },
      "hard_constraints": {
        "night_time": { "twilight": "Nautical" },
        "moon_altitude": { "min_deg": -90.0, "max_deg": 0.0 }
      }
    }
  ],
  "schedule_time_window": { "start_mjd_utc": 61710.0, "end_mjd_utc": 62076.0 },
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

### Schedule Output

Each schedule JSON exported from the registry (`lab registry export`) embeds:

- `schedule_metadata` — algorithm id, config, dataset id/label, scheduling horizon
- `schedule_metrics` — completion rates, priority statistics, gap analysis,
  fragmentation metrics
- The original problem structure annotated with `scheduled`, `scheduled_start_mjd_utc`,
  `scheduled_end_mjd_utc` per task

### Manifest

A manifest is a lightweight result record that references a schedule run. It is the
primary exchange artifact between the CLI and the webapp's Algorithm Results section.
Described by [`schemas/scheduling_statistics/manifest.schema.json`](schemas/scheduling_statistics/manifest.schema.json).

Key fields:

```json
{
  "manifest_schema_version": "1.0.0",
  "manifest_id": "<uuid>",
  "created_at": "<rfc3339>",
  "producer":   { "name": "phd", "version": "..." },
  "dataset":    { "id": "isdc_n", "name": "SDC North", ... },
  "algorithm":  { "id": "est", "label": "EST", "config": { ... } },
  "run":        { "run_id": "...", "kind": "matrix_cell", "status": "completed" },
  "horizon":    { "start_mjd_utc": 61771.0, "end_mjd_utc": 62137.0 },
  "metrics":    { ... }
}
```

---

## Datasets

The repository ships ready-to-use scheduling problem files under `data/`:

| File | Observatory | Period |
|---|---|---|
| `data/isdc_n.json` | SDC North (CTAO-N) | Full year |
| `data/isdc_s.json` | SDC South (CTAO-S) | Full year |
| `data/lst_2024.json` | LST | 2024 |
| `data/lst_2025.json` | LST | 2025 |
| `data/lst_2026.json` | LST | 2026 |

To convert a raw CTAO dataset directory:

```bash
cargo run -p lab --bin phd -- dataset adapt CTA-N
cargo run -p lab --bin phd -- dataset adapt CTA-S
# or with explicit paths:
cargo run -p lab --bin phd -- dataset adapt data/my_ctao_dir data/my_ctao_dir/scheduling_problem.json
```

---

## QA

```bash
cargo clippy --workspace --exclude tsi-rust --all-targets -- -D warnings
cargo fmt --all -- --check
cargo test --workspace --exclude tsi-rust --all-features
```

Shortcut:

```bash
./scripts/qa-pipeline.sh
```

Auto-fix formatting:

```bash
cargo fmt --all
```

---

#### Export a schedule from the registry

```bash
cargo run -p lab --bin lab -- registry export \
  --run-db .lab/runs.sqlite \
  --run 9f3a7c \
  --out out/best-density.json
```
