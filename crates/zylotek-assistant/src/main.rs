use std::{net::SocketAddr, process::ExitCode, time::Duration};

use clap::Parser;
use zylotek_assistant::{
    DEFAULT_TOP,
    server::{Config, run},
};

/// Answers questions about which wood to use, from the Zylotek catalogue.
#[derive(Parser)]
#[command(version)]
struct Cli {
    /// Base URL of zylotek-api, where the catalogue is read from.
    #[arg(long, env = "ZYLOTEK_API_URL", default_value = "http://localhost:8080")]
    api_url: String,

    /// Base URL of an OpenAI-compatible model server, such as llama.cpp's llama-server.
    #[arg(
        long,
        env = "ZYLOTEK_MODEL_URL",
        default_value = "http://localhost:8081"
    )]
    model_url: String,

    /// Model name sent with each request (llama-server ignores it).
    #[arg(long, env = "ZYLOTEK_MODEL_NAME", default_value = "local")]
    model_name: String,

    /// API key for the model server, if it wants one.
    #[arg(long, env = "ZYLOTEK_MODEL_API_KEY", hide_env_values = true)]
    model_api_key: Option<String>,

    /// Bearer token callers must send to /v1/ask. Strongly recommended.
    #[arg(long, env = "ZYLOTEK_ASSISTANT_TOKEN", hide_env_values = true)]
    token: Option<String>,

    /// Address to listen on.
    #[arg(
        long,
        env = "ZYLOTEK_ASSISTANT_LISTEN",
        default_value = "127.0.0.1:8090"
    )]
    listen: SocketAddr,

    /// Seconds between catalogue syncs.
    #[arg(long, env = "ZYLOTEK_ASSISTANT_REFRESH_SECS", default_value_t = 300,
        value_parser = clap::value_parser!(u64).range(1..))]
    refresh_secs: u64,

    /// How many woods an answer lists.
    #[arg(long, default_value_t = DEFAULT_TOP as u64,
        value_parser = clap::value_parser!(u64).range(1..=50))]
    top: u64,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    let result = run(Config {
        api_url: cli.api_url,
        model_url: cli.model_url,
        model_name: cli.model_name,
        model_api_key: cli.model_api_key,
        token: cli.token,
        listen: cli.listen,
        refresh: Duration::from_secs(cli.refresh_secs),
        top: cli.top as usize,
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
