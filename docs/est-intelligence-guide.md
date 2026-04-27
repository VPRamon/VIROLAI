# EST Intelligence Panel — User Guide

This document explains the **EST parameter sweep** and **EST Intelligence** webapp pages, how to run experiments, and how to feed the results into the interactive dashboards.

---

## Table of contents

1. [Overview](#overview)
2. [Running an experiment](#running-an-experiment)
   - [Prerequisites](#prerequisites)
   - [Quick start — single run](#quick-start--single-run)
   - [Sweep mode — scanning the parameter space](#sweep-mode--scanning-the-parameter-space)
   - [Spec-file approach (recommended for large sweeps)](#spec-file-approach-recommended-for-large-sweeps)
   - [Output artifacts](#output-artifacts)
3. [Importing results into the webapp](#importing-results-into-the-webapp)
4. [EST Sweep tab](#est-sweep-tab)
5. [EST Intelligence tabs](#est-intelligence-tabs)
   - [Overview tab](#overview-tab)
   - [Sensitivity tab](#sensitivity-tab)
   - [Pareto tab](#pareto-tab)
   - [Internals tab](#internals-tab)
   - [Statistics tab](#statistics-tab)
6. [Configuration parameters reference](#configuration-parameters-reference)
7. [Architecture notes](#architecture-notes)

---

## Overview

The EST (Earliest-Start-Time beam-search) scheduler is parameterised by three knobs:

| Parameter | CLI flag | Meaning |
|---|---|---|
| `e` — endangered threshold | `--est-e-values` | Tasks with temporal flexibility ≤ e are promoted to the front of the beam |
| `k` — beam width | `--est-k-values` | Number of candidate schedules kept alive per search round |
| `b` — branching factor | `--est-b-values` | Maximum number of children explored per node |

The **experiment runner** (`scripts/est_experiment`) runs a full Cartesian product of any combination of e, k, b values in parallel and writes:

- One `schedule_<e>_<k>_<b>.json` per run — compatible with the webapp upload.
- One `schedule_<e>_<k>_<b>.est_trace.jsonl` per run — per-round algorithm internals.
- A `comparison.csv` and `manifest.json` summarising all runs.

The **webapp** provides two complementary analytical pages:

- **EST Sweep** — quick side-by-side comparison of up to ~20 runs.
- **EST Intelligence** — deep multidimensional analysis: sensitivity heatmaps, Pareto fronts, FOM traces, statistical correlations.

---

## Running an experiment

### Prerequisites

```bash
# From the repository root
cargo build --release
```

Both the scheduler library and the experiment binary must build cleanly.  The QA pipeline (`./scripts/qa-pipeline.sh`) verifies this.

---

### Quick start — single run

```bash
cargo run --release --bin est_experiment -- \
  data/ctao_n.json \
  --output-dir out/my_run
```

This runs the EST scheduler with its default parameters (`e=1, k=1, b=1`) and writes two files under `out/my_run/schedules/`:

```
out/my_run/schedules/
  est_e1_k1_b1.json             # schedule (upload this to the webapp)
  est_e1_k1_b1.est_trace.jsonl  # trace (attach alongside the schedule)
```

---

### Sweep mode — scanning the parameter space

Pass comma-separated values or inclusive integer ranges for any axis.  All combinations are run in parallel using Rayon.

```bash
# Sweep e ∈ {1, 2}, k ∈ {1, 5, 10}, b ∈ {1..5}  →  2 × 3 × 5 = 30 runs
cargo run --release --bin est_experiment -- \
  data/ctao_n.json \
  --output-dir out/sweep_1 \
  --est-e-values 1,2 \
  --est-k-values 1,5,10 \
  --est-b-values 1-5
```

**Range syntax** — any flag accepts:

| Format | Example | Meaning |
|---|---|---|
| Single value | `3` | just 3 |
| Comma list | `1,3,5` | 1, 3, 5 |
| Inclusive range | `1-5` | 1, 2, 3, 4, 5 |
| Mixed | `1,3-5,8` | 1, 3, 4, 5, 8 |

**Controlling trace output:**

```bash
# Traces are ON by default.  Suppress them with:
--no-trace

# Explicitly enable (redundant but clear):
--trace
```

**Using a custom observing window** (instead of the one embedded in the JSON):

```bash
cargo run --release --bin est_experiment -- \
  data/ctao_n.json \
  --output-dir out/sweep_1 \
  <start_mjd> <end_mjd>   # e.g. 60000 60030
```

---

### Spec-file approach (recommended for large sweeps)

For reproducible experiments create a JSON spec file and check it in alongside the data:

```json
{
  "input_json": "../../data/ctao_n.json",
  "output_dir": "../../out/paper_sweep",
  "emit_trace": true,
  "sweep": {
    "endangered_thresholds": [1, 2, 3],
    "k_beams": [1, 5, 10, 20],
    "branching_factors": [1, 2, 5, 10]
  }
}
```

Run with:

```bash
cargo run --release --bin est_experiment -- --spec experiments/paper_sweep.json
```

CLI flags override spec-file values when both are present.

---

### Output artifacts

After a successful run the output directory contains:

```
out/my_sweep/
  manifest.json               # run metadata: input path, horizon, run count
  comparison.csv              # per-run metrics (scheduled_count, priority sums, etc.)
  schedules/
    est_e1_k1_b1.json
    est_e1_k1_b1.est_trace.jsonl
    est_e1_k5_b1.json
    est_e1_k5_b1.est_trace.jsonl
    ...
```

`comparison.csv` can be opened in any spreadsheet tool for a quick sanity check before importing into the webapp.

---

## Importing results into the webapp

### Starting the stack

```bash
./webapp/setup.sh
```

Open <http://localhost:3000> in your browser.

### Uploading a schedule + trace

1. Click **Import Schedule** on the landing page.
2. Under **"Choose JSON file"** select `est_e{e}_k{k}_b{b}.json`.
3. Under **"Optional: attach EST trace (.jsonl) for Intelligence panel"** select the matching `est_e{e}_k{k}_b{b}.est_trace.jsonl`.
4. Optionally set a human-readable name (leave blank to use the filename).
5. Click **Upload**.  A live log stream confirms processing.

> **Tip:** Upload all runs from a sweep before opening the Intelligence page.  React Query caches each fetch, so the panels render progressively as data arrives — no need to wait.

### Bulk upload script (optional)

For large sweeps, loop over the schedules directory:

```bash
BASE_URL="http://localhost:8080"

for json_file in out/my_sweep/schedules/*.json; do
  stem="${json_file%.json}"
  trace_file="${stem}.est_trace.jsonl"
  name=$(basename "$stem")

  payload=$(jq -n \
    --argjson sched "$(cat "$json_file")" \
    --arg name "$name" \
    --arg trace "$([ -f "$trace_file" ] && cat "$trace_file" || echo "")" \
    '{name: $name, schedule_json: $sched, populate_analytics: true, est_trace_jsonl: ($trace | if . == "" then null else . end)}')

  curl -s -X POST "$BASE_URL/v1/schedules" \
    -H "Content-Type: application/json" \
    -d "$payload" | jq .job_id
done
```

---

## EST Sweep tab

**URL:** `/algorithm/est/sweep` (legacy: `/est-sweep` redirects here)

The sweep tab is designed for a quick side-by-side comparison of runs that differ in **e, k, b** specifically.

### How to use

1. Open **Algorithm Analysis** from the global nav.
2. **Select schedules** in the left panel (tick checkboxes; use All / None buttons).
3. Switch to the **Sweep** tab.
4. **Choose an X axis** — which of e, k, b to plot on the horizontal axis.
5. **Choose a metric** — scheduling rate, scheduled count, or mean priority.
6. Inspect the **2D line chart** (grouped by the non-axis params) and **3D scatter** (axes = e/k/b, colour = metric).
7. The summary table at the bottom lists raw numbers for every selected run.

> All EST tabs share the same selection — switching between Overview / Sweep / Internals / etc. keeps the chosen schedules.

---

## EST Intelligence tabs

**URL:** `/algorithm/est/{tab}` where `tab` ∈ `overview | sensitivity | pareto | internals | statistics` (legacy: `/est-intelligence` redirects to `/algorithm/est/overview`)

The Intelligence tabs provide **five analytical lenses** over the selected runs, generic over any `algorithm_config` fields stored in the trace (not just the e/k/b trio).

### Selecting runs

Use the left panel identically to the Sweep tab — tick the runs you want to compare across all tabs.  Data loads progressively in the background.

---

### Overview tab

**What it shows:** High-level KPIs + a full run inventory.

| Section | Contents |
|---|---|
| KPI cards | Best scheduling rate, most scheduled, best mean priority across all selected runs |
| Run inventory table | Per-run: algorithm, full config snapshot, scheduled count, rate, mean priority |

Use this tab to orient yourself — confirm which runs loaded correctly and spot the top performer at a glance.

---

### Sensitivity tab

**What it shows:** How the chosen outcome metric varies with the configuration knobs.

Works automatically over **any numeric `algorithm_config` dimensions** that vary across the selected runs.

| Chart | Description |
|---|---|
| **Configuration cube** | 3D scatter: first three numeric dims on x/y/z axes, metric mapped to colour (Viridis scale).  Best for runs where ≥ 3 dimensions vary. |
| **2D scatter** | Metric vs the first numeric dimension.  Always shown. |
| **Parallel coordinates** | All numeric config dims + metric shown as parallel axes.  Drag axes to reorder; brush a range on any axis to filter. |

**Toolbar:**  toggle the outcome metric (rate / count / priority) at the top.

> The dimension set is computed automatically — if your sweep only varies e and k, the cube degenerates to a 2D scatter and the cube panel is hidden.

---

### Pareto tab

**What it shows:** Multi-objective optimality across scheduling rate, mean priority, and fragmentation.

- Axes: **scheduling rate** (maximise) · **mean priority** (maximise) · **fragmentation** (minimise, measured as idle fraction of operable time).
- Non-dominated runs (Pareto front) are highlighted in **green**.
- Dominated runs appear in **grey**.

Hover over points to identify the schedule name.  Use this to make a principled parameter choice when two objectives pull in opposite directions.

---

### Internals tab

**What it shows:** Algorithm behaviour round-by-round, sourced from the `est_trace` endpoint.

> Requires that schedules were uploaded with a `.est_trace.jsonl` file.  If no traces are available, the tab shows a prompt to re-run with `--trace`.

| Chart | Description |
|---|---|
| **Score trajectory** | One overlay per run: solid line = best beam score, dotted = median, dashed = worst.  Shows how the beam ensemble converges (or stagnates) over rounds. |
| **Beam-score heatmap** | Row = beam rank (best beam at top), column = round.  Colour = score.  Rendered for the first selected run with trace data. Reveals how beam diversity evolves. |
| **Wall time per round** | Line chart of milliseconds spent per search round per run.  Useful for profiling. |

---

### Statistics tab

**What it shows:** Concise statistical summary + Pearson correlations.

For each outcome metric (rate, count, priority):

| Column | Meaning |
|---|---|
| Mean, Std | Average and standard deviation across selected runs |
| Min, Max | Range |
| Best | Value and schedule name of the top-performing run |
| Correlation with config dims | Per-dimension Pearson r coefficient.  r ≈ +1: metric increases with this dim; r ≈ −1: decreases; r ≈ 0: no linear relationship |

---

## Configuration parameters reference

| Parameter | Range | Effect |
|---|---|---|
| `e` (`endangered_threshold`) | 0 … ∞ (typically 0–5) | Tasks with scheduling flexibility ≤ e are prioritised in every round.  Higher e = more aggressive time-critical protection. |
| `k` (`k_beams`) | 1 … 100 (typically 1–20) | Width of the beam.  k=1 is greedy best-first.  Larger k explores more alternatives but is O(k·b) per round. |
| `b` (`branching_factor`) | 1 … ∞ (typically 1–10) | Candidates considered per beam per round.  b=1 + k=1 degenerates to a greedy heuristic.  Higher b finds better solutions at higher compute cost. |

**Rule of thumb:**  Start with `e=1, k=5, b=3`.  Sweep k first (most impactful), then b, then e.

---

## Architecture notes

```
Rust experiment runner
│
├── cargo run --bin est_experiment
│     ├── Produces  schedule.json              ← scheduling-blocks + metadata
│     └── Produces  schedule.est_trace.jsonl   ← per-round algorithm trace
│
TSI backend (Axum + Diesel + Postgres)
│
├── POST /v1/schedules          ← uploads schedule.json + algorithm_trace_jsonl
│     │                           (legacy `est_trace_jsonl` accepted as alias)
│     ├── Parses JSONL → (algorithm, summary, iterations)
│     └── Stores in algorithm_traces table (schedule_id FK, cascade delete)
│
└── GET  /v1/schedules/{id}/algorithm_trace   ← AlgorithmTraceResponse { summary, iterations }
                                                 (legacy `/est_trace` aliased to same handler)
│
TSI frontend (React + Plotly)
│
└── /algorithm/est                  ← AlgorithmAnalysisPage (TSI core, agnostic)
      │                               selects active algorithm from selection
      ├── /sweep        — 2D line / 3D scatter / table (metric vs e/k/b)
      ├── /overview     — KPI cards, run table
      ├── /sensitivity  — 3D scatter, scatter, parallel coords
      ├── /pareto       — non-dominated front (rate × priority × fragmentation)
      ├── /internals    — FOM trace overlay, beam heatmap, wall-time
      └── /statistics   — mean/std/min/max + Pearson r per config dim

EST tabs (Overview/Sensitivity/Pareto/Internals/Statistics/Sweep) live in
`webapp/phd-extensions/pages/algorithms/est/` and are contributed via the
`algorithms` field of `TsiExtensions`.  Future scheduling algorithms register
their own tabs the same way — TSI core ships no algorithm-specific code.
```

The `algorithm_traces` table is keyed by `schedule_id` (foreign key to `schedules`, cascade delete).  Re-uploading a schedule upserts the trace row — so iterating on an experiment and re-uploading will update Intelligence panel data without leaving stale residuals.
