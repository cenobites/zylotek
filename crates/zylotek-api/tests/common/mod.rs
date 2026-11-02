//! A real stack — router, use cases, SQLite repository and event bus — driven in-process.

#![allow(dead_code)]

use std::{net::SocketAddr, sync::Arc};

use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle};
use tower::ServiceExt;
use zylotek_api::{
    auth::{Auth, testing},
    build_state,
    router::router,
    woods::mappers::Convert,
};
use zylotek_contracts::woods::WoodDetailsSchema;
use zylotek_domain::testing::full_details;
use zylotek_infrastructure::{
    persistence::woods::repositories::PostgresWoodRepository, testing::memory_db,
};

pub async fn app() -> Router {
    app_with(testing::auth()).await
}

pub async fn app_with(auth: Auth) -> Router {
    let db = memory_db().await.unwrap();

    router(build_state(Arc::new(PostgresWoodRepository::new(db)), auth))
}

/// The router served on an ephemeral local port, until `shutdown` is signalled.
pub struct Server {
    pub address: SocketAddr,
    pub shutdown: oneshot::Sender<()>,
    pub task: JoinHandle<std::io::Result<()>>,
}

pub async fn serve() -> Server {
    serve_with(testing::auth()).await
}

pub async fn serve_with(auth: Auth) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (shutdown, signal) = oneshot::channel::<()>();
    let router = app_with(auth).await;
    let task = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async {
                let _ = signal.await;
            })
            .await
    });

    Server {
        address,
        shutdown,
        task,
    }
}

/// A `POST`/`PUT` body with every detail populated.
pub fn wood_body(name: &str, scientific_name: &str) -> Value {
    let details: WoodDetailsSchema = (&full_details()).convert();
    let mut body = serde_json::to_value(details).unwrap();
    body["name"] = json!(name);
    body["scientific_name"] = json!(scientific_name);
    body
}

pub async fn send(
    router: &Router,
    method: Method,
    uri: &str,
    body: Option<&Value>,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, bearer(testing::write_token()));
    let body = match body {
        Some(json) => {
            request = request.header(header::CONTENT_TYPE, "application/json");
            Body::from(json.to_string())
        }
        None => Body::empty(),
    };

    send_raw(router, request.body(body).unwrap()).await
}

pub fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}

pub async fn send_raw(router: &Router, request: Request<Body>) -> (StatusCode, Value) {
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&bytes).into_owned();

    (
        status,
        serde_json::from_str(&text).unwrap_or(Value::String(text)),
    )
}
