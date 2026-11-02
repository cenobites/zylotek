//! OAuth 2.0 for the API: a token endpoint that hands machine clients short-lived access
//! tokens (the client-credentials grant), and a guard that makes the write routes demand one.
//!
//! Tokens are EdDSA-signed JWTs (RFC 9068). The public key is published as a JWKS and the
//! verifier looks keys up there, so the built-in issuer can later be replaced by an external
//! authorization server without touching the routes.
//!
//! Reading the API needs no token. With no signing key or clients file configured, the write
//! routes answer `503` rather than staying open.

mod clients;
mod error;
mod guard;
mod keys;
mod oauth;
mod tokens;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

use std::{
    fmt, fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

pub use clients::{Client, ClientRegistry, create_client, list_clients};
pub use error::AuthError;
pub use guard::WriteAccess;
pub use keys::{generate_signing_key_pem, write_new_signing_key};
pub use oauth::{jwks_handler, metadata_handler, token_handler};
pub use tokens::{Claims, TokenError};

use clients::io_error;
use keys::SigningKeys;
use tokens::Authority;

/// The scope that allows creating, replacing and deleting woods.
pub const WRITE_SCOPE: &str = "woods:write";
/// The `aud` claim of every access token: this API.
pub const AUDIENCE: &str = "zylotek-api";
pub const DEFAULT_ISSUER: &str = "http://localhost:8080";
/// Looked for in the working directory when no path is given.
pub const DEFAULT_SIGNING_KEY_FILE: &str = "zylotek-oauth-key.pem";
pub const DEFAULT_CLIENTS_FILE: &str = "zylotek-oauth-clients.json";
pub const DEFAULT_TOKEN_TTL: Duration = Duration::from_secs(3600);
/// Clock skew tolerated when checking `exp`.
pub const DEFAULT_LEEWAY: Duration = Duration::from_secs(30);

pub struct AuthConfig {
    /// Ed25519 private key, PKCS#8 PEM.
    pub signing_key_pem: String,
    pub clients: ClientRegistry,
    /// The `iss` claim and the base of the URLs in the discovery document.
    pub issuer: String,
    pub token_ttl: Duration,
    pub leeway: Duration,
}

impl AuthConfig {
    pub fn new(signing_key_pem: String, clients: ClientRegistry, issuer: String) -> Self {
        Self {
            signing_key_pem,
            clients,
            issuer,
            token_ttl: DEFAULT_TOKEN_TTL,
            leeway: DEFAULT_LEEWAY,
        }
    }
}

/// Cheap to clone: a handle on the token authority, or on nothing when OAuth is not set up.
#[derive(Clone)]
pub struct Auth {
    authority: Option<Arc<Authority>>,
}

impl fmt::Debug for Auth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Auth")
            .field("enabled", &self.is_enabled())
            .finish()
    }
}

impl Auth {
    /// No tokens can be issued or accepted: every write route answers `503`.
    pub fn disabled() -> Self {
        Self { authority: None }
    }

    pub fn new(config: AuthConfig) -> Result<Self, AuthError> {
        let keys = SigningKeys::from_pem(&config.signing_key_pem)?;
        let authority = Authority::new(
            keys,
            config.clients,
            config.issuer.trim_end_matches('/').to_owned(),
            config.token_ttl,
            config.leeway,
        );

        Ok(Self {
            authority: Some(Arc::new(authority)),
        })
    }

