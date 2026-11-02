use std::time::Duration;

use jsonwebtoken::{
    Algorithm, Validation, decode, decode_header, errors::ErrorKind, get_current_timestamp,
    jwk::JwkSet,
};
use serde::{Deserialize, Serialize};
use zylotek_contracts::oauth::TokenResponse;

use crate::auth::{
    AUDIENCE,
    clients::{Client, ClientRegistry},
    error::AuthError,
    keys::{ACCESS_TOKEN_TYPE, SigningKeys, random_string},
};

/// The claims of an access token, following RFC 9068.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claims {
    pub iss: String,
    /// The client the token was issued to.
    pub sub: String,
    pub aud: String,
    pub exp: u64,
    pub iat: u64,
    /// Unique per token, for tracing a request back to its token.
    pub jti: String,
    pub client_id: String,
    /// Space-delimited scopes.
    pub scope: String,
}

impl Claims {
    pub fn has_scope(&self, scope: &str) -> bool {
        self.scope.split(' ').any(|granted| granted == scope)
    }
}

/// Why an access token was not accepted. None of these says anything a caller could use to
/// forge a token, so the reason may be shown to them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TokenError {
    #[error("the access token is malformed")]
    Malformed,

    #[error("the token is not an access token")]
    WrongType,

    #[error("the token was signed with an unknown key")]
    UnknownKey,

    #[error("the access token has expired")]
    Expired,

    #[error("the access token is not valid")]
    Invalid,
}

/// Registered clients, the signing key, and the rules for issuing and checking tokens.
pub(crate) struct Authority {
    keys: SigningKeys,
    clients: ClientRegistry,
    issuer: String,
    ttl: Duration,
    leeway: Duration,
}

impl Authority {
    pub fn new(
        keys: SigningKeys,
        clients: ClientRegistry,
        issuer: String,
        ttl: Duration,
        leeway: Duration,
    ) -> Self {
        Self {
            keys,
            clients,
            issuer,
            ttl,
            leeway,
        }
    }

    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    pub fn jwks(&self) -> &JwkSet {
        self.keys.jwks()
    }

    pub fn clients(&self) -> &ClientRegistry {
        &self.clients
    }

    /// Checks the client's secret. This is deliberately slow, so call it off the async threads.
    pub fn authenticate(&self, client_id: &str, secret: &str) -> Option<Client> {
        self.clients.authenticate(client_id, secret).cloned()
    }

    /// Signs a token for `client` carrying `scopes`.
    pub fn issue(&self, client: &Client, scopes: &[String]) -> Result<TokenResponse, AuthError> {
        let now = get_current_timestamp();
        let claims = Claims {
            iss: self.issuer.clone(),
            sub: client.client_id.clone(),
            aud: AUDIENCE.to_owned(),
            exp: now + self.ttl.as_secs(),
            iat: now,
            jti: random_string(16)?,
            client_id: client.client_id.clone(),
            scope: scopes.join(" "),
        };

        Ok(TokenResponse {
            access_token: self.sign(&claims)?,
            token_type: "Bearer".to_owned(),
            expires_in: self.ttl.as_secs(),
            scope: claims.scope,
        })
    }

    pub fn sign(&self, claims: &Claims) -> Result<String, AuthError> {
        self.keys.sign(claims)
    }

    /// Accepts only an EdDSA-signed `at+jwt` from this issuer, for this API, that has not
    /// expired. The algorithm is fixed here rather than read from the token, so a token cannot
    /// pick a weaker one.
    pub fn verify(&self, token: &str) -> Result<Claims, TokenError> {
        let header = decode_header(token).map_err(|_| TokenError::Malformed)?;
        if !header
            .typ
            .as_deref()
            .is_some_and(|typ| typ.eq_ignore_ascii_case(ACCESS_TOKEN_TYPE))
        {
            return Err(TokenError::WrongType);
        }
        let key = header
            .kid
            .as_deref()
            .and_then(|key_id| self.keys.verifying_key(key_id))
            .ok_or(TokenError::UnknownKey)?;

        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.set_issuer(&[&self.issuer]);
        validation.set_audience(&[AUDIENCE]);
        validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
        validation.leeway = self.leeway.as_secs();

        decode::<Claims>(token, &key, &validation)
            .map(|data| data.claims)
            .map_err(|error| match error.kind() {
                ErrorKind::ExpiredSignature => TokenError::Expired,
                ErrorKind::InvalidToken | ErrorKind::Base64(_) | ErrorKind::Json(_) => {
                    TokenError::Malformed
                }
                _ => TokenError::Invalid,
            })
    }
}

