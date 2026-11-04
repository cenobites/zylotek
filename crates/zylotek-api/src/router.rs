use std::time::Duration;

use axum::{
    Router,
    routing::{get, post},
};
use tower::limit::ConcurrencyLimitLayer;
use tower_http::{
    compression::CompressionLayer,
    cors::{Any, CorsLayer},
    limit::RequestBodyLimitLayer,
    timeout::TimeoutLayer,
    trace::TraceLayer,
};

use crate::{
    asks,
    auth::{jwks_handler, metadata_handler, token_handler},
    state::AppState,
    woods,
};

pub fn router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .nest("/api/v1/woods", woods::routes::router())
        .nest("/api/v1/asks", asks::routes::router())
        .route("/health", get(health_check_handler))
        .route("/oauth/token", post(token_handler))
        .route("/.well-known/jwks.json", get(jwks_handler))
        .route(
            "/.well-known/oauth-authorization-server",
            get(metadata_handler),
        )
        .with_state(state)
        .fallback(not_found_handler)
        .layer(axum::middleware::from_fn(
            crate::middleware::logging_middleware,
        ))
        .layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new())
        .layer(TimeoutLayer::with_status_code(
            axum::http::StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(5),
        ))
        .layer(RequestBodyLimitLayer::new(1024 * 1024)) // 1MB
        .layer(ConcurrencyLimitLayer::new(100)) // 100 concurrent requests
        .layer(cors)
}

async fn health_check_handler() -> &'static str {
    "OK"
}

async fn not_found_handler() -> (axum::http::StatusCode, &'static str) {
    (axum::http::StatusCode::NOT_FOUND, "404 Not Found")
}

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{Method, Request, StatusCode, header},
    };
    use serde_json::json;

    use crate::test_support::{app, send, send_request};

    #[tokio::test]
    async fn health_check_answers_ok() {
        let (status, body) = send(&app().router, Method::GET, "/health", None).await;

        assert_eq!((status, body), (StatusCode::OK, json!("OK")));
    }

    #[tokio::test]
    async fn unknown_paths_fall_back_to_404() {
        let (status, body) = send(&app().router, Method::GET, "/nowhere", None).await;

        assert_eq!(
            (status, body),
            (StatusCode::NOT_FOUND, json!("404 Not Found"))
        );
    }

    #[tokio::test]
    async fn unsupported_methods_are_rejected() {
        let id = "00000000-0000-0000-0000-000000000000";

        let (status, _) = send(
            &app().router,
            Method::PATCH,
            &format!("/api/v1/woods/{id}"),
            None,
        )
        .await;

        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    }

    #[tokio::test]
    async fn cors_preflight_allows_any_origin() {
        let request = Request::builder()
            .method(Method::OPTIONS)
            .uri("/api/v1/woods")
            .header(header::ORIGIN, "https://example.com")
            .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
            .body(Body::empty())
            .unwrap();

        let response = tower::ServiceExt::oneshot(app().router, request)
            .await
            .unwrap();

        assert!(response.status().is_success());
        assert_eq!(response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
    }

    #[tokio::test]
    async fn bodies_over_one_megabyte_are_rejected() {
        let oversized = vec![b'x'; 1024 * 1024 + 1];
        let request = Request::builder()
            .method(Method::POST)
            .uri("/api/v1/woods")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::CONTENT_LENGTH, oversized.len())
            .body(Body::from(oversized))
            .unwrap();

        let (status, _) = send_request(&app().router, request).await;

        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    }
}
