//! A real stack — router, use cases, SQLite repository and event bus — driven in-process.

#![allow(dead_code)]

use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;
use zylotek_api::{build_state, router::router, woods::schemas::WoodDetailsSchema};
use zylotek_domain::testing::full_details;
use zylotek_infrastructure::{
    persistence::woods::repositories::PostgresWoodRepository, testing::memory_db,
};

pub async fn app() -> Router {
    let db = memory_db().await.unwrap();

    router(build_state(Arc::new(PostgresWoodRepository::new(db))))
}

/// A `POST`/`PUT` body with every detail populated.
pub fn wood_body(name: &str, scientific_name: &str) -> Value {
    let mut body = serde_json::to_value(WoodDetailsSchema::from(&full_details())).unwrap();
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
    let mut request = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(json) => {
            request = request.header(header::CONTENT_TYPE, "application/json");
            Body::from(json.to_string())
        }
        None => Body::empty(),
    };

    send_raw(router, request.body(body).unwrap()).await
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
