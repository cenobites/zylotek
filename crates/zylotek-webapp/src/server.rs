//! Process wiring: connect the clients, bind, serve until interrupted. No decisions are
//! made here — keep it that way, it is outside the unit coverage gate.

use std::{net::SocketAddr, sync::Arc};

use crate::{
    clients::{ApiClient, AssistantClient},
    pages::{AppState, router},
};

pub struct Config {
    pub api_url: String,
    pub assistant_url: String,
    pub assistant_token: Option<String>,
    pub listen: SocketAddr,
}

pub async fn run(config: Config) -> std::io::Result<()> {
    let state = AppState {
        api: Arc::new(ApiClient::new(&config.api_url)),
        assistant: Arc::new(AssistantClient::new(
            &config.assistant_url,
            config.assistant_token,
        )),
    };
    let listener = tokio::net::TcpListener::bind(config.listen).await?;
    tracing::info!("website listening on http://{}", listener.local_addr()?);

    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await
}

/// Ctrl-C, or SIGTERM as sent by systemd and container runtimes.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                term.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
