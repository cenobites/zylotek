use std::collections::HashSet;

use std::sync::Arc;

use zylotek_domain::woods::{
    repositories::WoodRepository,
    value_objects::{ScientificName, WoodName},
};

use crate::woods::{
    create_wood::{CreateWood, CreateWoodCommand},
    errors::WoodUseCaseError,
    update_wood::{UpdateWood, UpdateWoodCommand},
};

/// Creates or updates many woods at once, as `sync` does.
///
/// The scientific name is the key: a wood whose name is already stored is updated, any
/// other is created. An update takes the new name and details but keeps what the stored
/// wood has and the new data lacks, so a run that could not reach a source does not erase
/// what an earlier run learned from it. A wood that would not change is left alone, which
/// makes importing the same data twice harmless.
///
/// Within one batch the first wood of a scientific name wins and the rest are reported as
/// duplicates. Any other failure is recorded and the rest of the batch carries on.
pub struct ImportWoods {
    create_wood: CreateWood,
    update_wood: UpdateWood,
    wood_repository: Arc<dyn WoodRepository>,
}

impl ImportWoods {
    pub fn new(
        create_wood: CreateWood,
        update_wood: UpdateWood,
        wood_repository: Arc<dyn WoodRepository>,
    ) -> Self {
        Self {
            create_wood,
            update_wood,
            wood_repository,
        }
    }

    pub async fn execute(
        &self,
        commands: impl IntoIterator<Item = CreateWoodCommand>,
    ) -> ImportReport {
        let mut report = ImportReport::default();
        let mut seen = HashSet::new();

        for command in commands {
            let scientific_name = command.scientific_name.clone();

            match self.upsert(command, &mut seen).await {
                Ok(Outcome::Created) => report.created += 1,
                Ok(Outcome::Updated) => report.updated += 1,
                Ok(Outcome::Unchanged) => report.unchanged += 1,
                Ok(Outcome::Duplicate) => report.duplicates.push(scientific_name),
                Err(error) => report.failed.push(ImportFailure {
                    scientific_name,
                    error,
                }),
            }
        }

        report
    }

    async fn upsert(
        &self,
        command: CreateWoodCommand,
        seen: &mut HashSet<String>,
    ) -> Result<Outcome, WoodUseCaseError> {
        let scientific_name = ScientificName::new(&command.scientific_name)?;
        let name = WoodName::new(&command.name)?;
        if !seen.insert(scientific_name.to_string()) {
            return Ok(Outcome::Duplicate);
        }

        let stored = self
            .wood_repository
            .find_by_scientific_name(scientific_name.as_str())
            .await
            .map_err(WoodUseCaseError::Persistence)?;
        let Some(stored) = stored else {
            self.create_wood.execute(command).await?;
            return Ok(Outcome::Created);
        };

        let details = command.details.fill_gaps_from(stored.details().clone());
        if *stored.name() == name && *stored.details() == details {
            return Ok(Outcome::Unchanged);
        }

        self.update_wood
            .execute(UpdateWoodCommand {
                id: stored.id(),
                name: command.name,
                scientific_name: command.scientific_name,
                details,
            })
            .await?;

        Ok(Outcome::Updated)
    }
}

enum Outcome {
    Created,
    Updated,
    Unchanged,
    Duplicate,
}

