use axum::{
    extract::FromRequestParts,
    http::{HeaderValue, header::AUTHORIZATION, request::Parts},
};

use crate::{
    auth::{ASKS_WRITE_SCOPE, Auth, Claims, WRITE_SCOPE},
    errors::ApiError,
    state::AppState,
};

const REALM: &str = "zylotek";

/// Proof that the request carried a valid access token with the write scope.
///
/// Make it the first parameter of every handler that changes data: extractors run in order,
/// so authentication then happens before the body is read, and an unauthenticated caller
/// learns nothing from validation errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteAccess {
    /// The client the token was issued to.
    pub client_id: String,
}

impl FromRequestParts<AppState> for WriteAccess {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let claims = state
            .auth
            .authorize(parts.headers.get(AUTHORIZATION), WRITE_SCOPE)?;

        Ok(Self {
            client_id: claims.client_id,
        })
    }
}

/// Like [`WriteAccess`], for the routes that open asks and record their progress: the token
/// must carry the asks scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskWriteAccess {
    pub client_id: String,
}

impl FromRequestParts<AppState> for AskWriteAccess {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let claims = state
            .auth
            .authorize(parts.headers.get(AUTHORIZATION), ASKS_WRITE_SCOPE)?;

        Ok(Self {
            client_id: claims.client_id,
        })
    }
}

impl Auth {
    /// Checks an `Authorization: Bearer` header for a valid token carrying `scope`, with the
    /// responses RFC 6750 prescribes: `401` for a missing or bad token, `403` for a token that
    /// lacks the scope.
    pub fn authorize(&self, header: Option<&HeaderValue>, scope: &str) -> Result<Claims, ApiError> {
        let authority = self.authority().ok_or_else(|| {
            ApiError::ServiceUnavailable(
                "write access is disabled: OAuth is not configured".to_owned(),
            )
        })?;

        let Some(token) = bearer_token(header) else {
            return Err(ApiError::Unauthorized {
                message: "authentication required".to_owned(),
                challenge: format!("Bearer realm=\"{REALM}\""),
            });
        };

        let claims = authority.verify(token).map_err(|error| ApiError::Unauthorized {
            message: error.to_string(),
            challenge: format!(
                "Bearer realm=\"{REALM}\", error=\"invalid_token\", error_description=\"{error}\""
            ),
        })?;

        if !claims.has_scope(scope) {
            return Err(ApiError::Forbidden {
                message: format!("the access token lacks the {scope} scope"),
                challenge: format!(
                    "Bearer realm=\"{REALM}\", error=\"insufficient_scope\", scope=\"{scope}\""
                ),
            });
        }

        Ok(claims)
    }
}

