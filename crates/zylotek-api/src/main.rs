use std::{path::PathBuf, process::ExitCode};

use clap::Parser;
use zylotek_api::{auth::DEFAULT_ISSUER, serve};
use zylotek_infrastructure::persistence::DEFAULT_DATABASE_URL;

/// The Zylotek HTTP API, on 0.0.0.0:8080.
#[derive(Parser)]
#[command(version)]
struct Args {
    /// Database the API serves from (`sqlite::memory:` for a throwaway one)
    #[arg(long, env = "ZYLOTEK_DATABASE_URL", default_value = DEFAULT_DATABASE_URL)]
    database_url: String,

    /// Ed25519 private key (PKCS#8 PEM) that signs access tokens
    /// [default: zylotek-oauth-key.pem, if it exists]
    #[arg(long, env = "ZYLOTEK_OAUTH_SIGNING_KEY")]
    oauth_signing_key: Option<PathBuf>,

    /// JSON file of the clients allowed to ask for tokens
    /// [default: zylotek-oauth-clients.json, if it exists]
    #[arg(long, env = "ZYLOTEK_OAUTH_CLIENTS")]
    oauth_clients: Option<PathBuf>,

    /// The `iss` of access tokens and the base URL of the OAuth endpoints
    #[arg(long, env = "ZYLOTEK_OAUTH_ISSUER", default_value = DEFAULT_ISSUER)]
    oauth_issuer: String,
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();

    let auth = zylotek_api::auth::Auth::from_files(
        args.oauth_signing_key.as_deref(),
        args.oauth_clients.as_deref(),
        &args.oauth_issuer,
    );
    let result = match auth {
        Ok(auth) => serve(&args.database_url, auth)
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
