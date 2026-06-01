# Lab

`lab` is the experiment runner for the PhD scheduling workspace. It expands
JSON sweep specifications into scheduler runs, executes the resulting matrix in
parallel, and stores each run's identity, configuration, metrics, and
deduplicated schedule body in a SQLite registry.

The crate provides three binaries:

| Binary | Purpose |
| --- | --- |
| `phd` | User-facing workflow CLI for sweeps, publishing, and dispatch to sibling tools. |
| `lab` | Lower-level matrix runner and registry query CLI. |
| `lab-ctao-adapter` | Dataset adapter used by `phd dataset adapt`. |

For routine experiments, use `phd sweep`. Use `lab run` directly when you need
fine-grained control over the DB path or override semantics.

## Common Workflows

### Run a Sweep

```bash
cargo run -p lab --bin phd -- sweep \
  --spec lab/sweep-fast.json
```

Results are stored in `.lab/runs.sqlite` (default). Pass `--run-db <PATH>` to
use a different database. Cells already present in the DB are skipped
automatically; add `--override` to re-execute them and update their stored row.

| Flag | Description |
| --- | --- |
| `--spec <FILE>` | Experiment specification JSON. |
| `--run-db <PATH>` | Registry database path. Defaults to `.lab/runs.sqlite`. |
| `--parallel <N>` | Override `max_parallel` from the spec. |
| `--override` | Re-execute cells that are already in the DB. |

### Run the Matrix Directly

```bash
cargo run -p lab --bin lab -- run \
  --spec lab/sweep-fast.json
```

`lab run` resolves the cell matrix, skips DB hits, executes the rest in
parallel, and upserts each result into the registry. No filesystem artifacts
are written — no `schedules/` dir, no `state.jsonl`, no `experiment.json`.

| Flag | Description |
| --- | --- |
| `--spec <FILE>` | Required experiment spec JSON. |
| `--run-db <PATH>` | Registry database path. Defaults to `.lab/runs.sqlite`. |
| `--override` | Re-execute cells that are already in the DB. |

### Publish Results

```bash
cargo run -p lab --bin phd -- publish \
  --workspace paper \
  --dir out/schedules \
  --create-workspace
```

`phd publish` walks a directory for self-contained schedule JSON files and
uploads them to the webapp in batches. Use `--url` or `PHD_WEBAPP_URL` to
target a non-default backend, and `--token` or `PHD_WEBAPP_TOKEN` when
authentication is required.

Export schedule files from the registry before publishing:

```bash
cargo run -p lab --bin lab -- registry export \
  --dataset isdc_n \
  --out-dir out/schedules
```

## Experiment Specifications

An experiment specification describes a Cartesian product of datasets,
algorithms, and per-algorithm parameter axes. The `output_dir` field is
accepted for backward compatibility but ignored by the runner.

