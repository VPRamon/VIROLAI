# Architecture — experiments to webapp pipeline

> **Status:** historical notes. The current scheduler workflow is documented in
> the repository [README](../README.md), [docs/algorithms/README.md](algorithms/README.md),
> and [docs/algorithms/sweep-configuration.md](algorithms/sweep-configuration.md).
> This file is no longer the canonical reference.

## 1. The pipeline at a glance

```
                researcher CLI                                      webapp
    ┌──────────────────────────────────┐         ┌──────────────────────────────────┐
    │                                  │         │                                  │
    │  phd sweep  ──►  runs.sqlite     │         │  POST /v1/workspaces/…/manifests │
    │     │                            │         │  POST /v1/workspaces/…/schedules │
    │     └──► lab registry export ────┼─ HTTP ─►│  GET  /v1/workspaces/…/comparison│
    │                                  │         │                                  │
    │  phd publish  ──────────────────►│         │  → /workspace/<id> UI            │
    │                                  │         │                                  │
    └──────────────────────────────────┘         └──────────────────────────────────┘
```

Current canonical path:

1. Run a sweep with `phd sweep --spec … --run-db …`.
2. Materialize selected schedules with `lab registry export`.
3. Publish the exported directory with `phd publish --workspace … --dir …`.
4. Open the workspace in the webapp.

## 2. The artefact contract

| Artefact | Schema | Contents | Role |
|---|---|---|---|
| **Schedule export** | `schemas/schedule/...` | Full per-task assignment + embedded `schedule_metadata` + embedded `schedule_metrics`. Self-contained. | Source of truth after export. Required for drill-down. |
| **Metrics** | `schemas/scheduling_statistics/schedule_metrics.schema.json` | Pure numeric/statistical block (completion ratio, priority histograms, fragmentation, utilisation, per-resource, ranking). | A **field inside** every schedule and every manifest. Never published as a standalone artefact. |
| **Manifest** | `schemas/scheduling_statistics/manifest.schema.json` | Versioned exchange envelope. Embeds a `metrics` block; references the exported full schedule by `{uri, sha256, size_bytes, media_type}`; carries `producer`, `dataset`, `algorithm`, `run`, `horizon`, `provenance`, `links`, `validation`, `extensions`. | Unit of comparison and indexing in the webapp. |
| **Trace** | (deprecated) | — | Removed from the canonical pipeline. If reintroduced, would be a workspace-stored artifact referenced by a manifest. |

### 2.1 Manifest vs metrics — the exact difference

- `metrics` is a **block of numbers**. It has no notion of identity, no
  dataset reference, no producer, no SHA-256, no URI. Two completely
  different schedules can share a metrics block by coincidence.

- `manifest` is an **envelope**. It carries a metrics block, *plus*
  everything needed to identify the run, reproduce it, and locate the
  heavy artefact it summarises.

Equivalently:

> Every manifest contains a metrics block.
> No metrics block on its own is a manifest.

This is why the webapp's comparison endpoint reads only manifests:
manifests are small (≤ a few KiB), self-describing, and sufficient for
ranking, Pareto plots, dataset/algorithm filters, etc. Schedules are
loaded only when the user drills down into a single result.

## 3. Storage layout (workspaces backend)

```
workspaces/<workspace_id>/
    workspace.json
    index.json                          # entries[] (manifests) + schedules[] (artifacts)
    manifests/<manifest_id>.json
    schedules/<sha256>.json             # content-addressed; deduplicated
```

Invariants enforced by `WorkspaceStore`:

- `index.entries[*].idempotency_key` is unique. Re-posting the same
  manifest is a no-op and returns the existing record.
- `index.schedules[*].sha256` is unique. Posting the same schedule
  twice (even from different manifests) writes one file on disk.
- Each `schedules[*].manifest_ids` cross-references every manifest
  whose `artifacts.schedule.uri == "ws:///schedules/<sha>.json"`.
- `DELETE /…/manifests/<mid>?delete_artifact=1` prunes the manifest
  from any schedule's `manifest_ids`; if the slot becomes empty, the
  schedule file is deleted and the registry entry is dropped (GC).

## 4. HTTP surface

