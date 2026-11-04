//! Helpers for driving the router in-process.

use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    http::{HeaderMap, Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;
use zylotek_application::asks::{AskFacade, SystemClock};
use zylotek_application::woods::{
    create_wood::CreateWood, delete_wood::DeleteWood, facade::WoodFacade, get_wood::GetWood,
    list_woods::ListWoods, update_wood::UpdateWood,
};
use zylotek_domain::{
    testing::{InMemoryAskRepository, InMemoryWoodRepository, full_details},
    woods::{
        events::WoodEvent,
        services::{WoodCreationService, WoodUpdateService},
    },
};
use zylotek_infrastructure::{event_bus::EventPublisher, testing::RecordingEventPublisher};

use zylotek_contracts::woods::WoodDetailsSchema;

use crate::{
    auth::{Auth, testing},
    router::router,
    state::AppState,
    woods::mappers::Convert,
};

pub(crate) struct TestApp {
    pub router: Router,
    pub repository: Arc<InMemoryWoodRepository>,
    pub asks: Arc<AskFacade>,
}

pub(crate) fn app() -> TestApp {
    app_with(
        InMemoryWoodRepository::new(),
        Arc::new(RecordingEventPublisher::new()),
    )
}

pub(crate) fn app_with(
    repository: InMemoryWoodRepository,
    publisher: Arc<dyn EventPublisher<WoodEvent>>,
) -> TestApp {
    app_with_auth(repository, publisher, testing::auth())
}

pub(crate) fn app_with_auth(
    repository: InMemoryWoodRepository,
    publisher: Arc<dyn EventPublisher<WoodEvent>>,
    auth: Auth,
) -> TestApp {
    let repository = Arc::new(repository);
    let facade = WoodFacade::new(
        CreateWood::new(
            publisher.clone(),
            repository.clone(),
            WoodCreationService::new(repository.clone()),
        ),
        GetWood::new(repository.clone()),
        ListWoods::new(repository.clone()),
        UpdateWood::new(
            publisher.clone(),
            repository.clone(),
            WoodUpdateService::new(repository.clone()),
        ),
        DeleteWood::new(publisher, repository.clone()),
    );

    let asks = Arc::new(AskFacade::new(
        Arc::new(InMemoryAskRepository::new()),
        Arc::new(SystemClock),
    ));

    TestApp {
        router: router(AppState {
            woods: Arc::new(facade),
            asks: asks.clone(),
            auth,
        }),
        repository,
        asks,
    }
}

/// A `POST`/`PUT` body with every detail populated.
pub(crate) fn wood_body(name: &str, scientific_name: &str) -> Value {
    let details: WoodDetailsSchema = (&full_details()).convert();
    let mut body = serde_json::to_value(details).unwrap();
    body["name"] = json!(name);
    body["scientific_name"] = json!(scientific_name);
    body
}

/// Sends a request carrying a valid write token, as an authorized client would.
pub(crate) async fn send(
    router: &Router,
    method: Method,
    uri: &str,
    body: Option<&Value>,
) -> (StatusCode, Value) {
    send_as(router, method, uri, body, Some(testing::write_token())).await
}

/// Sends a request with the given bearer token, or none.
pub(crate) async fn send_as(
    router: &Router,
    method: Method,
    uri: &str,
    body: Option<&Value>,
    token: Option<&str>,
) -> (StatusCode, Value) {
    let (status, _, body) = send_full(router, method, uri, body, token.map(bearer)).await;

    (status, body)
}

pub(crate) fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}

/// Sends a request with the given `Authorization` header value, and returns the headers too.
pub(crate) async fn send_full(
    router: &Router,
    method: Method,
    uri: &str,
    body: Option<&Value>,
    authorization: Option<String>,
) -> (StatusCode, HeaderMap, Value) {
    let mut request = Request::builder().method(method).uri(uri);
    if let Some(authorization) = authorization {
        request = request.header(header::AUTHORIZATION, authorization);
    }
    let body = match body {
        Some(json) => {
            request = request.header(header::CONTENT_TYPE, "application/json");
            Body::from(json.to_string())
        }
        None => Body::empty(),
    };

    send_request_full(router, request.body(body).unwrap()).await
}

pub(crate) async fn send_request(router: &Router, request: Request<Body>) -> (StatusCode, Value) {
    let (status, _, body) = send_request_full(router, request).await;

    (status, body)
}

pub(crate) async fn send_request_full(
    router: &Router,
    request: Request<Body>,
) -> (StatusCode, HeaderMap, Value) {
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&bytes).into_owned();

    (
        status,
        headers,
        serde_json::from_str(&text).unwrap_or(Value::String(text)),
    )
}

/// An app whose OAuth is not configured.
pub(crate) fn app_without_auth() -> TestApp {
    app_with_auth(
        InMemoryWoodRepository::new(),
        Arc::new(RecordingEventPublisher::new()),
        Auth::disabled(),
    )
}
