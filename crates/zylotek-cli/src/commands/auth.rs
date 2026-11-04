use std::path::PathBuf;

use clap::Subcommand;
use zylotek_api::auth::{
    ASKS_WRITE_SCOPE, DEFAULT_CLIENTS_FILE, DEFAULT_SIGNING_KEY_FILE, WRITE_SCOPE, create_client,
    list_clients, write_new_signing_key,
};

type Error = Box<dyn std::error::Error + Send + Sync>;

/// Where the OAuth key and clients live.
#[derive(Debug, Clone)]
pub struct OAuthSettings {
    pub signing_key: Option<PathBuf>,
    pub clients: Option<PathBuf>,
}

impl OAuthSettings {
    pub fn signing_key_path(&self) -> PathBuf {
        self.signing_key
            .clone()
            .unwrap_or_else(|| PathBuf::from(DEFAULT_SIGNING_KEY_FILE))
    }

    pub fn clients_path(&self) -> PathBuf {
        self.clients
            .clone()
            .unwrap_or_else(|| PathBuf::from(DEFAULT_CLIENTS_FILE))
    }
}

#[derive(Subcommand, Debug, PartialEq, Eq, Clone)]
pub enum AuthCommand {
    /// Create the key that signs access tokens
    Keygen,

    /// Manage the machines allowed to ask for tokens
    #[command(subcommand)]
    Client(ClientCommand),
}

#[derive(Subcommand, Debug, PartialEq, Eq, Clone)]
pub enum ClientCommand {
    /// Register a client and print its secret, once
    Create {
        /// Letters, digits, '.', '_' or '-'
        client_id: String,

        /// What the client may ask for; repeat for several (the web app needs asks:write)
        #[arg(
            short,
            long = "scope",
            default_value = WRITE_SCOPE,
            value_parser = clap::builder::PossibleValuesParser::new([WRITE_SCOPE, ASKS_WRITE_SCOPE]),
        )]
        scopes: Vec<String>,
    },

    /// List the registered clients
    List,
}

pub fn execute(cmd: &AuthCommand, settings: &OAuthSettings) -> Result<(), Error> {
    match cmd {
        AuthCommand::Keygen => {
            let path = settings.signing_key_path();
            write_new_signing_key(&path)?;

            log::info!("Wrote a new signing key to {}", path.display());
            log::info!("Only you can read it. Back it up: without it, issued tokens stop working.");
        }

        AuthCommand::Client(ClientCommand::Create { client_id, scopes }) => {
            let path = settings.clients_path();
            let secret = create_client(&path, client_id, scopes)?;

            log::info!(
                "Created client {client_id} ({}) in {}",
                scopes.join(" "),
                path.display()
            );
            log::info!("client_secret: {secret}");
            log::info!(
                "Save the secret now. Only its hash is stored, so it cannot be shown again."
            );
        }

        AuthCommand::Client(ClientCommand::List) => {
            let path = settings.clients_path();
            if !path.exists() {
                log::info!("No clients yet ({} does not exist)", path.display());
                return Ok(());
            }
            for client in list_clients(&path)? {
                log::info!("{}\t{}", client.client_id, client.scopes.join(" "));
            }
        }
    }

    Ok(())
}
