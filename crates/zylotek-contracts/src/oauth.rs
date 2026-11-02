//! The OAuth 2.0 token endpoint's wire format (RFC 6749 sections 4.4 and 5).

use serde::{Deserialize, Serialize};

/// The only grant the server supports.
pub const CLIENT_CREDENTIALS: &str = "client_credentials";

/// Form body of `POST /oauth/token`. A client may put `client_id` and `client_secret` here
/// or send them in an HTTP Basic header, but not both.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenRequest {
    pub grant_type: Option<String>,
    /// Space-delimited scopes; omitted means every scope the client may have.
    pub scope: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
}

/// A successful token response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    /// Always `Bearer`.
    pub token_type: String,
    /// Lifetime in seconds.
    pub expires_in: u64,
    /// The scopes actually granted, space-delimited.
    pub scope: String,
}

/// A failed token request, e.g. `invalid_client` or `invalid_scope`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthErrorResponse {
    pub error: String,
    pub error_description: Option<String>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_token_response_has_the_rfc_6749_fields() {
        let wire = json!({
            "access_token": "abc.def.ghi",
            "token_type": "Bearer",
            "expires_in": 3600,
            "scope": "woods:write",
        });

        let response: TokenResponse = serde_json::from_value(wire.clone()).unwrap();

        assert_eq!(serde_json::to_value(&response).unwrap(), wire);
        assert_eq!(response.clone(), response);
        assert!(format!("{response:?}").contains("Bearer"));
    }

    #[test]
    fn a_token_request_is_all_optional_so_the_server_can_report_what_is_missing() {
        let request: TokenRequest = serde_json::from_value(json!({})).unwrap();

        assert_eq!(request, TokenRequest::default());
        let full = TokenRequest {
            grant_type: Some(CLIENT_CREDENTIALS.into()),
            scope: Some("woods:write".into()),
            client_id: Some("assistant".into()),
            client_secret: Some("s3cret".into()),
        };
        assert_eq!(
            serde_json::from_value::<TokenRequest>(serde_json::to_value(&full).unwrap()).unwrap(),
            full
        );
        assert!(format!("{full:?}").contains("assistant"));
    }

    #[test]
    fn an_error_response_carries_a_code_and_an_optional_description() {
        let error: OAuthErrorResponse =
            serde_json::from_value(json!({"error": "invalid_client"})).unwrap();

        assert_eq!(error.error, "invalid_client");
        assert_eq!(error.error_description, None);
        assert_eq!(error.clone(), error);
        assert!(format!("{error:?}").contains("invalid_client"));
    }
}
