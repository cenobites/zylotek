use std::sync::Arc;

use zylotek_domain::woods::{
    entities::Wood,
    events::WoodEvent,
    repositories::WoodRepository,
    services::WoodCreationService,
    value_objects::{ScientificName, WoodDetails, WoodName},
};
use zylotek_infrastructure::event_bus::EventPublisher;

use crate::woods::errors::WoodUseCaseError;

pub struct CreateWood {
    event_publisher: Arc<dyn EventPublisher<WoodEvent>>,
    wood_repository: Arc<dyn WoodRepository>,
    wood_creation_service: WoodCreationService,
}

impl CreateWood {
    pub fn new(
        event_publisher: Arc<dyn EventPublisher<WoodEvent>>,
        wood_repository: Arc<dyn WoodRepository>,
        wood_creation_service: WoodCreationService,
    ) -> Self {
        Self {
            event_publisher,
            wood_repository,
            wood_creation_service,
        }
    }

    pub async fn execute(&self, command: CreateWoodCommand) -> Result<Wood, WoodUseCaseError> {
        let name = WoodName::new(command.name)?;
        let scientific_name = ScientificName::new(command.scientific_name)?;

        let (wood, event) = self
            .wood_creation_service
            .create(name, scientific_name, command.details)
            .await?;

        let wood = self
            .wood_repository
            .save(&wood)
            .await
            .map_err(WoodUseCaseError::Persistence)?;

        self.event_publisher
            .publish(event)
            .await
            .map_err(WoodUseCaseError::Event)?;

        Ok(wood)
    }
}

pub struct CreateWoodCommand {
    pub name: String,
    pub scientific_name: String,
    pub details: WoodDetails,
}

#[cfg(test)]
mod tests {
    use zylotek_domain::{
        errors::RepositoryError,
        testing::{Operation, wood},
        woods::errors::WoodError,
    };

    use super::*;
    use crate::woods::support::{Harness, create_command};

    #[tokio::test]
    async fn stores_the_wood_and_publishes_wood_created() {
        let harness = Harness::new([]);

        let created = harness
            .create
            .execute(create_command("  Aroeira ", "Myracrodruon   urundeuva"))
            .await
            .unwrap();

        assert_eq!(created.name().as_str(), "Aroeira");
        assert_eq!(created.scientific_name().as_str(), "Myracrodruon urundeuva");
        assert_eq!(harness.repository.len(), 1);
        assert!(matches!(
            harness.events.events().as_slice(),
            [WoodEvent::WoodCreated { wood_id }] if *wood_id == created.id()
        ));
    }

    #[tokio::test]
    async fn rejects_invalid_names_without_side_effects() {
        let harness = Harness::new([]);

        for (name, scientific_name) in
            [(" ", "Myracrodruon urundeuva"), ("Aroeira", "Myracrodruon")]
        {
            let error = harness
                .create
                .execute(create_command(name, scientific_name))
                .await
                .unwrap_err();

            assert!(matches!(error, WoodUseCaseError::Domain(_)));
        }
        assert!(harness.repository.is_empty());
        assert!(harness.events.events().is_empty());
    }

    #[tokio::test]
    async fn rejects_a_taken_scientific_name() {
        let harness = Harness::new([wood("Aroeira", "Myracrodruon urundeuva")]);

        let error = harness
            .create
            .execute(create_command("Outra", "Myracrodruon urundeuva"))
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            WoodUseCaseError::Domain(WoodError::ScientificNameAlreadyExists { .. })
        ));
        assert_eq!(harness.repository.len(), 1);
        assert!(harness.events.events().is_empty());
    }

    #[tokio::test]
    async fn reports_a_failing_uniqueness_check_as_persistence() {
        let harness = Harness::failing_on(Operation::ExistsByScientificName, []);

        let error = harness
            .create
            .execute(create_command("Aroeira", "Myracrodruon urundeuva"))
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            WoodUseCaseError::Persistence(RepositoryError::Unavailable)
        ));
    }

    #[tokio::test]
    async fn reports_a_failing_save_as_persistence_and_publishes_nothing() {
        let harness = Harness::failing_on(Operation::Save, []);

        let error = harness
            .create
            .execute(create_command("Aroeira", "Myracrodruon urundeuva"))
            .await
            .unwrap_err();

        assert!(matches!(error, WoodUseCaseError::Persistence(_)));
        assert!(harness.events.events().is_empty());
    }

    #[tokio::test]
    async fn reports_a_publish_failure_even_though_the_wood_was_stored() {
        let harness = Harness::without_subscribers([]);

        let error = harness
            .create
            .execute(create_command("Aroeira", "Myracrodruon urundeuva"))
            .await
            .unwrap_err();

        assert!(matches!(error, WoodUseCaseError::Event(_)));
        assert_eq!(harness.repository.len(), 1);
    }
}
