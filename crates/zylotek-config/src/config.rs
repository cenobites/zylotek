//! Configuration for every Zylotek binary, read from one TOML file.
//!
//! The file is `.env.toml`, or `.env.<ZYLOTEK_ENV>.toml` when `ZYLOTEK_ENV` is set, or whatever
//! path the caller passes. Every section and key is optional, so a binary only needs the part it
//! uses, and a missing default file is the same as an empty one. An explicit path that does not
//! exist is an error, because the caller asked for that file.
//!
//! ```toml
//! [database]
//! url = "sqlite:zylotek.db"
//!
//! [oauth]
//! signing_key = "zylotek-oauth-key.pem"
//! clients = "zylotek-oauth-clients.json"
//! issuer = "http://localhost:8080"
//!
//! [assistant]
//! model_url = "http://gpu-box:11434"
//! token = "secret"
//! ```
//!
//! Precedence is the caller's job: command-line flags and `ZYLOTEK_*` variables should win over
//! the file, and the file over the built-in defaults.

use std::{
    fs, io,
    net::SocketAddr,
    path::{Path, PathBuf},
};

use serde::Deserialize;

/// The file read when neither an override nor `ZYLOTEK_ENV` is given.
pub const DEFAULT_CONFIG_PATH: &str = ".env.toml";

/// The variable that picks `.env.<name>.toml`.
pub const ENV_VAR: &str = "ZYLOTEK_ENV";

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot read config file {}: {source}", path.display())]
    Read { path: PathBuf, source: io::Error },

    #[error("invalid config file {}: {source}", path.display())]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
}

/// Everything the binaries read from the config file.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Config {
    pub database: DatabaseConfig,
    pub oauth: OAuthConfig,
    pub api: ApiConfig,
    pub assistant: AssistantConfig,
    pub webapp: WebappConfig,
}

/// Shared by `zylotek-api`, `zylotek-cli` (sync) and anything else that opens the database.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct DatabaseConfig {
    /// `None` leaves the default (`DEFAULT_DATABASE_URL`) to the infrastructure crate.
    pub url: Option<String>,
}

