use clap::Subcommand;

use zylotek_infrastructure::persistence::{DEFAULT_DATABASE_URL, cli};

type Error = Box<dyn std::error::Error + Send + Sync>;

#[derive(Subcommand, Debug, PartialEq, Eq, Clone)]
pub enum DbCommand {
    Setup {
        #[arg(
            long,
            global = true,
            env = "ZYLOTEK_DATABASE_URL",
            default_value = DEFAULT_DATABASE_URL
        )]
        database_url: String,
    },
    Migrate {
        #[arg(
            long,
            global = true,
            env = "ZYLOTEK_DATABASE_URL",
            default_value = DEFAULT_DATABASE_URL
        )]
        database_url: String,
    },
}

pub fn execute(cmd: &DbCommand) -> Result<(), Error> {
    match cmd {
        DbCommand::Setup { database_url } => cmd_cli(
            database_url,
            vec!["migration", "generate", "--name", "initial"],
        ),
        DbCommand::Migrate { database_url } => cmd_cli(database_url, vec!["migration", "apply"]),
    }
}

fn cmd_cli(database_url: &str, args: Vec<&str>) -> Result<(), Error> {
    Ok(cli(database_url, args)?)
}
