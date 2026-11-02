//! Arbitrary requests against the whole router: it answers, and never with a 5xx — the
//! in-memory repository cannot fail, so a server error means a bug.

#![no_main]

use std::sync::{Arc, OnceLock};

use arbitrary::Arbitrary;
use axum::{
    Router,
    body::Body,
    http::{Method, Request, header},
};
use libfuzzer_sys::fuzz_target;
use tokio::runtime::Runtime;
use tower::ServiceExt;
use zylotek_api::{build_state, router::router};
use zylotek_domain::testing::InMemoryWoodRepository;

#[derive(Debug, Arbitrary)]
struct Input {
    method: u8,
    path: PathChoice,
    query: String,
    json: bool,
    body: Vec<u8>,
}

#[derive(Debug, Arbitrary)]
enum PathChoice {
    Collection,
    Health,
    Item(String),
    Any(String),
}

struct Harness {
    runtime: Runtime,
    router: Router,
}

fn harness() -> &'static Harness {
    static HARNESS: OnceLock<Harness> = OnceLock::new();
    HARNESS.get_or_init(|| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let router = runtime
            .block_on(async { router(build_state(Arc::new(InMemoryWoodRepository::new()))) });
        Harness { runtime, router }
    })
}

fuzz_target!(|input: Input| {
    let method = [
        Method::GET,
        Method::POST,
        Method::PUT,
        Method::DELETE,
        Method::PATCH,
        Method::OPTIONS,
    ][input.method as usize % 6]
        .clone();
    let path = match &input.path {
        PathChoice::Collection => "/api/v1/woods".to_owned(),
        PathChoice::Health => "/health".to_owned(),
        PathChoice::Item(id) => format!("/api/v1/woods/{}", id.replace(['/', '?', '#', ' '], "_")),
        PathChoice::Any(path) => format!("/{}", path.replace(['?', '#', ' ', '\n', '\r'], "_")),
    };
    let query: String = input
        .query
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || "=&_-".contains(*c))
        .collect();
    let uri = if query.is_empty() {
        path
    } else {
        format!("{path}?{query}")
    };

    let mut builder = Request::builder().method(method).uri(&uri);
    if input.json {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
    }
    let Ok(request) = builder.body(Body::from(input.body)) else {
        return;
    };

    let harness = harness();
    let response = harness
        .runtime
        .block_on(harness.router.clone().oneshot(request))
        .unwrap();

    assert!(
        !response.status().is_server_error(),
        "{uri} -> {}",
        response.status()
    );
});
