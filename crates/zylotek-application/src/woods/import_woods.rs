use zylotek_domain::woods::errors::WoodError;

use crate::woods::{
    create_wood::{CreateWood, CreateWoodCommand},
    errors::WoodUseCaseError,
};

/// Creates many woods at once, as `sync` does.
///
/// A scientific name is registered once, so a wood whose name is already taken — by an
/// earlier run or by an earlier row of the same batch — is skipped rather than treated as
/// an error. That makes importing the same file twice harmless. Any other failure is
/// recorded and the rest of the batch carries on.
pub struct ImportWoods {
    create_wood: CreateWood,
}

impl ImportWoods {
    pub fn new(create_wood: CreateWood) -> Self {
        Self { create_wood }
    }

    pub async fn execute(
        &self,
        commands: impl IntoIterator<Item = CreateWoodCommand>,
    ) -> ImportReport {
        let mut report = ImportReport::default();

        for command in commands {
            let scientific_name = command.scientific_name.clone();

            match self.create_wood.execute(command).await {
                Ok(_) => report.created += 1,
                Err(WoodUseCaseError::Domain(WoodError::ScientificNameAlreadyExists {
                    ..
                })) => report.already_registered.push(scientific_name),
                Err(error) => report.failed.push(ImportFailure {
                    scientific_name,
                    error,
                }),
            }
        }

        report
    }
}

#[derive(Debug, Default)]
pub struct ImportReport {
    pub created: usize,
    /// Scientific names that were skipped because they were already registered.
    pub already_registered: Vec<String>,
    pub failed: Vec<ImportFailure>,
}

#[derive(Debug)]
pub struct ImportFailure {
    pub scientific_name: String,
    pub error: WoodUseCaseError,
}

#[cfg(test)]
mod tests {
    use zylotek_domain::{
        errors::RepositoryError,
        testing::{Operation, full_details, wood},
    };

    use super::*;
    use crate::woods::support::{Harness, create_command};

    fn import(harness: Harness) -> (ImportWoods, Harness) {
        // `Harness` owns its `CreateWood`, so build a second one over the same collaborators.
        let create = CreateWood::new(
            harness.events.clone(),
            harness.repository.clone(),
            zylotek_domain::woods::services::WoodCreationService::new(harness.repository.clone()),
        );

        (ImportWoods::new(create), harness)
    }

    #[tokio::test]
    async fn creates_every_new_wood() {
        let (import, harness) = import(Harness::new([]));

        let report = import
            .execute([
                create_command("Aroeira", "Myracrodruon urundeuva"),
                create_command("Muiracatiara", "Astronium lecointei"),
            ])
            .await;

        assert_eq!(report.created, 2);
        assert!(report.already_registered.is_empty() && report.failed.is_empty());
        assert_eq!(harness.repository.len(), 2);
        assert_eq!(harness.events.events().len(), 2);
    }

    #[tokio::test]
    async fn skips_names_registered_before_or_earlier_in_the_batch() {
        let (import, harness) = import(Harness::new([wood("Aroeira", "Myracrodruon urundeuva")]));

        let report = import
            .execute([
                create_command("Aroeira do sertão", "Myracrodruon urundeuva"),
                create_command("Muiracatiara", "Astronium lecointei"),
                create_command("Muiracatiara 2", "Astronium   lecointei"),
            ])
            .await;

        assert_eq!(report.created, 1);
        assert_eq!(
            report.already_registered,
            ["Myracrodruon urundeuva", "Astronium   lecointei"]
        );
        assert!(report.failed.is_empty());
        assert_eq!(harness.repository.len(), 2);
    }

    #[tokio::test]
    async fn importing_the_same_batch_twice_changes_nothing_the_second_time() {
        let (import, harness) = import(Harness::new([]));
        let batch = || [create_command("Aroeira", "Myracrodruon urundeuva")];

        import.execute(batch()).await;
        let second = import.execute(batch()).await;

        assert_eq!((second.created, second.already_registered.len()), (0, 1));
        assert_eq!(harness.repository.len(), 1);
    }

    #[tokio::test]
    async fn records_invalid_woods_and_carries_on() {
        let (import, harness) = import(Harness::new([]));
        let mut invalid = create_command("Aroeira", "Solo");
        invalid.details = full_details();

        let report = import
            .execute([
                invalid,
                create_command("Muiracatiara", "Astronium lecointei"),
            ])
            .await;

        assert_eq!(report.created, 1);
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].scientific_name, "Solo");
        assert!(matches!(
            report.failed[0].error,
            WoodUseCaseError::Domain(_)
        ));
        assert_eq!(harness.repository.len(), 1);
    }

    #[tokio::test]
    async fn records_storage_failures_per_wood() {
        let (import, _) = import(Harness::failing_on(Operation::Save, []));

        let report = import
            .execute([
                create_command("Aroeira", "Myracrodruon urundeuva"),
                create_command("Muiracatiara", "Astronium lecointei"),
            ])
            .await;

        assert_eq!(report.created, 0);
        assert_eq!(report.failed.len(), 2);
        assert!(matches!(
            report.failed[0].error,
            WoodUseCaseError::Persistence(RepositoryError::Unavailable)
        ));
        assert!(format!("{:?}", report.failed[0]).contains("Myracrodruon"));
    }

    #[tokio::test]
    async fn an_empty_batch_is_a_no_op() {
        let (import, harness) = import(Harness::new([]));

        let report = import.execute([]).await;

        assert_eq!(report.created, 0);
        assert!(harness.repository.is_empty());
    }
}