| Method | Path | Purpose |
|---|---|---|
| `POST` | `/v1/workspaces` | Create a workspace. |
| `POST` | `/v1/workspaces/{id}/manifests` | Single manifest ingest. |
| `POST` | `/v1/workspaces/{id}/manifests/batch` | Batch manifest ingest. |
| `POST` | `/v1/workspaces/{id}/schedules` | Single self-contained schedule ingest. Server derives the manifest and persists the schedule. |
| `POST` | `/v1/workspaces/{id}/schedules/batch` | Batch self-contained schedule ingest. |
| `GET`  | `/v1/workspaces/{id}/manifests` | List manifests. |
| `GET`  | `/v1/workspaces/{id}/manifests/{mid}` | Get one manifest. |
| `GET`  | `/v1/workspaces/{id}/manifests/{mid}/schedule` | Drill-down: full schedule, 404 if not persisted. |
| `GET`  | `/v1/workspaces/{id}/comparison` | Lightweight comparison summary (manifests only). |
| `GET`  | `/v1/workspaces/{id}/cohorts` | List cohorts (manifests grouped by `(dataset, observatory, period, block_pool_hash)`). |
| `GET`  | `/v1/workspaces/{id}/cohorts/{cohort_key}/blocks` | Per-block breakdown across the schedules persisted in a cohort. |
| `DELETE` | `/v1/workspaces/{id}/manifests/{mid}?delete_artifact=1` | Remove manifest, GC orphaned schedule. |

All POST endpoints accept an `idempotency_key`. Manifests use
`manifest_id` by default; schedules use the SHA-256 of the canonical
JSON bytes.

## 5. CLI surface

| Command | Purpose |
|---|---|
| `phd sweep --spec … --run-db …` | Run an experiment matrix into the SQLite registry. |
| `lab registry export --out-dir …` | Materialize schedule JSON files from selected registry rows. |
| `phd publish --workspace … --dir …` | Walk an export directory and batch-upload schedules/manifests. |

`lab run` remains available for advanced direct use; it shares the same
registry contract as `phd sweep`.

## 6. UI surface (`/workspace`)

`/workspace` is the **only** workspace surface in the app. It is reached
from the Landing page (no navbar entry). The list view shows existing
workspaces and lets the user create new ones; the detail view
(`/workspace/:id`) groups uploaded results by **cohort** — a
`(dataset, observatory, period, block_pool_hash)` tuple derived from
`extensions.workspace_context` on each manifest.

Each cohort renders:

- a **summary table** built exclusively from manifest metrics
  (no schedules required);
- a **per-block table** (only when at least one schedule was persisted
  in the cohort) with one column per schedule and a `differences only`
  filter; priority bins are user-configurable, default `N=5`,
  persisted in `localStorage`.

The upload zone accepts mixed batches (manifests and self-contained
schedules) by drag-and-drop, including whole folders. Each file is
classified by content (presence of `manifest_schema_version` vs
`schedule_metadata`) and routed to the matching batch endpoint with
per-file status. **Standalone `schedule_metrics.json` files are
rejected**; embed metrics inside a manifest or upload the full schedule.

Legacy `environments` / `EnvironmentCompare` / `AlgorithmAnalysis`
surfaces and the previous `/workspaces` extension are gone; they have
no redirects.

## 7. Migration notes (one-shot)

The previous architecture had:

- Separate `metrics/` and `traces/` directories per run — gone.
- An `est_experiment` binary and `est_trace.jsonl` outputs — gone.
- A `Cell.emit_trace` flag in the runner — gone.
- A `scripts/upload_results.sh` that POSTed to non-existent endpoints
  with broken payloads — replaced by a thin wrapper over `phd publish`.
- Two separate UI inputs for "manifest" and "schedule" uploads —
  replaced by a single mixed-classification drop zone in `/workspace`.
- An `environments` REST surface (`/v1/environments/...`) and the
  matching `/environments/*` UI — removed entirely; the `workspace`
  domain is now the single home for comparable runs.
- A standalone `schedule_metrics.json` artefact accepted as input —
  no longer accepted; the metrics block lives inside the manifest.

If you find a doc still referring to those, treat the doc as stale and
prefer this file.
