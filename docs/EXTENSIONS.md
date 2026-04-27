# TSI Extension Guide

TSI (Telescope Scheduling Intelligence) is intentionally **algorithm
agnostic**: nothing in the `tsi-rust` backend or the TSI React frontend
should know about EST, HAP, or any other scheduling algorithm. Algorithm
specific code lives in *integrator packs* such as `webapp/phd-extensions/`
(frontend) and `webapp/scripts/phd_tsi_*.rs` (backend), wired together
through the public extension contracts described below.

The PhD/EST integrator is the worked example used throughout this
document.

## Contract versioning

Both contracts expose a numeric `EXTENSION_CONTRACT_VERSION` constant.
Integrators **must** assert against it at startup so a mismatched build
fails loudly:

* Backend: `tsi_rust::http::EXTENSION_CONTRACT_VERSION` (currently `1`).
* Frontend: `EXTENSION_CONTRACT_VERSION` exported from
  `tsi-extensions-pack` (currently `1`).

Bumping either constant signals a breaking change to the surface and
every integrator must review the diff before upgrading.

## Backend extension surface

The backend exposes its contract from `tsi_rust::http::extensions`:

```rust
use tsi_rust::http::{
    create_router_with_extensions, AlgorithmTraceValidator, AppState,
    BackendExtensions, EXTENSION_CONTRACT_VERSION,
};
```

Integrators may contribute:

1. **Extra axum routes** — mounted under `/v1` alongside the built-in
   handlers. Path collisions are the integrator's responsibility; pick a
   prefix you own (e.g. `/v1/est/...`).
2. **Algorithm trace validators** — a `Send + Sync + 'static` struct
   implementing `AlgorithmTraceValidator`. The trait is keyed by an
   algorithm identifier (the `algorithm` field embedded in trace
   summaries). When a schedule is uploaded with an
   `algorithm_trace_jsonl` payload the registered validator for that
   algorithm receives the parsed summary and may reject the upload by
   returning `Err(...)`. Uploads tagged with an algorithm that has no
   registered validator are accepted unchanged.

Extensions **may not** mutate the core repository contract or intercept
built-in handlers. If you need that level of integration, fork TSI.

The PhD integrator (see `webapp/scripts/phd_tsi_server.rs`) demonstrates
the full setup: build a `BackendExtensions` with
`BackendExtensions::builder().with_trace_validator(EstTraceValidator).build()`
and pass it to `create_router_with_extensions(state, extensions)`.

## Frontend extension surface

The frontend contract is re-exported from
`webapp/TSI/frontend/src/extensions.ts`. Vite resolves
`tsi-extensions-pack` through the `VITE_TSI_EXTENSIONS_PATH` env var so
external packs do not need to edit `vite.config.ts`:

```bash
VITE_TSI_EXTENSIONS_PATH=../../my-pack npm run build
```

The default value (`../../phd-extensions`) reproduces the behaviour the
in-tree PhD/EST extension expects.

A pack must export the `EXTENSION_CONTRACT_VERSION` constant matching
the value baked into the TSI build it targets, plus a default-export
object containing:

* `routes` — extra React Router routes injected into the app shell.
* `navItems` — sidebar navigation entries for the routes above.
* `algorithms` — algorithm-specific tab descriptors used by the
  schedule-analysis page (lazy-loaded via `React.lazy`).

The PhD pack at `webapp/phd-extensions/` ships the EST
algorithm-comparator, including the EST-specific trace iteration types,
SchedDropZone integrations, and run-matrix UI. None of that code lives
inside `tsi-rust` or the TSI frontend tree.

## Performance notes for integrators

* Keep validators cheap — they run on the upload hot path. The PhD/EST
  validator only checks for required summary keys.
* Backend integrators should call `tsi_rust::configure_rayon_thread_pool()`
  in `main()` to cap rayon's worker pool to `num_cpus - 1` and avoid
  oversubscribing alongside the tokio runtime and Diesel connection
  pool.
* Frontend packs should lazy-load heavy panels (Plotly, large data
  grids) so the core TSI bundle stays small.

## When NOT to use extensions

Extensions are for *adding* algorithm-specific surface area. They are
**not** the right tool when you want to:

* change the schedule data model or repository contract,
* intercept or rewrite built-in TSI responses,
* ship core schema migrations.

For any of the above you should fork TSI and submit a PR upstream. The
extension contract is deliberately narrow so TSI can keep evolving
without breaking integrator packs.
