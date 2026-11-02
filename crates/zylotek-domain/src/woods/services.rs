use std::sync::Arc;

use thiserror::Error;

use crate::{
    errors::RepositoryError,
    woods::{
        entities::Wood,
        errors::WoodError,
        events::WoodEvent,
        repositories::WoodRepository,
        value_objects::{ScientificName, WoodDetails, WoodName},
    },
};

/// Creates woods, enforcing that a scientific name is registered only once.
pub struct WoodCreationService {
    wood_repository: Arc<dyn WoodRepository>,
}

impl WoodCreationService {
    pub fn new(wood_repository: Arc<dyn WoodRepository>) -> Self {
        Self { wood_repository }
    }

    pub async fn create(
        &self,
        name: WoodName,
        scientific_name: ScientificName,
        details: WoodDetails,
    ) -> Result<(Wood, WoodEvent), WoodServiceError> {
        ensure_scientific_name_available(self.wood_repository.as_ref(), &scientific_name).await?;

        Ok(Wood::create(name, scientific_name, details))
    }
}

/// Updates woods, enforcing that a renamed scientific name is not taken.
pub struct WoodUpdateService {
    wood_repository: Arc<dyn WoodRepository>,
}

impl WoodUpdateService {
    pub fn new(wood_repository: Arc<dyn WoodRepository>) -> Self {
        Self { wood_repository }
    }

    pub async fn update(
        &self,
        wood: &mut Wood,
        name: WoodName,
        scientific_name: ScientificName,
        details: WoodDetails,
    ) -> Result<WoodEvent, WoodServiceError> {
        if wood.scientific_name() != &scientific_name {
            ensure_scientific_name_available(self.wood_repository.as_ref(), &scientific_name)
                .await?;
        }

        Ok(wood.update(name, scientific_name, details))
    }
}

async fn ensure_scientific_name_available(
    wood_repository: &dyn WoodRepository,
    scientific_name: &ScientificName,
) -> Result<(), WoodServiceError> {
    let exists = wood_repository
        .exists_by_scientific_name(scientific_name.as_str())
        .await
        .map_err(WoodServiceError::Repository)?;

    if exists {
        return Err(WoodServiceError::Domain(
            WoodError::ScientificNameAlreadyExists {
                scientific_name: scientific_name.to_string(),
            },
        ));
    }

    Ok(())
}

#[derive(Error, Debug)]
pub enum WoodServiceError {
    #[error(transparent)]
    Domain(#[from] WoodError),

    #[error(transparent)]
    Repository(#[from] RepositoryError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{InMemoryWoodRepository, Operation, full_details, wood};

    fn names(name: &str, scientific_name: &str) -> (WoodName, ScientificName) {
        (
            WoodName::new(name).unwrap(),
            ScientificName::new(scientific_name).unwrap(),
        )
    }

    #[tokio::test]
    async fn create_builds_a_wood_when_the_scientific_name_is_free() {
        let service = WoodCreationService::new(Arc::new(InMemoryWoodRepository::new()));
        let (name, scientific_name) = names("Aroeira", "Myracrodruon urundeuva");

        let (wood, event) = service
            .create(name, scientific_name, full_details())
            .await
            .unwrap();

        assert_eq!(wood.scientific_name().as_str(), "Myracrodruon urundeuva");
        assert!(matches!(event, WoodEvent::WoodCreated { wood_id } if wood_id == wood.id()));
    }

    #[tokio::test]
    async fn create_rejects_a_taken_scientific_name() {
        let repository =
            InMemoryWoodRepository::with_woods([wood("Aroeira", "Myracrodruon urundeuva")]);
        let service = WoodCreationService::new(Arc::new(repository));
        let (name, scientific_name) = names("Other", "Myracrodruon urundeuva");

        let error = service
            .create(name, scientific_name, WoodDetails::default())
            .await
            .unwrap_err();

        assert!(matches!(
            &error,
            WoodServiceError::Domain(WoodError::ScientificNameAlreadyExists { scientific_name })
                if scientific_name == "Myracrodruon urundeuva"
        ));
        assert_eq!(
            error.to_string(),
            "wood scientific name 'Myracrodruon urundeuva' already exists"
        );
    }

    #[tokio::test]
    async fn create_propagates_repository_failures() {
        let repository = InMemoryWoodRepository::new().fail_on(Operation::ExistsByScientificName);
        let service = WoodCreationService::new(Arc::new(repository));
        let (name, scientific_name) = names("Aroeira", "Myracrodruon urundeuva");

        let error = service
            .create(name, scientific_name, WoodDetails::default())
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            WoodServiceError::Repository(RepositoryError::Unavailable)
        ));
        assert_eq!(error.to_string(), "repository unavailable");
    }

    #[tokio::test]
    async fn update_keeping_the_scientific_name_skips_the_uniqueness_check() {
        let mut existing = wood("Aroeira", "Myracrodruon urundeuva");
        // A failing lookup proves the check is skipped.
        let repository = InMemoryWoodRepository::with_woods([existing.clone()])
            .fail_on(Operation::ExistsByScientificName);
        let service = WoodUpdateService::new(Arc::new(repository));
        let (name, scientific_name) = names("Aroeira-do-sertão", "Myracrodruon urundeuva");

        let event = service
            .update(&mut existing, name, scientific_name, WoodDetails::default())
            .await
            .unwrap();

        assert!(matches!(event, WoodEvent::WoodUpdated { wood_id } if wood_id == existing.id()));
        assert_eq!(existing.name().as_str(), "Aroeira-do-sertão");
        assert_eq!(existing.details(), &WoodDetails::default());
    }

    #[tokio::test]
    async fn update_to_a_free_scientific_name_succeeds() {
        let mut existing = wood("Aroeira", "Myracrodruon urundeuva");
        let service = WoodUpdateService::new(Arc::new(InMemoryWoodRepository::with_woods([
            existing.clone(),
        ])));
        let (name, scientific_name) = names("Muiracatiara", "Astronium lecointei");

        service
            .update(&mut existing, name, scientific_name, full_details())
            .await
            .unwrap();

        assert_eq!(existing.scientific_name().as_str(), "Astronium lecointei");
    }

    #[tokio::test]
    async fn update_to_a_taken_scientific_name_fails() {
        let mut existing = wood("Aroeira", "Myracrodruon urundeuva");
        let other = wood("Muiracatiara", "Astronium lecointei");
        let service = WoodUpdateService::new(Arc::new(InMemoryWoodRepository::with_woods([
            existing.clone(),
            other,
        ])));
        let (name, scientific_name) = names("Aroeira", "Astronium lecointei");

        let error = service
            .update(&mut existing, name, scientific_name, WoodDetails::default())
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            WoodServiceError::Domain(WoodError::ScientificNameAlreadyExists { .. })
        ));
        assert_eq!(
            existing.scientific_name().as_str(),
            "Myracrodruon urundeuva"
        );
    }

    #[test]
    fn service_errors_convert_from_their_sources() {
        assert!(matches!(
            WoodServiceError::from(WoodError::InvalidUrl("x".into())),
            WoodServiceError::Domain(_)
        ));
        assert!(matches!(
            WoodServiceError::from(RepositoryError::Database),
            WoodServiceError::Repository(_)
        ));
        assert!(
            format!("{:?}", WoodServiceError::from(RepositoryError::Database)).contains("Database")
        );
    }
}
