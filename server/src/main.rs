use std::net::SocketAddr;

use anyhow::Context;
use riposte_server::{
    app_with_web, spawn_idle_sweeper, AppState, ROOM_IDLE_TIMEOUT, SWEEP_INTERVAL,
};
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // RUST_LOG=debug cargo run -p riposte-server for more detail.
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    // Hosting platforms tell us which port to use through PORT.
    let port: u16 = match std::env::var("PORT") {
        Ok(p) => p.parse().context("PORT must be a number")?,
        Err(_) => 8080,
    };
    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    // WEB_DIR=web also serves the browser build from this same address.
    let web_dir = std::env::var_os("WEB_DIR").map(std::path::PathBuf::from);
    if let Some(dir) = &web_dir {
        if dir.join("index.html").exists() {
            info!(dir = %dir.display(), "serving the browser build");
        } else {
            tracing::warn!(dir = %dir.display(), "WEB_DIR has no index.html; run scripts/build-web.sh");
        }
    }

    let state = AppState::new();
    spawn_idle_sweeper(state.clone(), SWEEP_INTERVAL, ROOM_IDLE_TIMEOUT);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("could not listen on {addr}"))?;
    info!(%addr, "relay listening on /ws");
    axum::serve(listener, app_with_web(state, web_dir))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    info!("shut down");
    Ok(())
}

/// Resolves on Ctrl-C, or on SIGTERM, which is what `docker stop` sends.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut sig) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            sig.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
