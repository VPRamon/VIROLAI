# TSI extension guide

TSI is intentionally algorithm-agnostic. Algorithm-specific integration code lives outside the core TSI backend and frontend and is connected through public extension contracts.

VIROLAI's in-tree integration is an example of that boundary. Historical source paths still use names such as `webapp/phd-extensions/` and `webapp/src/phd_tsi_adapter.rs`; new user-facing documentation refers to the integration as VIROLAI-TSI.

## Contract versioning

Both backend and frontend contracts expose `EXTENSION_CONTRACT_VERSION`. Integrators should verify the expected version at startup so incompatible builds fail early.

Backend:

```rust
use tsi_rust::http::{
    create_router_with_extensions, AlgorithmTraceValidator, AppState,
    BackendExtensions, EXTENSION_CONTRACT_VERSION,
};
```

Frontend extensions use the corresponding contract exported by `tsi-extensions-pack`.

## Backend extension surface

Integrators may provide additional axum routes and algorithm trace validators. Extensions do not mutate the core repository contract or intercept built-in handlers.

The VIROLAI backend uses `BackendExtensions` to attach workspace routes and EST trace validation while leaving the TSI core algorithm-agnostic.

## Frontend extension surface

The TSI frontend resolves an external extension pack through `VITE_TSI_EXTENSIONS_PATH`:

```bash
VITE_TSI_EXTENSIONS_PATH=../../my-pack npm run build
```

A pack exports the contract version and a `TsiExtensions` object containing optional routes, navigation items, and algorithm-specific panels.

The current in-tree pack remains under `webapp/phd-extensions/` for source compatibility. Its directory name is not part of the VIROLAI public CLI or scheduler model.

## Integration rules

- Keep validators inexpensive because they run on upload paths.
- Keep algorithm-specific code outside TSI core.
- Lazy-load large frontend panels.
- Use extension routes for additive integration rather than rewriting built-in TSI behavior.

Fork or modify TSI itself only when a change requires a different core data model, repository contract, response behavior, or schema migration.
