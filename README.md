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
cargo run --bin scheduler -- <input_json> [horizon_start_mjd horizon_end_mjd] [--est-endangered-threshold <u32>]
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
cargo run --bin scheduler -- data/CTA-N/scheduling_problem.json --est-endangered-threshold 2
```

The scheduler writes a result file next to the input using the pattern:

```text
<input_stem>_schedule.json
```

For example:

```text
data/CTA-N/scheduling_problem_schedule.json
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