/// The token of a `Bearer` credential. Any other scheme counts as no credentials at all.
fn bearer_token(header: Option<&HeaderValue>) -> Option<&str> {
    let value = header?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    let token = token.trim();

    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then_some(token)
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, Method, StatusCode, header};
    use serde_json::{Value, json};
    use zylotek_domain::woods::entities::WoodId;

    use super::*;
    use crate::{
        auth::testing,
        test_support::{app, app_without_auth, bearer, send_full, wood_body},
    };

    const WOODS: &str = "/api/v1/woods";

    fn value(text: &str) -> HeaderValue {
        HeaderValue::from_str(text).unwrap()
    }

    fn challenge(error: &ApiError) -> &str {
        error.challenge().expect("a challenge")
    }

    #[test]
    fn bearer_tokens_are_read_from_the_authorization_header() {
        let ok = |text: &str| bearer_token(Some(&value(text))).map(str::to_owned);

        assert_eq!(ok("Bearer abc.def.ghi").as_deref(), Some("abc.def.ghi"));
        assert_eq!(ok("bearer abc").as_deref(), Some("abc"));
        assert_eq!(ok("BEARER abc").as_deref(), Some("abc"));
        assert_eq!(ok("Bearer   abc  ").as_deref(), Some("abc"));
        for not_bearer in [
            "",
            "Bearer",
            "Bearer ",
            "Basic dXNlcjpwYXNz",
            "Token abc",
            "abc",
            "Bearerabc",
        ] {
            assert_eq!(ok(not_bearer), None, "{not_bearer:?}");
        }
        assert_eq!(bearer_token(None), None);
        assert_eq!(
            bearer_token(Some(&HeaderValue::from_bytes(b"Bearer \xff\xfe").unwrap())),
            None
        );
    }

    #[test]
    fn a_valid_write_token_is_accepted_and_names_its_client() {
        let claims = testing::auth()
            .authorize(Some(&value(&bearer(testing::write_token()))), WRITE_SCOPE)
            .unwrap();

        assert_eq!(claims.client_id, testing::WRITER);
        assert!(claims.has_scope(WRITE_SCOPE));
    }

    #[test]
    fn missing_or_foreign_credentials_get_a_bare_challenge_without_an_error_code() {
        let auth = testing::auth();
        let cases = [
            None,
            Some(value("Basic dXNlcjpwYXNz")),
            Some(value("Bearer")),
            Some(value("Bearer   ")),
            Some(HeaderValue::from_bytes(b"Bearer \xff").unwrap()),
        ];

        for header in cases {
            let error = auth.authorize(header.as_ref(), WRITE_SCOPE).unwrap_err();

            assert!(matches!(error, ApiError::Unauthorized { .. }), "{header:?}");
            assert_eq!(challenge(&error), "Bearer realm=\"zylotek\"");
            assert_eq!(error.to_string(), "authentication required");
        }
    }

    #[test]
    fn a_bad_token_is_invalid_token_and_says_why() {
        let auth = testing::auth();

        let junk = auth
            .authorize(Some(&value("Bearer not.a.token")), WRITE_SCOPE)
            .unwrap_err();
        let truncated = {
            let token = testing::write_token();
            auth.authorize(
                Some(&value(&bearer(&token[..token.len() - 3]))),
                WRITE_SCOPE,
            )
            .unwrap_err()
        };

        for error in [junk, truncated] {
            assert!(matches!(error, ApiError::Unauthorized { .. }));
            assert!(
                challenge(&error).starts_with("Bearer realm=\"zylotek\", error=\"invalid_token\"")
            );
            assert!(challenge(&error).contains("error_description="));
        }
    }

    #[test]
    fn a_valid_token_without_the_scope_is_forbidden_and_the_challenge_names_the_scope() {
        let error = testing::auth()
            .authorize(
                Some(&value(&bearer(testing::read_only_token()))),
                WRITE_SCOPE,
            )
            .unwrap_err();

        assert!(matches!(error, ApiError::Forbidden { .. }));
        assert!(challenge(&error).contains("error=\"insufficient_scope\""));
        assert!(challenge(&error).contains("scope=\"woods:write\""));
    }

    #[test]
    fn without_oauth_configured_nothing_can_be_authorized() {
        let error = Auth::disabled()
            .authorize(Some(&value(&bearer(testing::write_token()))), WRITE_SCOPE)
            .unwrap_err();

        assert!(matches!(error, ApiError::ServiceUnavailable(_)));
        assert!(error.to_string().contains("not configured"));
    }

    async fn write_request(
        router: &axum::Router,
        method: Method,
        authorization: Option<String>,
    ) -> (StatusCode, HeaderMap, Value) {
        let id = WoodId::new();
        let (uri, body) = match method {
            Method::POST => (
                WOODS.to_owned(),
                Some(wood_body("Aroeira", "Myracrodruon urundeuva")),
            ),
            Method::PUT => (
                format!("{WOODS}/{id}"),
                Some(wood_body("Aroeira", "Myracrodruon urundeuva")),
            ),
            _ => (format!("{WOODS}/{id}"), None),
        };

        send_full(router, method, &uri, body.as_ref(), authorization).await
    }

    #[tokio::test]
    async fn reads_need_no_token() {
        let app = app();
        let id = WoodId::new();

        for uri in [
            WOODS.to_owned(),
            format!("{WOODS}?limit=5"),
            format!("{WOODS}/{id}"),
            "/health".to_owned(),
        ] {
            let (status, headers, _) = send_full(&app.router, Method::GET, &uri, None, None).await;

            assert_ne!(status, StatusCode::UNAUTHORIZED, "{uri}");
            assert_ne!(status, StatusCode::FORBIDDEN, "{uri}");
            assert!(headers.get(header::WWW_AUTHENTICATE).is_none(), "{uri}");
        }
    }

    #[tokio::test]
    async fn every_write_without_a_token_is_refused_with_a_challenge() {
        let app = app();

        for method in [Method::POST, Method::PUT, Method::DELETE] {
            let (status, headers, body) = write_request(&app.router, method.clone(), None).await;

            assert_eq!(status, StatusCode::UNAUTHORIZED, "{method}");
            assert_eq!(
                headers[header::WWW_AUTHENTICATE],
                "Bearer realm=\"zylotek\""
            );
            assert_eq!(body["code"], "UNAUTHORIZED");
        }
        assert!(app.repository.is_empty(), "nothing was written");
    }

    #[tokio::test]
    async fn bad_credentials_never_reach_the_handlers() {
        let app = app();
        let cases = [
            (
                Some("Bearer garbage".to_owned()),
                StatusCode::UNAUTHORIZED,
                "invalid_token",
            ),
            (
                Some("Basic dXNlcjpwYXNz".to_owned()),
                StatusCode::UNAUTHORIZED,
                "realm",
            ),
            (
                Some(bearer(testing::read_only_token())),
                StatusCode::FORBIDDEN,
                "insufficient_scope",
            ),
        ];

        for method in [Method::POST, Method::PUT, Method::DELETE] {
            for (authorization, expected, fragment) in &cases {
                let (status, headers, _) =
                    write_request(&app.router, method.clone(), authorization.clone()).await;

                assert_eq!(status, *expected, "{method} {authorization:?}");
                let challenge = headers[header::WWW_AUTHENTICATE].to_str().unwrap();
                assert!(challenge.contains(fragment), "{challenge}");
            }
        }
        assert!(app.repository.is_empty());
    }

    #[tokio::test]
    async fn authentication_comes_before_the_body_is_looked_at() {
        let app = app();
        let request = |content_type: &str, body: &str| {
            axum::http::Request::builder()
                .method(Method::POST)
                .uri(WOODS)
                .header(header::CONTENT_TYPE, content_type.to_owned())
                .body(axum::body::Body::from(body.to_owned()))
                .unwrap()
        };

        for (content_type, body) in [
            ("application/json", "{not json"),
            ("application/json", r#"{"name": 7}"#),
            ("text/plain", "hello"),
        ] {
            let (status, _, _) =
                crate::test_support::send_request_full(&app.router, request(content_type, body))
                    .await;

            assert_eq!(
                status,
                StatusCode::UNAUTHORIZED,
                "an anonymous caller learns nothing: {body}"
            );
        }
    }

    #[tokio::test]
    async fn a_token_in_the_query_string_or_the_body_does_not_count() {
        let app = app();
        let uri = format!("{WOODS}?access_token={}", testing::write_token());
        let mut body = wood_body("Aroeira", "Myracrodruon urundeuva");
        body["access_token"] = json!(testing::write_token());

        let (status, _, _) = send_full(&app.router, Method::POST, &uri, Some(&body), None).await;

        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(app.repository.is_empty());
    }

    #[tokio::test]
    async fn a_valid_write_token_does_the_write() {
        let app = app();

        let (status, _, body) = send_full(
            &app.router,
            Method::POST,
            WOODS,
            Some(&wood_body("Aroeira", "Myracrodruon urundeuva")),
            Some(bearer(testing::write_token())),
        )
        .await;

        assert_eq!(status, StatusCode::CREATED, "{body}");
        assert_eq!(app.repository.len(), 1);
    }

    #[tokio::test]
    async fn the_cors_preflight_needs_no_token() {
        let request = axum::http::Request::builder()
            .method(Method::OPTIONS)
            .uri(WOODS)
            .header(header::ORIGIN, "https://example.com")
            .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
            .header(header::ACCESS_CONTROL_REQUEST_HEADERS, "authorization")
            .body(axum::body::Body::empty())
            .unwrap();

        let response = tower::ServiceExt::oneshot(app().router, request)
            .await
            .unwrap();

        assert!(response.status().is_success());
        let allowed = response.headers()[header::ACCESS_CONTROL_ALLOW_HEADERS]
            .to_str()
            .unwrap()
            .to_lowercase();
        assert!(
            allowed.contains("authorization") || allowed == "*",
            "{allowed}"
        );
    }

    #[tokio::test]
    async fn with_oauth_unconfigured_reads_work_and_writes_say_why_they_cannot() {
        let app = app_without_auth();

        let (read, _, _) = send_full(&app.router, Method::GET, WOODS, None, None).await;
        let (write, headers, body) = write_request(
            &app.router,
            Method::POST,
            Some(bearer(testing::write_token())),
        )
        .await;

        assert_eq!(read, StatusCode::OK);
        assert_eq!(
            write,
            StatusCode::SERVICE_UNAVAILABLE,
            "even a token from elsewhere is useless"
        );
        assert!(headers.get(header::WWW_AUTHENTICATE).is_none());
        assert_eq!(body["code"], "SERVICE_UNAVAILABLE");
        assert!(app.repository.is_empty());
    }
}
