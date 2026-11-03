mod cli;
mod commands;

use clap::Parser;

use cli::{Cli, Commands};
use commands::{auth, db, external, sync};
use zylotek_infrastructure::logging::{self, LoggingConfig};

fn main() {
    let cli = Cli::parse();
    let oauth = cli.oauth();

    // Progress and warnings go to stderr, so stdout stays for what a command prints.
    if let Err(err) = logging::init_with_env(LoggingConfig::cli()) {
        eprintln!("ERROR: {err}");
        std::process::exit(1);
    }

    let result = match cli.command {
        Some(Commands::Db(cmd)) => db::execute(&cmd),
        Some(Commands::Sync(cmd)) => sync::execute(&cmd, &cli.database_url),
        Some(Commands::Auth(cmd)) => auth::execute(&cmd, &oauth),
        Some(Commands::External(args)) => external::execute(&args),
        None => {
            let _ = Cli::parse_from(["zylotek", "--help"]);
            Ok(())
        }
    };

    if let Err(err) = result {
        eprintln!("ERROR: {err}");
        std::process::exit(1);
    }
}
