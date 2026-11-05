//! Process entry point: binds the fixed port, installs logging and waits for a signal.
//! Covered by the binary tests in `tests/binary.rs` rather than unit tests.

use std::{net::SocketAddr, sync::Arc};

use tokio::{net::TcpListener, signal};
use zylotek_infrastructure::persistence::{
    asks::repositories::SqliteAskRepository, build_db, woods::repositories::PostgresWoodRepository,
};

use crate::{auth::Auth, build_state, errors::ApiError, router::router};

async fn run(listen: SocketAddr, database_url: &str, auth: Auth) -> anyhow::Result<()> {
    let db = build_db(database_url).await?;
    if !auth.is_enabled() {
        log::warn!(
            "OAuth is not configured: the write routes answer 503. Create a key and a client \
             with `zylotek-cli auth keygen` and `zylotek-cli auth client create`"
        );
    }
    let state = build_state(
        Arc::new(PostgresWoodRepository::new(db.clone())),
        Arc::new(SqliteAskRepository::new(db)),
        auth,
    );

    let app = router(state);
    let listener = TcpListener::bind(listen).await?;
    tracing::info!("api listening on http://{}", listener.local_addr()?);

    print_banner();

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

/// Serves the API on port 8080 until a termination signal arrives.
pub async fn serve(listen: SocketAddr, database_url: &str, auth: Auth) -> Result<(), ApiError> {
    run(listen, database_url, auth)
        .await
        .map_err(ApiError::Internal)
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
