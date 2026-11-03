//! The `homedit` binary: reads its settings from the environment and serves
//! the dashboard.
//!
//! | Variable           | Default         | Meaning                          |
//! |--------------------|-----------------|----------------------------------|
//! | `HOMEDIT_LISTEN`   | `0.0.0.0:3000`  | Address to listen on             |
//! | `HOMEDIT_DATA_DIR` | `/data`         | Database, uploads and other data |
//! | `HOMEDIT_IMPORT_DIR` | unset         | gethomepage config folder to import on first start |
//! | `HOMEDIT_DOCKER_HOST` | unset        | `unix:///var/run/docker.sock` or `tcp://host:port` for label discovery |
//! | `RUST_LOG`         | `info`          | Log filter                       |
//!
//! Login, secrets and scanner settings are read by their crates; see the
//! README for the full list.
//!
//! `homedit healthcheck` probes the running server and exits 0 or 1.

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::Context;
use tracing_subscriber::EnvFilter;

fn main() -> anyhow::Result<()> {
    // `homedit healthcheck` is the container HEALTHCHECK: the image has no
    // shell or curl, so the binary probes itself.
    if std::env::args().nth(1).as_deref() == Some("healthcheck") {
        let listen = std::env::var("HOMEDIT_LISTEN").unwrap_or_else(|_| "0.0.0.0:3000".into());
        let addr = homedit_ops::probe_addr_from_listen(&listen)
            .context("HOMEDIT_LISTEN is not a socket address")?;
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        return match rt.block_on(homedit_ops::probe(addr, std::time::Duration::from_secs(3))) {
            Ok(()) => Ok(()),
            Err(e) => {
                eprintln!("unhealthy: {e}");
                std::process::exit(1);
            }
        };
    }
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(serve())
}

async fn serve() -> anyhow::Result<()> {
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
    if let Ok(dir) = std::env::var("HOMEDIT_IMPORT_DIR")
        && let Err(e) = homedit::import::import_on_start(&state, std::path::Path::new(&dir)).await
    {
        tracing::error!(error = %e, dir, "cannot import gethomepage config");
    }
    homedit::status::spawn_checker(state.clone());
    homedit::discovery::spawn(state.clone());

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
