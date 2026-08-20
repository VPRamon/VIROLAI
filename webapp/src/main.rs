use std::env;
use std::net::SocketAddr;

use tracing::info;
use tracing_subscriber::EnvFilter;

use tsi_rust::db::RepositoryFactory;
use tsi_rust::http::{
    AlgorithmTraceValidator, AppState, BackendExtensions, EXTENSION_CONTRACT_VERSION,
    create_router_with_extensions,
};

mod phd_tsi_adapter;
mod workspaces;

use phd_tsi_adapter::phd_schedule_import_adapter;

/// Validator for EST algorithm traces.
///
/// Lives in the VIROLAI integrator (not in TSI) so the core remains
/// algorithm-agnostic. Rejects trace summaries that do not declare the
/// EST-specific knobs the analytics UI relies on.
struct EstTraceValidator;

impl AlgorithmTraceValidator for EstTraceValidator {
    fn algorithm(&self) -> &'static str {
        "est"
    }

    fn validate_summary(&self, summary: &serde_json::Value) -> Result<(), String> {
        for required in ["k_beams", "branching_factor"] {
            if summary.get(required).is_none() {
                return Err(format!("missing required EST summary field `{required}`"));
            }
        }
        Ok(())
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(true)
        .with_thread_ids(true)
        .init();

    info!(
        "Starting VIROLAI-TSI server (extension contract v{})",
        EXTENSION_CONTRACT_VERSION
    );

    tsi_rust::configure_rayon_thread_pool();

    let repository = RepositoryFactory::from_env()
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    info!("Repository initialized successfully");

    let import_adapter = phd_schedule_import_adapter();
    info!("Using import adapter: {}", import_adapter.name());
    let state = AppState::with_import_adapter(repository, import_adapter);

    let workspaces_state = workspaces::state_from_env()
        .map_err(|e| anyhow::anyhow!("failed to init workspaces domain: {e}"))?;
    info!(
        "Workspaces backend ready (root={})",
        workspaces_state.store.root().display()
    );
    let workspaces_router = workspaces::workspaces_router(workspaces_state);

    let extensions = BackendExtensions::builder()
        .with_routes(workspaces_router)
        .with_trace_validator(EstTraceValidator)
        .build();

    let app = create_router_with_extensions(state, extensions);

    let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port: u16 = env::var("PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(8080);
    let addr: SocketAddr = format!("{}:{}", host, port).parse()?;

    info!("Server listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