#[cfg(test)]
mod tests {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    use jsonwebtoken::{EncodingKey, Header};

    use super::*;
    use crate::auth::keys::generate_signing_key_pem;

    struct Setup {
        authority: Authority,
        pem: String,
        client: Client,
        secret: String,
    }

    fn setup(ttl: Duration, leeway: Duration) -> Setup {
        let mut clients = ClientRegistry::default();
        let secret = clients
            .create(
                "assistant",
                &["woods:write".to_owned(), "woods:read".to_owned()],
            )
            .unwrap();
        let client = clients.get("assistant").cloned().unwrap();
        let pem = generate_signing_key_pem().unwrap();
        let authority = Authority::new(
            SigningKeys::from_pem(&pem).unwrap(),
            clients,
            "https://zylotek.example".to_owned(),
            ttl,
            leeway,
        );

        Setup {
            authority,
            pem,
            client,
            secret,
        }
    }

    fn standard() -> Setup {
        setup(Duration::from_secs(3600), Duration::ZERO)
    }

    fn claims(setup: &Setup) -> Claims {
        let now = get_current_timestamp();
        Claims {
            iss: setup.authority.issuer().to_owned(),
            sub: "assistant".into(),
            aud: AUDIENCE.into(),
            exp: now + 600,
            iat: now,
            jti: "id".into(),
            client_id: "assistant".into(),
            scope: "woods:write".into(),
        }
    }

    fn key_id(setup: &Setup) -> String {
        setup.authority.jwks().keys[0]
            .common
            .key_id
            .clone()
            .unwrap()
    }

