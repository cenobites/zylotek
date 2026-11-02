//! The OAuth 2.0 journey over a real socket and a real database: reads are open, writes need
//! a token, and a token is earned from the token endpoint.

mod common;

use std::time::Duration;

use common::{Server, serve, serve_with, wood_body};
use reqwest::{Client, Response, StatusCode};
use serde_json::Value;
use zylotek_api::auth::{
    Auth, AuthConfig, ClientRegistry, WRITE_SCOPE, generate_signing_key_pem, testing,
};

const FORM: &str = "application/x-www-form-urlencoded";

fn url(server: &Server, path: &str) -> String {
    format!("http://{}{path}", server.address)
}

async fn token(client: &Client, server: &Server, id: &str, secret: &str) -> String {
    let response = client
        .post(url(server, "/oauth/token"))
        .basic_auth(id, Some(secret))
        .header("content-type", FORM)
        .body("grant_type=client_credentials")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = serde_json::from_str(&response.text().await.unwrap()).unwrap();

    body["access_token"].as_str().unwrap().to_owned()
}

async fn create(client: &Client, server: &Server, bearer: Option<&str>) -> Response {
    let mut request = client
        .post(url(server, "/api/v1/woods"))
        .header("content-type", "application/json")
        .body(wood_body("Aroeira", "Myracrodruon urundeuva").to_string());
    if let Some(bearer) = bearer {
        request = request.bearer_auth(bearer);
    }

    request.send().await.unwrap()
}

#[tokio::test]
async fn a_client_earns_a_token_and_writes_while_reads_stay_open() {
    let server = serve().await;
    let client = Client::new();
    let (id, secret) = testing::writer_credentials();

    // No token: reading works, writing is challenged and nothing is stored.
    let listing = client
        .get(url(&server, "/api/v1/woods"))
        .send()
        .await
        .unwrap();
    assert_eq!(listing.status(), StatusCode::OK);
    let anonymous = create(&client, &server, None).await;
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        anonymous.headers()["www-authenticate"],
        "Bearer realm=\"zylotek\""
    );

    // With a token from the endpoint, the write goes through.
    let bearer = token(&client, &server, id, secret).await;
    let created = create(&client, &server, Some(&bearer)).await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let created: Value = serde_json::from_str(&created.text().await.unwrap()).unwrap();
    let wood = url(
        &server,
        &format!("/api/v1/woods/{}", created["id"].as_str().unwrap()),
    );

    // Anyone can read it; only a token holder can delete it.
    assert_eq!(
        client.get(&wood).send().await.unwrap().status(),
        StatusCode::OK
    );
    assert_eq!(
        client.delete(&wood).send().await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        client.get(&wood).send().await.unwrap().status(),
        StatusCode::OK,
        "still there"
    );
    let deleted = client
        .delete(&wood)
        .bearer_auth(&bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        client.get(&wood).send().await.unwrap().status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn a_token_without_the_write_scope_reads_but_cannot_write() {
    let server = serve().await;
    let client = Client::new();

    let refused = create(&client, &server, Some(testing::read_only_token())).await;

    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    let challenge = refused.headers()["www-authenticate"]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(challenge.contains("insufficient_scope"), "{challenge}");
    let read = client
        .get(url(&server, "/api/v1/woods"))
        .bearer_auth(testing::read_only_token())
        .send()
        .await
        .unwrap();
    assert_eq!(read.status(), StatusCode::OK);
}

/// A server whose tokens live one second, and the secret of its only client.
async fn short_lived_server() -> (Server, String) {
    let mut clients = ClientRegistry::default();
    let secret = clients
        .create("importer", &[WRITE_SCOPE.to_owned()])
        .unwrap();
    let mut config = AuthConfig::new(
        generate_signing_key_pem().unwrap(),
        clients,
        "http://localhost:8080".to_owned(),
    );
    config.token_ttl = Duration::from_secs(1);
    config.leeway = Duration::ZERO;

    (serve_with(Auth::new(config).unwrap()).await, secret)
}

#[tokio::test]
async fn a_token_stops_working_when_it_expires() {
    let (server, secret) = short_lived_server().await;
    let client = Client::new();
    let bearer = token(&client, &server, "importer", &secret).await;

    assert_eq!(
        create(&client, &server, Some(&bearer)).await.status(),
        StatusCode::CREATED
    );
    tokio::time::sleep(Duration::from_millis(2200)).await;
    let late = create(&client, &server, Some(&bearer)).await;

    assert_eq!(late.status(), StatusCode::UNAUTHORIZED);
    let challenge = late.headers()["www-authenticate"]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(
        challenge.contains("invalid_token") && challenge.contains("expired"),
        "{challenge}"
    );
    let fresh = token(&client, &server, "importer", &secret).await;
    assert_eq!(
        client
            .delete(url(
                &server,
                "/api/v1/woods/00000000-0000-0000-0000-000000000000"
            ))
            .bearer_auth(&fresh)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND,
        "a fresh token is accepted again"
    );
}

#[tokio::test]
async fn a_token_from_a_different_server_is_refused() {
    let (issuer, secret) = short_lived_server().await;
    let (other, _) = short_lived_server().await;
    let client = Client::new();
    let foreign = token(&client, &issuer, "importer", &secret).await;

    let refused = create(&client, &other, Some(&foreign)).await;

    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn many_clients_can_fetch_tokens_at_once() {
    let server = serve().await;
    let client = Client::new();
    let (id, secret) = testing::writer_credentials();

    let requests: Vec<_> = (0..16)
        .map(|_| {
            let request = client
                .post(url(&server, "/oauth/token"))
                .basic_auth(id, Some(secret))
                .header("content-type", FORM)
                .body("grant_type=client_credentials");
            tokio::spawn(async move { request.send().await.unwrap().status() })
        })
        .collect();

    for request in requests {
        assert_eq!(request.await.unwrap(), StatusCode::OK);
    }
}

#[tokio::test]
async fn a_server_without_oauth_serves_reads_and_refuses_writes() {
    let server = serve_with(Auth::disabled()).await;
    let client = Client::new();

    let read = client
        .get(url(&server, "/api/v1/woods"))
        .send()
        .await
        .unwrap();
    let write = create(&client, &server, Some(testing::write_token())).await;
    let token = client
        .post(url(&server, "/oauth/token"))
        .header("content-type", FORM)
        .body("grant_type=client_credentials")
        .send()
        .await
        .unwrap();

    assert_eq!(read.status(), StatusCode::OK);
    assert_eq!(write.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(token.status(), StatusCode::SERVICE_UNAVAILABLE);
}
