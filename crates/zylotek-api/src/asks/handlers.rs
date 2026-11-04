use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use zylotek_application::asks::{AskUseCaseError, Progress};
use zylotek_contracts::{
    asks::{AskProgressRequest, AskRecordResponse, AskStateSchema, CreateAskRequest},
    assistant::AskResponse,
};
use zylotek_domain::asks::{
    entities::{Ask, AskId, AskState},
    errors::AskError,
};

use crate::{auth::AskWriteAccess, errors::ApiError, state::AppState};

pub async fn create_ask_handler(
    _access: AskWriteAccess,
    State(state): State<AppState>,
    Json(request): Json<CreateAskRequest>,
) -> Result<(StatusCode, Json<AskRecordResponse>), ApiError> {
    let ask = state.asks.create(&request.question).await?;

    Ok((StatusCode::CREATED, Json(record(&ask)?)))
}

pub async fn get_ask_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<AskRecordResponse>, ApiError> {
    let ask = state.asks.get(parse_id(&id)?).await?;

    Ok(Json(record(&ask)?))
}

pub async fn record_progress_handler(
    _access: AskWriteAccess,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<AskProgressRequest>,
) -> Result<Json<AskRecordResponse>, ApiError> {
    let progress = match request {
        AskProgressRequest::Started => Progress::Started,
        AskProgressRequest::Ranked { answer } => Progress::Ranked(
            serde_json::to_string(&answer).expect("an assistant answer always serializes"),
        ),
        AskProgressRequest::Explaining => Progress::Explaining,
        AskProgressRequest::Explained { text } => Progress::Explained(text),
        AskProgressRequest::Answered { complete } => Progress::Answered { complete },
        AskProgressRequest::Failed { reason } => Progress::Failed(reason),
    };
    let ask = state.asks.record(parse_id(&id)?, progress).await?;

    Ok(Json(record(&ask)?))
}

fn parse_id(id: &str) -> Result<AskId, ApiError> {
    id.parse::<AskId>()
        .map_err(|error| ApiError::BadRequest(error.to_string()))
}

fn state_schema(state: AskState) -> AskStateSchema {
    match state {
        AskState::Queued => AskStateSchema::Queued,
        AskState::Understanding => AskStateSchema::Understanding,
        AskState::Ranked => AskStateSchema::Ranked,
        AskState::Explaining => AskStateSchema::Explaining,
        AskState::Answered => AskStateSchema::Answered,
        AskState::Failed => AskStateSchema::Failed,
    }
}

/// The stored ranking was written by this API from a typed answer, so a document that no
/// longer reads as one is a fault here, not the caller's.
fn record(ask: &Ask) -> Result<AskRecordResponse, ApiError> {
    let answer = ask
        .ranking()
        .map(|ranking| serde_json::from_str::<AskResponse>(ranking.as_str()))
        .transpose()
        .map_err(|error| ApiError::Internal(anyhow::Error::new(error)))?;

    Ok(AskRecordResponse {
        id: ask.id().to_string(),
        question: ask.question().as_str().to_owned(),
        state: state_schema(ask.state()),
        answer,
        explanation: ask.explanation().to_owned(),
        explanation_complete: ask.explanation_complete(),
        failure: ask.failure().map(str::to_owned),
        created_at: ask.created_at().0,
        updated_at: ask.updated_at().0,
    })
}