    /// A token built by hand, to forge what the issuer would never produce.
    fn forge(pem: &str, key_id: Option<&str>, typ: Option<&str>, claims: &Claims) -> String {
        let mut header = Header::new(Algorithm::EdDSA);
        header.typ = typ.map(str::to_owned);
        header.kid = key_id.map(str::to_owned);

        jsonwebtoken::encode(
            &header,
            claims,
            &EncodingKey::from_ed_pem(pem.as_bytes()).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn an_issued_token_carries_the_rfc_9068_claims_and_verifies() {
        let setup = standard();
        let scopes = vec!["woods:write".to_owned()];

        let issued = setup.authority.issue(&setup.client, &scopes).unwrap();
        let claims = setup.authority.verify(&issued.access_token).unwrap();

        assert_eq!(
            (issued.token_type.as_str(), issued.expires_in),
            ("Bearer", 3600)
        );
        assert_eq!(issued.scope, "woods:write");
        assert_eq!(claims.iss, "https://zylotek.example");
        assert_eq!(
            (claims.sub.as_str(), claims.client_id.as_str()),
            ("assistant", "assistant")
        );
        assert_eq!(claims.aud, AUDIENCE);
        assert_eq!(claims.exp - claims.iat, 3600);
        assert_eq!(claims.scope, "woods:write");
        assert!(claims.has_scope("woods:write"));
    }

    #[test]
    fn every_token_has_its_own_id() {
        let setup = standard();
        let scopes = vec!["woods:write".to_owned()];

        let first = setup.authority.issue(&setup.client, &scopes).unwrap();
        let second = setup.authority.issue(&setup.client, &scopes).unwrap();

        let id = |token: &str| setup.authority.verify(token).unwrap().jti;
        assert_ne!(id(&first.access_token), id(&second.access_token));
    }

    #[test]
    fn scopes_match_whole_words_only() {
        let claims = Claims {
            scope: "woods:write woods:read".into(),
            ..claims(&standard())
        };

        assert!(claims.has_scope("woods:write"));
        assert!(claims.has_scope("woods:read"));
        assert!(!claims.has_scope("woods"));
        assert!(!claims.has_scope("woods:wri"));
        assert!(!claims.has_scope("woods:write2"));
        assert!(!claims.has_scope(""));
    }

    #[test]
    fn the_authority_authenticates_its_clients() {
        let setup = standard();

        assert!(
            setup
                .authority
                .authenticate("assistant", &setup.secret)
                .is_some()
        );
        assert!(setup.authority.authenticate("assistant", "nope").is_none());
        assert_eq!(setup.authority.clients().clients().len(), 1);
    }

    #[test]
    fn a_token_past_its_expiry_is_refused() {
        let setup = standard();
        let now = get_current_timestamp();
        let expired = Claims {
            exp: now - 10,
            iat: now - 3600,
            ..claims(&setup)
        };
        let token = setup.authority.sign(&expired).unwrap();

        assert_eq!(setup.authority.verify(&token), Err(TokenError::Expired));
    }

    #[test]
    fn clock_skew_is_tolerated_only_up_to_the_leeway() {
        let lenient = setup(Duration::from_secs(60), Duration::from_secs(30));
        let now = get_current_timestamp();
        let recent = Claims {
            exp: now - 10,
            ..claims(&lenient)
        };
        let old = Claims {
            exp: now - 120,
            ..claims(&lenient)
        };

        assert!(
            lenient
                .authority
                .verify(&lenient.authority.sign(&recent).unwrap())
                .is_ok()
        );
        assert_eq!(
            lenient
                .authority
                .verify(&lenient.authority.sign(&old).unwrap()),
            Err(TokenError::Expired)
        );
    }

    #[test]
    fn a_token_must_name_this_issuer_and_this_api() {
        let setup = standard();
        let wrong_issuer = Claims {
            iss: "https://evil.example".into(),
            ..claims(&setup)
        };
        let wrong_audience = Claims {
            aud: "another-api".into(),
            ..claims(&setup)
        };

        for forged in [wrong_issuer, wrong_audience] {
            let token = setup.authority.sign(&forged).unwrap();

            assert_eq!(
                setup.authority.verify(&token),
                Err(TokenError::Invalid),
                "{forged:?}"
            );
        }
    }

    #[test]
    fn a_jwt_that_is_not_an_access_token_is_refused_even_when_validly_signed() {
        let setup = standard();
        let kid = key_id(&setup);

        for typ in [None, Some("JWT"), Some("id_token")] {
            let token = forge(&setup.pem, Some(&kid), typ, &claims(&setup));

            assert_eq!(
                setup.authority.verify(&token),
                Err(TokenError::WrongType),
                "{typ:?}"
            );
        }
        let upper = forge(&setup.pem, Some(&kid), Some("AT+JWT"), &claims(&setup));
        assert!(
            setup.authority.verify(&upper).is_ok(),
            "the media type is case-insensitive"
        );
    }

    #[test]
    fn a_token_signed_by_another_key_is_refused_whatever_key_id_it_claims() {
        let setup = standard();
        let attacker = generate_signing_key_pem().unwrap();
        let own_kid = key_id(&setup);

        let unknown_kid = forge(
            &attacker,
            Some("attacker-key"),
            Some("at+jwt"),
            &claims(&setup),
        );
        let stolen_kid = forge(&attacker, Some(&own_kid), Some("at+jwt"), &claims(&setup));
        let no_kid = forge(&attacker, None, Some("at+jwt"), &claims(&setup));

        assert_eq!(
            setup.authority.verify(&unknown_kid),
            Err(TokenError::UnknownKey)
        );
        assert_eq!(
            setup.authority.verify(&stolen_kid),
            Err(TokenError::Invalid)
        );
        assert_eq!(setup.authority.verify(&no_kid), Err(TokenError::UnknownKey));
    }

    #[test]
    fn editing_a_signed_token_breaks_it() {
        let setup = standard();
        let scopes = vec!["woods:read".to_owned()];
        let token = setup
            .authority
            .issue(&setup.client, &scopes)
            .unwrap()
            .access_token;
        let parts: Vec<&str> = token.split('.').collect();
        assert_eq!(parts.len(), 3, "a JWT has three parts");
        let (header, payload, signature) = (parts[0], parts[1], parts[2]);
        let mut edited: Claims =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).unwrap()).unwrap();
        edited.scope = "woods:write".into();
        let forged_payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&edited).unwrap());

        let escalated = format!("{header}.{forged_payload}.{signature}");
        let unsigned = format!("{header}.{payload}.");
        let other_first = if signature.starts_with('A') { "B" } else { "A" };
        let flipped = format!("{header}.{payload}.{other_first}{}", &signature[1..]);

        assert_eq!(setup.authority.verify(&escalated), Err(TokenError::Invalid));
        assert!(setup.authority.verify(&unsigned).is_err());
        assert!(setup.authority.verify(&token).is_ok());
        assert!(setup.authority.verify(&flipped).is_err());
    }

    #[test]
    fn the_algorithm_cannot_be_downgraded_or_swapped() {
        let setup = standard();
        let kid = key_id(&setup);
        let public_key = serde_json::to_value(&setup.authority.jwks().keys[0]).unwrap()["x"]
            .as_str()
            .unwrap()
            .to_owned();

        // HS256 with the public key as the secret: the classic algorithm-confusion attack.
        let mut header = Header::new(Algorithm::HS256);
        header.typ = Some("at+jwt".to_owned());
        header.kid = Some(kid.clone());
        let confused = jsonwebtoken::encode(
            &header,
            &claims(&setup),
            &EncodingKey::from_secret(public_key.as_bytes()),
        )
        .unwrap();
        // "alg":"none" with no signature at all.
        let none = format!(
            "{}.{}.",
            URL_SAFE_NO_PAD.encode(format!(r#"{{"alg":"none","typ":"at+jwt","kid":"{kid}"}}"#)),
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims(&setup)).unwrap()),
        );

        assert!(setup.authority.verify(&confused).is_err());
        assert!(setup.authority.verify(&none).is_err());
    }

    #[test]
    fn a_token_missing_a_required_claim_is_refused() {
        let setup = standard();
        let kid = key_id(&setup);
        let mut payload = serde_json::to_value(claims(&setup)).unwrap();

        for claim in ["exp", "iss", "aud", "sub"] {
            let removed = payload.as_object_mut().unwrap().remove(claim).unwrap();
            let mut header = Header::new(Algorithm::EdDSA);
            header.typ = Some("at+jwt".to_owned());
            header.kid = Some(kid.clone());
            let token = jsonwebtoken::encode(
                &header,
                &payload,
                &EncodingKey::from_ed_pem(setup.pem.as_bytes()).unwrap(),
            )
            .unwrap();

            assert!(setup.authority.verify(&token).is_err(), "{claim}");
            payload[claim] = removed;
        }
    }

    #[test]
    fn rubbish_is_malformed_and_never_panics() {
        let setup = standard();

        for rubbish in [
            "",
            ".",
            "..",
            "a.b.c",
            "not a token",
            "eyJ.eyJ.sig",
            "\u{0}",
            &"x".repeat(10_000),
        ] {
            let error = setup.authority.verify(rubbish).unwrap_err();

            assert_eq!(error, TokenError::Malformed, "{rubbish:.20?}");
        }
    }

    #[test]
    fn every_error_has_a_message_that_gives_nothing_away() {
        for (error, message) in [
            (TokenError::Malformed, "the access token is malformed"),
            (TokenError::WrongType, "the token is not an access token"),
            (
                TokenError::UnknownKey,
                "the token was signed with an unknown key",
            ),
            (TokenError::Expired, "the access token has expired"),
            (TokenError::Invalid, "the access token is not valid"),
        ] {
            assert_eq!(error.to_string(), message);
            assert!(format!("{error:?}").len() > 3);
        }
    }
}
