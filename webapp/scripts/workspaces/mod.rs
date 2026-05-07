//! Workspaces backend domain.
//!
//! Lightweight, filesystem-backed sibling of the `experiments` domain.
//! Exposes `/v1/workspaces/...` endpoints that consume manifests
//! (produced by `phd manifest create`) without ever loading the full
//! schedule artifacts they reference. Heavy schedule loading remains
//! the responsibility of TSI.

pub mod errors;
pub mod routes;
pub mod store;

use std::path::PathBuf;
use std::sync::Arc;

#[allow(unused_imports)]
pub use errors::{WorkspaceError, WorkspaceResult};
pub use routes::{WorkspacesState, workspaces_router};
#[allow(unused_imports)]
pub use store::{ManifestEntry, ManifestSummary, WorkspaceRecord, WorkspaceStatus, WorkspaceStore};

/// Initialise the workspaces state from environment variables.
///
/// `PHD_WORKSPACES_DIR` (default: `./workspaces`) controls where the
/// store is rooted. The directory is created on demand.
pub fn state_from_env() -> WorkspaceResult<Arc<WorkspacesState>> {
    let root = std::env::var("PHD_WORKSPACES_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("./workspaces"));
    let store = Arc::new(WorkspaceStore::open(root)?);
    Ok(Arc::new(WorkspacesState { store }))
}

#[cfg(test)]
mod tests;
