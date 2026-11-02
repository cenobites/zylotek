use zylotek_domain::{
    errors::RepositoryError,
    woods::{entities::WoodId, errors::WoodError, services::WoodServiceError},
};
use zylotek_infrastructure::event_bus::EventPublishError;

#[derive(Debug, thiserror::Error)]
pub enum WoodUseCaseError {
    #[error("wood {0} not found")]
    NotFound(WoodId),

    #[error(transparent)]
    Domain(#[from] WoodError),

    #[error("failed to persist wood")]
    Persistence(#[source] RepositoryError),

    #[error("failed to publish wood event")]
    Event(#[source] EventPublishError),
}

impl From<WoodServiceError> for WoodUseCaseError {
    fn from(error: WoodServiceError) -> Self {
        match error {
            WoodServiceError::Domain(error) => Self::Domain(error),
            WoodServiceError::Repository(error) => Self::Persistence(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_errors_keep_their_category() {
        let domain =
            WoodUseCaseError::from(WoodServiceError::Domain(WoodError::InvalidUrl("x".into())));
        let repository =
            WoodUseCaseError::from(WoodServiceError::Repository(RepositoryError::Database));

        assert!(matches!(
            domain,
            WoodUseCaseError::Domain(WoodError::InvalidUrl(_))
        ));
        assert!(matches!(
            repository,
            WoodUseCaseError::Persistence(RepositoryError::Database)
        ));
    }

    #[test]
    fn use_case_errors_describe_themselves() {
        let id = WoodId::new();

        assert_eq!(
            WoodUseCaseError::NotFound(id).to_string(),
            format!("wood {id} not found")
        );
        assert_eq!(
            WoodUseCaseError::Persistence(RepositoryError::Database).to_string(),
            "failed to persist wood"
        );
        assert_eq!(
            WoodUseCaseError::Event(EventPublishError::NoSubscribers).to_string(),
            "failed to publish wood event"
        );
        assert_eq!(
            WoodUseCaseError::from(WoodError::InvalidId("x".into())).to_string(),
            "invalid wood id 'x'"
        );
        assert!(format!("{:?}", WoodUseCaseError::NotFound(id)).contains("NotFound"));
    }
}
