mod cli;
mod commands;

use clap::Parser;

use cli::{Cli, Commands};
use commands::{api, auth, db, sync};

fn main() {
    let cli = Cli::parse();
    let oauth = cli.oauth();

    let result = match cli.command {
        Some(Commands::Db(cmd)) => db::execute(&cmd),
        Some(Commands::Sync(cmd)) => sync::execute(&cmd, &cli.database_url),
        Some(Commands::Api(cmd)) => api::execute(&cmd, &cli.database_url, &oauth),
        Some(Commands::Auth(cmd)) => auth::execute(&cmd, &oauth),
        _ => {
            let _ = Cli::parse_from(["zylotek", "--help"]);
            Ok(())
        }
    };

    if let Err(err) = result {
        eprintln!("ERROR: {err}");
        std::process::exit(1);
    }
}