/// Shared by `zylotek-api` and the `auth` commands of `zylotek-cli`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct OAuthConfig {
    /// Ed25519 private key (PKCS#8 PEM) that signs access tokens.
    pub signing_key: Option<PathBuf>,
    /// JSON file of the clients allowed to ask for tokens.
    pub clients: Option<PathBuf>,
    /// The `iss` of access tokens; `None` leaves the default to the API crate.
    pub issuer: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ApiConfig {
    /// Base URL clients (the assistant and the web app) reach the API at.
    pub url: String,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            url: "http://localhost:8080".to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct AssistantConfig {
    /// Base URL of the assistant, as the web app reaches it.
    pub url: String,
    /// Address the assistant listens on.
    pub listen: SocketAddr,
    /// Base URL of an OpenAI-compatible model server.
    pub model_url: String,
    /// Model name sent with each request.
    pub model_name: String,
    pub model_api_key: Option<String>,
    /// Bearer token callers must send; the web app sends the same one.
    pub token: Option<String>,
    /// Seconds between catalogue syncs.
    pub refresh_secs: u64,
    /// How many woods an answer lists.
    pub top: u64,
}

impl Default for AssistantConfig {
    fn default() -> Self {
        Self {
            url: "http://localhost:8090".to_owned(),
            listen: SocketAddr::from(([127, 0, 0, 1], 8090)),
            model_url: "http://localhost:11434".to_owned(),
            model_name: "qwen2.5:3b".to_owned(),
            model_api_key: None,
            token: None,
            refresh_secs: 300,
            top: 5,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct WebappConfig {
    /// Address the site listens on.
    pub listen: SocketAddr,
    /// The OAuth client the site uses to open asks on the API (scope `asks:write`).
    pub api_client_id: Option<String>,
    pub api_client_secret: Option<String>,
}

impl Default for WebappConfig {
    fn default() -> Self {
        Self {
            listen: SocketAddr::from(([127, 0, 0, 1], 3000)),
            api_client_id: None,
            api_client_secret: None,
        }
    }
}

/// Resolve config file path from an optional override or ZYLOTEK_ENV.
pub fn resolve_config_path(override_path: Option<&str>) -> String {
    resolve_with_env(override_path, std::env::var(ENV_VAR).ok().as_deref())
}

fn resolve_with_env(override_path: Option<&str>, env: Option<&str>) -> String {
    if let Some(path) = override_path {
        return path.to_string();
    }

    match env {
        None | Some("") => DEFAULT_CONFIG_PATH.to_string(),
        Some(env) => format!(".env.{env}.toml"),
    }
}

impl Config {
    /// Resolve the path (see [`resolve_config_path`]) and load it.
    pub fn load(override_path: Option<&str>) -> Result<Self, ConfigError> {
        let path = resolve_config_path(override_path);
        // Only a file nobody asked for by name may be absent.
        let optional = override_path.is_none();
        Self::from_path(Path::new(&path), optional)
    }

    /// Read `path`; with `optional`, a missing file gives the defaults.
    pub fn from_path(path: &Path, optional: bool) -> Result<Self, ConfigError> {
        match fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).map_err(|source| ConfigError::Parse {
                path: path.to_owned(),
                source,
            }),
            Err(error) if optional && error.kind() == io::ErrorKind::NotFound => {
                Ok(Self::default())
            }
            Err(source) => Err(ConfigError::Read {
                path: path.to_owned(),
                source,
            }),
        }
    }

    pub fn parse(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_override_wins_over_the_environment() {
        assert_eq!(resolve_with_env(Some("x.toml"), Some("prod")), "x.toml");
        assert_eq!(resolve_with_env(Some("x.toml"), None), "x.toml");
    }

    #[test]
    fn the_environment_name_picks_the_file() {
        assert_eq!(resolve_with_env(None, Some("prod")), ".env.prod.toml");
    }

    #[test]
    fn no_or_empty_environment_is_the_default_file() {
        assert_eq!(resolve_with_env(None, None), ".env.toml");
        assert_eq!(resolve_with_env(None, Some("")), ".env.toml");
    }

    #[test]
    fn the_public_resolver_honours_an_override() {
        // Does not touch the process environment, so it is safe to run in parallel.
        assert_eq!(resolve_config_path(Some("a.toml")), "a.toml");
    }

    #[test]
    fn an_empty_file_is_all_defaults() {
        let config = Config::parse("").unwrap();
        assert_eq!(config, Config::default());
        assert_eq!(config.api.url, "http://localhost:8080");
        assert_eq!(config.assistant.listen.to_string(), "127.0.0.1:8090");
        assert_eq!(config.assistant.refresh_secs, 300);
        assert_eq!(config.webapp.listen.to_string(), "127.0.0.1:3000");
        assert_eq!(config.webapp.api_client_id, None);
        assert_eq!(config.database.url, None);
    }

    #[test]
    fn a_section_can_set_some_keys_and_default_the_rest() {
        let config = Config::parse(
            r#"
            [database]
            url = "sqlite::memory:"

            [oauth]
            signing_key = "k.pem"

            [assistant]
            model_url = "http://gpu-box:11434"
            token = "secret"
            listen = "0.0.0.0:9000"
            "#,
        )
        .unwrap();

        assert_eq!(config.database.url.as_deref(), Some("sqlite::memory:"));
        assert_eq!(config.oauth.signing_key, Some(PathBuf::from("k.pem")));
        assert_eq!(config.oauth.clients, None);
        assert_eq!(config.assistant.model_url, "http://gpu-box:11434");
        assert_eq!(config.assistant.token.as_deref(), Some("secret"));
        assert_eq!(config.assistant.listen.to_string(), "0.0.0.0:9000");
        assert_eq!(config.assistant.model_name, "qwen2.5:3b");
    }

    #[test]
    fn the_webapp_can_be_given_its_api_client() {
        let config =
            Config::parse("[webapp]\napi_client_id = \"webapp\"\napi_client_secret = \"s3cret\"")
                .unwrap();

        assert_eq!(config.webapp.api_client_id.as_deref(), Some("webapp"));
        assert_eq!(config.webapp.api_client_secret.as_deref(), Some("s3cret"));
    }

    #[test]
    fn a_misspelt_key_is_an_error_not_silently_ignored() {
        assert!(Config::parse("[database]\nurl_ = \"x\"").is_err());
        assert!(Config::parse("[nope]").is_err());
        assert!(Config::parse("[webapp]\nlisten = \"not an address\"").is_err());
    }

    #[test]
    fn a_missing_default_file_is_the_defaults_but_a_missing_named_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("absent.toml");

        assert_eq!(
            Config::from_path(&missing, true).unwrap(),
            Config::default()
        );
        let error = Config::from_path(&missing, false).unwrap_err();
        assert!(matches!(error, ConfigError::Read { .. }));
        assert!(error.to_string().contains("absent.toml"));
    }

    #[test]
    fn load_reads_the_named_file_and_reports_a_bad_one() {
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("good.toml");
        fs::write(&good, "[api]\nurl = \"http://api:1\"").unwrap();
        let bad = dir.path().join("bad.toml");
        fs::write(&bad, "[api\n").unwrap();

        let config = Config::load(good.to_str()).unwrap();
        assert_eq!(config.api.url, "http://api:1");

        let error = Config::load(bad.to_str()).unwrap_err();
        assert!(matches!(error, ConfigError::Parse { .. }));
        assert!(error.to_string().contains("bad.toml"));

        assert!(Config::load(dir.path().join("none.toml").to_str()).is_err());
    }
}

#[cfg(test)]
mod sample_tests {
    use super::*;

    #[test]
    fn the_sample_file_parses_and_matches_the_defaults() {
        let sample = include_str!("../../../.env.sample.toml");
        let config = Config::parse(sample).unwrap();

        // The sample spells out the SQLite default the infrastructure crate owns.
        assert_eq!(config.database.url.as_deref(), Some("sqlite:zylotek.db"));
        let expected = Config {
            database: config.database.clone(),
            ..Config::default()
        };
        assert_eq!(config, expected);
    }
}
