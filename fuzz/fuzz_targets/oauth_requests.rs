//! Arbitrary credentials against the OAuth surface: nothing panics, nothing answers with a
//! 5xx, and garbage is never accepted — as a bearer token or at the token endpoint.

#![no_main]

use std::sync::{Arc, OnceLock};

use arbitrary::Arbitrary;
use axum::{
    Router,
    body::Body,
    http::{HeaderValue, Method, Request, header},
};
use libfuzzer_sys::fuzz_target;
use tokio::runtime::Runtime;
use tower::ServiceExt;
use zylotek_api::{auth::testing, build_state, router::router};
use zylotek_domain::testing::{InMemoryAskRepository, InMemoryWoodRepository};

#[derive(Debug, Arbitrary)]
struct Input {
    /// Raw bytes of an `Authorization` header value.
    authorization: Vec<u8>,
    /// Which client authentication to dress the bytes in.
    scheme: Scheme,
    /// The form body of a token request.
    form: Vec<u8>,
    form_content_type: bool,
}

#[derive(Debug, Arbitrary)]
enum Scheme {
    Raw,
    Bearer,
    Basic,
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
        let router = runtime.block_on(async {
            router(build_state(
                Arc::new(InMemoryWoodRepository::new()),
                Arc::new(InMemoryAskRepository::new()),
                testing::auth(),
            ))
        });
        Harness { runtime, router }
    })
}

fuzz_target!(|input: Input| {
    let mut value = match input.scheme {
        Scheme::Raw => Vec::new(),
        Scheme::Bearer => b"Bearer ".to_vec(),
        Scheme::Basic => b"Basic ".to_vec(),
    };
    value.extend_from_slice(&input.authorization);
    let Ok(value) = HeaderValue::from_bytes(&value) else {
        return;
    };

    // As a bearer token, junk never authorizes.
    assert!(
        testing::auth()
            .authorize(Some(&value), "woods:write")
            .is_err(),
        "accepted {value:?}"
    );

    // At the token endpoint, junk credentials never earn a token and never crash the server.
    let content_type = if input.form_content_type {
        "application/x-www-form-urlencoded"
    } else {
        "application/json"
    };
    let request = Request::builder()
        .method(Method::POST)
        .uri("/oauth/token")
        .header(header::CONTENT_TYPE, content_type)
        .header(header::AUTHORIZATION, value)
        .body(Body::from(input.form))
        .unwrap();
    let harness = harness();
    let response = harness
        .runtime
        .block_on(harness.router.clone().oneshot(request))
        .unwrap();

    assert!(
        !response.status().is_server_error(),
        "token endpoint answered {}",
        response.status()
    );
    assert!(
        !response.status().is_success(),
        "token endpoint accepted junk credentials"
    );
});
