use clap::Subcommand;

use zylotek_api::cli;

type Error = Box<dyn std::error::Error + Send + Sync>;

#[derive(Subcommand, Debug, PartialEq, Eq, Clone)]
pub enum ApiCommand {
    Run,
}

pub fn execute(cmd: &ApiCommand, database_url: &str) -> Result<(), Error> {
    match cmd {
        ApiCommand::Run => Ok(cli(database_url)?),
    }
}
