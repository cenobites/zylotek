/// Why OAuth could not be set up or a client could not be registered.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("invalid signing key: {0}")]
    SigningKey(String),

    #[error("invalid client id {0:?}: use 1 to 64 letters, digits, '.', '_' or '-'")]
    InvalidClientId(String),

    #[error("invalid scope {0:?}: use lowercase letters, digits, ':', '.', '_' or '-'")]
    InvalidScope(String),

    #[error("a client needs at least one scope")]
    NoScopes,

    #[error("client {0:?} already exists")]
    DuplicateClient(String),

    #[error("invalid clients file {path}: {reason}")]
    ClientsFile { path: String, reason: String },

    #[error("{path}: {reason}")]
    Io { path: String, reason: String },

    #[error("{0} already exists; refusing to overwrite it")]
    AlreadyExists(String),

    #[error(
        "OAuth is only half configured: {found} exists but {missing} does not. \
         Create it, or remove {found} to leave write access disabled"
    )]
    HalfConfigured { found: String, missing: String },

    #[error("the system could not provide random bytes")]
    Random,

    #[error("could not hash the client secret")]
    Hash,
}
