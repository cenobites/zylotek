//! Process wiring: connect the pieces, sync once, serve until interrupted. No decisions are
//! made here — keep it that way, it is outside the unit coverage gate.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use crate::{
    assistant::Assistant,
    catalogue::{self, ApiClient, Catalogue},
    http,
    llm::OpenAiCompatible,
};

pub struct Config {
    pub api_url: String,
    pub model_url: String,
    pub model_name: String,
    pub model_api_key: Option<String>,
    pub token: Option<String>,
    pub listen: SocketAddr,
    pub refresh: Duration,
    pub top: usize,
}

pub async fn run(config: Config) -> std::io::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .json()
        .init();

    if config.token.is_none() {
        tracing::warn!("no --token set: anyone who can reach this port can use the model");
    }
    let api = Arc::new(ApiClient::new(&config.api_url));
    let catalogue = Arc::new(Catalogue::new());
    match catalogue::sync(&api, &catalogue).await {
        Ok(count) => tracing::info!(count, "catalogue loaded from {}", config.api_url),
        Err(error) => {
            tracing::warn!(%error, "first sync failed; retrying every {:?}", config.refresh)
        }
    }
    tokio::spawn(catalogue::keep_in_sync(
        api,
        catalogue.clone(),
        config.refresh,
    ));

    let model = OpenAiCompatible::new(&config.model_url, &config.model_name, config.model_api_key);
    let assistant = Arc::new(Assistant::new(Arc::new(model), catalogue, config.top));
    let listener = tokio::net::TcpListener::bind(config.listen).await?;
    tracing::info!("assistant listening on {}", listener.local_addr()?);

    axum::serve(listener, http::router(assistant, config.token))
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
