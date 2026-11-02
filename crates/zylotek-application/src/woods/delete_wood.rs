use std::sync::Arc;

use zylotek_domain::woods::{entities::WoodId, events::WoodEvent, repositories::WoodRepository};
use zylotek_infrastructure::event_bus::EventPublisher;

use crate::woods::errors::WoodUseCaseError;

pub struct DeleteWood {
    event_publisher: Arc<dyn EventPublisher<WoodEvent>>,
    wood_repository: Arc<dyn WoodRepository>,
}

impl DeleteWood {
    pub fn new(
        event_publisher: Arc<dyn EventPublisher<WoodEvent>>,
        wood_repository: Arc<dyn WoodRepository>,
    ) -> Self {
        Self {
            event_publisher,
            wood_repository,
        }
    }

    pub async fn execute(&self, id: WoodId) -> Result<(), WoodUseCaseError> {
        let wood = self
            .wood_repository
            .find_by_id(id)
            .await
            .map_err(WoodUseCaseError::Persistence)?
            .ok_or(WoodUseCaseError::NotFound(id))?;

        let event = wood.delete();

        self.wood_repository
            .delete(id)
            .await
            .map_err(WoodUseCaseError::Persistence)?;

        self.event_publisher
            .publish(event)
            .await
            .map_err(WoodUseCaseError::Event)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use zylotek_domain::testing::{Operation, wood};

    use super::*;
    use crate::woods::support::Harness;

    #[tokio::test]
    async fn removes_the_wood_and_publishes_wood_deleted() {
        let stored = wood("Aroeira", "Myracrodruon urundeuva");
        let harness = Harness::new([stored.clone()]);

        harness.delete.execute(stored.id()).await.unwrap();

        assert!(harness.repository.is_empty());
        assert!(matches!(
            harness.events.events().as_slice(),
            [WoodEvent::WoodDeleted { wood_id }] if *wood_id == stored.id()
        ));
    }

    #[tokio::test]
    async fn reports_an_unknown_id_as_not_found() {
        let harness = Harness::new([]);

        let error = harness.delete.execute(WoodId::new()).await.unwrap_err();

        assert!(matches!(error, WoodUseCaseError::NotFound(_)));
        assert!(harness.events.events().is_empty());
    }

    #[tokio::test]
    async fn reports_a_failing_lookup_as_persistence() {
        let harness = Harness::failing_on(Operation::FindById, []);

        let error = harness.delete.execute(WoodId::new()).await.unwrap_err();

        assert!(matches!(error, WoodUseCaseError::Persistence(_)));
    }

    #[tokio::test]
    async fn keeps_the_wood_and_stays_silent_when_the_delete_fails() {
        let stored = wood("Aroeira", "Myracrodruon urundeuva");
        let harness = Harness::failing_on(Operation::Delete, [stored.clone()]);

        let error = harness.delete.execute(stored.id()).await.unwrap_err();

        assert!(matches!(error, WoodUseCaseError::Persistence(_)));
        assert_eq!(harness.repository.len(), 1);
        assert!(harness.events.events().is_empty());
    }

    #[tokio::test]
    async fn reports_a_publish_failure_even_though_the_wood_was_deleted() {
        let stored = wood("Aroeira", "Myracrodruon urundeuva");
        let harness = Harness::without_subscribers([stored.clone()]);

        let error = harness.delete.execute(stored.id()).await.unwrap_err();

        assert!(matches!(error, WoodUseCaseError::Event(_)));
        assert!(harness.repository.is_empty());
    }
}
