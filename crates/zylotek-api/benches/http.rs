//! The HTTP stack in-process — router, handlers, use cases, SQLite — seeded with 200 woods.

use std::{hint::black_box, sync::Arc};

use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tokio::runtime::Runtime;
use tower::ServiceExt;
use zylotek_api::{
    auth::testing,
    build_state,
    router::router,
    woods::mappers::{Convert, TryConvert},
};
use zylotek_contracts::woods::{WoodDetailsSchema, WoodResponse};
use zylotek_domain::testing::{full_details, wood};
use zylotek_infrastructure::{
    persistence::woods::repositories::PostgresWoodRepository, testing::memory_db,
};

const SEEDED: usize = 200;

fn body(name: &str, scientific_name: &str) -> Value {
    let details: WoodDetailsSchema = (&full_details()).convert();
    let mut body = serde_json::to_value(details).unwrap();
    body["name"] = json!(name);
    body["scientific_name"] = json!(scientific_name);
    body
}

async fn call(
    router: &Router,
    method: Method,
    uri: &str,
    payload: Option<&Value>,
) -> (StatusCode, Vec<u8>) {
    let mut request = Request::builder().method(method).uri(uri).header(
        header::AUTHORIZATION,
        format!("Bearer {}", testing::write_token()),
    );
    let body = match payload {
        Some(json) => {
            request = request.header(header::CONTENT_TYPE, "application/json");
            Body::from(json.to_string())
        }
        None => Body::empty(),
    };
    let response = router
        .clone()
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();

    (
        status,
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
}

fn seeded(runtime: &Runtime) -> (Router, String) {
    runtime.block_on(async {
        let db = memory_db().await.unwrap();
        let router = router(build_state(
            Arc::new(PostgresWoodRepository::new(db)),
            testing::auth(),
        ));
        let mut middle = String::new();
        for i in 0..SEEDED {
            let (status, created) = call(
                &router,
                Method::POST,
                "/api/v1/woods",
                Some(&body("Madeira", &format!("Genus species{i:04}"))),
            )
            .await;
            assert_eq!(status, StatusCode::CREATED);
            if i == SEEDED / 2 {
                let created: Value = serde_json::from_slice(&created).unwrap();
                middle = created["id"].as_str().unwrap().to_owned();
            }
        }
        (router, middle)
    })
}

fn requests(c: &mut Criterion) {
    let runtime = Runtime::new().unwrap();
    let (router, id) = seeded(&runtime);
    let mut group = c.benchmark_group("http");

    group.bench_function("health", |b| {
        b.to_async(&runtime)
            .iter(|| async { call(&router, Method::GET, "/health", None).await })
    });
    group.bench_function("get_wood", |b| {
        let uri = format!("/api/v1/woods/{id}");
        b.to_async(&runtime)
            .iter(|| async { call(&router, Method::GET, black_box(&uri), None).await })
    });
    group.bench_function("get_wood_not_found", |b| {
        b.to_async(&runtime).iter(|| async {
            call(
                &router,
                Method::GET,
                "/api/v1/woods/00000000-0000-0000-0000-000000000000",
                None,
            )
            .await
        })
    });
    group.throughput(Throughput::Elements(20));
    group.bench_function("list_20", |b| {
        b.to_async(&runtime).iter(|| async {
            call(
                &router,
                Method::GET,
                "/api/v1/woods?offset=40&limit=20",
                None,
            )
            .await
        })
    });
    group.throughput(Throughput::Elements(1));
    group.bench_function("update_wood", |b| {
        let uri = format!("/api/v1/woods/{id}");
        let payload = body("Madeira", "Genus species0100");
        b.to_async(&runtime)
            .iter(|| async { call(&router, Method::PUT, &uri, Some(black_box(&payload))).await })
    });
    group.bench_function("create_then_delete", |b| {
        let mut n = 0u64;
        b.to_async(&runtime).iter(|| {
            n += 1;
            let payload = body("Madeira", &format!("Bench item{n}"));
            let router = router.clone();
            async move {
                let (_, created) =
                    call(&router, Method::POST, "/api/v1/woods", Some(&payload)).await;
                let created: Value = serde_json::from_slice(&created).unwrap();
                call(
                    &router,
                    Method::DELETE,
                    &format!("/api/v1/woods/{}", created["id"].as_str().unwrap()),
                    None,
                )
                .await
            }
        })
    });
    group.finish();
}

fn schemas(c: &mut Criterion) {
    let mut group = c.benchmark_group("schemas");
    let stored = wood("Aroeira", "Myracrodruon urundeuva");
    let response: WoodResponse = (&stored).convert();
    let json = serde_json::to_string(&response).unwrap();

    group.throughput(Throughput::Bytes(json.len() as u64));
    group.bench_function("domain_to_response", |b| {
        b.iter(|| -> WoodResponse { black_box(&stored).convert() })
    });
    group.bench_function("serialize_response", |b| {
        b.iter(|| serde_json::to_string(black_box(&response)))
    });
    group.bench_function("deserialize_details", |b| {
        b.iter(|| serde_json::from_str::<WoodDetailsSchema>(black_box(&json)))
    });
    group.bench_function("schema_to_domain", |b| {
        b.iter_batched(
            || serde_json::from_str::<WoodDetailsSchema>(&json).unwrap(),
            |schema: WoodDetailsSchema| -> Result<zylotek_domain::woods::value_objects::WoodDetails, _> {
                schema.try_convert()
            },
            criterion::BatchSize::SmallInput,
        )
    });
    group.finish();
}

/// What every write pays for authentication, and what a client pays to get a token.
fn oauth(c: &mut Criterion) {
    let runtime = Runtime::new().unwrap();
    let auth = testing::auth();
    let (client_id, secret) = testing::writer_credentials();
    let mut group = c.benchmark_group("oauth");

    group.bench_function("verify_token", |b| {
        b.iter(|| {
            auth.authorize(black_box(Some(&bearer())), "woods:write")
                .unwrap()
        })
    });
    group.bench_function("reject_forged_token", |b| {
        let forged = header::HeaderValue::from_static("Bearer eyJhbGciOiJFZERTQSJ9.e30.AAAA");
        b.iter(|| {
            auth.authorize(black_box(Some(&forged)), "woods:write")
                .unwrap_err()
        })
    });

    // Argon2 makes this slow by design; the number to watch is that it stays well under the
    // request timeout.
    group.sample_size(20);
    group.bench_function("token_endpoint", |b| {
        let router = runtime.block_on(async {
            router(build_state(
                Arc::new(zylotek_domain::testing::InMemoryWoodRepository::new()),
                testing::auth(),
            ))
        });
        let body =
            format!("grant_type=client_credentials&client_id={client_id}&client_secret={secret}");
        b.to_async(&runtime).iter(|| async {
            let request = Request::builder()
                .method(Method::POST)
                .uri("/oauth/token")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(body.clone()))
                .unwrap();
            router.clone().oneshot(request).await.unwrap().status()
        })
    });
    group.finish();
}

fn bearer() -> header::HeaderValue {
    header::HeaderValue::from_str(&format!("Bearer {}", testing::write_token())).unwrap()
}

criterion_group!(benches, requests, schemas, oauth);
criterion_main!(benches);
