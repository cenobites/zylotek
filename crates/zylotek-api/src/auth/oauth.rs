//! The authorization server half: the token endpoint (RFC 6749 section 4.4, client
//! credentials), the key set that verifies its tokens, and the discovery document (RFC 8414).

use axum::{
    Form, Json,
    extract::{State, rejection::FormRejection},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use percent_encoding::percent_decode_str;
use serde::Serialize;
use zylotek_contracts::oauth::{CLIENT_CREDENTIALS, OAuthErrorResponse, TokenRequest};

use crate::{
    auth::{ASKS_WRITE_SCOPE, Client, WRITE_SCOPE},
    errors::ApiError,
    state::AppState,
};

/// A failed token request, rendered the way RFC 6749 section 5.2 asks.
#[derive(Debug)]
pub struct OAuthError {
    status: StatusCode,
    error: &'static str,
    description: String,
    /// Set when the client must be told how to authenticate.
    challenge: Option<&'static str>,
}

impl OAuthError {
    fn new(status: StatusCode, error: &'static str, description: impl Into<String>) -> Self {
        Self {
            status,
            error,
            description: description.into(),
            challenge: None,
        }
    }

    fn invalid_request(description: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_request", description)
    }

    /// Client authentication failed. A client that tried the `Authorization` header gets a
    /// `401` and a `WWW-Authenticate` challenge, as the RFC requires.
    fn invalid_client(used_header: bool) -> Self {
        let mut error = Self::new(
            if used_header {
                StatusCode::UNAUTHORIZED
            } else {
                StatusCode::BAD_REQUEST
            },
            "invalid_client",
            "client authentication failed",
        );
        error.challenge = used_header.then_some("Basic realm=\"zylotek\"");
        error
    }

    fn invalid_scope(description: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_scope", description)
    }

    fn unavailable() -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "temporarily_unavailable",
            "OAuth is not configured on this server",
        )
    }

    fn server_error() -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            "the token could not be issued",
        )
    }
}

impl IntoResponse for OAuthError {
    fn into_response(self) -> Response {
        let body = OAuthErrorResponse {
            error: self.error.to_owned(),
            error_description: Some(self.description),
        };
        let mut response = no_store((self.status, Json(body)).into_response());
        if let Some(challenge) = self.challenge {
            response.headers_mut().insert(
                header::WWW_AUTHENTICATE,
                HeaderValue::from_static(challenge),
            );
        }

        response
    }
}

/// Token responses must not be cached (RFC 6749 section 5.1).
fn no_store(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));

    response
}

/// `POST /oauth/token`
pub async fn token_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    form: Result<Form<TokenRequest>, FormRejection>,
) -> Result<Response, OAuthError> {
    let authority = state
        .auth
        .authority()
        .cloned()
        .ok_or_else(OAuthError::unavailable)?;
    let Form(request) = form.map_err(|_| {
        OAuthError::invalid_request("send application/x-www-form-urlencoded parameters")
    })?;

    match request.grant_type.as_deref() {
        Some(CLIENT_CREDENTIALS) => {}
        Some(_) => {
            return Err(OAuthError::new(
                StatusCode::BAD_REQUEST,
                "unsupported_grant_type",
                "only the client_credentials grant is supported",
            ));
        }
        None => return Err(OAuthError::invalid_request("grant_type is required")),
    }

    let credentials = credentials(&headers, &request)?;

    // Checking a secret is slow on purpose, so keep it off the async worker threads.
    let rejected_id = credentials.client_id.clone();
    let used_header = credentials.used_header;
    let checking = authority.clone();
    let client = tokio::task::spawn_blocking(move || {
        checking.authenticate(&credentials.client_id, &credentials.secret)
    })
    .await
    .map_err(|_| OAuthError::server_error())?
    .ok_or_else(|| {
        log::warn!("token request rejected: bad credentials for client {rejected_id:?}");
        OAuthError::invalid_client(used_header)
    })?;

    let scopes = granted_scopes(&client, request.scope.as_deref())?;
    let token = authority
        .issue(&client, &scopes)
        .map_err(|_| OAuthError::server_error())?;

    Ok(no_store(Json(token).into_response()))
}

