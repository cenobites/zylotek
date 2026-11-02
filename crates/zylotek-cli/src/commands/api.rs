use clap::Subcommand;

use zylotek_api::cli;

use crate::commands::auth::OAuthSettings;

type Error = Box<dyn std::error::Error + Send + Sync>;

#[derive(Subcommand, Debug, PartialEq, Eq, Clone)]
pub enum ApiCommand {
    Run,
}

pub fn execute(cmd: &ApiCommand, database_url: &str, oauth: &OAuthSettings) -> Result<(), Error> {
    match cmd {
        ApiCommand::Run => Ok(cli(database_url, oauth.load()?)?),
    }
}
