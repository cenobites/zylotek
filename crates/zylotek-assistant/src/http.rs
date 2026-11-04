//! The assistant's HTTP face: `GET /health` is open, `POST /v1/ask` needs the shared bearer
//! token when one is configured. The assistant usually runs on another machine, so unlike the
//! read-only wood API it spends the GPU and is not left open by accident.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, FromRequestParts, Request, State, rejection::JsonRejection},
    http::{HeaderValue, StatusCode, header, request::Parts},
    middleware::Next,
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use futures::{Stream, StreamExt, stream};
use tower::limit::ConcurrencyLimitLayer;
use tower_http::{
    compression::CompressionLayer,
    cors::{Any, CorsLayer},
    limit::RequestBodyLimitLayer,
    timeout::TimeoutLayer,
    trace::TraceLayer,
};
use zylotek_contracts::{
    ErrorResponse,
    assistant::{AskRequest, AskResponse, AssistantHealth, events},
};

use crate::{
    assistant::{Assistant, MODEL_CALLS},
    error::AskError,
    llm::REQUEST_TIMEOUT,
};

/// A question is at most 1000 characters; this leaves room for JSON and multi-byte text.
const MAX_BODY_BYTES: usize = 8 * 1024;
/// An ask makes up to `MODEL_CALLS` model calls before it answers. Longer than all of them, so a
/// slow model, one still loading say, ends in the model client's own timeout instead of the
/// request being dropped mid-load.
const ASK_TIMEOUT: Duration = Duration::from_secs(MODEL_CALLS * REQUEST_TIMEOUT.as_secs() + 10);
const HEALTH_TIMEOUT: Duration = Duration::from_secs(5);

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

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let timeout = |after| TimeoutLayer::with_status_code(StatusCode::REQUEST_TIMEOUT, after);

    Router::new()
        .route("/health", get(health).layer(timeout(HEALTH_TIMEOUT)))
        .route("/v1/ask", post(ask).layer(timeout(ASK_TIMEOUT)))
        .route(
            "/v1/ask/stream",
            post(ask_stream).layer(timeout(ASK_TIMEOUT)),
        )
        .layer(axum::middleware::from_fn(logging_middleware))
        .layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new())
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(RequestBodyLimitLayer::new(1024 * 1024)) // 1MB
        .layer(ConcurrencyLimitLayer::new(100)) // 100 concurrent requests
        .layer(cors)
        .with_state(state)
}

