use std::sync::Arc;

use anyhow::{Context, Result};
use singularis_server::{AppConfig, InMemoryEventStore, router};
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("singularis_server=info,tower_http=info")),
        )
        .init();

    let config = AppConfig::from_env()?;
    let listener = TcpListener::bind(config.bind_addr)
        .await
        .with_context(|| format!("could not bind {}", config.bind_addr))?;

    info!(
        address = %config.bind_addr,
        max_ttl_seconds = config.max_server_ttl.as_seconds(),
        storage = "volatile-prototype",
        "Singularis server listening"
    );

    axum::serve(
        listener,
        router(config, Arc::new(InMemoryEventStore::default())),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .context("server stopped unexpectedly")
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::error!(%error, "could not install shutdown signal handler");
    }
}
