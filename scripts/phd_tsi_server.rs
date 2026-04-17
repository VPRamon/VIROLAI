use std::env;
use std::net::SocketAddr;
use std::sync::Arc;

use tracing::info;
use tracing_subscriber::EnvFilter;

use tsi_rust::db;
use tsi_rust::http::{AppState, create_router};

mod phd_tsi_adapter;

use phd_tsi_adapter::phd_schedule_import_adapter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(true)
        .with_thread_ids(true)
        .init();

    info!("Starting PhD-TSI server");

    db::init_repository_async()
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    let repository = Arc::clone(db::get_repository()?);
    info!("Repository initialized successfully");

    let import_adapter = phd_schedule_import_adapter();
    info!("Using import adapter: {}", import_adapter.name());
    let state = AppState::with_import_adapter(repository, import_adapter);

    let app = create_router(state);

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
