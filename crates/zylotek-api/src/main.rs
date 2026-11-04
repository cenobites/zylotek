use std::{path::PathBuf, process::ExitCode};

use clap::Parser;
use zylotek_api::{auth::DEFAULT_ISSUER, serve};
use zylotek_config::Config;
use zylotek_infrastructure::{
    logging::{self, LoggingConfig},
    persistence::DEFAULT_DATABASE_URL,
};

/// The Zylotek HTTP API, on 0.0.0.0:8080.
#[derive(Parser)]
#[command(version)]
struct Args {
    /// Database the API serves from (`sqlite::memory:` for a throwaway one)
    /// [default: sqlite:zylotek.db]
    #[arg(long, env = "ZYLOTEK_DATABASE_URL")]
    database_url: Option<String>,

    /// Ed25519 private key (PKCS#8 PEM) that signs access tokens
    /// [default: zylotek-oauth-key.pem, if it exists]
    #[arg(long, env = "ZYLOTEK_OAUTH_SIGNING_KEY")]
    oauth_signing_key: Option<PathBuf>,

    /// JSON file of the clients allowed to ask for tokens
    /// [default: zylotek-oauth-clients.json, if it exists]
    #[arg(long, env = "ZYLOTEK_OAUTH_CLIENTS")]
    oauth_clients: Option<PathBuf>,

    /// The `iss` of access tokens and the base URL of the OAuth endpoints
    /// [default: http://localhost:8080]
    #[arg(long, env = "ZYLOTEK_OAUTH_ISSUER")]
    oauth_issuer: Option<String>,

    /// TOML config file; flags and `ZYLOTEK_*` variables win over it
    /// [default: .env.toml, or .env.$ZYLOTEK_ENV.toml, if it exists]
    #[arg(long, env = "ZYLOTEK_CONFIG_FILE", value_name = "PATH")]
    config_file: Option<String>,
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();

    let file = match Config::load(args.config_file.as_deref()) {
        Ok(file) => file,
        Err(error) => {
            eprintln!("ERROR: {error}");
            return ExitCode::FAILURE;
        }
    };

    if let Err(error) = logging::init_with_env(LoggingConfig::server()) {
        eprintln!("ERROR: {error}");
        return ExitCode::FAILURE;
    }

    let database_url = args
        .database_url
        .or(file.database.url)
        .unwrap_or_else(|| DEFAULT_DATABASE_URL.to_owned());
    let oauth_signing_key = args.oauth_signing_key.or(file.oauth.signing_key);
    let oauth_clients = args.oauth_clients.or(file.oauth.clients);
    let oauth_issuer = args
        .oauth_issuer
        .or(file.oauth.issuer)
        .unwrap_or_else(|| DEFAULT_ISSUER.to_owned());

    let auth = zylotek_api::auth::Auth::from_files(
        oauth_signing_key.as_deref(),
        oauth_clients.as_deref(),
        &oauth_issuer,
    );
    let result = match auth {
        Ok(auth) => serve(&database_url, auth)
            .await
            .map_err(|error| error.to_string()),
        Err(error) => Err(error.to_string()),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ERROR: {error}");
            ExitCode::FAILURE
        }
    }
}
