use std::{net::SocketAddr, process::ExitCode};

use clap::Parser;
use zylotek_webapp::server::{Config, run};

/// The Zylotek website.
#[derive(Parser)]
#[command(version)]
struct Cli {
    /// Base URL of zylotek-api.
    #[arg(long, env = "ZYLOTEK_API_URL", default_value = "http://localhost:8080")]
    api_url: String,

    /// Base URL of zylotek-assistant.
    #[arg(
        long,
        env = "ZYLOTEK_ASSISTANT_URL",
        default_value = "http://localhost:8090"
    )]
    assistant_url: String,

    /// Bearer token the assistant expects, if it was started with one.
    #[arg(long, env = "ZYLOTEK_ASSISTANT_TOKEN", hide_env_values = true)]
    assistant_token: Option<String>,

    /// Address to listen on.
    #[arg(long, env = "ZYLOTEK_WEBAPP_LISTEN", default_value = "127.0.0.1:3000")]
    listen: SocketAddr,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    let result = run(Config {
        api_url: cli.api_url,
        assistant_url: cli.assistant_url,
        assistant_token: cli.assistant_token,
        listen: cli.listen,
    })
    .await;

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
