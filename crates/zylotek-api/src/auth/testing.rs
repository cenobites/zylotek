//! A working `Auth` for tests, benches and fuzz targets.
//!
//! Compiled for this crate's tests and, behind the `testing` feature, for other crates'. It
//! is built once per process from a freshly generated key, so no key is ever committed, and
//! every app in the process shares it: a token from [`write_token`] is accepted by all of them.

use std::sync::OnceLock;

use crate::auth::{
    ASKS_WRITE_SCOPE, Auth, AuthConfig, ClientRegistry, DEFAULT_ISSUER, WRITE_SCOPE,
    generate_signing_key_pem,
};

pub const WRITER: &str = "test-writer";
pub const READER: &str = "test-reader";
/// A client that may open asks, as the web app does.
pub const ASKER: &str = "test-asker";
/// A scope the API does not use, to test that a token with the wrong scope is refused.
pub const READ_SCOPE: &str = "woods:read";

struct Fixture {
    auth: Auth,
    writer_secret: String,
    write_token: String,
    read_token: String,
    asker_secret: String,
    asks_token: String,
}

fn fixture() -> &'static Fixture {
    static FIXTURE: OnceLock<Fixture> = OnceLock::new();

    FIXTURE.get_or_init(|| {
        let mut clients = ClientRegistry::default();
        let writer_secret = clients
            .create(WRITER, &[WRITE_SCOPE.to_owned()])
            .expect("a valid test client");
        clients
            .create(READER, &[READ_SCOPE.to_owned()])
            .expect("a valid test client");
        let asker_secret = clients
            .create(ASKER, &[ASKS_WRITE_SCOPE.to_owned()])
            .expect("a valid test client");
        let writer = clients.get(WRITER).cloned().expect("just created");
        let asker = clients.get(ASKER).cloned().expect("just created");
        let reader = clients.get(READER).cloned().expect("just created");

        let auth = Auth::new(AuthConfig::new(
            generate_signing_key_pem().expect("a key"),
            clients,
            DEFAULT_ISSUER.to_owned(),
        ))
        .expect("a valid configuration");
        let authority = auth.authority().expect("enabled");
        let issue = |client, scope: &str| {
            authority
                .issue(client, &[scope.to_owned()])
                .expect("a signed token")
                .access_token
        };

        Fixture {
            write_token: issue(&writer, WRITE_SCOPE),
            read_token: issue(&reader, READ_SCOPE),
            asks_token: issue(&asker, ASKS_WRITE_SCOPE),
            asker_secret,
            writer_secret,
            auth,
        }
    })
}

/// An enabled `Auth` with a writer client and a reader client.
pub fn auth() -> Auth {
    fixture().auth.clone()
}

/// A valid access token with the write scope.
pub fn write_token() -> &'static str {
    &fixture().write_token
}

/// A valid access token that lacks the write scope.
pub fn read_only_token() -> &'static str {
    &fixture().read_token
}

/// The id and secret of the writer client, for exercising the token endpoint.
pub fn writer_credentials() -> (&'static str, &'static str) {
    (WRITER, &fixture().writer_secret)
}

/// A valid access token with the asks scope only.
pub fn asks_token() -> &'static str {
    &fixture().asks_token
}

/// The id and secret of the asker client, for a service that fetches its own tokens.
pub fn asker_credentials() -> (&'static str, &'static str) {
    (ASKER, &fixture().asker_secret)
}
