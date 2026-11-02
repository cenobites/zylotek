//! Helpers for driving the router in-process.

use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;
use zylotek_application::woods::{
    create_wood::CreateWood, delete_wood::DeleteWood, facade::WoodFacade, get_wood::GetWood,
    list_woods::ListWoods, update_wood::UpdateWood,
};
use zylotek_domain::{
    testing::{InMemoryWoodRepository, full_details},
    woods::{
        events::WoodEvent,
        services::{WoodCreationService, WoodUpdateService},
    },
};
use zylotek_infrastructure::{event_bus::EventPublisher, testing::RecordingEventPublisher};

use zylotek_contracts::woods::WoodDetailsSchema;

use crate::{router::router, state::AppState, woods::mappers::Convert};

pub(crate) struct TestApp {
    pub router: Router,
    pub repository: Arc<InMemoryWoodRepository>,
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

    TestApp {
        router: router(AppState {
            woods: Arc::new(facade),
        }),
        repository,
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

pub(crate) async fn send(
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

    send_request(router, request.body(body).unwrap()).await
}

pub(crate) async fn send_request(router: &Router, request: Request<Body>) -> (StatusCode, Value) {
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&bytes).into_owned();

    (
        status,
        serde_json::from_str(&text).unwrap_or(Value::String(text)),
    )
}
