use std::sync::Arc;

use zylotek_domain::asks::{
    entities::{Ask, AskId},
    repositories::AskRepository,
};

use crate::asks::errors::AskUseCaseError;

pub struct GetAsk {
    ask_repository: Arc<dyn AskRepository>,
}

impl GetAsk {
    pub fn new(ask_repository: Arc<dyn AskRepository>) -> Self {
        Self { ask_repository }
    }

    pub async fn execute(&self, id: AskId) -> Result<Ask, AskUseCaseError> {
        self.ask_repository
            .find_by_id(id)
            .await
            .map_err(AskUseCaseError::Persistence)?
            .ok_or(AskUseCaseError::NotFound(id))
    }
}

#[cfg(test)]
mod tests {
    use zylotek_domain::{errors::RepositoryError, testing::Operation};

    use super::*;
    use crate::asks::support::{Harness, question};

    #[tokio::test]
    async fn returns_the_stored_ask_or_says_it_is_missing() {
        let harness = Harness::new();
        let created = harness.create.execute(question("a table")).await.unwrap();
        let missing = AskId::new();

        let found = harness.get.execute(created.id()).await.unwrap();
        let error = harness.get.execute(missing).await.unwrap_err();

        assert_eq!(found.id(), created.id());
        assert_eq!(error.to_string(), format!("ask {missing} not found"));
    }

    #[tokio::test]
    async fn a_storage_failure_is_reported() {
        let error = Harness::failing_on(Operation::FindById)
            .get
            .execute(AskId::new())
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            AskUseCaseError::Persistence(RepositoryError::Unavailable)
        ));
    }
}
