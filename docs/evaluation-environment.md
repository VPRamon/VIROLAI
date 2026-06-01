# Algorithm Evaluation Environment

> **Status:** legacy design note. The current experiment runner is DB-only and
> is documented in [README.md](../README.md) and
> [docs/algorithms/sweep-configuration.md](algorithms/sweep-configuration.md).
> This file no longer describes the canonical implementation.

The historical design was built around four layers:

```
┌──────────────────┐   ┌─────────────────────┐   ┌────────────────────┐   ┌──────────────────────┐
│ schedulers::metrics│ │ lab runner          │  │ webapp      │  │ phd-extensions UI    │
│ (canonical metric │→ │ (matrix runner +     │→ │ (filesystem catalog │→ │ (Experiments section │
│  shape)           │  │  on-disk artefacts)  │  │  + REST + SSE)      │  │  in the webapp)      │
└──────────────────┘   └─────────────────────┘   └────────────────────┘   └──────────────────────┘
```

## 1. Metric shape — `schedulers::metrics`

Single source of truth for evaluation metrics. See `schedulers/src/metrics.rs`.

`ScheduleMetrics::compute(&Schedule, &SchedulingProblem, &Period<MJD>, &MetricsContext)` returns:

- `scheduled_task_count`, `total_task_count`
- `priority_stats` — `count, sum, min, max, mean, std, p25, p50, p75, p90`
- `fragmentation` — `gap_count, gap_total_sec, largest_gap_sec, fragmentation_index`
- `utilization` — fraction of the horizon that is actually scheduled
- `per_resource: Vec<ResourceMetrics>` — currently length 1 (single
  telescope), typed as `Vec` for forward compatibility with multi-resource
  problems
- objective/descriptive run metrics such as scheduled-task ratio,
  scheduled-priority ratio, priority density, utilization, fragmentation,
  and scheduler runtime. Ranking policy belongs in query/analysis commands,
  not in sweep execution.

All fields implement `Serialize`/`Deserialize` so cells can be persisted
and round-tripped without recomputing schedules. **Do not duplicate
objective metric computation** anywhere else in the tree — extend this module.

## 2. Matrix runner — `lab` (historical)

Today the runner writes into the SQLite registry via:

```bash
cargo run --release -p lab --bin phd -- sweep \
    --spec my-experiment.json \
    --run-db .lab/runs.sqlite
```

### Spec shape

`ExperimentSpec` (see `lab/src/spec.rs`) is a Cartesian
product of:

- `datasets: Vec<DatasetRef>` — by id or path
- `algorithms: Vec<AlgorithmEntry>` — current kinds are `est`, `lst`,
  `multi_cursor`, and `hap`

The runner produces one **cell** per
`(dataset, algorithm, config)` triple. Cells run in parallel through a
bounded rayon pool with one shared `PreparedProblem` per dataset.

### Historical on-disk layout

```
<output_dir>/<experiment_slug>/run-<ts>/
  experiment.json                   # spec + resolved cell list
  state.jsonl                       # append-only checkpoint events
  summary.csv                       # 25-column flatten of ScheduleMetrics
  schedules/<cell_id>.json          # raw schedule
  metrics/<cell_id>.json            # serialized ScheduleMetrics
  traces/<cell_id>.jsonl            # algorithm trace
```

- `cell_id = <dataset_id>__<algo>__<config_slug>`.
- `state.jsonl` events: `Started`, `CellCompleted`, `CellFailed`,
  `Finished`. The backend tails this file to drive live progress.

### CLI

| Flag | Purpose |
|------|---------|
| `--spec <path>` | Spec file to run. |
| `--output-dir <path>` | Root experiments directory. |
| `--dry-run` | Resolve cells and write `experiment.json` only. |
| `--resume <run-dir>` | Skip cells already marked `CellCompleted`. |
| `migrate --legacy <dir> --output <dir>` | Port pre-existing `est_experiment` runs. |

## 3. Backend — `webapp` Experiments domain (historical)

Lives entirely in `webapp/src/experiments/` (the upstream TSI
submodule under `webapp/TSI/` is **not** modified). Mounted into the
TSI router via `BackendExtensions::with_routes(...)`.

### Catalog

Filesystem-backed (no migrations). Discovers
`<root>/<slug>/run-*/experiment.json`, caches an in-memory index with a
short TTL, and lazily reads `state.jsonl` to derive live status.
Configured via `PHD_EXPERIMENTS_DIR` (default `./experiments`).

### Orchestrator

