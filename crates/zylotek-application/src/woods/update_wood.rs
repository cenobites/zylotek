use std::sync::Arc;

use zylotek_domain::woods::{
    entities::{Wood, WoodId},
    events::WoodEvent,
    repositories::WoodRepository,
    services::WoodUpdateService,
    value_objects::{ScientificName, WoodDetails, WoodName},
};
use zylotek_infrastructure::event_bus::EventPublisher;

use crate::woods::errors::WoodUseCaseError;

pub struct UpdateWood {
    event_publisher: Arc<dyn EventPublisher<WoodEvent>>,
    wood_repository: Arc<dyn WoodRepository>,
    wood_update_service: WoodUpdateService,
}

impl UpdateWood {
    pub fn new(
        event_publisher: Arc<dyn EventPublisher<WoodEvent>>,
        wood_repository: Arc<dyn WoodRepository>,
        wood_update_service: WoodUpdateService,
    ) -> Self {
        Self {
            event_publisher,
            wood_repository,
            wood_update_service,
        }
    }

    /// Replaces every attribute of the wood with the command's values.
    pub async fn execute(&self, command: UpdateWoodCommand) -> Result<Wood, WoodUseCaseError> {
        let name = WoodName::new(command.name)?;
        let scientific_name = ScientificName::new(command.scientific_name)?;

        let mut wood = self
            .wood_repository
            .find_by_id(command.id)
            .await
            .map_err(WoodUseCaseError::Persistence)?
            .ok_or(WoodUseCaseError::NotFound(command.id))?;

        let event = self
            .wood_update_service
            .update(&mut wood, name, scientific_name, command.details)
            .await?;

        let wood = self
            .wood_repository
            .update(&wood)
            .await
            .map_err(WoodUseCaseError::Persistence)?;

        self.event_publisher
            .publish(event)
            .await
            .map_err(WoodUseCaseError::Event)?;

        Ok(wood)
    }
}

pub struct UpdateWoodCommand {
    pub id: WoodId,
    pub name: String,
    pub scientific_name: String,
    pub details: WoodDetails,
}

#[cfg(test)]
mod tests {
    use zylotek_domain::{
        testing::{Operation, wood},
        woods::errors::WoodError,
    };

    use super::*;
    use crate::woods::support::{Harness, update_command};

    #[tokio::test]
    async fn replaces_the_wood_and_publishes_wood_updated() {
        let stored = wood("Aroeira", "Myracrodruon urundeuva");
        let harness = Harness::new([stored.clone()]);

        let updated = harness
            .update
            .execute(update_command(
                stored.id(),
                "Muiracatiara",
                "Astronium lecointei",
            ))
            .await
            .unwrap();

        assert_eq!(updated.id(), stored.id());
        assert_eq!(updated.name().as_str(), "Muiracatiara");
        assert_eq!(updated.scientific_name().as_str(), "Astronium lecointei");
        assert!(matches!(
            harness.events.events().as_slice(),
            [WoodEvent::WoodUpdated { wood_id }] if *wood_id == stored.id()
        ));
    }

    #[tokio::test]
    async fn allows_keeping_the_same_scientific_name() {
        let stored = wood("Aroeira", "Myracrodruon urundeuva");
        let harness = Harness::new([stored.clone()]);

        let updated = harness
            .update
            .execute(update_command(
                stored.id(),
                "Aroeira-do-sertão",
                "Myracrodruon urundeuva",
            ))
            .await
            .unwrap();

        assert_eq!(updated.name().as_str(), "Aroeira-do-sertão");
    }

    #[tokio::test]
    async fn rejects_invalid_names_before_touching_the_repository() {
        let stored = wood("Aroeira", "Myracrodruon urundeuva");
        let harness = Harness::failing_on(Operation::FindById, [stored.clone()]);

        for (name, scientific_name) in [("", "Astronium lecointei"), ("Aroeira", "Astronium")] {
            let error = harness
                .update
                .execute(update_command(stored.id(), name, scientific_name))
                .await
                .unwrap_err();

            assert!(matches!(error, WoodUseCaseError::Domain(_)));
        }
    }

    #[tokio::test]
    async fn reports_an_unknown_id_as_not_found() {
        let harness = Harness::new([]);

        let error = harness
            .update
            .execute(update_command(
                WoodId::new(),
                "Aroeira",
                "Myracrodruon urundeuva",
            ))
            .await
            .unwrap_err();

        assert!(matches!(error, WoodUseCaseError::NotFound(_)));
    }

    #[tokio::test]
    async fn reports_a_failing_lookup_as_persistence() {
        let harness = Harness::failing_on(Operation::FindById, []);

        let error = harness
            .update
            .execute(update_command(
                WoodId::new(),
                "Aroeira",
                "Myracrodruon urundeuva",
            ))
            .await
            .unwrap_err();

        assert!(matches!(error, WoodUseCaseError::Persistence(_)));
    }

    #[tokio::test]
    async fn rejects_a_scientific_name_owned_by_another_wood() {
        let stored = wood("Aroeira", "Myracrodruon urundeuva");
        let other = wood("Muiracatiara", "Astronium lecointei");
        let harness = Harness::new([stored.clone(), other]);

        let error = harness
            .update
            .execute(update_command(
                stored.id(),
                "Aroeira",
                "Astronium lecointei",
            ))
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            WoodUseCaseError::Domain(WoodError::ScientificNameAlreadyExists { .. })
        ));
        assert!(harness.events.events().is_empty());
    }

    #[tokio::test]
    async fn reports_a_failing_write_as_persistence_and_publishes_nothing() {
        let stored = wood("Aroeira", "Myracrodruon urundeuva");
        let harness = Harness::failing_on(Operation::Update, [stored.clone()]);

        let error = harness
            .update
            .execute(update_command(
                stored.id(),
                "Aroeira",
                "Myracrodruon urundeuva",
            ))
            .await
            .unwrap_err();

        assert!(matches!(error, WoodUseCaseError::Persistence(_)));
        assert!(harness.events.events().is_empty());
    }

    #[tokio::test]
    async fn reports_a_publish_failure_even_though_the_wood_was_stored() {
        let stored = wood("Aroeira", "Myracrodruon urundeuva");
        let harness = Harness::without_subscribers([stored.clone()]);

        let error = harness
            .update
            .execute(update_command(
                stored.id(),
                "Nova",
                "Myracrodruon urundeuva",
            ))
            .await
            .unwrap_err();

        assert!(matches!(error, WoodUseCaseError::Event(_)));
    }
}
