//! The `homedit` binary: reads its settings from the environment and serves
//! the dashboard.
//!
//! | Variable           | Default         | Meaning                          |
//! |--------------------|-----------------|----------------------------------|
//! | `HOMEDIT_LISTEN`   | `0.0.0.0:3000`  | Address to listen on             |
//! | `HOMEDIT_DATA_DIR` | `/data`         | Database, uploads and other data |
//! | `RUST_LOG`         | `info`          | Log filter                       |

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::Context;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let listen: SocketAddr = std::env::var("HOMEDIT_LISTEN")
        .unwrap_or_else(|_| "0.0.0.0:3000".into())
        .parse()
        .context("HOMEDIT_LISTEN is not a socket address")?;
    let data_dir =
        PathBuf::from(std::env::var("HOMEDIT_DATA_DIR").unwrap_or_else(|_| "/data".into()));

    let state = homedit::AppState::open(&data_dir).await?;
    homedit::status::spawn_checker(state.clone());

    let listener = tokio::net::TcpListener::bind(listen)
        .await
        .with_context(|| format!("cannot listen on {listen}"))?;
    tracing::info!(%listen, data_dir = %data_dir.display(), "homedit started");
    axum::serve(
        listener,
        homedit::app(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown())
    .await?;
    Ok(())
}

/// Resolves on Ctrl-C or SIGTERM (what `docker stop` sends).
async fn shutdown() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = term => {} }
    tracing::info!("shutting down");
}
