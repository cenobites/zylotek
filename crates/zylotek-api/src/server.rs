//! Process entry point: binds the fixed port, installs logging and waits for a signal.
//! Covered by the CLI's binary tests rather than unit tests.

use std::sync::Arc;

use tokio::{net::TcpListener, signal};
use zylotek_infrastructure::persistence::{build_db, woods::repositories::PostgresWoodRepository};

use crate::{build_state, errors::ApiError, router::router};

async fn run(database_url: &str) -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .json()
        .init();

    let db = build_db(database_url).await?;
    let state = build_state(Arc::new(PostgresWoodRepository::new(db)));

    let app = router(state);
    let listener = TcpListener::bind("0.0.0.0:8080").await?;

    print_banner();

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

/// Serves the API on port 8080 until a termination signal arrives.
pub fn cli(database_url: &str) -> Result<(), ApiError> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| ApiError::Internal(anyhow::Error::new(e)))?;

    rt.block_on(run(database_url)).map_err(ApiError::Internal)
}

fn print_banner() {
    let banner = r#"
            _       _       _                  _
           | |     | |     | |                (_)
  _____   _| | ___ | |_ ___| | __   __ _ _ __  _
 |_  / | | | |/ _ \| __/ _ \ |/ /  / _` | '_ \| |
  / /| |_| | | (_) | ||  __/   <  | (_| | |_) | |
 /___|\__, |_|\___/ \__\___|_|\_\  \__,_| .__/|_|
       __/ |                            | |
      |___/                             |_|
    "#;
    println!("{}", banner);
    println!("Version: {}", env!("CARGO_PKG_VERSION"));
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
