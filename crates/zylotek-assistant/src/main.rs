use std::{net::SocketAddr, process::ExitCode, time::Duration};

use clap::Parser;
use zylotek_assistant::server::{Config, run};
use zylotek_config::Config as FileConfig;
use zylotek_infrastructure::logging::{self, LoggingConfig};

/// Answers questions about which wood to use, from the Zylotek catalogue.
#[derive(Parser)]
#[command(version)]
struct Cli {
    /// Base URL of zylotek-api, where the catalogue is read from.
    /// [default: http://localhost:8080]
    #[arg(long, env = "ZYLOTEK_API_URL")]
    api_url: Option<String>,

    /// Base URL of an OpenAI-compatible model server, such as Ollama.
    /// [default: http://localhost:11434]
    #[arg(long, env = "ZYLOTEK_MODEL_URL")]
    model_url: Option<String>,

    /// Model name sent with each request; Ollama needs one it has pulled.
    /// [default: qwen2.5:3b]
    #[arg(long, env = "ZYLOTEK_MODEL_NAME")]
    model_name: Option<String>,

    /// API key for the model server, if it wants one.
    #[arg(long, env = "ZYLOTEK_MODEL_API_KEY", hide_env_values = true)]
    model_api_key: Option<String>,

    /// Bearer token callers must send to /v1/ask. Strongly recommended.
    #[arg(long, env = "ZYLOTEK_ASSISTANT_TOKEN", hide_env_values = true)]
    token: Option<String>,

    /// Address to listen on.
    /// [default: 127.0.0.1:8090]
    #[arg(long, env = "ZYLOTEK_ASSISTANT_LISTEN")]
    listen: Option<SocketAddr>,

    /// Seconds between catalogue syncs.
    /// [default: 300]
    #[arg(long, env = "ZYLOTEK_ASSISTANT_REFRESH_SECS",
        value_parser = clap::value_parser!(u64).range(1..))]
    refresh_secs: Option<u64>,

    /// How many woods an answer lists.
    /// [default: 5]
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..=50))]
    top: Option<u64>,

    /// TOML config file; flags and `ZYLOTEK_*` variables win over it
    /// [default: .env.toml, or .env.$ZYLOTEK_ENV.toml, if it exists]
    #[arg(long, env = "ZYLOTEK_CONFIG_FILE", value_name = "PATH")]
    config_file: Option<String>,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    let file = match FileConfig::load(cli.config_file.as_deref()) {
        Ok(file) => file,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::FAILURE;
        }
    };

    if let Err(error) = logging::init_with_env(LoggingConfig::server()) {
        eprintln!("error: {error}");
        return ExitCode::FAILURE;
    }

    let result = run(Config {
        api_url: cli.api_url.unwrap_or(file.api.url),
        model_url: cli.model_url.unwrap_or(file.assistant.model_url),
        model_name: cli.model_name.unwrap_or(file.assistant.model_name),
        model_api_key: cli.model_api_key.or(file.assistant.model_api_key),
        token: cli.token.or(file.assistant.token),
        listen: cli.listen.unwrap_or(file.assistant.listen),
        refresh: Duration::from_secs(cli.refresh_secs.unwrap_or(file.assistant.refresh_secs)),
        top: cli.top.unwrap_or(file.assistant.top) as usize,
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
