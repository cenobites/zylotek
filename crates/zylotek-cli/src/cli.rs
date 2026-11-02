use clap::{Parser, Subcommand};
use zylotek_infrastructure::persistence::DEFAULT_DATABASE_URL;

use crate::commands::{api::ApiCommand, db::DbCommand, sync::SyncCommand};

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Cli {
    /// Database that `sync` writes to and `api` serves from (`sqlite::memory:` for a throwaway one)
    #[arg(
        long,
        global = true,
        env = "ZYLOTEK_DATABASE_URL",
        default_value = DEFAULT_DATABASE_URL
    )]
    pub database_url: String,

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
    use crate::commands::sync::{DEFAULT_FILE, Source};

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("zylotek").chain(args.iter().copied()))
    }

    fn sync(source: Source, file: &str) -> Commands {
        Commands::Sync(SyncCommand::Run {
            config: None,
            source,
            file: file.into(),
        })
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
            (&["api", "run"], Commands::Api(ApiCommand::Run)),
            (&["sync", "run"], sync(Source::File, DEFAULT_FILE)),
        ];

        for (args, expected) in cases {
            assert_eq!(parse(args).unwrap().command, Some(expected), "{args:?}");
        }
    }

    #[test]
    fn sync_reads_the_bundled_file_unless_told_otherwise() {
        let cases = [
            (&["sync", "run"][..], sync(Source::File, DEFAULT_FILE)),
            (
                &["sync", "run", "--source", "web"],
                sync(Source::Web, DEFAULT_FILE),
            ),
            (
                &["sync", "run", "-s", "file"],
                sync(Source::File, DEFAULT_FILE),
            ),
            (
                &["sync", "run", "--file", "woods.csv"],
                sync(Source::File, "woods.csv"),
            ),
            (
                &["sync", "run", "-s", "web", "-f", "woods.csv"],
                sync(Source::Web, "woods.csv"),
            ),
            (
                &["sync", "run", "--config", "lpf.toml"],
                Commands::Sync(SyncCommand::Run {
                    config: Some("lpf.toml".into()),
                    source: Source::File,
                    file: DEFAULT_FILE.into(),
                }),
            ),
        ];

        for (args, expected) in cases {
            assert_eq!(parse(args).unwrap().command, Some(expected), "{args:?}");
        }
    }

    #[test]
    fn the_database_url_defaults_to_a_file_and_can_be_set_anywhere() {
        assert_eq!(parse(&[]).unwrap().database_url, DEFAULT_DATABASE_URL);
        assert_eq!(
            parse(&["--database-url", "sqlite::memory:", "api", "run"])
                .unwrap()
                .database_url,
            "sqlite::memory:"
        );
        assert_eq!(
            parse(&["sync", "run", "--database-url", "sqlite:x.db"])
                .unwrap()
                .database_url,
            "sqlite:x.db"
        );
    }

    #[test]
    fn rejects_unknown_input() {
        for args in [
            &["bogus"][..],
            &["db"],
            &["db", "drop"],
            &["sync", "run", "--nope"],
            &["sync", "run", "--source", "ftp"],
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
