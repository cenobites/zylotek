use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use zylotek_application::woods::{
    create_wood::CreateWoodCommand, errors::WoodUseCaseError, list_woods::ListWoodsQuery,
    update_wood::UpdateWoodCommand,
};
use zylotek_contracts::woods::{ListWoodsParams, WoodPageResponse, WoodRequest, WoodResponse};
use zylotek_domain::woods::{entities::WoodId, errors::WoodError};

use crate::{
    auth::WriteAccess,
    errors::ApiError,
    state::AppState,
    woods::mappers::{Convert, TryConvert},
};

pub async fn create_wood_handler(
    _access: WriteAccess,
    State(state): State<AppState>,
    Json(request): Json<WoodRequest>,
) -> Result<(StatusCode, Json<WoodResponse>), ApiError> {
    let command = CreateWoodCommand {
        name: request.name,
        scientific_name: request.scientific_name,
        details: request
            .details
            .try_convert()
            .map_err(WoodUseCaseError::from)?,
    };

    let wood = state.woods.create_wood(command).await?;

    Ok((StatusCode::CREATED, Json((&wood).convert())))
}

pub async fn list_woods_handler(
    State(state): State<AppState>,
    Query(params): Query<ListWoodsParams>,
) -> Result<Json<WoodPageResponse>, ApiError> {
    let query = ListWoodsQuery {
        offset: params.offset,
        limit: params.limit,
    };

    let page = state.woods.list_woods(query).await?;

    Ok(Json(page.convert()))
}

pub async fn get_wood_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<WoodResponse>, ApiError> {
    let wood = state.woods.get_wood(parse_id(&id)?).await?;

    Ok(Json((&wood).convert()))
}

pub async fn update_wood_handler(
    _access: WriteAccess,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<WoodRequest>,
) -> Result<Json<WoodResponse>, ApiError> {
    let command = UpdateWoodCommand {
        id: parse_id(&id)?,
        name: request.name,
        scientific_name: request.scientific_name,
        details: request
            .details
            .try_convert()
            .map_err(WoodUseCaseError::from)?,
    };

    let wood = state.woods.update_wood(command).await?;

    Ok(Json((&wood).convert()))
}

pub async fn delete_wood_handler(
    _access: WriteAccess,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    state.woods.delete_wood(parse_id(&id)?).await?;

    Ok(StatusCode::NO_CONTENT)
}

fn parse_id(id: &str) -> Result<WoodId, ApiError> {
    id.parse::<WoodId>()
        .map_err(|error| ApiError::BadRequest(error.to_string()))
}