`ExperimentRunner` spawns the matrix binary as a Tokio child process,
captures stdout/stderr to per-run log files, supports cancel
(SIGTERM on Unix) and resume, and caps concurrent runs through
`PHD_EXPERIMENTS_MAX_CONCURRENT` (default 1, since the runner already
saturates rayon). Locate the binary via
`PHD_EXPERIMENT_MATRIX_BIN` if it isn't a sibling of the server.

### Routes (under `/v1/experiments`)

| Method + path | Purpose |
|---|---|
| `GET /` | List experiments + their runs |
| `POST /` | Submit a new spec → `{slug, run_id, output_dir}` |
| `GET /:slug/runs/:run_id` | Spec + status |
| `POST /:slug/runs/:run_id/cancel` | Cancel running orchestration |
| `POST /:slug/runs/:run_id/resume` | Resume a stopped run |
| `GET /:slug/runs/:run_id/cells?filter=&limit=&offset=` | Cell headlines |
| `POST /:slug/runs/:run_id/cells/bulk` | **One-shot** metrics fetch for many cells |
| `GET /:slug/runs/:run_id/cells/:cell_id` | Full cell |
| `GET /:slug/runs/:run_id/cells/:cell_id/schedule` | Raw schedule |
| `GET /:slug/runs/:run_id/cells/:cell_id/trace` | Trace JSONL stream |
| `GET /:slug/runs/:run_id/summary.csv` | Streamed CSV |
| `GET /:slug/runs/:run_id/pareto?x=&y=&xmax=&ymax=` | Pareto front |
| `GET /:slug/runs/:run_id/ranking?by=dataset\|algorithm` | Aggregated ranking |
| `GET /:slug/runs/:run_id/events` | **SSE** state stream |

The bulk endpoint is the answer to the previous "webapp slow when
inspecting many schedules" pain point — frontends must use it instead
of fanning out one HTTP call per cell.

## 4. Frontend — `phd-extensions` Experiments section

A new top-level nav item registered through the v1 extension contract
(see `webapp/phd-extensions/index.tsx`). All page code is
`React.lazy`-loaded and lives under
`webapp/phd-extensions/pages/experiments/`.

### Routes

| Path | View |
|---|---|
| `/experiments` | List of experiments |
| `/experiments/new` | Submit spec |
| `/experiments/:slug/:runId/overview` | Live KPIs + headline distributions |
| `/experiments/:slug/:runId/matrix` | Heatmap (datasets × algorithm/config) |
| `/experiments/:slug/:runId/pareto` | Pareto front with metric pickers |
| `/experiments/:slug/:runId/per-dataset` | Per-dataset rankings |
| `/experiments/:slug/:runId/per-algorithm` | Algorithm-sensitivity view |
| `/experiments/:slug/:runId/cells/:cellId` | Cell detail |

### Data layer (`webapp/phd-extensions/lib/experiments/`)

- `useExperimentRun(slug, runId)` — owns the single `EventSource` for the
  run, reconnects with exponential backoff (1s → 30s).
- `useBulkCellMetrics(cellIds)` — dedupes + sorts requested ids,
  debounces 50ms, and issues one `POST /cells/bulk` per render batch;
  drops stale responses via a sequence counter.

These two hooks codify the perf rules: never per-cell GETs, never
per-tab SSE.

## 5. Running the QA pipeline

```bash
./scripts/qa-pipeline.sh
```

Equivalent to:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all-features
```

The frontend has its own gate:

```bash
cd webapp/TSI/frontend
npm run type-check
npm test
npm run build
```

## 6. Extending the environment

- **New metric**: add a field to `ScheduleMetrics` in `src/metrics.rs`,
  cover it with a unit test, then surface it in
  (a) the runner's `summary.csv` flatten, (b) the matrix tab's metric
  selector, (c) the pareto tab's axis dropdowns.
- **New algorithm**: add an arm to the runner's per-algorithm execution
  branch (`lab/src/cell.rs`), define its sweep-axis
  shape in `spec.rs`, and add the algorithm id to the New Experiment
  form.
- **New tab**: drop a component under
  `webapp/phd-extensions/pages/experiments/tabs/` and add a `<Route>` in
  `ExperimentDetailPage.tsx`. Reuse `_ui.tsx` primitives so the visual
  language stays consistent.

## Known v1 limitations

- `per_resource` is always length 1 because `SchedulingProblem` carries a
  single telescope today; the shape is already plural for the future
  multi-resource refactor.
- The orchestrator caps concurrent matrix runs at 1 by default to avoid
  oversubscribing CPUs — multi-run scheduling can be revisited once the
  runner exposes its rayon budget.
- The catalog uses TTL+poll, not inotify; SSE polls `state.jsonl` at
  1Hz. Both are intentionally simple for v1.
- Per-algorithm sensitivity tab ships as an empty-state placeholder
  pending parallel-coordinates design work.
