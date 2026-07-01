# Lab Cheatsheet — Commands & Queries

Quick reference for common development commands, run targets, and useful queries when working in the lab crate and the PhD workspace.

## Quick Cargo / workspace

- Run the `lab` binary (release):

```bash
cargo run -p lab --bin phd --release -- sweep --spec lab/sweep-all.json
```

- Run `lab` in debug:

```bash
cargo run -p lab --bin phd -- sweep --spec lab/sweep-custom.json
```

- Run all workspace tests (exclude tsi-rust if needed):

```bash
cargo test --workspace --exclude tsi-rust --all-features
```

## Sweeps & experiments

- Run a specific sweep spec file:

```bash
cargo run -p lab --bin phd -- sweep --spec lab/sweep-fast.json
```

- Run a single experiment or manifest (example style):

```bash
# Example: run a single sweep/spec JSON file
cargo run -p lab --bin phd -- release -- sweep --spec path/to/your_spec.json
```

## Registry CLI — Query local runs

The `lab` crate exposes a read-only registry CLI that queries the local
SQLite registry (default `.lab/runs.sqlite`). Useful when inspecting sweep
results without touching the DB directly.

- List runs with filters and sorting:

```bash
cargo run -p lab --bin lab -- registry list --run-db .lab/runs.sqlite \
  --dataset <DATASET_ID> --algorithm est \
  --metric utilization --min 0.5 --sort utilization:desc --limit 50
```

- Show best runs for a dataset:

```bash
cargo run -p lab --bin lab -- registry best --run-db .lab/runs.sqlite \
  --dataset <DATASET_ID> --limit 5
```

- Inspect a single run (full stored record):

```bash
cargo run -p lab --bin lab -- registry inspect --run <RUN_KEY> --run-db .lab/runs.sqlite
```

- Export matching schedules to files (filtered export):

```bash
cargo run -p lab --bin lab -- registry export --run-db .lab/runs.sqlite \
  --dataset <DATASET_ID> --out-dir out/ --sort utilization:desc --limit 100
```


## QA / formatting / linting

- Run the repository QA pipeline (recommended):

```bash
./scripts/qa-pipeline.sh
```

- Clippy for the workspace (fail on warnings):

```bash
cargo clippy --workspace --exclude tsi-rust --all-targets -- -D warnings
```

- Format check and fix:

```bash
cargo fmt --all -- --check   # check
cargo fmt --all              # fix
```

## Useful file & test targets

- Run only `lab` tests:

```bash
cargo test -p lab
```

- Run a single test (example):

```bash
cargo test name_of_test -- --nocapture
```

## Data & datasets

- Example dataset files under `lab/datasets/`:
  - `lab/datasets/isdc_n.json`
  - `lab/datasets/isdc_s.json`

- Quick jq query examples:

```bash
# Print top-level keys
jq 'keys' lab/datasets/isdc_n.json

# Filter entries by some field (example)
jq '.[] | select(.field=="value")' lab/datasets/isdc_n.json
```

## Grep / code search

- Find occurrences of a symbol or term in the workspace:

```bash
rg "search_term" -S
```

### Local `.lab` database

- The project keeps a local SQLite DB at `.lab/runs.sqlite`.

```bash
# List tables
sqlite3 .lab/runs.sqlite ".tables"

# Show schema for a table
sqlite3 .lab/runs.sqlite ".schema table_name"

# Inspect table columns
sqlite3 .lab/runs.sqlite "PRAGMA table_info(table_name);"

# Run a query (example: show last 10 rows of an assumed `runs` table)
sqlite3 .lab/runs.sqlite "SELECT * FROM runs ORDER BY created_at DESC LIMIT 10;"
```

If you don't know the table names, start with `.tables` or query `sqlite_master`:

```sql
SELECT name, type FROM sqlite_master WHERE type IN ('table','index') ORDER BY name;
```

## Webapp / docker (quick)

- Webapp QA script:

```bash
cd webapp && ./qa-pipeline.sh
```

- Setup/teardown helper scripts in `webapp/`:

```bash
cd webapp && ./setup.sh
cd webapp && ./teardown.sh
```

## Troubleshooting & tips

- If a run fails with an exit code, re-run with `--nocapture` or inspect logs produced by the binary.
- When adding new dependencies, run `./scripts/qa-pipeline.sh` to catch style and lint issues early.

## Example common workflows

- Run full QA locally (format, clippy, tests):

```bash
./scripts/qa-pipeline.sh
```

- Run a full release sweep and capture output (example):

```bash
cargo run -p lab --bin phd --release -- sweep --spec lab/sweep-all.json | tee sweep-run.log
```

## Where to look next

- lab/README.md for lab-specific documentation and example sweep specs.
- AGENTS.md for repo overview and QA requirements.

---
*Created as a quick reference — expand this file with project-specific examples as needed.*
