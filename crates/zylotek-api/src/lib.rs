use std::sync::Arc;

use zylotek_application::asks::{
    clock::SystemClock, create_ask::CreateAsk, facade::AskFacade, get_ask::GetAsk,
    handlers::AskEventHandler, record_ask_progress::RecordAskProgress,
};
use zylotek_application::woods::{
    create_wood::CreateWood, delete_wood::DeleteWood, facade::WoodFacade, get_wood::GetWood,
    handlers::WoodEventHandler, list_woods::ListWoods, update_wood::UpdateWood,
};
use zylotek_domain::asks::{
    events::AskEvent, repositories::AskRepository, services::AskWorkflowService,
};
use zylotek_domain::woods::{
    events::WoodEvent,
    repositories::WoodRepository,
    services::{WoodCreationService, WoodUpdateService},
};
use zylotek_infrastructure::event_bus::tokio::{TokioEventPublisher, run_event_handler};

use crate::{auth::Auth, state::AppState};

pub mod asks;
pub mod auth;
pub mod errors;
pub mod middleware;
pub mod router;
pub mod server;
pub mod state;
pub mod woods;

pub use server::serve;

#[cfg(test)]
mod test_support;

/// Wires the use cases and event handlers on top of the repositories, and guards the write
/// routes with `auth`.
///
/// Spawns the event handlers, so it must be called inside a Tokio runtime.
pub fn build_state(
    wood_repository: Arc<dyn WoodRepository>,
    ask_repository: Arc<dyn AskRepository>,
    auth: Auth,
) -> AppState {
    let wood_event_publisher = Arc::new(TokioEventPublisher::<WoodEvent>::new(1024));
    let wood_event_handler = WoodEventHandler::new();

    let create_wood = CreateWood::new(
        wood_event_publisher.clone(),
        wood_repository.clone(),
        WoodCreationService::new(wood_repository.clone()),
    );
    let get_wood = GetWood::new(wood_repository.clone());
    let list_woods = ListWoods::new(wood_repository.clone());
    let update_wood = UpdateWood::new(
        wood_event_publisher.clone(),
        wood_repository.clone(),
        WoodUpdateService::new(wood_repository.clone()),
    );
    let delete_wood = DeleteWood::new(wood_event_publisher.clone(), wood_repository.clone());
    let wood_facade = WoodFacade::new(create_wood, get_wood, list_woods, update_wood, delete_wood);

    tokio::spawn(run_event_handler(
        wood_event_publisher.subscribe(),
        wood_event_handler,
    ));

    AppState {
        woods: Arc::new(wood_facade),
        asks: Arc::new(build_ask_facade(ask_repository)),
        auth,
    }
}

fn build_ask_facade(ask_repository: Arc<dyn AskRepository>) -> AskFacade {
    let ask_event_publisher = Arc::new(TokioEventPublisher::<AskEvent>::new(1024));
    let clock = Arc::new(SystemClock);
    let ask_workflow_service = AskWorkflowService::new();

    let create_ask = CreateAsk::new(
        ask_event_publisher.clone(),
        ask_repository.clone(),
        ask_workflow_service,
        clock.clone(),
    );
    let get_ask = GetAsk::new(ask_repository.clone());
    let record_ask_progress = RecordAskProgress::new(
        ask_event_publisher.clone(),
        ask_repository,
        ask_workflow_service,
        clock,
    );

    tokio::spawn(run_event_handler(
        ask_event_publisher.subscribe(),
        AskEventHandler::new(),
    ));

    AskFacade::new(create_ask, get_ask, record_ask_progress)
}

#[cfg(test)]
mod tests {
    use axum::http::{Method, StatusCode};
    use serde_json::json;
    use zylotek_domain::testing::InMemoryWoodRepository;

    use super::*;
    use crate::{
        router::router,
        test_support::{send, send_as, wood_body},
    };

    #[tokio::test]
    async fn build_state_wires_a_working_stack_with_a_live_event_handler() {
        let state = build_state(
            Arc::new(InMemoryWoodRepository::new()),
            Arc::new(zylotek_domain::testing::InMemoryAskRepository::new()),
            crate::auth::testing::auth(),
        );
        let router = router(state);

        // Publishing succeeds only because the spawned handler subscribed to the bus.
        let (status, created) = send(
            &router,
            Method::POST,
            "/api/v1/woods",
            Some(&wood_body("Aroeira", "Myracrodruon urundeuva")),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);

        let uri = format!("/api/v1/woods/{}", created["id"].as_str().unwrap());
        let (status, fetched) = send(&router, Method::GET, &uri, None).await;
        assert_eq!(
            (status, &fetched["name"]),
            (StatusCode::OK, &json!("Aroeira"))
        );

        let (status, _) = send(&router, Method::DELETE, &uri, None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);

        // Likewise for asks: opening one publishes `AskOpened` to the spawned ask handler.
        let (status, ask) = send_as(
            &router,
            Method::POST,
            "/api/v1/asks",
            Some(&json!({"question": "a table"})),
            Some(crate::auth::testing::asks_token()),
        )
        .await;
        assert_eq!(
            (status, &ask["state"]),
            (StatusCode::CREATED, &json!("queued"))
        );
    }
}