struct Credentials {
    client_id: String,
    secret: String,
    used_header: bool,
}

/// The client's id and secret from an HTTP Basic header or from the form, but not both.
fn credentials(headers: &HeaderMap, request: &TokenRequest) -> Result<Credentials, OAuthError> {
    let in_form = request.client_id.is_some() || request.client_secret.is_some();

    match (headers.get(header::AUTHORIZATION), in_form) {
        (Some(_), true) => Err(OAuthError::invalid_request(
            "authenticate with the Authorization header or the form, not both",
        )),
        (Some(value), false) => {
            basic_credentials(value).ok_or_else(|| OAuthError::invalid_client(true))
        }
        (None, true) => match (&request.client_id, &request.client_secret) {
            (Some(client_id), Some(secret)) => Ok(Credentials {
                client_id: client_id.clone(),
                secret: secret.clone(),
                used_header: false,
            }),
            _ => Err(OAuthError::invalid_client(false)),
        },
        (None, false) => Err(OAuthError::invalid_client(true)),
    }
}

/// `Authorization: Basic base64(id:secret)`, where id and secret are form-urlencoded first
/// (RFC 6749 section 2.3.1).
fn basic_credentials(value: &HeaderValue) -> Option<Credentials> {
    let (scheme, encoded) = value.to_str().ok()?.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("basic") {
        return None;
    }
    let decoded = String::from_utf8(STANDARD.decode(encoded.trim()).ok()?).ok()?;
    let (client_id, secret) = decoded.split_once(':')?;
    let decode = |part: &str| {
        percent_decode_str(part)
            .decode_utf8()
            .ok()
            .map(String::from)
    };

    Some(Credentials {
        client_id: decode(client_id)?,
        secret: decode(secret)?,
        used_header: true,
    })
}

/// The scopes to grant: those requested, each of which the client must be allowed, or all of
/// the client's scopes when none are requested.
fn granted_scopes(client: &Client, requested: Option<&str>) -> Result<Vec<String>, OAuthError> {
    let Some(requested) = requested.map(str::trim).filter(|scopes| !scopes.is_empty()) else {
        return Ok(client.scopes.clone());
    };

    let mut granted: Vec<String> = Vec::new();
    for scope in requested.split_whitespace() {
        if !client.scopes.iter().any(|allowed| allowed == scope) {
            return Err(OAuthError::invalid_scope(format!(
                "this client may not request the {scope} scope"
            )));
        }
        if !granted.iter().any(|already| already == scope) {
            granted.push(scope.to_owned());
        }
    }

    Ok(granted)
}

/// `GET /.well-known/jwks.json`: the public keys that verify access tokens.
pub async fn jwks_handler(State(state): State<AppState>) -> Result<Response, ApiError> {
    let authority = state.auth.authority().ok_or_else(not_configured)?;
    let mut response = Json(authority.jwks()).into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=300"),
    );

    Ok(response)
}

/// `GET /.well-known/oauth-authorization-server`: how a client finds the endpoints (RFC 8414).
pub async fn metadata_handler(State(state): State<AppState>) -> Result<Response, ApiError> {
    let authority = state.auth.authority().ok_or_else(not_configured)?;
    let issuer = authority.issuer();
    let mut scopes: Vec<String> = authority
        .clients()
        .clients()
        .iter()
        .flat_map(|client| client.scopes.iter().cloned())
        .chain([WRITE_SCOPE.to_owned(), ASKS_WRITE_SCOPE.to_owned()])
        .collect();
    scopes.sort();
    scopes.dedup();

    Ok(Json(Metadata {
        issuer: issuer.to_owned(),
        token_endpoint: format!("{issuer}/oauth/token"),
        jwks_uri: format!("{issuer}/.well-known/jwks.json"),
        grant_types_supported: [CLIENT_CREDENTIALS],
        token_endpoint_auth_methods_supported: ["client_secret_basic", "client_secret_post"],
        scopes_supported: scopes,
        access_token_signing_alg_values_supported: ["EdDSA"],
    })
    .into_response())
}

