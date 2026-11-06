use clap::{Parser, Subcommand};
use std::{ffi::OsString, path::PathBuf};

use zylotek_infrastructure::persistence::DEFAULT_DATABASE_URL;

use crate::commands::{
    auth::{AuthCommand, OAuthSettings},
    db::DbCommand,
    sync::SyncCommand,
};

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Cli {
    /// Database that `sync` writes to (`sqlite::memory:` for a throwaway one)
    #[arg(
        long,
        global = true,
        env = "ZYLOTEK_DATABASE_URL",
        default_value = DEFAULT_DATABASE_URL
    )]
    pub database_url: String,

    /// Ed25519 private key (PKCS#8 PEM) that signs access tokens
    /// [default: zylotek-oauth-key.pem, if it exists]
    #[arg(long, global = true, env = "ZYLOTEK_OAUTH_SIGNING_KEY")]
    pub oauth_signing_key: Option<PathBuf>,

    /// JSON file of the clients allowed to ask for tokens
    /// [default: zylotek-oauth-clients.json, if it exists]
    #[arg(long, global = true, env = "ZYLOTEK_OAUTH_CLIENTS")]
    pub oauth_clients: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

impl Cli {
    pub fn oauth(&self) -> OAuthSettings {
        OAuthSettings {
            signing_key: self.oauth_signing_key.clone(),
            clients: self.oauth_clients.clone(),
        }
    }
}

#[derive(Subcommand, Debug, PartialEq, Eq)]
pub enum Commands {
    #[command(subcommand)]
    Db(DbCommand),

    #[command(subcommand)]
    Sync(SyncCommand),

    #[command(subcommand)]
    Auth(AuthCommand),

    /// Any other name runs the `zylotek-<name>` executable (for example `api`, `webapp` or
    /// `assistant`), passing the remaining arguments through
    #[command(external_subcommand)]
    External(Vec<OsString>),
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory, error::ErrorKind};

    use super::*;
    use crate::commands::{
        auth::{AuthCommand, ClientCommand},
        sync::DEFAULT_FILE,
    };
    use zylotek_infrastructure::persistence::DEFAULT_DATABASE_URL;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("zylotek").chain(args.iter().copied()))
    }

    fn sync(file: &str, listing_url: Option<&str>, offline: bool) -> Commands {
        Commands::Sync(SyncCommand::Run {
            config: None,
            file: file.into(),
            listing_url: listing_url.map(str::to_owned),
            offline,
        })
    }

    fn db_setup(database_url: &str) -> Commands {
        Commands::Db(DbCommand::Setup {
            database_url: database_url.into(),
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
            (&["db", "setup"][..], db_setup(DEFAULT_DATABASE_URL)),
            (
                &["db", "migrate"],
                Commands::Db(DbCommand::Migrate {
                    database_url: DEFAULT_DATABASE_URL.into(),
                }),
            ),
            (
                &["db", "setup", "--database-url", "sqlite:other.db"],
                db_setup("sqlite:other.db"),
            ),
            (&["sync", "run"], sync(DEFAULT_FILE, None, false)),
        ];

        for (args, expected) in cases {
            assert_eq!(parse(args).unwrap().command, Some(expected), "{args:?}");
        }
    }

    #[test]
    fn sync_reads_the_bundled_file_and_the_live_site_unless_told_otherwise() {
        let cases = [
            (&["sync", "run"][..], sync(DEFAULT_FILE, None, false)),
            (
                &["sync", "run", "--file", "woods.csv"],
                sync("woods.csv", None, false),
            ),
            (
                &["sync", "run", "--offline", "-f", "woods.csv"],
                sync("woods.csv", None, true),
            ),
            (
                &[
                    "sync",
                    "run",
                    "--listing-url",
                    "http://localhost:1/madeiras",
                ],
                sync(DEFAULT_FILE, Some("http://localhost:1/madeiras"), false),
            ),
            (
                &["sync", "run", "--config", "lpf.toml"],
                Commands::Sync(SyncCommand::Run {
                    config: Some("lpf.toml".into()),
                    file: DEFAULT_FILE.into(),
                    listing_url: None,
                    offline: false,
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
            parse(&["--database-url", "sqlite::memory:", "sync", "run"])
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
            &["db"][..],
            &["db", "drop"],
            &["sync", "run", "--nope"],
            &["sync", "run", "--source", "ftp"],
            &["sync", "run", "--offline", "--listing-url", "http://x/y"],
        ] {
            assert!(parse(args).is_err(), "{args:?}");
        }
    }

    #[test]
    fn unknown_names_are_external_subcommands_with_their_arguments() {
        let command = parse(&["webapp", "--listen", "0.0.0.0:80"])
            .unwrap()
            .command;
        assert_eq!(
            command,
            Some(Commands::External(vec![
                "webapp".into(),
                "--listen".into(),
                "0.0.0.0:80".into()
            ]))
        );
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
        assert!(format!("{:?}", parse(&["sync", "run"]).unwrap()).contains("Sync"));
    }

    #[test]
    fn parses_the_auth_commands() {
        let create = |scopes: &[&str]| {
            Commands::Auth(AuthCommand::Client(ClientCommand::Create {
                client_id: "assistant".into(),
                scopes: scopes.iter().map(|s| (*s).to_owned()).collect(),
            }))
        };
        let cases = [
            (&["auth", "keygen"][..], Commands::Auth(AuthCommand::Keygen)),
            (
                &["auth", "client", "list"],
                Commands::Auth(AuthCommand::Client(ClientCommand::List)),
            ),
            (
                &["auth", "client", "create", "assistant"],
                create(&["woods:write"]),
            ),
            (
                &[
                    "auth",
                    "client",
                    "create",
                    "assistant",
                    "--scope",
                    "asks:write",
                    "-s",
                    "woods:write",
                ],
                create(&["asks:write", "woods:write"]),
            ),
        ];

        for (args, expected) in cases {
            assert_eq!(parse(args).unwrap().command, Some(expected), "{args:?}");
        }
        assert!(parse(&["auth"]).is_err());
        assert!(parse(&["auth", "client", "create"]).is_err());
        assert!(parse(&["auth", "client", "create", "webapp", "-s", "ask:write"]).is_err());
    }

    #[test]
    fn the_oauth_options_are_global_and_have_sensible_defaults() {
        let defaults = parse(&["auth", "keygen"]).unwrap().oauth();
        assert_eq!(defaults.signing_key, None);
        assert_eq!(defaults.clients, None);
        assert_eq!(
            defaults.signing_key_path(),
            PathBuf::from("zylotek-oauth-key.pem")
        );
        assert_eq!(
            defaults.clients_path(),
            PathBuf::from("zylotek-oauth-clients.json")
        );

        let given = parse(&[
            "--oauth-signing-key",
            "k.pem",
            "auth",
            "keygen",
            "--oauth-clients",
            "c.json",
        ])
        .unwrap()
        .oauth();
        assert_eq!(given.signing_key_path(), PathBuf::from("k.pem"));
        assert_eq!(given.clients_path(), PathBuf::from("c.json"));
    }
}
