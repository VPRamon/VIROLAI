# Scripts

## `ctao_adapter`

Rust CLI that converts CTA dataset files in one directory (`*_internalSDC.json`) into one aggregated `scheduling_problem.json` file compliant with `schemas/scheduling_problem.schema.json`.

The output is a minimal scheduler-ready PhD payload:

- top-level object fields: `resources`, `schedule_time_window`, `scheduling_blocks`
- `resources` contains one inferred telescope resource with `id`, `name`, geodetic `location` (`longitude_deg`, `latitude_deg`, `height_m`), and telescope `hard_constraints`
- adapter sets telescope `hard_constraints.night_time.twilight = "Astronomical"` and `hard_constraints.moon_altitude = {"min_deg": -90, "max_deg": 0}` to enforce night-only windows with Moon below horizon
- adapter infers dataset observatory (`CTA-N` or `CTA-S`) and translates it to site coordinates (`CTA-N` -> Roque de los Muchachos, `CTA-S` -> El Paranal)
- `schedule_time_window` defaults to UTC `[2028-01-01T00:00:00Z, 2029-01-01T00:00:00Z)` expressed in MJD
- each scheduling block has `id`, `tasks`, and `dependencies`
- `tasks` contains task objects (not CTA configuration payloads)
- each task object includes at least `id`, `requested_duration_sec`, `hard_constraints`, and optional `soft_constraints.priority`
- each CTA scheduling block is currently converted to one task object (`tasks: [{...}]`)

### Run

```bash
cargo run --bin ctao_adapter -- <dataset_dir> [output_json]
```

### Examples

```bash
cargo run --bin ctao_adapter -- CTA-N
cargo run --bin ctao_adapter -- CTA-S
cargo run --bin ctao_adapter -- data/CTA-N data/CTA-N/scheduling_problem.json
```

### Notes

- If `dataset_dir` is `CTA-N` or `CTA-S`, the script also checks `data/<dataset_dir>`.
- Default output is `<dataset_dir>/scheduling_problem.json`.
- For current CTA datasets, each exported scheduling block contains exactly one task object.

## `phd_tsi_server`

Rust HTTP server that embeds the TSI backend and registers a custom import adapter for this repository's current scheduling schema (`schemas/scheduling_problem.schema.json`).

The adapter accepts payloads with top-level fields:

- `resources` (uses `resources[0].location` as the TSI observing site)
- `schedule_time_window`
- `scheduling_blocks[*].tasks[*]` (full task objects only)

Each task object is mapped into one TSI scheduling block.

### Run locally

```bash
cargo run --bin phd_tsi_server
```

The API starts at `http://localhost:8080`.

### Run with Docker

From the repository root:

```bash
./webapp/setup.sh
```

Stop services while keeping database data:

```bash
./webapp/teardown.sh
```

Remove services and database data:

```bash
./webapp/teardown.sh --purge-db
```

Equivalent direct command:

```bash
docker compose -f webapp/docker/docker-compose.yml up --build
```

Services:

- frontend: `http://localhost:3000`
- backend health: `http://localhost:8080/health`

The Docker stack uses a persistent PostgreSQL volume (`postgres_data`) by default.

Upload any `scheduling_problem.json` compatible with `schemas/scheduling_problem.schema.json` directly in the TSI UI.

## `est_experiment`

Rust CLI that runs one scheduling problem across a resolved sweep of EST configurations and writes:

- one annotated scheduler output JSON per configuration under `output_dir/run-<timestamp>/schedules/`
- `output_dir/run-<timestamp>/comparison.csv` with compact per-run metrics
- `output_dir/run-<timestamp>/manifest.json` describing the resolved run set

Schedule files use this naming pattern:

- `e{endangered_threshold}-k{k_beams}-b{branching_factor}.json`

The comparison CSV includes only these columns:

- `run_slug`
- `is_baseline`
- `scheduled_task_count`
- `fitness_priority_sum` (sum of scheduled-task priorities)
- `scheduled_priority_p25`
- `scheduled_priority_p50`
- `scheduled_priority_p75`
- `scheduled_priority_p90`

`run_slug` encodes configuration as `e{endangered_threshold}-k{k_beams}-b{branching_factor}`.

The default baseline EST configuration is:

- `fom = "soft_constraint"`
- `endangered_threshold = 1`
- `k_beams = 1`
- `branching_factor = 1`

### Run with a spec file

```bash
cargo run --bin est_experiment -- --spec experiments/ctao_n_est.json
```

### Run with CLI overrides

```bash
cargo run --bin est_experiment -- data/CTA-N/scheduling_problem.json \
  --output-dir out/ctao_n_est \
  --est-e-values 1,2 \
  --est-k-values 1,4 \
  --est-b-values 1,2
```

### Spec format

```json
{
  "input_json": "data/CTA-N/scheduling_problem.json",
  "output_dir": "out/ctao_n_est",
  "horizon_override": {
    "start_mjd": 61710.0,
    "end_mjd": 61720.0
  },
  "sweep": {
    "endangered_thresholds": [1, 2],
    "k_beams": [1, 4],
    "branching_factors": [1, 2]
  }
}
```

### Notes

- Relative `input_json` and `output_dir` paths in the spec are resolved relative to the spec file location.
- CLI sweep values override the spec sweep axes; unspecified axes fall back to the spec and then to the baseline singleton.
- Each run creates a new timestamped subdirectory (`run-<timestamp>`) under `output_dir`, so multiple runs can coexist.
