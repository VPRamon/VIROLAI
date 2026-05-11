# Architecture — experiments to webapp pipeline

This document is the canonical reference for how experiment artefacts
flow from a Rust binary on a researcher's laptop to the webapp UI. If
another doc disagrees with this one, this one wins.

## 1. The pipeline at a glance

```
                researcher CLI                                      webapp
    ┌──────────────────────────────────┐         ┌──────────────────────────────────┐
    │                                  │         │                                  │
    │  phd sweep  ──►  out/sweep/      │  HTTP   │  POST /v1/workspaces/…/manifests  │
    │     │            ├ <cell>.json   │ ──────► │  POST /v1/workspaces/…/schedules  │
    │     │            └ <cell>.manifest.json     │  GET  /v1/workspaces/…/comparison │
    │     ▼                                       │                                  │
    │  phd publish  ──────────────────────────►   │  → /workspaces/<id> UI           │
    │                                             │     • Manifests table (paged)    │
    │                                             │     • Compare (manifests only)    │
    │                                             │     • Drill-down → schedule view  │
    └──────────────────────────────────┘         └──────────────────────────────────┘
```

There is exactly one canonical path:

1. Run a sweep with `phd sweep --spec … --out … --manifest`.
2. Publish the output directory with `phd publish --workspace … --dir …
   --include-schedules`.
3. Open the workspace in the webapp.

Everything else (`experiments run`, `phd manifest create`, the
`upload_results.sh` wrapper) is a supporting tool, not a separate
pathway.

## 2. The artefact contract

| Artefact | Schema | Contents | Role |
|---|---|---|---|
| **Schedule** | `schemas/schedule/...` | Full per-task assignment + embedded `schedule_metadata` + embedded `schedule_metrics`. Self-contained. | Source of truth. Reanalysable. Required for drill-down. |
| **Metrics** | `schemas/scheduling_statistics/schedule_metrics.schema.json` | Pure numeric/statistical block (completion ratio, priority histograms, fragmentation, utilisation, per-resource, ranking). | A **field inside** every schedule and every manifest. Never published as a standalone artefact. |
| **Manifest** | `schemas/scheduling_statistics/manifest.schema.json` | Versioned exchange envelope. Embeds a `metrics` block; references the full schedule by `{uri, sha256, size_bytes, media_type}`; carries `producer`, `dataset`, `algorithm`, `run`, `horizon`, `provenance`, `links`, `validation`, `extensions`. | Unit of comparison and indexing in the webapp. |
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
| `DELETE` | `/v1/workspaces/{id}/manifests/{mid}?delete_artifact=1` | Remove manifest, GC orphaned schedule. |

All POST endpoints accept an `idempotency_key`. Manifests use
`manifest_id` by default; schedules use the SHA-256 of the canonical
JSON bytes.

## 5. CLI surface

| Command | Purpose |
|---|---|
| `phd sweep --spec … --out … --manifest` | Run an experiment matrix; emit flat `<cell>.json` + `<cell>.manifest.json` pairs. |
| `phd publish --workspace … --dir … --include-schedules` | Walk a directory, classify each `.json`, batch-upload manifests and (optionally) schedules. |
| `phd manifest create --schedule …` / `--run …` | Build a manifest post-hoc from a single schedule or a `run-<ts>/` directory. |
| `phd manifest validate` | Validate a manifest against the schema. |

`experiments run` and `experiments matrix` remain available for
advanced direct use; they share the same artefact contract.

## 6. UI surface (`/workspaces`)

The detail page treats the manifest as the unit of work. The upload
zone accepts mixed batches (manifests and self-contained schedules) by
drag-and-drop, including whole folders. Each file is classified by
content (presence of `manifest_schema_version` vs `schedule_metadata`)
and routed to the matching batch endpoint with progress reported per
file. Comparison and Pareto views read only manifests; opening a row
issues a single drill-down request to fetch the full schedule.

## 7. Migration notes (one-shot)

The previous architecture had:

- Separate `metrics/` and `traces/` directories per run — gone.
- An `est_experiment` binary and `est_trace.jsonl` outputs — gone.
- A `Cell.emit_trace` flag in the runner — gone.
- A `scripts/upload_results.sh` that POSTed to non-existent endpoints
  with broken payloads — replaced by a thin wrapper over `phd publish`.
- Two separate UI inputs for "manifest" and "schedule" uploads — being
  replaced by a single mixed-classification drop zone.

If you find a doc still referring to those, treat the doc as stale and
prefer this file.