impl From<WoodUseCaseError> for ApiError {
    fn from(error: WoodUseCaseError) -> Self {
        match error {
            WoodUseCaseError::NotFound(_) => ApiError::NotFound(error.to_string()),

            WoodUseCaseError::Domain(WoodError::Repository(error)) => {
                ApiError::Internal(anyhow::Error::new(error))
            }
            WoodUseCaseError::Domain(WoodError::InvalidId(_)) => {
                ApiError::BadRequest(error.to_string())
            }
            WoodUseCaseError::Domain(_) => ApiError::UnprocessableEntity(error.to_string()),

            WoodUseCaseError::Persistence(_) | WoodUseCaseError::Event(_) => {
                ApiError::Internal(anyhow::Error::new(error))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::http::Method;
    use serde_json::{Value, json};
    use zylotek_domain::{
        errors::RepositoryError,
        testing::{InMemoryWoodRepository, Operation, wood},
        woods::entities::WoodId,
    };
    use zylotek_infrastructure::{
        event_bus::EventPublishError,
        testing::{FailingEventPublisher, RecordingEventPublisher},
    };

    use super::*;
    use crate::test_support::{TestApp, app, app_with, send, wood_body};

    const WOODS: &str = "/api/v1/woods";

    async fn create(app: &TestApp, name: &str, scientific_name: &str) -> Value {
        let (status, body) = send(
            &app.router,
            Method::POST,
            WOODS,
            Some(&wood_body(name, scientific_name)),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        body
    }

    #[tokio::test]
    async fn create_returns_201_with_the_stored_wood() {
        let app = app();

        let body = create(&app, "  Aroeira ", "Myracrodruon  urundeuva").await;

        assert_eq!(body["name"], "Aroeira");
        assert_eq!(body["scientific_name"], "Myracrodruon urundeuva");
        assert_eq!(body["genus"], "Myracrodruon");
        assert_eq!(body["family"], "Anacardiaceae");
        assert_eq!(
            body["physical_properties"]["shrinkage"]["tangential_radial_ratio"],
            1.5
        );
        assert!(body["id"].as_str().unwrap().parse::<WoodId>().is_ok());
        assert_eq!(app.repository.len(), 1);
    }

    #[tokio::test]
    async fn create_accepts_a_body_with_only_the_names() {
        let (status, body) = send(
            &app().router,
            Method::POST,
            WOODS,
            Some(&json!({"name": "Aroeira", "scientific_name": "Myracrodruon urundeuva"})),
        )
        .await;

        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(body["common_names"], json!([]));
        assert_eq!(body["family"], Value::Null);
    }

    #[tokio::test]
    async fn create_rejects_invalid_values_with_422() {
        let app = app();
        let cases = [
            ("name", json!(" ")),
            ("scientific_name", json!("Astronium")),
            ("family", json!("")),
            (
                "physical_properties",
                json!({"shrinkage": {"radial": 120.0}}),
            ),
            ("images", json!({"main": "/relative.jpg"})),
            ("source", json!({"species_id": 1, "page_url": "ftp://x"})),
        ];

        for (field, value) in cases {
            let mut body = wood_body("Aroeira", "Myracrodruon urundeuva");
            body[field] = value;

            let (status, error) = send(&app.router, Method::POST, WOODS, Some(&body)).await;

            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{field}: {error}");
            assert_eq!(error["code"], "UNPROCESSABLE_ENTITY");
        }
        assert!(app.repository.is_empty());
    }

    #[tokio::test]
    async fn create_rejects_a_duplicate_scientific_name_with_422() {
        let app = app();
        create(&app, "Aroeira", "Myracrodruon urundeuva").await;

        let (status, error) = send(
            &app.router,
            Method::POST,
            WOODS,
            Some(&wood_body("Outra", "Myracrodruon urundeuva")),
        )
        .await;

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            error["message"],
            "wood scientific name 'Myracrodruon urundeuva' already exists"
        );
    }

    #[tokio::test]
    async fn get_returns_the_wood_or_404() {
        let app = app();
        let created = create(&app, "Aroeira", "Myracrodruon urundeuva").await;
        let id = created["id"].as_str().unwrap();

        let (status, body) = send(&app.router, Method::GET, &format!("{WOODS}/{id}"), None).await;
        assert_eq!((status, &body), (StatusCode::OK, &created));

        let missing = WoodId::new();
        let (status, error) = send(
            &app.router,
            Method::GET,
            &format!("{WOODS}/{missing}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(error["code"], "NOT_FOUND");
        assert_eq!(error["message"], format!("wood {missing} not found"));
    }

    #[tokio::test]
    async fn malformed_ids_are_rejected_with_400() {
        let app = app();
        let body = wood_body("Aroeira", "Myracrodruon urundeuva");

        for (method, payload) in [
            (Method::GET, None),
            (Method::PUT, Some(&body)),
            (Method::DELETE, None),
        ] {
            let (status, error) =
                send(&app.router, method, &format!("{WOODS}/not-a-uuid"), payload).await;

            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert_eq!(error["code"], "BAD_REQUEST");
            assert_eq!(error["message"], "invalid wood id 'not-a-uuid'");
        }
    }

    #[tokio::test]
    async fn list_paginates_in_scientific_name_order() {
        let app = app();
        for scientific_name in [
            "Cedrela fissilis",
            "Astronium lecointei",
            "Bertholletia excelsa",
        ] {
            create(&app, "Madeira", scientific_name).await;
        }

        let (status, page) = send(
            &app.router,
            Method::GET,
            &format!("{WOODS}?offset=1&limit=1"),
            None,
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            (&page["total"], &page["offset"], &page["limit"]),
            (&json!(3), &json!(1), &json!(1))
        );
        assert_eq!(page["items"][0]["scientific_name"], "Bertholletia excelsa");
        assert_eq!(page["items"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn list_defaults_and_clamps() {
        let app = app();

        let (_, page) = send(&app.router, Method::GET, WOODS, None).await;
        assert_eq!((&page["limit"], &page["items"]), (&json!(20), &json!([])));

        let (_, page) = send(
            &app.router,
            Method::GET,
            &format!("{WOODS}?limit=100000"),
            None,
        )
        .await;
        assert_eq!(page["limit"], 100);
    }

    #[tokio::test]
    async fn list_rejects_unparsable_query_parameters() {
        let (status, _) = send(
            &app().router,
            Method::GET,
            &format!("{WOODS}?limit=many"),
            None,
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn update_replaces_the_wood() {
        let app = app();
        let created = create(&app, "Aroeira", "Myracrodruon urundeuva").await;
        let id = created["id"].as_str().unwrap();

        let (status, updated) = send(
            &app.router,
            Method::PUT,
            &format!("{WOODS}/{id}"),
            Some(&json!({"name": "Muiracatiara", "scientific_name": "Astronium lecointei"})),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(updated["id"], created["id"]);
        assert_eq!(updated["name"], "Muiracatiara");
        assert_eq!(
            updated["family"],
            Value::Null,
            "PUT replaces every attribute"
        );
    }

    #[tokio::test]
    async fn update_reports_404_422_and_conflicts() {
        let app = app();
        create(&app, "Aroeira", "Myracrodruon urundeuva").await;
        let other = create(&app, "Muiracatiara", "Astronium lecointei").await;
        let other_id = other["id"].as_str().unwrap();

        let (status, _) = send(
            &app.router,
            Method::PUT,
            &format!("{WOODS}/{}", WoodId::new()),
            Some(&wood_body("A", "Genus species")),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let (status, _) = send(
            &app.router,
            Method::PUT,
            &format!("{WOODS}/{other_id}"),
            Some(&wood_body("Muiracatiara", "Myracrodruon urundeuva")),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

        let (status, _) = send(
            &app.router,
            Method::PUT,
            &format!("{WOODS}/{other_id}"),
            Some(&json!({"name": "", "scientific_name": "Astronium lecointei"})),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn delete_returns_204_then_404() {
        let app = app();
        let created = create(&app, "Aroeira", "Myracrodruon urundeuva").await;
        let uri = format!("{WOODS}/{}", created["id"].as_str().unwrap());

        let (status, body) = send(&app.router, Method::DELETE, &uri, None).await;
        assert_eq!(
            (status, body),
            (StatusCode::NO_CONTENT, Value::String(String::new()))
        );

        let (status, _) = send(&app.router, Method::DELETE, &uri, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn repository_failures_surface_as_500_without_details() {
        let stored = wood("Aroeira", "Myracrodruon urundeuva");
        let uri = format!("{WOODS}/{}", stored.id());
        let body = wood_body("Nova", "Myracrodruon urundeuva");
        let cases = [
            (Operation::FindById, Method::GET, uri.clone(), None),
            (Operation::List, Method::GET, WOODS.to_owned(), None),
            (Operation::Count, Method::GET, WOODS.to_owned(), None),
            (
                Operation::Save,
                Method::POST,
                WOODS.to_owned(),
                Some(wood_body("N", "Genus species")),
            ),
            (Operation::Update, Method::PUT, uri.clone(), Some(body)),
            (Operation::Delete, Method::DELETE, uri, None),
            (
                Operation::ExistsByScientificName,
                Method::POST,
                WOODS.to_owned(),
                Some(wood_body("N", "Genus species")),
            ),
        ];

        for (operation, method, uri, payload) in cases {
            let repository =
                InMemoryWoodRepository::with_woods([stored.clone()]).fail_on(operation);
            let app = app_with(repository, Arc::new(RecordingEventPublisher::new()));

            let (status, error) = send(&app.router, method, &uri, payload.as_ref()).await;

            assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{operation:?}");
            assert_eq!(error["message"], "internal server error");
        }
    }

    #[tokio::test]
    async fn a_failing_event_bus_surfaces_as_500() {
        let app = app_with(
            InMemoryWoodRepository::new(),
            Arc::new(FailingEventPublisher),
        );

        let (status, _) = send(
            &app.router,
            Method::POST,
            WOODS,
            Some(&wood_body("Aroeira", "Myracrodruon urundeuva")),
        )
        .await;

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            app.repository.len(),
            1,
            "the write happened before publishing"
        );
    }

    #[test]
    fn use_case_errors_map_to_api_errors() {
        let id = WoodId::new();
        let cases = [
            (WoodUseCaseError::NotFound(id), "NotFound"),
            (
                WoodUseCaseError::Domain(WoodError::InvalidId("x".into())),
                "BadRequest",
            ),
            (
                WoodUseCaseError::Domain(WoodError::InvalidUrl("x".into())),
                "UnprocessableEntity",
            ),
            (
                WoodUseCaseError::Domain(WoodError::Repository(RepositoryError::Database)),
                "Internal",
            ),
            (
                WoodUseCaseError::Persistence(RepositoryError::Database),
                "Internal",
            ),
            (
                WoodUseCaseError::Event(EventPublishError::NoSubscribers),
                "Internal",
            ),
        ];

        for (error, expected) in cases {
            let mapped = ApiError::from(error);

            assert!(
                format!("{mapped:?}").starts_with(expected),
                "{mapped:?} is not {expected}"
            );
        }
    }
}
