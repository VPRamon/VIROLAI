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

# 2. Run a sweep experiment (multiple datasets × algorithm configurations)
cargo run -p lab --bin phd --release -- sweep \
  --spec lab/est_sweep.json \
  --out out/my-sweep \
  --manifest

# 3. Start the web app
./webapp/setup.sh -d          # Docker (frontend + backend + postgres)

# 4. Open the web app
#    http://localhost:3000  → Workspace page → Algorithm Results section
#    Drop the generated *.manifest.json files from out/my-sweep/ onto a result workspace
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
| `sweep` | Run a parameter sweep and collect flat results (primary workflow) |
| `matrix` | Lower-level alias — delegates directly to the `lab` binary |
| `run` | Run a single scheduling problem (delegates to `schedulers`) |
| `dataset adapt` | Convert a CTAO dataset directory into `scheduling_problem.json` |
| `manifest create` | Build manifest(s) from a sweep run-directory or a single schedule JSON |
| `manifest validate` | Validate a manifest against structural rules |

---

### `phd sweep`

```
phd sweep --spec <FILE> --out <DIR> [--manifest] [--parallel <N>]
```

Runs the full experiment matrix described in `<FILE>` and writes **one
self-contained schedule JSON per cell** into `<DIR>` (flat — no
subdirectories). Use `--manifest` to also emit a companion
`<cell_id>.manifest.json` next to every schedule.

> Note: `phd sweep` invokes the sibling `lab` binary from
> `target/debug/lab` or `target/release/lab`. If you encounter
> `failed to spawn lab: No such file or directory`, build it first with:
>
> ```bash
> cargo build -p lab --bin lab
> ```
>
> For a release run, build the release binary as well:
>
> ```bash
> cargo build --release -p lab --bin lab
> ```

| Flag | Required | Description |
|---|---|---|
| `--spec <FILE>` | ✅ | Path to the experiment spec JSON (see [Spec Format](#spec-format)) |
| `--out <DIR>` | ✅ | Output directory — created if absent |
| `--manifest` | — | Emit `<cell_id>.manifest.json` alongside each schedule |
| `--parallel <N>` | — | Override the number of parallel worker threads (defaults to the spec's `max_parallel` or the CPU count) |

**Output layout:**

```
out/my-sweep/
  cta_n__est__e0_k1_b1.json               # self-contained schedule
  cta_n__est__e0_k1_b1.manifest.json      # companion manifest  (with --manifest)
  cta_n__est__e0_k1_b2.json
  cta_n__est__e0_k1_b2.manifest.json
  ...
```

Each schedule JSON embeds `schedule_metadata` (algorithm, config, dataset id/label,
scheduling horizon) and `schedule_metrics` (completion rates, priority statistics,
fragmentation, etc.) so it is fully self-contained.

---

### `phd manifest create`

Build manifests after the fact, either for an entire run directory or for a single
schedule file.

#### From a whole sweep run directory (`--run`)

```
phd manifest create --run <run-dir> [--out <dir>] [--skip-existing]
```

Walks the `<run-dir>/cells/` subdirectory produced by `phd matrix`, builds one
manifest per cell, and writes them to `<out>` (defaults to `<run-dir>/cells/`).

| Flag | Description |
|---|---|
| `--run <DIR>` | Path to the `run-<ts>/` directory from `phd matrix` |
| `--out <DIR>` | Override output directory |
| `--skip-existing` | Skip cells whose `.manifest.json` already exists |

#### From a single schedule file (`--schedule`)

```
phd manifest create --schedule <file.json> [--out <file.manifest.json>]
```

Reads the embedded `schedule_metadata` and `schedule_metrics` from the schedule JSON
and writes a manifest to `--out` (or to stdout if omitted).

| Flag | Description |
|---|---|
| `--schedule <FILE>` | Self-contained schedule JSON (must have embedded metadata and metrics) |
| `--out <PATH>` | Output file path (default: stdout) |

> **Note:** `--run` and `--schedule` are mutually exclusive.

---

### `phd manifest validate`

```
phd manifest validate <manifest.json>
```

Runs structural validation against the manifest schema rules and prints a report.
Exits with a non-zero status if any errors are found.

---

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
  "output_dir": "out/my-experiment",
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
| `output_dir` | string | — | Where to write the run directory (`phd matrix`) or flat output (`phd sweep --out` overrides this) |
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

This section walks through the complete workflow: **run many configurations → generate
manifests → upload to the webapp**.

### Step 1 — Prepare your experiment spec

Copy one of the bundled examples from `lab/` and edit it:

```bash
cp lab/est_sweep.json lab/my_sweep.json
```

Edit `my_sweep.json` to point at your datasets and set the parameter ranges.

### Step 2 — Run the sweep

```bash
cargo run -p lab --bin phd --release -- sweep \
  --spec lab/my_sweep.json \
  --out out/my-sweep \
  --manifest
```

- `--release` is recommended for large sweeps (significantly faster).
- `--manifest` writes a `.manifest.json` alongside every schedule.
- Progress is logged to stderr; each cell runs in parallel.

After completion:

```
out/my-sweep/
  isdc_n__est__e0_k1_b1.json
  isdc_n__est__e0_k1_b1.manifest.json
  isdc_n__est__e0_k2_b1.json
  isdc_n__est__e0_k2_b1.manifest.json
  ...
```

### Step 3 — (Optional) Build manifests from existing schedules

If you ran the sweep without `--manifest`, or want to regenerate manifests:

```bash
# For a single schedule:
cargo run -p lab --bin phd -- manifest create \
  --schedule out/my-sweep/isdc_n__est__e0_k1_b1.json \
  --out out/my-sweep/isdc_n__est__e0_k1_b1.manifest.json

# For all schedules in a run directory (phd matrix output):
cargo run -p lab --bin phd -- manifest create \
  --run out/matrix-run/run-20260101T120000Z \
  --out out/matrix-run/manifests
```

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

> **Tip:** The fastest workflow is `phd sweep --manifest` followed by drag-and-drop
> of the entire `out/my-sweep/` folder onto the drop zone. Manifests are uploaded
> directly; any plain schedule JSONs are converted server-side.

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

Each schedule JSON produced by `phd sweep` embeds:

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



#### Generate schedule

cargo run -p lab --bin lab -- registry regenerate \
  --run-db .lab/runs.sqlite \
  --run 9f3a7c \
  --out out/best-density.json
