# Experiments section

Top-level UI surface for the algorithm-evaluation environment. Mounted
under `/experiments` via `extensions.routes` (see
`webapp/phd-extensions/index.tsx`).

## Section structure

```
pages/experiments/
├── _ui.tsx                   shared design-system primitives
├── ExperimentsListPage.tsx   GET /v1/experiments
├── NewExperimentPage.tsx     POST /v1/experiments
├── ExperimentDetailPage.tsx  shell for one (slug, run_id), opens SSE
├── CellDetailPage.tsx        full breakdown of one cell
└── tabs/
    ├── OverviewTab.tsx       KPIs + top-3 + distributions
    ├── MatrixTab.tsx         dataset × algorithm/config heatmap
    ├── ParetoTab.tsx         X/Y scatter with Pareto front overlay
    ├── PerDatasetTab.tsx     ranked dataset table with weight controls
    └── PerAlgorithmTab.tsx   ⏳ stubbed (see "Known limitations" below)

lib/experiments/
├── types.ts                  TypeScript mirror of backend DTOs
├── api.ts                    axios client (baseURL = /api/v1/experiments)
├── useAsync.ts               tiny request/response hook
├── useExperimentRun.ts       run summary + SSE-driven live status
└── useBulkCellMetrics.ts     POST /cells/bulk wrapper (perf-critical)
```

The pack only imports from the v1 public surface (`@/extensions`,
`@/components`) — no `@/pages/...` imports. The EST tabs in
`pages/algorithms/est/` violate that and should be migrated when next
touched, but they're explicitly out of scope for this section.

## Routes

| Route                                                | Page                       |
| ---------------------------------------------------- | -------------------------- |
| `/experiments`                                       | `ExperimentsListPage`      |
| `/experiments/new`                                   | `NewExperimentPage`        |
| `/experiments/:slug/:runId/overview`                 | detail shell + Overview    |
| `/experiments/:slug/:runId/matrix`                   | detail shell + Matrix      |
| `/experiments/:slug/:runId/pareto`                   | detail shell + Pareto      |
| `/experiments/:slug/:runId/per-dataset`              | detail shell + PerDataset  |
| `/experiments/:slug/:runId/per-algorithm`            | detail shell + stub        |
| `/experiments/:slug/:runId/cells/:cellId`            | detail shell + Cell detail |

The detail shell registers a single top-level route (`:slug/:runId/*`)
and uses an inner `<Routes>` so tab navigation does not unmount the
shell — meaning the SSE stream is opened exactly once per run.

## Adding a new metric to the heatmap selector

1. Open `tabs/MatrixTab.tsx`.
2. Append a `{ value, label }` entry to the `METRICS` array.
3. Extend the `extractMetric()` switch to project the new field out of
   `ScheduleMetrics`.
4. (Optional) add the same option to `ParetoTab.tsx`'s `METRICS` array
   and `metricValue()` so it's pickable on the Pareto X/Y axes too.

Same recipe applies to derived metrics (e.g. priority/util ratio):
compute them in `extractMetric()` from the existing
`ScheduleMetrics` fields. No backend or DTO change needed.

## Bulk-fetch performance pattern

The legacy EST tabs fan out one request per schedule; that's the source
of the "very slow when uploading many schedules" pain point. The new
section avoids it via `useBulkCellMetrics(slug, runId, cellIds)`, which:

- de-duplicates and stably sorts the input so re-renders that change
  cell order do not refire the request,
- debounces input changes by 50 ms so rapid filter/pivot churn coalesces
  into a single backend call,
- issues a single `POST /cells/bulk` and returns a
  `Map<cell_id, BulkCellMetricsItem>` for O(1) lookups by the consumer,
- cancels stale responses (the `seq` ref pattern) so an earlier slow
  request doesn't overwrite a fresher one.

When adding a new tab that consumes per-cell metrics, **always** route
through this hook — never call `getCell` in a loop.

## SSE live-update strategy

`useExperimentRun(slug, runId)` opens an `EventSource` against
`GET /v1/experiments/:slug/:runId/events` once per (slug, runId) pair.
Each `state` event is parsed into a `StateEvent` and stored by
`cell_id` in a `Map` so re-emitted events for the same cell do not
double-count. Counters are recomputed in a `useMemo` over the map plus
the catalog totals from the initial `getRun` fetch.

On stream error, the source is closed and reconnected with capped
exponential backoff (1 s → 30 s); a small green/grey dot in the
detail-page header reflects the live connection state. The browser's
built-in `EventSource` reconnect handles transient network blips; our
explicit reconnect handles cases where the server terminates the stream
(e.g. nginx idle timeout).

The `RunCtx` provider in `ExperimentDetailPage.tsx` shares this state
across all tabs so each tab gets the same live counters without each
opening its own connection.

## Known limitations

- **Per-algorithm tab is stubbed.** A polished sensitivity surface
  (parallel coordinates over each algorithm's sweep axes) needs more
  domain modelling than fits in v1; the matrix already covers most
  algorithm-level investigation. The placeholder is intentional.
- **No virtualization on the experiments list.** `react-window` is
  available; if the list grows beyond ~200 cards, swap the
  `ExperimentsListPage` grid for a virtualized one. For now the simpler
  CSS grid avoids subtle layout bugs.
- **Schedule timeline on cell detail is a link, not an inline gantt.**
  The schedule artifact can be MB-scale; eagerly fetching it on every
  cell view would regress the slowness fix. Inline previews can be
  added once `/schedule` exposes a paged variant.
