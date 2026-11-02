use std::sync::Arc;

use zylotek_domain::woods::{
    entities::{Wood, WoodId},
    repositories::WoodRepository,
};

use crate::woods::errors::WoodUseCaseError;

pub struct GetWood {
    wood_repository: Arc<dyn WoodRepository>,
}

impl GetWood {
    pub fn new(wood_repository: Arc<dyn WoodRepository>) -> Self {
        Self { wood_repository }
    }

    pub async fn execute(&self, id: WoodId) -> Result<Wood, WoodUseCaseError> {
        self.wood_repository
            .find_by_id(id)
            .await
            .map_err(WoodUseCaseError::Persistence)?
            .ok_or(WoodUseCaseError::NotFound(id))
    }
}

#[cfg(test)]
mod tests {
    use zylotek_domain::{
        errors::RepositoryError,
        testing::{Operation, wood},
    };

    use super::*;
    use crate::woods::support::Harness;

    #[tokio::test]
    async fn returns_the_stored_wood() {
        let stored = wood("Aroeira", "Myracrodruon urundeuva");
        let harness = Harness::new([stored.clone()]);

        let found = harness.get.execute(stored.id()).await.unwrap();

        assert_eq!(found.id(), stored.id());
        assert_eq!(found.name(), stored.name());
    }

    #[tokio::test]
    async fn reports_an_unknown_id_as_not_found() {
        let harness = Harness::new([]);
        let id = WoodId::new();

        let error = harness.get.execute(id).await.unwrap_err();

        assert!(matches!(error, WoodUseCaseError::NotFound(missing) if missing == id));
        assert_eq!(error.to_string(), format!("wood {id} not found"));
    }

    #[tokio::test]
    async fn reports_repository_failures_as_persistence() {
        let harness = Harness::failing_on(Operation::FindById, []);

        let error = harness.get.execute(WoodId::new()).await.unwrap_err();

        assert!(matches!(
            error,
            WoodUseCaseError::Persistence(RepositoryError::Unavailable)
        ));
    }
}