```json
{
  "name": "fast-comparison",
  "max_parallel": 16,
  "datasets": [
    {
      "id": "isdc_n",
      "path": "datasets/isdc_n.json",
      "label": "ISDC North"
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
    },
    {
      "kind": "lst",
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
| `name` | Human-readable experiment name. |
| `max_parallel` | Optional concurrency cap. Defaults to available logical CPU capacity. |
| `datasets` | Input scheduling problems to evaluate. |
| `algorithms` | One or more `est`, `lst`, `multi_cursor`, or `hap` sweep blocks. |
| `output_dir` | Ignored — present only for backward compatibility with old specs. |

Dataset entries support:

| Field | Description |
| --- | --- |
| `id` | Filesystem-safe dataset identifier used in cell IDs. |
| `path` | Scheduling problem JSON. Relative paths are resolved from the spec file's directory. |
| `label` | Optional human-readable label stored in schedule metadata. |
| `horizon_override` | Optional `{ "start_mjd": <f64>, "end_mjd": <f64> }` observing window override. |

Current runnable examples live in this directory:

| Spec | Purpose |
| --- | --- |
| `lab/sweep-fast.json` | Small EST/LST comparison suitable for smoke tests and iteration. |
| `lab/sweep-custom.json` | Broader EST/LST comparison over all bundled sample datasets. |
| `lab/sweep-full.json` | Larger EST/HAP comparison template. Review dataset paths and cell count before running. |
| `lab/sweep-all.json` | All-datasets, all-algorithms example including `est_lst_split` and `four_quarter_forward` multi-cursor layouts. |

## Algorithm Axes

`est` and `lst` use the same sweep axes:

| Axis | Description |
| --- | --- |
| `endangered_thresholds` | Residual-flexibility thresholds for endangered-task promotion. |
| `k_beams` | Beam counts to evaluate. |
| `branching_factors` | Branching factors to evaluate. |
| `foms` | Figure-of-merit variants, such as `soft_constraint` or `future_flexibility`. |

`multi_cursor` adds:

| Axis | Description |
| --- | --- |
| `layouts` | Layout names such as `est_lst_split`, `start_mid_forward`, `four_quarter_forward`, `dynamic_est_lst_meet`, or `dynamic_start_mid_forward`. |
| `endangered_thresholds` | Same residual-flexibility threshold semantics as EST/LST. |
| `k_beams` | Beam counts to evaluate. |
| `branching_factors` | Branching factors to evaluate. |
| `foms` | Figure-of-merit variants, such as `soft_constraint` or `future_flexibility`. |

`hap` supports:

| Axis | Description |
| --- | --- |
| `iota_max_values` | CRU task-scheduling iteration caps. |
| `rho_values` | CRU-S stochastic candidate ranges. |
| `population_sizes` | Multi-start population sizes per block. |
| `survivor_modes` | Survivor strategies: `greedy_one`, `elitist_top_k`, or `pareto_front`. |
| `survivor_caps` | Capacity limits for the selected survivor mode. |
| `seeds` | Deterministic master RNG seeds. |

Empty axis lists use algorithm defaults, so a block with omitted axes still
produces at least one run.

## Registry

The registry is a SQLite database that stores each successful run's identity,
metrics, and full schedule JSON. Runs are keyed by a SHA-256 hash of the inputs,
so the same cell is naturally idempotent across reruns.

Registry query commands:

| Command | Purpose |
| --- | --- |
| `lab registry list` | List stored runs with optional filters and metric ranges. |
| `lab registry sort` | Sort runs by one or more `metric:asc` or `metric:desc` keys. |
| `lab registry best` | Show best runs for a required dataset. |
| `lab registry rank` | Compute a weighted query-time score from `--weight metric=value` inputs. |
| `lab registry pareto` | Compute a Pareto front over maximize/minimize objectives. |
| `lab registry inspect` | Print the full stored record for a run key or unique prefix. |
| `lab registry export` | Export stored schedule JSON(s) to files. |

Example:

```bash
cargo run -p lab --bin lab -- registry sort \
  --run-db .lab/runs.sqlite \
  --dataset isdc_n \
  --sort priority_density:desc \
  --limit 10