pub async fn logging_middleware(req: Request, next: Next) -> Response {
    let time = Instant::now();
    let method = req.method().clone();
    let uri = req.uri().clone();
    let response = next.run(req).await;
    // Worked out before the macro: `log` skips its arguments when the level is off.
    let status = response.status();
    let elapsed = time.elapsed().as_millis();
    log::info!("{} {} -> {} ({:?} ms)", method, uri, status, elapsed);
    response
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

/// Same question and answer, delivered as server-sent events: the ranking first, then the
/// explanation as the model writes it (see `contracts::assistant::events`). Errors before the
/// ranking is ready are ordinary JSON errors; a failure after that is a `failed` event.
async fn ask_stream(
    _authorized: Authorized,
    State(state): State<AppState>,
    body: Result<Json<AskRequest>, JsonRejection>,
) -> Result<Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>>, ApiError> {
    let Json(request) = body.map_err(|rejection| ApiError {
        status: rejection.status(),
        code: "BAD_REQUEST",
        message: rejection.body_text(),
    })?;
    let streamed = state.assistant.ask_stream(&request.question).await?;

    let first = stream::once(async move { event(events::ANSWER, &streamed.answer) });
    let tokens = stream::iter(streamed.explanation)
        .flatten()
        .map(|piece| match piece {
            Ok(text) => event(events::TOKEN, &text),
            Err(error) => {
                tracing::warn!(%error, "the explanation stopped midway");
                event(events::FAILED, "the explanation was cut short")
            }
        });
    let done = stream::once(async { event(events::DONE, &serde_json::json!({})) });

    Ok(Sse::new(first.chain(tokens).chain(done)).keep_alive(KeepAlive::default()))
}

fn event<T: serde::Serialize + ?Sized>(
    name: &str,
    data: &T,
) -> Result<Event, std::convert::Infallible> {
    Ok(Event::default()
        .event(name)
        .data(serde_json::to_string(data).expect("events serialize")))
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
        let app = app(
            vec![
                Ok(NEED.into()),
                Ok("English".into()),
                Ok("Hard wins.".into()),
            ],
            None,
            true,
        );

        let (status, body, _) = call(&app, ask_request(&question("a hard table"), None)).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["recommendations"][0]["name"], "Hard");
        assert_eq!(body["explanation"], "Hard wins.");
        assert_eq!(body["catalogue_size"], 2);
    }

    /// A model that takes its time on every call, like one still being loaded.
    struct SlowModel(Arc<ScriptedModel>, Duration);

    #[async_trait::async_trait]
    impl crate::llm::Model for SlowModel {
        async fn complete(
            &self,
            messages: &[crate::llm::Message],
            schema: Option<&Value>,
        ) -> Result<String, ModelError> {
            tokio::time::sleep(self.1).await;
            self.0.complete(messages, schema).await
        }

        async fn stream(
            &self,
            messages: &[crate::llm::Message],
        ) -> Result<crate::llm::TextStream, ModelError> {
            tokio::time::sleep(self.1).await;
            self.0.stream(messages).await
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_slow_model_is_waited_for_instead_of_cut_off() {
        let catalogue = Arc::new(Catalogue::new());
        catalogue.replace(vec![hardness("Hard", 1500.0), hardness("Soft", 300.0)]);
        let model = ScriptedModel::new(vec![
            Ok(NEED.into()),
            Ok("English".into()),
            Ok("Hard wins.".into()),
        ]);
        let slow = Arc::new(SlowModel(model, Duration::from_secs(60)));
        let app = router(Arc::new(Assistant::new(slow, catalogue, 5)), None);

        let (status, body, _) = call(&app, ask_request(&question("a hard table"), None)).await;

        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["explanation"], "Hard wins.");
        assert!(ASK_TIMEOUT > 3 * REQUEST_TIMEOUT, "room for every call");
    }

    #[tokio::test]
    async fn with_a_token_configured_only_the_right_bearer_gets_in() {
        let app = app(
            vec![Ok(NEED.into()), Ok("English".into()), Ok("ok".into())],
            Some("s3cret"),
            true,
        );
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

    /// `(event, data)` pairs of a server-sent-events body.
    fn events_of(body: &str) -> Vec<(String, String)> {
        body.split("\n\n")
            .filter(|block| block.contains("event:"))
            .map(|block| {
                let field = |name: &str| {
                    block
                        .lines()
                        .find_map(|line| line.strip_prefix(name))
                        .unwrap_or_default()
                        .trim()
                        .to_owned()
                };

                (field("event:"), field("data:"))
            })
            .collect()
    }

    async fn stream_text(
        app: &Router,
        body: &str,
        authorization: Option<&str>,
    ) -> (StatusCode, String, axum::http::HeaderMap) {
        let mut request = Request::builder()
            .method(Method::POST)
            .uri("/v1/ask/stream")
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(authorization) = authorization {
            request = request.header(header::AUTHORIZATION, authorization);
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::from(body.to_owned())).unwrap())
            .await
            .unwrap();
        let (parts, body) = response.into_parts();
        let bytes = body.collect().await.unwrap().to_bytes();

        (
            parts.status,
            String::from_utf8(bytes.to_vec()).unwrap(),
            parts.headers,
        )
    }

    #[tokio::test]
    async fn the_stream_sends_the_ranking_then_the_words_then_done() {
        let app = app(
            vec![
                Ok(NEED.into()),
                Ok("English".into()),
                Ok("Hard wins easily.".into()),
            ],
            None,
            true,
        );

        let (status, body, headers) = stream_text(&app, &question("a hard table"), None).await;

        assert_eq!(status, StatusCode::OK);
        assert!(
            headers[header::CONTENT_TYPE]
                .to_str()
                .unwrap()
                .starts_with("text/event-stream")
        );
        let events = events_of(&body);
        let names: Vec<_> = events.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, ["answer", "token", "token", "token", "done"]);
        let answer: Value = serde_json::from_str(&events[0].1).unwrap();
        assert_eq!(answer["recommendations"][0]["name"], "Hard");
        assert_eq!(answer["explanation"], Value::Null);
        let words: String = events[1..4]
            .iter()
            .map(|(_, data)| serde_json::from_str::<String>(data).unwrap())
            .collect();
        assert_eq!(words, "Hard wins easily.");
    }

    #[tokio::test]
    async fn no_explanation_means_the_ranking_and_done() {
        let need = r#"{"priorities": [{"property": "lightness", "weight": 2}]}"#;
        let app = app(vec![Ok(need.into())], None, true);

        let (_, body, _) = stream_text(&app, &question("a light box"), None).await;

        let names: Vec<_> = events_of(&body).into_iter().map(|(name, _)| name).collect();
        assert_eq!(names, ["answer", "done"]);
    }

    #[tokio::test]
    async fn a_model_that_stops_midway_is_reported_inside_the_stream() {
        let app = app(
            vec![
                Ok(NEED.into()),
                Ok("English".into()),
                Ok("Hard is <<cut>>".into()),
            ],
            None,
            true,
        );

        let (status, body, _) = stream_text(&app, &question("a hard table"), None).await;

        assert_eq!(status, StatusCode::OK, "the ranking was already sent");
        let events = events_of(&body);
        let names: Vec<_> = events.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, ["answer", "token", "token", "failed", "done"]);
        assert!(events[3].1.contains("cut short"));
    }

    #[tokio::test]
    async fn the_stream_endpoint_has_the_same_guards_and_errors_as_ask() {
        let secured = app(vec![], Some("s3cret"), true);
        let unsynced = app(vec![], None, false);
        let rude = app(vec![Err(ModelError::Transport("down".into()))], None, true);

        let (anonymous, _, _) = stream_text(&secured, &question("x"), None).await;
        let (wrong, _, _) = stream_text(&secured, &question("x"), Some("Bearer nope")).await;
        let (garbage, garbage_body, _) = stream_text(&app(vec![], None, true), "nope", None).await;
        let (blank, blank_body, _) =
            stream_text(&app(vec![], None, true), &question(" "), None).await;
        let (not_ready, _, _) = stream_text(&unsynced, &question("x"), None).await;
        let (down, down_body, _) = stream_text(&rude, &question("x"), None).await;

        assert_eq!(
            (anonymous, wrong),
            (StatusCode::UNAUTHORIZED, StatusCode::UNAUTHORIZED)
        );
        assert!(garbage.is_client_error() && garbage_body.contains("BAD_REQUEST"));
        assert!(blank == StatusCode::BAD_REQUEST && blank_body.contains("INVALID_QUESTION"));
        assert_eq!(not_ready, StatusCode::SERVICE_UNAVAILABLE);
        assert!(down == StatusCode::BAD_GATEWAY && down_body.contains("MODEL_UNAVAILABLE"));
    }
}
