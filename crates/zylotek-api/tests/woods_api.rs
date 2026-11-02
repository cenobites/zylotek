//! The wood CRUD flow against the real persistence layer.

mod common;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use common::{app, send, send_raw, wood_body};
use serde_json::{Value, json};

const WOODS: &str = "/api/v1/woods";

#[tokio::test]
async fn a_wood_goes_through_create_read_update_list_delete() {
    let app = app().await;

    let (status, created) = send(
        &app,
        Method::POST,
        WOODS,
        Some(&wood_body("Aroeira", "Myracrodruon urundeuva")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let uri = format!("{WOODS}/{}", created["id"].as_str().unwrap());

    let (status, fetched) = send(&app, Method::GET, &uri, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        fetched, created,
        "what is stored is what was returned on create"
    );

    let (status, updated) = send(
        &app,
        Method::PUT,
        &uri,
        Some(&wood_body("Aroeira-do-sertão", "Myracrodruon urundeuva")),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["name"], "Aroeira-do-sertão");
    assert_eq!(updated["id"], created["id"]);

    let (_, page) = send(&app, Method::GET, WOODS, None).await;
    assert_eq!(page["total"], 1);
    assert_eq!(page["items"][0], updated);

    assert_eq!(
        send(&app, Method::DELETE, &uri, None).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send(&app, Method::GET, &uri, None).await.0,
        StatusCode::NOT_FOUND
    );
    let (_, page) = send(&app, Method::GET, WOODS, None).await;
    assert_eq!(page["total"], 0);
}

#[tokio::test]
async fn every_field_survives_the_database() {
    let app = app().await;
    let body = wood_body("Aroeira", "Myracrodruon urundeuva");

    let (_, created) = send(&app, Method::POST, WOODS, Some(&body)).await;
    let (_, fetched) = send(
        &app,
        Method::GET,
        &format!("{WOODS}/{}", created["id"].as_str().unwrap()),
        None,
    )
    .await;

    let mut expected = body;
    expected["id"] = fetched["id"].clone();
    expected["genus"] = json!("Myracrodruon");
    expected["physical_properties"]["shrinkage"]["tangential_radial_ratio"] = json!(1.5);
    assert_eq!(fetched, expected);
}

#[tokio::test]
async fn the_response_shape_is_stable() {
    let app = app().await;

    let (_, created) = send(
        &app,
        Method::POST,
        WOODS,
        Some(&wood_body("Aroeira", "Myracrodruon urundeuva")),
    )
    .await;

    insta::assert_json_snapshot!(created, {".id" => "[id]"});
}

#[tokio::test]
async fn the_database_enforces_scientific_name_uniqueness() {
    let app = app().await;
    send(
        &app,
        Method::POST,
        WOODS,
        Some(&wood_body("Aroeira", "Myracrodruon urundeuva")),
    )
    .await;

    let (status, error) = send(
        &app,
        Method::POST,
        WOODS,
        Some(&wood_body("Outra", "Myracrodruon urundeuva")),
    )
    .await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(error["code"], "UNPROCESSABLE_ENTITY");
    let (_, page) = send(&app, Method::GET, WOODS, None).await;
    assert_eq!(page["total"], 1);
}

#[tokio::test]
async fn concurrent_creates_of_one_name_leave_exactly_one_wood() {
    let app = app().await;

    let results = futures_join((0..8).map(|i| {
        let app = app.clone();
        async move {
            send(
                &app,
                Method::POST,
                WOODS,
                Some(&wood_body(
                    &format!("Aroeira {i}"),
                    "Myracrodruon urundeuva",
                )),
            )
            .await
        }
    }))
    .await;

    // The losers currently surface as 500: the unique index rejects them as a generic
    // `RepositoryError::Database`. 422 would be the better answer; either is accepted here
    // because the property under test is that exactly one wood is stored.
    let created = results
        .iter()
        .filter(|(status, _)| *status == StatusCode::CREATED)
        .count();
    assert_eq!(created, 1, "{results:?}");
    assert!(
        results
            .iter()
            .filter(|(status, _)| *status != StatusCode::CREATED)
            .all(|(status, _)| matches!(
                *status,
                StatusCode::UNPROCESSABLE_ENTITY | StatusCode::INTERNAL_SERVER_ERROR
            )),
        "{results:?}"
    );
    let (_, page) = send(&app, Method::GET, WOODS, None).await;
    assert_eq!(page["total"], 1);
}

#[tokio::test]
async fn listing_is_ordered_and_paginated_across_the_whole_set() {
    let app = app().await;
    let names = [
        "Swietenia macrophylla",
        "Cedrela fissilis",
        "Astronium lecointei",
        "Handroanthus serratifolius",
        "Bertholletia excelsa",
    ];
    for scientific_name in names {
        send(
            &app,
            Method::POST,
            WOODS,
            Some(&wood_body("Madeira", scientific_name)),
        )
        .await;
    }

    let mut seen = Vec::new();
    for offset in (0..5).step_by(2) {
        let (_, page) = send(
            &app,
            Method::GET,
            &format!("{WOODS}?offset={offset}&limit=2"),
            None,
        )
        .await;
        assert_eq!(page["total"], 5);
        seen.extend(
            page["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|item| item["scientific_name"].as_str().unwrap().to_owned()),
        );
    }

    let mut sorted = names.map(String::from).to_vec();
    sorted.sort();
    assert_eq!(seen, sorted);
}

#[tokio::test]
async fn updating_to_a_name_owned_by_another_wood_is_rejected_and_changes_nothing() {
    let app = app().await;
    send(
        &app,
        Method::POST,
        WOODS,
        Some(&wood_body("Aroeira", "Myracrodruon urundeuva")),
    )
    .await;
    let (_, other) = send(
        &app,
        Method::POST,
        WOODS,
        Some(&wood_body("Muiracatiara", "Astronium lecointei")),
    )
    .await;
    let uri = format!("{WOODS}/{}", other["id"].as_str().unwrap());

    let (status, _) = send(
        &app,
        Method::PUT,
        &uri,
        Some(&wood_body("Muiracatiara", "Myracrodruon urundeuva")),
    )
    .await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (_, unchanged) = send(&app, Method::GET, &uri, None).await;
    assert_eq!(unchanged["scientific_name"], "Astronium lecointei");
}

#[tokio::test]
async fn malformed_requests_are_rejected_before_reaching_the_domain() {
    let app = app().await;
    let post = |content_type: Option<&str>, body: &str| {
        let mut request = Request::builder().method(Method::POST).uri(WOODS);
        if let Some(content_type) = content_type {
            request = request.header(header::CONTENT_TYPE, content_type);
        }
        request.body(Body::from(body.to_owned())).unwrap()
    };

    let cases = [
        (
            post(Some("application/json"), "{not json"),
            StatusCode::BAD_REQUEST,
        ),
        (
            post(Some("application/json"), r#"{"scientific_name": "A b"}"#),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            post(
                Some("application/json"),
                r#"{"name": 7, "scientific_name": "A b"}"#,
            ),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            post(
                Some("application/json"),
                r#"{"name": "A", "scientific_name": "A b", "workabilities": {"treatabilities": {"heartwood": "nope"}}}"#,
            ),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            post(Some("text/plain"), "hello"),
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
        ),
        (post(None, "{}"), StatusCode::UNSUPPORTED_MEDIA_TYPE),
    ];

    for (request, expected) in cases {
        let uri = request.uri().to_string();
        let (status, _) = send_raw(&app, request).await;

        assert_eq!(status, expected, "{uri}");
    }
    let (_, page) = send(&app, Method::GET, WOODS, None).await;
    assert_eq!(page["total"], 0);
}

#[tokio::test]
async fn unknown_json_fields_are_ignored() {
    let app = app().await;
    let mut body = wood_body("Aroeira", "Myracrodruon urundeuva");
    body["surprise"] = json!(true);
    body["id"] = json!("client-supplied-ids-are-ignored");
    body["genus"] = json!("Ignored");

    let (status, created) = send(&app, Method::POST, WOODS, Some(&body)).await;

    assert_eq!(status, StatusCode::CREATED);
    assert_ne!(created["id"], "client-supplied-ids-are-ignored");
    assert_eq!(created["genus"], "Myracrodruon");
    assert_eq!(created.get("surprise"), None::<&Value>);
}

/// Runs the futures concurrently without pulling in `futures` as a dev-dependency.
async fn futures_join<F, T>(futures: impl IntoIterator<Item = F>) -> Vec<T>
where
    F: std::future::Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let handles: Vec<_> = futures.into_iter().map(tokio::spawn).collect();
    let mut results = Vec::new();
    for handle in handles {
        results.push(handle.await.unwrap());
    }
    results
}
