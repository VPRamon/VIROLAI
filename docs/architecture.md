# Architecture: experiments to webapp

VIROLAI separates scheduling, experiment execution, result storage, and presentation. CTAO-specific conversion is an input adapter and does not participate in the scheduler core.

## Pipeline

1. A scheduling problem is provided directly in the common JSON model or produced by a dataset adapter.
2. `virolai run` executes one problem, or `virolai sweep` evaluates an experiment matrix.
3. The `lab` registry stores run identity, configuration, metrics, metadata, and deduplicated schedule bodies.
4. `lab registry export` materializes selected schedules as self-contained JSON files.
5. `virolai publish` uploads exported schedules to a webapp workspace.
6. The webapp groups and compares runs and provides schedule drill-down.

## Components

| Component | Responsibility |
| --- | --- |
| `schedulers` | Generic scheduling model and algorithms |
| `lab` | Experiment matrix execution and SQLite registry |
| `virolai` | User-facing workflow CLI |
| dataset adapters | Translation from external formats to `scheduling_problem.json` |
| `webapp` | Result storage, comparison, and TSI integration |

## Scheduling boundary

The scheduler accepts the common scheduling problem model. Domain-specific adapters must translate external data before scheduling. This keeps resource allocation, constraints, dependencies, objective metrics, and algorithm configuration independent from CTAO or any other dataset producer.

The current CTAO adapter is implemented in `lab/src/bin/lab_ctao_adapter/`. It is one integration of this boundary, not part of the scheduler API.

## Registry model

The registry uses two main tables:

- `runs` stores run identity, configuration, metrics, metadata, source information, and a schedule hash.
- `schedules` stores one invariant schedule body per semantic schedule hash.

Run-specific metadata and metrics remain on the run row. Two runs can therefore share a schedule body without sharing their configuration or evaluation data.

## Export

`lab registry export` reconstructs a complete schedule from the invariant body and the selected run metadata:

```bash
lab registry export \
  --run <KEY|PREFIX> \
  --out schedule.json \
  --run-db .lab/runs.sqlite
```

For a set of runs:

```bash
lab registry export \
  --out-dir out/results \
  --run-db .lab/runs.sqlite
```

## Webapp contract

The webapp accepts self-contained schedules and manifests under `/v1/workspaces`. A workspace is the unit used to group comparable runs.

The main routes are:

| Method | Path | Purpose |
| --- | --- | --- |
| `POST` | `/v1/workspaces` | Create a workspace |
| `POST` | `/v1/workspaces/{id}/schedules` | Store a schedule and derive its manifest |
| `POST` | `/v1/workspaces/{id}/schedules/batch` | Store a batch of schedules |
| `POST` | `/v1/workspaces/{id}/manifests` | Store a manifest |
| `GET` | `/v1/workspaces/{id}/comparison` | Return comparison data |
| `GET` | `/v1/workspaces/{id}/cohorts` | Group comparable runs |

`virolai publish` is the supported CLI path for uploading exported schedules.

## Configuration compatibility

New public environment variables use the `VIROLAI_` prefix. Compatibility fallbacks are retained for existing `PHD_` variables where they were already part of local workflows.

Examples:

- `VIROLAI_WORKSPACES_DIR`, with `PHD_WORKSPACES_DIR` accepted as a fallback
- `VIROLAI_WEBAPP_URL`, with `PHD_WEBAPP_URL` accepted as a fallback
- `VIROLAI_WEBAPP_TOKEN`, with `PHD_WEBAPP_TOKEN` accepted as a fallback
