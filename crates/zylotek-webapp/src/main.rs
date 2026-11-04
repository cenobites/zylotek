use std::{net::SocketAddr, process::ExitCode};

use clap::Parser;
use zylotek_config::Config as FileConfig;
use zylotek_infrastructure::logging::{self, LoggingConfig};
use zylotek_webapp::server::{Config, run};

/// The Zylotek website.
#[derive(Parser)]
#[command(version)]
struct Cli {
    /// Base URL of zylotek-api.
    /// [default: http://localhost:8080]
    #[arg(long, env = "ZYLOTEK_API_URL")]
    api_url: Option<String>,

    /// Base URL of zylotek-assistant.
    /// [default: http://localhost:8090]
    #[arg(long, env = "ZYLOTEK_ASSISTANT_URL")]
    assistant_url: Option<String>,

    /// Bearer token the assistant expects, if it was started with one.
    #[arg(long, env = "ZYLOTEK_ASSISTANT_TOKEN", hide_env_values = true)]
    assistant_token: Option<String>,

    /// Address to listen on.
    /// [default: 127.0.0.1:3000]
    #[arg(long, env = "ZYLOTEK_WEBAPP_LISTEN")]
    listen: Option<SocketAddr>,

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
        assistant_url: cli.assistant_url.unwrap_or(file.assistant.url),
        assistant_token: cli.assistant_token.or(file.assistant.token),
        listen: cli.listen.unwrap_or(file.webapp.listen),
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
