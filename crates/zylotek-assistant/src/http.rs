//! The assistant's HTTP face: `GET /health` is open, `POST /v1/ask` needs the shared bearer
//! token when one is configured. The assistant usually runs on another machine, so unlike the
//! read-only wood API it spends the GPU and is not left open by accident.

use std::{sync::Arc, time::Instant};

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, FromRequestParts, State, rejection::JsonRejection},
    http::{HeaderValue, StatusCode, header, request::Parts},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use zylotek_contracts::{
    ErrorResponse,
    assistant::{AskRequest, AskResponse, AssistantHealth},
};

use crate::{assistant::Assistant, error::AskError};

/// A question is at most 1000 characters; this leaves room for JSON and multi-byte text.
const MAX_BODY_BYTES: usize = 8 * 1024;

#[derive(Clone)]
pub struct AppState {
    assistant: Arc<Assistant>,
    token: Option<Arc<str>>,
}

pub fn router(assistant: Arc<Assistant>, token: Option<String>) -> Router {
    let state = AppState {
        assistant,
        token: token.map(Arc::from),
    };

    Router::new()
        .route("/health", get(health))
        .route("/v1/ask", post(ask))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(state)
}

async fn health(State(state): State<AppState>) -> Json<AssistantHealth> {
    let snapshot = state.assistant.catalogue().snapshot();

    Json(AssistantHealth {
        status: if snapshot.is_some() { "ok" } else { "syncing" }.to_owned(),
        woods: snapshot.as_ref().map_or(0, |s| s.ranker.len()),
        synced_seconds_ago: snapshot.map(|s| Instant::now().duration_since(s.fetched_at).as_secs()),
    })
}

async fn ask(
    _authorized: Authorized,
    State(state): State<AppState>,
    body: Result<Json<AskRequest>, JsonRejection>,
) -> Result<Json<AskResponse>, ApiError> {
    let Json(request) = body.map_err(|rejection| ApiError {
        status: rejection.status(),
        code: "BAD_REQUEST",
        message: rejection.body_text(),
    })?;

    Ok(Json(state.assistant.ask(&request.question).await?))
}

/// Proof that the request carried the right bearer token. It is the first extractor of the
/// handler, so a stranger is turned away before the body is even read.
struct Authorized;

impl FromRequestParts<AppState> for Authorized {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let Some(expected) = &state.token else {
            return Ok(Self);
        };
        let given = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "));

        match given {
            Some(token) if same(token.as_bytes(), expected.as_bytes()) => Ok(Self),
            _ => Err(ApiError {
                status: StatusCode::UNAUTHORIZED,
                code: "UNAUTHORIZED",
                message: "a valid bearer token is required".to_owned(),
            }),
        }
    }
}

/// Compares without stopping at the first difference.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |diff, (x, y)| diff | (x ^ y)) == 0
}

struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl From<AskError> for ApiError {
    fn from(error: AskError) -> Self {
        let (status, code) = match error {
            AskError::EmptyQuestion | AskError::QuestionTooLong => {
                (StatusCode::BAD_REQUEST, "INVALID_QUESTION")
            }
            AskError::EmptyCatalogue => (StatusCode::SERVICE_UNAVAILABLE, "CATALOGUE_NOT_READY"),
            AskError::Model(_) => (StatusCode::BAD_GATEWAY, "MODEL_UNAVAILABLE"),
            AskError::Understanding(_) => (StatusCode::BAD_GATEWAY, "MODEL_REPLY_UNUSABLE"),
        };
        if status.is_server_error() {
            tracing::warn!(%error, code, "could not answer");
        }

        Self {
            status,
            code,
            message: error.to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut response = (
            self.status,
            Json(ErrorResponse {
                code: self.code.to_owned(),
                message: self.message,
            }),
        )
            .into_response();
        if self.status == StatusCode::UNAUTHORIZED {
            response
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }

        response
    }
}

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{Method, Request},
    };
    use http_body_util::BodyExt;
    use serde_json::{Value, json};
    use tower::ServiceExt;

    use super::*;
    use crate::{
        assistant::testing::ScriptedModel, catalogue::Catalogue, error::ModelError,
        ranking::testing::hardness,
    };

    const NEED: &str =
        r#"{"summary": "hard", "priorities": [{"property": "hardness", "weight": 2}]}"#;

    fn app(replies: Vec<Result<String, ModelError>>, token: Option<&str>, synced: bool) -> Router {
        let catalogue = Arc::new(Catalogue::new());
        if synced {
            catalogue.replace(vec![hardness("Hard", 1500.0), hardness("Soft", 300.0)]);
        }
        let assistant = Assistant::new(ScriptedModel::new(replies), catalogue, 5);

        router(Arc::new(assistant), token.map(str::to_owned))
    }

    async fn call(
        app: &Router,
        request: Request<Body>,
    ) -> (StatusCode, Value, axum::http::HeaderMap) {
        let response = app.clone().oneshot(request).await.unwrap();
        let (parts, body) = response.into_parts();
        let bytes = body.collect().await.unwrap().to_bytes();

        (
            parts.status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            parts.headers,
        )
    }

    fn ask_request(body: &str, authorization: Option<&str>) -> Request<Body> {
        let mut request = Request::builder()
            .method(Method::POST)
            .uri("/v1/ask")
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(authorization) = authorization {
            request = request.header(header::AUTHORIZATION, authorization);
        }

        request.body(Body::from(body.to_owned())).unwrap()
    }

    fn question(text: &str) -> String {
        json!({"question": text}).to_string()
    }

    #[tokio::test]
    async fn health_reports_whether_the_catalogue_has_arrived() {
        let get = || Request::get("/health").body(Body::empty()).unwrap();

        let (status, syncing, _) = call(&app(vec![], None, false), get()).await;
        let (_, ready, _) = call(&app(vec![], None, true), get()).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            syncing,
            json!({"status": "syncing", "woods": 0, "synced_seconds_ago": null})
        );
        assert_eq!(ready["status"], "ok");
        assert_eq!(ready["woods"], 2);
        assert_eq!(ready["synced_seconds_ago"], 0);
    }

    #[tokio::test]
    async fn asking_returns_a_ranked_answer() {
        let app = app(vec![Ok(NEED.into()), Ok("Hard wins.".into())], None, true);

        let (status, body, _) = call(&app, ask_request(&question("a hard table"), None)).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["recommendations"][0]["name"], "Hard");
        assert_eq!(body["explanation"], "Hard wins.");
        assert_eq!(body["catalogue_size"], 2);
    }

    #[tokio::test]
    async fn with_a_token_configured_only_the_right_bearer_gets_in() {
        let app = app(vec![Ok(NEED.into()), Ok("ok".into())], Some("s3cret"), true);
        let body = question("a hard table");

        for authorization in [
            None,
            Some("Bearer wrong"),
            Some("Bearer s3cre"),
            Some("Basic s3cret"),
            Some("s3cret"),
        ] {
            let (status, error, headers) = call(&app, ask_request(&body, authorization)).await;

            assert_eq!(status, StatusCode::UNAUTHORIZED, "{authorization:?}");
            assert_eq!(error["code"], "UNAUTHORIZED");
            assert_eq!(headers[header::WWW_AUTHENTICATE], "Bearer");
        }
        let (status, _, _) = call(&app, ask_request(&body, Some("Bearer s3cret"))).await;
        assert_eq!(status, StatusCode::OK);
        let (health, _, _) = call(&app, Request::get("/health").body(Body::empty()).unwrap()).await;
        assert_eq!(health, StatusCode::OK, "health stays open");
    }

    #[tokio::test]
    async fn a_stranger_is_turned_away_before_the_body_is_read() {
        let app = app(vec![], Some("s3cret"), true);

        let (status, _, _) = call(&app, ask_request("not json at all", None)).await;

        assert_eq!(status, StatusCode::UNAUTHORIZED, "not 400");
    }

    #[tokio::test]
    async fn malformed_and_oversized_bodies_are_client_errors_in_the_usual_shape() {
        let app = app(vec![], None, true);

        for body in ["", "not json", r#"{"q": 1}"#, r#"{"question": 7}"#] {
            let (status, error, _) = call(&app, ask_request(body, None)).await;

            assert!(status.is_client_error(), "{body:?} gave {status}");
            assert_eq!(error["code"], "BAD_REQUEST");
            assert!(!error["message"].as_str().unwrap().is_empty());
        }
        let huge = question(&"x".repeat(MAX_BODY_BYTES));
        let (status, error, _) = call(&app, ask_request(&huge, None)).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(error["code"], "BAD_REQUEST");
    }

    #[tokio::test]
    async fn each_failure_has_its_own_status_and_code() {
        let long = "x".repeat(2000);
        let cases = [
            (
                question("   "),
                vec![],
                true,
                StatusCode::BAD_REQUEST,
                "INVALID_QUESTION",
            ),
            (
                question(&long),
                vec![],
                true,
                StatusCode::BAD_REQUEST,
                "INVALID_QUESTION",
            ),
            (
                question("x"),
                vec![],
                false,
                StatusCode::SERVICE_UNAVAILABLE,
                "CATALOGUE_NOT_READY",
            ),
            (
                question("x"),
                vec![Err(ModelError::Transport("down".into()))],
                true,
                StatusCode::BAD_GATEWAY,
                "MODEL_UNAVAILABLE",
            ),
            (
                question("x"),
                vec![Ok("gibberish".into())],
                true,
                StatusCode::BAD_GATEWAY,
                "MODEL_REPLY_UNUSABLE",
            ),
        ];

        for (body, replies, synced, expected, code) in cases {
            let (status, error, _) =
                call(&app(replies, None, synced), ask_request(&body, None)).await;

            assert_eq!((status, error["code"].as_str()), (expected, Some(code)));
        }
    }

    #[test]
    fn token_comparison_needs_equal_length_and_content() {
        assert!(same(b"abc", b"abc"));
        assert!(same(b"", b""));
        assert!(!same(b"abc", b"abd"));
        assert!(!same(b"abc", b"abcd"));
        assert!(!same(b"", b"a"));
    }
}