impl From<AskUseCaseError> for ApiError {
    fn from(error: AskUseCaseError) -> Self {
        match error {
            AskUseCaseError::NotFound(_) => ApiError::NotFound(error.to_string()),
            AskUseCaseError::Domain(AskError::InvalidTransition { .. }) => {
                ApiError::Conflict(error.to_string())
            }
            AskUseCaseError::Domain(AskError::InvalidId(_)) => {
                ApiError::BadRequest(error.to_string())
            }
            AskUseCaseError::Domain(AskError::Repository(_)) | AskUseCaseError::Persistence(_) => {
                ApiError::Internal(anyhow::Error::new(error))
            }
            AskUseCaseError::Domain(_) => ApiError::UnprocessableEntity(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::http::Method;
    use serde_json::{Value, json};
    use zylotek_domain::errors::RepositoryError;

    use super::*;
    use crate::{
        auth::testing,
        test_support::{app, send_as},
    };

    const ASKS: &str = "/api/v1/asks";

    fn answer() -> Value {
        json!({"summary": "A table", "criteria": [{"property": "hardness", "weight": 3}],
               "recommendations": [], "explanation": null, "catalogue_size": 225})
    }

    async fn post(router: &axum::Router, uri: &str, body: Value) -> (StatusCode, Value) {
        send_as(
            router,
            Method::POST,
            uri,
            Some(&body),
            Some(testing::asks_token()),
        )
        .await
    }

    #[tokio::test]
    async fn an_ask_is_opened_walked_through_its_steps_and_read_back() {
        let app = app();
        let (status, created) = post(&app.router, ASKS, json!({"question": " a table "})).await;
        let id = created["id"].as_str().unwrap().to_owned();
        let progress = format!("{ASKS}/{id}/progress");

        for step in [
            json!({"step": "started"}),
            json!({"step": "ranked", "answer": answer()}),
            json!({"step": "explaining"}),
            json!({"step": "explained", "text": "Cedro "}),
            json!({"step": "explained", "text": "is steady."}),
            json!({"step": "answered", "complete": true}),
        ] {
            let (status, body) = post(&app.router, &progress, step.clone()).await;
            assert_eq!(status, StatusCode::OK, "{step} → {body}");
        }
        let (read, ask) = send_as(
            &app.router,
            Method::GET,
            &format!("{ASKS}/{id}"),
            None,
            None,
        )
        .await;

        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(
            (created["state"].as_str(), created["question"].as_str()),
            (Some("queued"), Some("a table"))
        );
        assert_eq!(read, StatusCode::OK, "reading needs no token");
        assert_eq!(ask["state"], "answered");
        assert_eq!(ask["answer"], answer());
        assert_eq!(ask["explanation"], "Cedro is steady.");
        assert_eq!(ask["explanation_complete"], true);
        assert!(ask["updated_at"].as_u64() >= ask["created_at"].as_u64());
    }

    #[tokio::test]
    async fn a_step_out_of_order_is_a_conflict_and_bad_input_is_refused() {
        let app = app();
        let (_, created) = post(&app.router, ASKS, json!({"question": "a table"})).await;
        let progress = format!("{ASKS}/{}/progress", created["id"].as_str().unwrap());

        let (early, conflict) = post(
            &app.router,
            &progress,
            json!({"step": "answered", "complete": true}),
        )
        .await;
        let (blank, _) = post(&app.router, ASKS, json!({"question": "   "})).await;
        let (failed, body) = post(
            &app.router,
            &progress,
            json!({"step": "failed", "reason": "model down"}),
        )
        .await;
        let (unknown, _) = post(
            &app.router,
            &format!("{ASKS}/00000000-0000-4000-8000-000000000000/progress"),
            json!({"step": "started"}),
        )
        .await;
        let (bad_id, _) = send_as(
            &app.router,
            Method::GET,
            &format!("{ASKS}/nope"),
            None,
            None,
        )
        .await;
        let (missing, _) = send_as(
            &app.router,
            Method::GET,
            &format!("{ASKS}/00000000-0000-4000-8000-000000000000"),
            None,
            None,
        )
        .await;

        assert_eq!(early, StatusCode::CONFLICT);
        assert_eq!(conflict["code"], "CONFLICT");
        assert_eq!(blank, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            (failed, &body["failure"]),
            (StatusCode::OK, &json!("model down"))
        );
        assert_eq!(
            (unknown, bad_id, missing),
            (
                StatusCode::NOT_FOUND,
                StatusCode::BAD_REQUEST,
                StatusCode::NOT_FOUND
            )
        );
    }

    #[tokio::test]
    async fn writing_needs_the_asks_scope_and_the_woods_scope_is_not_enough() {
        let app = app();
        let body = json!({"question": "a table"});

        let (anonymous, _) = send_as(&app.router, Method::POST, ASKS, Some(&body), None).await;
        let (woods_writer, _) = send_as(
            &app.router,
            Method::POST,
            ASKS,
            Some(&body),
            Some(testing::write_token()),
        )
        .await;

        assert_eq!(anonymous, StatusCode::UNAUTHORIZED);
        assert_eq!(woods_writer, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn a_stored_ranking_that_no_longer_reads_is_an_internal_error() {
        let app = app();
        let ask = app.asks.create("a table").await.unwrap();
        app.asks.record(ask.id(), Progress::Started).await.unwrap();
        app.asks
            .record(ask.id(), Progress::Ranked("not an answer".into()))
            .await
            .unwrap();

        let (status, _) = send_as(
            &app.router,
            Method::GET,
            &format!("{ASKS}/{}", ask.id()),
            None,
            None,
        )
        .await;

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn use_case_errors_become_statuses() {
        let internal = ApiError::from(AskUseCaseError::Persistence(RepositoryError::Database));
        let repository = ApiError::from(AskUseCaseError::Domain(AskError::Repository(
            RepositoryError::Database,
        )));
        let bad_id = ApiError::from(AskUseCaseError::Domain(AskError::InvalidId("x".into())));

        assert!(matches!(internal, ApiError::Internal(_)));
        assert!(matches!(repository, ApiError::Internal(_)));
        assert!(matches!(bad_id, ApiError::BadRequest(_)));
        for state in AskState::ALL {
            assert_eq!(
                serde_json::to_value(state_schema(state)).unwrap(),
                json!(state.as_str())
            );
        }
    }
}
