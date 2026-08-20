# Experiments section

This directory contains the historical experiments UI integration for VIROLAI. The source path remains under `webapp/phd-extensions/` for compatibility, but user-facing project naming is VIROLAI.

The current primary workflow is the SQLite registry plus workspace publishing. See the repository README and `docs/architecture.md` before extending this older experiments surface.

## Routes

| Route | Page |
| --- | --- |
| `/experiments` | experiment list |
| `/experiments/new` | new experiment form |
| `/experiments/:slug/:runId/overview` | run overview |
| `/experiments/:slug/:runId/matrix` | matrix view |
| `/experiments/:slug/:runId/pareto` | Pareto view |
| `/experiments/:slug/:runId/per-dataset` | per-dataset view |
| `/experiments/:slug/:runId/per-algorithm` | per-algorithm view |
| `/experiments/:slug/:runId/cells/:cellId` | cell detail |

## Data access

`useExperimentRun(slug, runId)` owns the event stream for a run. `useBulkCellMetrics(slug, runId, cellIds)` batches per-cell metric retrieval to avoid one request per cell.

New views that require many cell metrics should use the bulk hook rather than calling `getCell` in a loop.

## Extension boundary

The pack should import only public TSI extension and component surfaces. Algorithm-specific views belong in the extension pack, not in TSI core.

## Known limitations

- The per-algorithm sensitivity view is incomplete.
- Large experiment lists are not virtualized.
- Cell detail links to the schedule timeline rather than loading large schedule artifacts eagerly.