```

Most registry commands accept `--dataset`, `--algorithm`, `--limit`, and
`--format table|json`. Commands that return ordered results accept repeatable
`--sort <metric:asc|desc>` arguments.

**Registry DB columns**

The registry schema stores run rows in the `runs` table and deduplicated
schedule JSON in the `schedules` table. Key columns and their meanings:

| Column | Description |
| --- | --- |
| `run_key` | Primary key: 64-char SHA-256 hash identifying the run (dataset hash, algorithm, config, horizon, versions). |
| `dataset_id` | Dataset identifier from the experiment spec. |
| `dataset_path` | Filesystem path to the dataset JSON used for the run. |
| `dataset_hash` | Content hash of the dataset file used to detect changes. |
| `algorithm` | Algorithm kind: `est`, `lst`, or `hap`. |
| `config_slug` | Short human-readable slug for the run configuration (e.g. `e1-k3-b1`). |
| `config_json` | Serialized run configuration JSON. |
| `horizon_json` | Optional serialized horizon override JSON. |
| `scheduler_version` | Version string for the scheduler implementation used. |
| `metrics_version` | Version/schema of the `metrics_json` payload. |
| `identity_json` | Full serialized `RunIdentity` object stored with the row. |
| `metrics_json` | Serialized schedule/metrics JSON (objective and descriptive metrics). |
| `schedule_hash` | Reference key into the `schedules` table for the stored schedule JSON (deduplication). |
| `task_ratio` | Indexed metric: fraction of tasks scheduled (descriptive). |
| `priority_ratio` | Indexed metric: fraction of total priority that was scheduled. |
| `priority_density` | Indexed metric used for ranking by priority per unit time. |
| `utilization` | Indexed metric: fraction of available time used. |
| `fragmentation_index` | Indexed metric describing schedule fragmentation (lower preferred). |
| `runtime_ms` | Indexed metric: scheduler runtime in milliseconds. |
| `requested_time_sec` | Total requested observation time (seconds). |
| `scheduled_time_sec` | Total scheduled observation time (seconds). |
| `scheduled_time_ratio` | Ratio `scheduled_time_sec / requested_time_sec`. |
| `created_at` | Row creation timestamp (ISO 8601). |
| `last_seen_at` | Last upsert/refresh timestamp (ISO 8601). |
| `source_cell_id` | Optional `cell_id` from the originating experiment manifest. |

Schedules table (`schedules`):

| Column | Description |
| --- | --- |
| `schedule_hash` | Primary key: canonical schedule hash used for deduplication. |
| `dataset_hash` | Dataset content hash associated with the schedule. |
| `schedule_json` | Deduplicated invariant schedule body (problem + placements only). |
| `created_at` | Timestamp when the schedule JSON was inserted. |

### `registry export`

Export one run by key:

```bash
lab registry export \
  --run b50d151629d65018 \
  --out schedule.json
```

Export filtered runs to a directory:

```bash
lab registry export \
  --dataset isdc_n \
  --sort priority_density:desc \
  --limit 20 \
  --out-dir out/top20
```

Exported files are named `<dataset>__<algorithm>__<config>.json`. On filename
collision the run key prefix is appended. Use `--force` to overwrite existing
files. Rows without a stored schedule (pre-migration rows) print a guidance
message directing you to rerun with `--override`.

## CLI Reference

### `lab run`

| Flag | Description |
| --- | --- |
| `--spec <FILE>` | Required experiment spec JSON. |
| `--run-db <PATH>` | Registry database path. Defaults to `.lab/runs.sqlite`. |
| `--override` | Re-execute cells already in the DB and update their stored row. |

### `phd`

| Command | Description |
| --- | --- |
| `phd run` | Dispatch to the `schedulers` binary for a single scheduling problem. |
| `phd matrix` | Dispatch raw arguments to the `lab` binary. |
| `phd sweep` | Run a sweep via `lab run` and store results in SQLite. |
| `phd dataset adapt` | Dispatch to `lab-ctao-adapter`. |
| `phd publish` | Upload schedule JSON files from a directory to the webapp. |

## Operational Notes

- Build sibling binaries explicitly when invoking `phd` outside `cargo run`:

  ```bash
  cargo build -p lab --bin lab --bin phd --bin lab-ctao-adapter
  cargo build --release -p lab --bin lab --bin phd --bin lab-ctao-adapter
  ```

- No filesystem artifacts are written by `lab run`. All outputs (schedule JSON
  and metrics) are stored in the registry database. Use `registry export` to
  export schedule files.
- Size sweeps deliberately. Cell count grows as
  `datasets × algorithms × axis-products`.
- Use `max_parallel` or `phd sweep --parallel` to control CPU pressure.
- Prefer registry queries for ranking and comparison. The legacy `ranking` spec
  field is accepted only for backward compatibility.
