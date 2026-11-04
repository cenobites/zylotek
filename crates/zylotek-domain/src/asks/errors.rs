use thiserror::Error;

use crate::{asks::entities::AskState, errors::RepositoryError};

#[derive(Debug, Error)]
pub enum AskError {
    #[error("a question must have between 1 and {max} characters")]
    InvalidQuestion { max: usize },

    #[error("an answer's ranking must not be empty")]
    EmptyRanking,

    #[error("an ask cannot go from {from} to {to}")]
    InvalidTransition { from: AskState, to: AskState },

    #[error("invalid ask id '{0}'")]
    InvalidId(String),

    #[error("invalid ask state '{0}'")]
    InvalidState(String),

    #[error("repository error")]
    Repository(#[from] RepositoryError),
}