fn not_configured() -> ApiError {
    ApiError::ServiceUnavailable("OAuth is not configured".to_owned())
}

#[derive(Serialize)]
struct Metadata {
    issuer: String,
    token_endpoint: String,
    jwks_uri: String,
    grant_types_supported: [&'static str; 1],
    token_endpoint_auth_methods_supported: [&'static str; 2],
    scopes_supported: Vec<String>,
    access_token_signing_alg_values_supported: [&'static str; 1],
}

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{Method, Request},
    };
    use jsonwebtoken::{Algorithm, DecodingKey, Validation, jwk::JwkSet};
    use serde_json::{Value, json};
    use tower::ServiceExt;
    use zylotek_contracts::oauth::TokenResponse;

    use super::*;
    use crate::{
        auth::{AUDIENCE, testing},
        test_support::{app, app_without_auth, send_full, send_request_full, wood_body},
    };

    fn basic(client_id: &str, secret: &str) -> String {
        format!("Basic {}", STANDARD.encode(format!("{client_id}:{secret}")))
    }

    fn writer_basic() -> String {
        let (id, secret) = testing::writer_credentials();

        basic(id, secret)
    }

    async fn post_token(
        router: &axum::Router,
        authorization: Option<String>,
        content_type: &str,
        body: &str,
    ) -> (StatusCode, HeaderMap, Value) {
        let mut request = Request::builder()
            .method(Method::POST)
            .uri("/oauth/token")
            .header(header::CONTENT_TYPE, content_type.to_owned());
        if let Some(authorization) = authorization {
            request = request.header(header::AUTHORIZATION, authorization);
        }

        send_request_full(router, request.body(Body::from(body.to_owned())).unwrap()).await
    }

    async fn form(
        router: &axum::Router,
        authorization: Option<String>,
        body: &str,
    ) -> (StatusCode, HeaderMap, Value) {
        post_token(
            router,
            authorization,
            "application/x-www-form-urlencoded",
            body,
        )
        .await
    }

    fn assert_oauth_error(
        response: &(StatusCode, HeaderMap, Value),
        status: StatusCode,
        error: &str,
    ) {
        assert_eq!(response.0, status, "{:?}", response.2);
        assert_eq!(response.2["error"], error, "{:?}", response.2);
        assert!(response.2["error_description"].is_string());
        assert_eq!(response.1[header::CACHE_CONTROL], "no-store");
        assert_eq!(response.1[header::PRAGMA], "no-cache");
    }

    #[tokio::test]
    async fn the_client_credentials_grant_issues_a_bearer_token_with_basic_authentication() {
        let app = app();

        let (status, headers, body) = form(
            &app.router,
            Some(writer_basic()),
            "grant_type=client_credentials",
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{body}");
        let token: TokenResponse = serde_json::from_value(body).unwrap();
        assert_eq!(
            (token.token_type.as_str(), token.expires_in),
            ("Bearer", 3600)
        );
        assert_eq!(token.scope, "woods:write");
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
        assert_eq!(headers[header::PRAGMA], "no-cache");
        assert_eq!(token.access_token.split('.').count(), 3);
    }

    #[tokio::test]
    async fn credentials_in_the_form_work_too() {
        let (id, secret) = testing::writer_credentials();
        let body = format!("grant_type=client_credentials&client_id={id}&client_secret={secret}");

        let (status, _, body) = form(&app().router, None, &body).await;

        assert_eq!(status, StatusCode::OK, "{body}");
    }

    #[tokio::test]
    async fn an_issued_token_opens_the_write_routes() {
        let app = app();
        let (_, _, issued) = form(
            &app.router,
            Some(writer_basic()),
            "grant_type=client_credentials",
        )
        .await;
        let token = issued["access_token"].as_str().unwrap();

        let (status, _, body) = send_full(
            &app.router,
            Method::POST,
            "/api/v1/woods",
            Some(&wood_body("Aroeira", "Myracrodruon urundeuva")),
            Some(format!("Bearer {token}")),
        )
        .await;

        assert_eq!(status, StatusCode::CREATED, "{body}");
    }

    #[tokio::test]
    async fn the_published_key_set_verifies_tokens_the_way_an_outside_consumer_would() {
        let app = app();
        let (_, _, issued) = form(
            &app.router,
            Some(writer_basic()),
            "grant_type=client_credentials",
        )
        .await;
        let token = issued["access_token"].as_str().unwrap();
        let (status, headers, jwks) = send_full(
            &app.router,
            Method::GET,
            "/.well-known/jwks.json",
            None,
            None,
        )
        .await;
        let (_, _, metadata) = send_full(
            &app.router,
            Method::GET,
            "/.well-known/oauth-authorization-server",
            None,
            None,
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers[header::CACHE_CONTROL], "public, max-age=300");
        let set: JwkSet = serde_json::from_value(jwks.clone()).unwrap();
        let header = jsonwebtoken::decode_header(token).unwrap();
        let jwk = set
            .find(header.kid.as_deref().unwrap())
            .expect("the token's key is published");
        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.set_audience(&[AUDIENCE]);
        validation.set_issuer(&[metadata["issuer"].as_str().unwrap()]);
        let verified =
            jsonwebtoken::decode::<Value>(token, &DecodingKey::from_jwk(jwk).unwrap(), &validation)
                .unwrap();
        assert_eq!(verified.claims["client_id"], testing::WRITER);
        assert_eq!(jwks["keys"][0]["kty"], "OKP");
        assert_eq!(jwks["keys"][0]["crv"], "Ed25519");
        assert_eq!(jwks["keys"][0]["use"], "sig");
        assert!(
            jwks["keys"][0].get("d").is_none(),
            "no private key material: {jwks}"
        );
    }

    #[tokio::test]
    async fn the_discovery_document_points_at_this_servers_endpoints() {
        let (status, _, metadata) = send_full(
            &app().router,
            Method::GET,
            "/.well-known/oauth-authorization-server",
            None,
            None,
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(metadata["issuer"], "http://localhost:8080");
        assert_eq!(
            metadata["token_endpoint"],
            "http://localhost:8080/oauth/token"
        );
        assert_eq!(
            metadata["jwks_uri"],
            "http://localhost:8080/.well-known/jwks.json"
        );
        assert_eq!(
            metadata["grant_types_supported"],
            json!(["client_credentials"])
        );
        assert_eq!(
            metadata["token_endpoint_auth_methods_supported"],
            json!(["client_secret_basic", "client_secret_post"])
        );
        assert_eq!(
            metadata["access_token_signing_alg_values_supported"],
            json!(["EdDSA"])
        );
        assert_eq!(
            metadata["scopes_supported"],
            json!(["asks:write", "woods:read", "woods:write"])
        );
    }

    #[tokio::test]
    async fn a_client_may_not_use_both_authentication_methods_or_none() {
        let app = app();
        let (id, secret) = testing::writer_credentials();

        let both = form(
            &app.router,
            Some(writer_basic()),
            &format!("grant_type=client_credentials&client_id={id}&client_secret={secret}"),
        )
        .await;
        let neither = form(&app.router, None, "grant_type=client_credentials").await;

        assert_oauth_error(&both, StatusCode::BAD_REQUEST, "invalid_request");
        assert_oauth_error(&neither, StatusCode::UNAUTHORIZED, "invalid_client");
        assert_eq!(
            neither.1[header::WWW_AUTHENTICATE],
            "Basic realm=\"zylotek\""
        );
    }

    #[tokio::test]
    async fn wrong_credentials_are_invalid_client_and_never_say_which_part_was_wrong() {
        let app = app();
        let (id, secret) = testing::writer_credentials();
        let wrong_secret = basic(id, "wrong");
        let unknown_client = basic("nobody", secret);

        let responses = [
            form(
                &app.router,
                Some(wrong_secret),
                "grant_type=client_credentials",
            )
            .await,
            form(
                &app.router,
                Some(unknown_client),
                "grant_type=client_credentials",
            )
            .await,
        ];

        for response in &responses {
            assert_oauth_error(response, StatusCode::UNAUTHORIZED, "invalid_client");
            assert_eq!(
                response.1[header::WWW_AUTHENTICATE],
                "Basic realm=\"zylotek\""
            );
        }
        assert_eq!(
            responses[0].2, responses[1].2,
            "the two failures look identical"
        );

        let in_form = form(
            &app.router,
            None,
            &format!("grant_type=client_credentials&client_id={id}&client_secret=wrong"),
        )
        .await;
        assert_oauth_error(&in_form, StatusCode::BAD_REQUEST, "invalid_client");
        assert!(in_form.1.get(header::WWW_AUTHENTICATE).is_none());
    }

    #[tokio::test]
    async fn malformed_authentication_is_invalid_client() {
        let app = app();
        let (id, secret) = testing::writer_credentials();
        let incomplete = form(
            &app.router,
            None,
            &format!("grant_type=client_credentials&client_id={id}"),
        )
        .await;
        assert_oauth_error(&incomplete, StatusCode::BAD_REQUEST, "invalid_client");
        let secret_only = form(
            &app.router,
            None,
            &format!("grant_type=client_credentials&client_secret={secret}"),
        )
        .await;
        assert_oauth_error(&secret_only, StatusCode::BAD_REQUEST, "invalid_client");

        for header_value in [
            "Basic !!!not-base64!!!".to_owned(),
            format!("Basic {}", STANDARD.encode("no-colon-here")),
            format!("Basic {}", STANDARD.encode([0xff, 0xfe, b':', b'x'])),
            format!("Basic {}", STANDARD.encode("id:%ff%fe")),
            "Bearer abc".to_owned(),
            "Basic".to_owned(),
        ] {
            let response = form(
                &app.router,
                Some(header_value.clone()),
                "grant_type=client_credentials",
            )
            .await;

            assert_oauth_error(&response, StatusCode::UNAUTHORIZED, "invalid_client");
        }
    }

    #[tokio::test]
    async fn basic_credentials_are_percent_decoded_as_the_rfc_requires() {
        let (id, secret) = testing::writer_credentials();
        let encoded_id: String = id.bytes().map(|byte| format!("%{byte:02X}")).collect();

        let (status, _, body) = form(
            &app().router,
            Some(basic(&encoded_id, secret)),
            "grant_type=client_credentials",
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{body}");
    }

    #[tokio::test]
    async fn the_grant_type_must_be_client_credentials() {
        let app = app();

        let missing = form(&app.router, Some(writer_basic()), "scope=woods:write").await;
        let password = form(
            &app.router,
            Some(writer_basic()),
            "grant_type=password&username=a&password=b",
        )
        .await;
        let refresh = form(
            &app.router,
            Some(writer_basic()),
            "grant_type=refresh_token&refresh_token=x",
        )
        .await;

        assert_oauth_error(&missing, StatusCode::BAD_REQUEST, "invalid_request");
        assert_oauth_error(&password, StatusCode::BAD_REQUEST, "unsupported_grant_type");
        assert_oauth_error(&refresh, StatusCode::BAD_REQUEST, "unsupported_grant_type");
    }

    #[tokio::test]
    async fn the_request_must_be_a_form() {
        let app = app();

        let json = post_token(
            &app.router,
            Some(writer_basic()),
            "application/json",
            r#"{"grant_type":"client_credentials"}"#,
        )
        .await;
        let plain = post_token(
            &app.router,
            Some(writer_basic()),
            "text/plain",
            "grant_type=client_credentials",
        )
        .await;

        assert_oauth_error(&json, StatusCode::BAD_REQUEST, "invalid_request");
        assert_oauth_error(&plain, StatusCode::BAD_REQUEST, "invalid_request");
    }

    #[tokio::test]
    async fn the_token_endpoint_accepts_only_post() {
        let (status, _, _) =
            send_full(&app().router, Method::GET, "/oauth/token", None, None).await;

        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    }

    #[tokio::test]
    async fn requested_scopes_must_be_ones_the_client_has() {
        let app = app();
        let grant = |scope: &'static str| {
            let router = app.router.clone();
            async move {
                form(
                    &router,
                    Some(writer_basic()),
                    &format!("grant_type=client_credentials&scope={scope}"),
                )
                .await
            }
        };

        let exact = grant("woods:write").await;
        let spaced = grant("++woods:write++woods:write").await;
        let empty = grant("").await;
        let more = grant("woods:write+woods:read").await;
        let other = grant("woods:read").await;
        let admin = grant("admin").await;

        assert_eq!(exact.2["scope"], "woods:write");
        assert_eq!(spaced.2["scope"], "woods:write", "duplicates collapse");
        assert_eq!(
            empty.2["scope"], "woods:write",
            "an empty request means everything allowed"
        );
        assert_oauth_error(&more, StatusCode::BAD_REQUEST, "invalid_scope");
        assert_oauth_error(&other, StatusCode::BAD_REQUEST, "invalid_scope");
        assert_oauth_error(&admin, StatusCode::BAD_REQUEST, "invalid_scope");
    }

    #[tokio::test]
    async fn a_client_cannot_ask_for_a_scope_it_does_not_have() {
        // The writer is registered with woods:write only: what it asks for cannot widen that.
        let secret = testing::writer_credentials().1;

        let response = form(
            &app().router,
            Some(basic(testing::WRITER, secret)),
            "grant_type=client_credentials&scope=woods:read",
        )
        .await;

        assert_oauth_error(&response, StatusCode::BAD_REQUEST, "invalid_scope");
    }

    #[tokio::test]
    async fn unconfigured_oauth_answers_503_everywhere_it_is_asked() {
        let app = app_without_auth();

        let token = form(
            &app.router,
            Some(writer_basic()),
            "grant_type=client_credentials",
        )
        .await;
        let (jwks, _, _) = send_full(
            &app.router,
            Method::GET,
            "/.well-known/jwks.json",
            None,
            None,
        )
        .await;
        let (metadata, _, _) = send_full(
            &app.router,
            Method::GET,
            "/.well-known/oauth-authorization-server",
            None,
            None,
        )
        .await;

        assert_oauth_error(
            &token,
            StatusCode::SERVICE_UNAVAILABLE,
            "temporarily_unavailable",
        );
        assert_eq!(jwks, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(metadata, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn the_token_endpoint_serves_many_clients_at_once() {
        let app = app();

        let calls: Vec<_> = (0..16)
            .map(|_| {
                let router = app.router.clone();
                tokio::spawn(async move {
                    form(
                        &router,
                        Some(writer_basic()),
                        "grant_type=client_credentials",
                    )
                    .await
                    .0
                })
            })
            .collect();

        for call in calls {
            assert_eq!(call.await.unwrap(), StatusCode::OK);
        }
    }

    #[tokio::test]
    async fn an_internal_failure_is_reported_as_server_error_without_details() {
        let response = OAuthError::server_error().into_response();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"], "server_error");
        assert_eq!(body["error_description"], "the token could not be issued");
    }

    #[tokio::test]
    async fn error_bodies_are_debuggable_and_the_router_survives_a_garbage_body() {
        let error = OAuthError::invalid_scope("x");
        assert!(format!("{error:?}").contains("invalid_scope"));

        let request = Request::builder()
            .method(Method::POST)
            .uri("/oauth/token")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(vec![0xff, 0xfe, 0xfd]))
            .unwrap();
        let response = app().router.oneshot(request).await.unwrap();

        assert!(response.status().is_client_error());
    }
}
