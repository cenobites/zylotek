use zylotek_domain::{
    asks::{entities::AskId, errors::AskError},
    errors::RepositoryError,
};
use zylotek_infrastructure::event_bus::EventPublishError;

#[derive(Debug, thiserror::Error)]
pub enum AskUseCaseError {
    #[error("ask {0} not found")]
    NotFound(AskId),

    #[error(transparent)]
    Domain(#[from] AskError),

    #[error("failed to persist ask")]
    Persistence(#[source] RepositoryError),

    #[error("failed to publish ask event")]
    Event(#[source] EventPublishError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn use_case_errors_describe_themselves() {
        let id = AskId::new();

        assert_eq!(
            AskUseCaseError::NotFound(id).to_string(),
            format!("ask {id} not found")
        );
        assert_eq!(
            AskUseCaseError::Persistence(RepositoryError::Database).to_string(),
            "failed to persist ask"
        );
        assert_eq!(
            AskUseCaseError::Event(EventPublishError::NoSubscribers).to_string(),
            "failed to publish ask event"
        );
        assert_eq!(
            AskUseCaseError::from(AskError::InvalidId("x".into())).to_string(),
            "invalid ask id 'x'"
        );
    }
}
