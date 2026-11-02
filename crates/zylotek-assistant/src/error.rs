use thiserror::Error;

/// Longest question accepted, in characters.
pub const MAX_QUESTION_CHARS: usize = 1000;

#[derive(Debug, Error)]
pub enum ModelError {
    #[error("could not reach the model server: {0}")]
    Transport(String),
    #[error("the model server answered {status}: {body}")]
    Status { status: u16, body: String },
    #[error("the model server sent a reply that is not a chat completion: {0}")]
    Malformed(String),
    #[error("the model returned no text")]
    Empty,
}

#[derive(Debug, Error)]
pub enum AskError {
    #[error("the question is empty")]
    EmptyQuestion,
    #[error("the question is longer than {MAX_QUESTION_CHARS} characters")]
    QuestionTooLong,
    #[error("the wood catalogue has not been synced from the API yet")]
    EmptyCatalogue,
    #[error("the model is unavailable: {0}")]
    Model(#[from] ModelError),
    #[error("the model's reading of the question is unusable: {0}")]
    Understanding(String),
}

#[derive(Debug, Error)]
pub enum SyncError {
    #[error("could not reach the API: {0}")]
    Transport(String),
    #[error("the API answered {status} for {url}")]
    Status { status: u16, url: String },
    #[error("the API sent a page that is not a wood listing: {0}")]
    Malformed(String),
}