#[derive(Debug, Default)]
pub struct ImportReport {
    pub created: usize,
    pub updated: usize,
    /// Woods already stored exactly as the import would leave them.
    pub unchanged: usize,
    /// Scientific names repeated earlier in the same batch, which were left out.
    pub duplicates: Vec<String>,
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
        woods::{
            events::WoodEvent,
            services::{WoodCreationService, WoodUpdateService},
            value_objects::{Attributes, WoodDetails},
        },
    };

    use super::*;
    use crate::woods::support::{Harness, create_command};

    fn import(harness: Harness) -> (ImportWoods, Harness) {
        // `Harness` owns its use cases, so build second ones over the same collaborators.
        let repository = harness.repository.clone();
        let create = CreateWood::new(
            harness.events.clone(),
            repository.clone(),
            WoodCreationService::new(repository.clone()),
        );
        let update = UpdateWood::new(
            harness.events.clone(),
            repository.clone(),
            WoodUpdateService::new(repository.clone()),
        );

        (ImportWoods::new(create, update, repository), harness)
    }

    fn command(name: &str, scientific_name: &str, details: WoodDetails) -> CreateWoodCommand {
        CreateWoodCommand {
            details,
            ..create_command(name, scientific_name)
        }
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
        assert!(report.duplicates.is_empty() && report.failed.is_empty());
        assert_eq!(harness.repository.len(), 2);
        assert_eq!(harness.events.events().len(), 2);
    }

    #[tokio::test]
    async fn updates_a_stored_wood_instead_of_rejecting_its_name() {
        let stored = wood("Aroeira", "Myracrodruon urundeuva");
        let (import, harness) = import(Harness::new([stored.clone()]));

        let report = import
            .execute([create_command(
                "Aroeira-do-sertão",
                "Myracrodruon  urundeuva",
            )])
            .await;

        assert_eq!((report.created, report.updated), (0, 1));
        assert_eq!(harness.repository.len(), 1);
        let after = harness
            .repository
            .find_by_scientific_name("Myracrodruon urundeuva")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after.id(), stored.id(), "the same wood, not a new one");
        assert_eq!(after.name().as_str(), "Aroeira-do-sertão");
        assert_eq!(*after.details(), full_details());
        assert!(matches!(
            harness.events.events().as_slice(),
            [WoodEvent::WoodUpdated { wood_id }] if *wood_id == stored.id()
        ));
    }

    #[tokio::test]
    async fn an_update_keeps_what_the_stored_wood_has_and_the_new_data_lacks() {
        let mut details = full_details();
        details.description.end_uses = Some("stored".to_owned());
        details.description.tree = Attributes::new([("dap", "stored")]);
        let stored = zylotek_domain::woods::entities::Wood::create(
            WoodName::new("Aroeira").unwrap(),
            ScientificName::new("Myracrodruon urundeuva").unwrap(),
            details,
        )
        .0;
        let (import, harness) = import(Harness::new([stored]));
        let mut incoming = WoodDetails::default();
        incoming.description.tree = Attributes::new([("dap", "new")]);

        import
            .execute([command("Aroeira", "Myracrodruon urundeuva", incoming)])
            .await;

        let after = harness
            .repository
            .find_by_scientific_name("Myracrodruon urundeuva")
            .await
            .unwrap()
            .unwrap();
        // The new value wins; the one it lacks survives.
        assert_eq!(after.details().description.tree.get("dap"), Some("new"));
        assert_eq!(
            after.details().description.end_uses.as_deref(),
            Some("stored")
        );
        assert_eq!(after.details().images, full_details().images);
    }

    #[tokio::test]
    async fn a_wood_that_would_not_change_is_left_alone() {
        let (import, harness) = import(Harness::new([]));
        let batch = || [create_command("Aroeira", "Myracrodruon urundeuva")];

        import.execute(batch()).await;
        let second = import.execute(batch()).await;

        assert_eq!(
            (second.created, second.updated, second.unchanged),
            (0, 0, 1)
        );
        assert_eq!(harness.repository.len(), 1);
        assert_eq!(harness.events.events().len(), 1, "no second event");
    }

    #[tokio::test]
    async fn the_first_wood_of_a_name_in_a_batch_wins() {
        let (import, harness) = import(Harness::new([]));

        let report = import
            .execute([
                create_command("Muiracatiara", "Astronium lecointei"),
                create_command("Muiracatiara 2", "Astronium   lecointei"),
            ])
            .await;

        assert_eq!(report.created, 1);
        assert_eq!(report.duplicates, ["Astronium   lecointei"]);
        let stored = harness
            .repository
            .find_by_scientific_name("Astronium lecointei")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.name().as_str(), "Muiracatiara");
    }

    #[tokio::test]
    async fn records_invalid_woods_and_carries_on() {
        let (import, harness) = import(Harness::new([]));
        let mut invalid = create_command("Aroeira", "Solo");
        invalid.details = full_details();

        let report = import
            .execute([
                invalid,
                create_command("", "Cedrela fissilis"),
                create_command("Muiracatiara", "Astronium lecointei"),
            ])
            .await;

        assert_eq!(report.created, 1);
        assert_eq!(report.failed.len(), 2);
        assert_eq!(report.failed[0].scientific_name, "Solo");
        assert!(matches!(
            report.failed[0].error,
            WoodUseCaseError::Domain(_)
        ));
        assert_eq!(harness.repository.len(), 1);
    }

    #[tokio::test]
    async fn records_storage_failures_per_wood() {
        for operation in [Operation::Save, Operation::FindByScientificName] {
            let (import, _) = import(Harness::failing_on(operation, []));

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
    }

    #[tokio::test]
    async fn records_a_failed_update() {
        let stored = wood("Aroeira", "Myracrodruon urundeuva");
        let (import, _) = import(Harness::failing_on(Operation::Update, [stored]));

        let report = import
            .execute([create_command(
                "Aroeira-do-sertão",
                "Myracrodruon urundeuva",
            )])
            .await;

        assert_eq!((report.updated, report.failed.len()), (0, 1));
    }

    #[tokio::test]
    async fn an_empty_batch_is_a_no_op() {
        let (import, harness) = import(Harness::new([]));

        let report = import.execute([]).await;

        assert_eq!(report.created + report.updated + report.unchanged, 0);
        assert!(harness.repository.is_empty());
    }
}
