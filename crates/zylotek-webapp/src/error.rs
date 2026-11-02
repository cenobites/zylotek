use thiserror::Error;

/// A failure talking to the wood API or the assistant.
#[derive(Debug, Error)]
pub enum BackendError {
    #[error("{service} is unreachable: {detail}")]
    Unreachable {
        service: &'static str,
        detail: String,
    },
    #[error("{service} answered {status}: {message}")]
    Status {
        service: &'static str,
        status: u16,
        message: String,
    },
    #[error("{service} sent a reply that could not be read: {detail}")]
    Malformed {
        service: &'static str,
        detail: String,
    },
    #[error("no such wood")]
    NotFound,
}
