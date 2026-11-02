use clap::{Parser, Subcommand};

use crate::commands::{api::ApiCommand, db::DbCommand, sync::SyncCommand};

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug, PartialEq, Eq)]
pub enum Commands {
    #[command(subcommand)]
    Db(DbCommand),

    #[command(subcommand)]
    Sync(SyncCommand),

    #[command(subcommand)]
    Api(ApiCommand),
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory, error::ErrorKind};

    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("zylotek").chain(args.iter().copied()))
    }

    #[test]
    fn the_command_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn no_subcommand_is_allowed() {
        assert_eq!(parse(&[]).unwrap().command, None);
    }

    #[test]
    fn parses_every_subcommand() {
        let cases = [
            (&["db", "setup"][..], Commands::Db(DbCommand::Setup)),
            (&["db", "migrate"], Commands::Db(DbCommand::Migrate)),
            (
                &["sync", "run"],
                Commands::Sync(SyncCommand::Run { config: None }),
            ),
            (
                &["sync", "run", "--config", "lpf.toml"],
                Commands::Sync(SyncCommand::Run {
                    config: Some("lpf.toml".into()),
                }),
            ),
            (
                &["sync", "run", "-c", "lpf.toml"],
                Commands::Sync(SyncCommand::Run {
                    config: Some("lpf.toml".into()),
                }),
            ),
            (&["api", "run"], Commands::Api(ApiCommand::Run)),
        ];

        for (args, expected) in cases {
            assert_eq!(parse(args).unwrap().command, Some(expected), "{args:?}");
        }
    }

    #[test]
    fn rejects_unknown_input() {
        for args in [
            &["bogus"][..],
            &["db"],
            &["db", "drop"],
            &["sync", "run", "--nope"],
            &["api", "run", "extra"],
        ] {
            assert!(parse(args).is_err(), "{args:?}");
        }
    }

    #[test]
    fn help_and_version_are_available() {
        assert_eq!(
            parse(&["--help"]).unwrap_err().kind(),
            ErrorKind::DisplayHelp
        );
        assert_eq!(
            parse(&["--version"]).unwrap_err().kind(),
            ErrorKind::DisplayVersion
        );
        assert!(format!("{:?}", parse(&["api", "run"]).unwrap()).contains("Api"));
    }
}