    /// Loads the signing key and the clients file. A path left as `None` falls back to the
    /// default file name in the working directory, if that exists. With neither file around,
    /// OAuth stays off; with only one of them, that is a mistake worth stopping for.
    pub fn from_files(
        signing_key: Option<&Path>,
        clients: Option<&Path>,
        issuer: &str,
    ) -> Result<Self, AuthError> {
        let key_path = resolve(signing_key, DEFAULT_SIGNING_KEY_FILE);
        let clients_path = resolve(clients, DEFAULT_CLIENTS_FILE);

        match (key_path, clients_path) {
            (None, None) => Ok(Self::disabled()),
            (Some(key), Some(clients)) => {
                let pem = fs::read_to_string(&key).map_err(|error| io_error(&key, error))?;

                Self::new(AuthConfig::new(
                    pem,
                    ClientRegistry::load(&clients)?,
                    issuer.to_owned(),
                ))
            }
            (Some(found), None) => Err(AuthError::HalfConfigured {
                found: found.display().to_string(),
                missing: DEFAULT_CLIENTS_FILE.to_owned(),
            }),
            (None, Some(found)) => Err(AuthError::HalfConfigured {
                found: found.display().to_string(),
                missing: DEFAULT_SIGNING_KEY_FILE.to_owned(),
            }),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.authority.is_some()
    }

    pub(crate) fn authority(&self) -> Option<&Arc<Authority>> {
        self.authority.as_ref()
    }
}

/// The path given, or the default file name when that exists.
fn resolve(explicit: Option<&Path>, default: &str) -> Option<PathBuf> {
    match explicit {
        Some(path) => Some(path.to_path_buf()),
        None => Some(PathBuf::from(default)).filter(|path| path.exists()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pem() -> String {
        generate_signing_key_pem().unwrap()
    }

    /// A directory with a signing key and a clients file in it.
    fn configured() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let key = dir.path().join("key.pem");
        let clients = dir.path().join("clients.json");
        write_new_signing_key(&key).unwrap();
        create_client(&clients, "assistant", &[WRITE_SCOPE.to_owned()]).unwrap();

        (dir, key, clients)
    }

    #[test]
    fn a_disabled_auth_has_no_authority() {
        let auth = Auth::disabled();

        assert!(!auth.is_enabled());
        assert!(auth.authority().is_none());
        assert_eq!(format!("{auth:?}"), "Auth { enabled: false }");
    }

    #[test]
    fn the_config_defaults_to_hour_long_tokens_and_a_half_minute_of_skew() {
        let config = AuthConfig::new(pem(), ClientRegistry::default(), DEFAULT_ISSUER.to_owned());

        assert_eq!(config.token_ttl, Duration::from_secs(3600));
        assert_eq!(config.leeway, Duration::from_secs(30));
        assert!(Auth::new(config).unwrap().is_enabled());
    }

    #[test]
    fn a_trailing_slash_on_the_issuer_does_not_double_up_in_urls() {
        let config = AuthConfig::new(
            pem(),
            ClientRegistry::default(),
            "https://zylotek.example/".to_owned(),
        );

        let auth = Auth::new(config).unwrap();

        assert_eq!(
            auth.authority().unwrap().issuer(),
            "https://zylotek.example"
        );
        assert_eq!(format!("{auth:?}"), "Auth { enabled: true }");
    }

    #[test]
    fn a_bad_signing_key_stops_the_configuration() {
        let config = AuthConfig::new(
            "nonsense".into(),
            ClientRegistry::default(),
            DEFAULT_ISSUER.to_owned(),
        );

        let error = Auth::new(config).unwrap_err();

        assert!(matches!(error, AuthError::SigningKey(_)));
        assert!(
            error
                .to_string()
                .contains("openssl genpkey -algorithm ed25519")
        );
    }

    #[test]
    fn explicit_files_turn_oauth_on() {
        let (_dir, key, clients) = configured();

        let auth = Auth::from_files(Some(&key), Some(&clients), "https://zylotek.example").unwrap();

        assert!(auth.is_enabled());
        assert_eq!(auth.authority().unwrap().clients().clients().len(), 1);
    }

    #[test]
    fn with_no_files_given_or_found_oauth_stays_off() {
        // The test runs in the crate directory, which has no default key or clients file.
        let auth = Auth::from_files(None, None, DEFAULT_ISSUER).unwrap();

        assert!(!auth.is_enabled());
    }

    #[test]
    fn a_key_without_clients_or_clients_without_a_key_is_an_error_worth_stopping_for() {
        let (_dir, key, clients) = configured();

        let key_only = Auth::from_files(Some(&key), None, DEFAULT_ISSUER).unwrap_err();
        let clients_only = Auth::from_files(None, Some(&clients), DEFAULT_ISSUER).unwrap_err();

        assert!(
            matches!(&key_only, AuthError::HalfConfigured { missing, .. } if missing == DEFAULT_CLIENTS_FILE)
        );
        assert!(
            matches!(&clients_only, AuthError::HalfConfigured { missing, .. } if missing == DEFAULT_SIGNING_KEY_FILE)
        );
        assert!(key_only.to_string().contains("only half configured"));
    }

    #[test]
    fn a_named_file_that_is_missing_or_broken_is_an_error_not_a_silent_off_switch() {
        let (dir, key, clients) = configured();
        let missing = dir.path().join("nope");
        let broken = dir.path().join("broken.json");
        std::fs::write(&broken, "[1, 2]").unwrap();
        let bad_key = dir.path().join("bad.pem");
        std::fs::write(&bad_key, "junk").unwrap();

        assert!(matches!(
            Auth::from_files(Some(&missing), Some(&clients), DEFAULT_ISSUER),
            Err(AuthError::Io { .. })
        ));
        assert!(matches!(
            Auth::from_files(Some(&key), Some(&missing), DEFAULT_ISSUER),
            Err(AuthError::Io { .. })
        ));
        assert!(matches!(
            Auth::from_files(Some(&key), Some(&broken), DEFAULT_ISSUER),
            Err(AuthError::ClientsFile { .. })
        ));
        assert!(matches!(
            Auth::from_files(Some(&bad_key), Some(&clients), DEFAULT_ISSUER),
            Err(AuthError::SigningKey(_))
        ));
    }

    #[test]
    fn a_default_file_is_used_only_when_it_exists() {
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("there.json");
        std::fs::write(&existing, "[]").unwrap();
        let existing_name = existing.to_str().unwrap();

        assert_eq!(resolve(None, existing_name), Some(existing.clone()));
        assert_eq!(resolve(None, "definitely-not-a-file.json"), None);
        assert_eq!(
            resolve(Some(Path::new("given.json")), existing_name),
            Some(PathBuf::from("given.json"))
        );
    }

    #[test]
    fn the_default_file_names_are_what_the_docs_and_gitignore_say() {
        assert_eq!(DEFAULT_SIGNING_KEY_FILE, "zylotek-oauth-key.pem");
        assert_eq!(DEFAULT_CLIENTS_FILE, "zylotek-oauth-clients.json");
        assert_eq!(WRITE_SCOPE, "woods:write");
        assert_eq!(AUDIENCE, "zylotek-api");
    }

    #[test]
    fn every_setup_error_says_what_to_do() {
        for error in [
            AuthError::InvalidClientId("bad id".into()),
            AuthError::InvalidScope("BAD".into()),
            AuthError::NoScopes,
            AuthError::DuplicateClient("a".into()),
            AuthError::AlreadyExists("key.pem".into()),
            AuthError::Random,
            AuthError::Hash,
        ] {
            assert!(!error.to_string().is_empty());
            assert!(format!("{error:?}").len() > 3);
        }
    }
}
