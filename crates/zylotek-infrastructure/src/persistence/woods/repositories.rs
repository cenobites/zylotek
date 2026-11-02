use zylotek_domain::{
    errors::RepositoryError,
    woods::{entities as domain, repositories::WoodRepository},
};

use crate::persistence::woods::{mappers::WoodRecord, models::Wood};

pub struct PostgresWoodRepository {
    db: toasty::Db,
}

impl PostgresWoodRepository {
    pub fn new(db: toasty::Db) -> Self {
        Self { db }
    }
}

#[async_trait::async_trait]
impl WoodRepository for PostgresWoodRepository {
    async fn exists_by_scientific_name(
        &self,
        scientific_name: &str,
    ) -> Result<bool, RepositoryError> {
        let mut db = self.db.clone();

        let exists: Option<Wood> =
            Wood::filter(Wood::fields().scientific_name().eq(scientific_name))
                .first()
                .exec(&mut db)
                .await
                .map_err(database_error)?;

        Ok(exists.is_some())
    }

    async fn find_by_id(
        &self,
        id: domain::WoodId,
    ) -> Result<Option<domain::Wood>, RepositoryError> {
        let mut db = self.db.clone();

        let model: Option<Wood> = Wood::filter_by_id(id.as_uuid())
            .first()
            .exec(&mut db)
            .await
            .map_err(database_error)?;

        model.map(to_domain).transpose()
    }

    async fn list(
        &self,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<domain::Wood>, RepositoryError> {
        let mut db = self.db.clone();

        let models: Vec<Wood> = Wood::all()
            .order_by(Wood::fields().scientific_name().asc())
            .limit(limit)
            .offset(offset)
            .exec(&mut db)
            .await
            .map_err(database_error)?;

        models.into_iter().map(to_domain).collect()
    }

    async fn count(&self) -> Result<u64, RepositoryError> {
        let mut db = self.db.clone();

        Wood::all()
            .count()
            .exec(&mut db)
            .await
            .map_err(database_error)
    }

    async fn save(&self, wood: &domain::Wood) -> Result<domain::Wood, RepositoryError> {
        let mut db = self.db.clone();
        let record = WoodRecord::from(wood);

        let model = Wood::create()
            .id(record.id)
            .name(record.name)
            .scientific_name(record.scientific_name)
            .family(record.family)
            .collection_location(record.collection_location)
            .common_names(record.common_names)
            .source_species_id(record.source_species_id)
            .source_page_url(record.source_page_url)
            .tree(record.tree)
            .general_characteristics(record.general_characteristics)
            .description(record.description)
            .kiln_drying_schedule(record.kiln_drying_schedule)
            .kiln_drying_days(record.kiln_drying_days)
            .images(record.images)
            .physical_properties(record.physical_properties)
            .mechanical_properties(record.mechanical_properties)
            .workabilities(record.workabilities)
            .exec(&mut db)
            .await
            .map_err(database_error)?;

        to_domain(model)
    }

    async fn update(&self, wood: &domain::Wood) -> Result<domain::Wood, RepositoryError> {
        let mut db = self.db.clone();
        let record = WoodRecord::from(wood);

        Wood::filter_by_id(record.id)
            .update()
            .name(record.name)
            .scientific_name(record.scientific_name)
            .family(record.family)
            .collection_location(record.collection_location)
            .common_names(record.common_names)
            .source_species_id(record.source_species_id)
            .source_page_url(record.source_page_url)
            .tree(record.tree)
            .general_characteristics(record.general_characteristics)
            .description(record.description)
            .kiln_drying_schedule(record.kiln_drying_schedule)
            .kiln_drying_days(record.kiln_drying_days)
            .images(record.images)
            .physical_properties(record.physical_properties)
            .mechanical_properties(record.mechanical_properties)
            .workabilities(record.workabilities)
            .exec(&mut db)
            .await
            .map_err(database_error)?;

        self.find_by_id(wood.id())
            .await?
            .ok_or(RepositoryError::OperationFailed)
    }

    async fn delete(&self, id: domain::WoodId) -> Result<(), RepositoryError> {
        let mut db = self.db.clone();

        Wood::delete_by_id(&mut db, id.as_uuid())
            .await
            .map_err(database_error)
    }
}

fn to_domain(model: Wood) -> Result<domain::Wood, RepositoryError> {
    domain::Wood::try_from(model).map_err(|err| RepositoryError::InvalidData(err.to_string()))
}

fn database_error(err: toasty::Error) -> RepositoryError {
    tracing::error!(error = %err, "wood repository query failed");
    RepositoryError::Database
}

#[cfg(test)]
mod tests {
    use zylotek_domain::{
        testing::{full_details, wood},
        woods::{
            entities::Wood as DomainWood,
            value_objects::{ScientificName, WoodDetails, WoodName},
        },
    };

    use super::*;
    use crate::{persistence::woods::mappers::tests::model_from, testing::memory_db};

    async fn repository() -> PostgresWoodRepository {
        PostgresWoodRepository::new(memory_db().await.unwrap())
    }

    #[tokio::test]
    async fn a_saved_wood_is_found_by_id_with_every_field_intact() {
        let repository = repository().await;
        let original = wood("Aroeira", "Myracrodruon urundeuva");

        let saved = repository.save(&original).await.unwrap();
        let found = repository.find_by_id(original.id()).await.unwrap().unwrap();

        for stored in [saved, found] {
            assert_eq!(stored.id(), original.id());
            assert_eq!(stored.name(), original.name());
            assert_eq!(stored.scientific_name(), original.scientific_name());
            assert_eq!(stored.details(), &full_details());
        }
    }

    #[tokio::test]
    async fn a_missing_wood_is_not_found() {
        let repository = repository().await;

        assert!(
            repository
                .find_by_id(domain::WoodId::new())
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn exists_by_scientific_name_matches_exactly() {
        let repository = repository().await;
        repository
            .save(&wood("Aroeira", "Myracrodruon urundeuva"))
            .await
            .unwrap();

        assert!(
            repository
                .exists_by_scientific_name("Myracrodruon urundeuva")
                .await
                .unwrap()
        );
        assert!(
            !repository
                .exists_by_scientific_name("Myracrodruon")
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn saving_a_duplicate_scientific_name_violates_the_unique_index() {
        let repository = repository().await;
        repository
            .save(&wood("Aroeira", "Myracrodruon urundeuva"))
            .await
            .unwrap();

        let error = repository
            .save(&wood("Outra", "Myracrodruon urundeuva"))
            .await
            .unwrap_err();

        assert!(matches!(error, RepositoryError::Database));
        assert_eq!(repository.count().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn list_orders_by_scientific_name_and_paginates() {
        let repository = repository().await;
        for scientific_name in [
            "Cedrela fissilis",
            "Astronium lecointei",
            "Bertholletia excelsa",
        ] {
            repository
                .save(&wood("Madeira", scientific_name))
                .await
                .unwrap();
        }
        let names = |woods: Vec<DomainWood>| -> Vec<String> {
            woods
                .iter()
                .map(|w| w.scientific_name().to_string())
                .collect()
        };

        assert_eq!(
            names(repository.list(0, 10).await.unwrap()),
            [
                "Astronium lecointei",
                "Bertholletia excelsa",
                "Cedrela fissilis"
            ]
        );
        assert_eq!(
            names(repository.list(1, 1).await.unwrap()),
            ["Bertholletia excelsa"]
        );
        assert!(repository.list(3, 10).await.unwrap().is_empty());
        assert_eq!(repository.count().await.unwrap(), 3);
    }

    #[tokio::test]
    async fn update_replaces_every_column() {
        let repository = repository().await;
        let mut stored = wood("Aroeira", "Myracrodruon urundeuva");
        repository.save(&stored).await.unwrap();
        stored.update(
            WoodName::new("Muiracatiara").unwrap(),
            ScientificName::new("Astronium lecointei").unwrap(),
            WoodDetails::default(),
        );

        let updated = repository.update(&stored).await.unwrap();

        assert_eq!(updated.name().as_str(), "Muiracatiara");
        assert_eq!(updated.scientific_name().as_str(), "Astronium lecointei");
        assert_eq!(updated.details(), &WoodDetails::default());
        assert!(
            !repository
                .exists_by_scientific_name("Myracrodruon urundeuva")
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn updating_a_missing_wood_fails() {
        let repository = repository().await;

        let error = repository
            .update(&wood("Aroeira", "Myracrodruon urundeuva"))
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            RepositoryError::OperationFailed | RepositoryError::Database
        ));
    }

    #[tokio::test]
    async fn delete_removes_the_wood_and_is_idempotent() {
        let repository = repository().await;
        let stored = wood("Aroeira", "Myracrodruon urundeuva");
        repository.save(&stored).await.unwrap();

        repository.delete(stored.id()).await.unwrap();
        repository.delete(stored.id()).await.unwrap();

        assert!(repository.find_by_id(stored.id()).await.unwrap().is_none());
        assert_eq!(repository.count().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn a_corrupt_row_is_reported_as_invalid_data() {
        let db = memory_db().await.unwrap();
        let repository = PostgresWoodRepository::new(db.clone());
        let mut model = model_from(WoodRecord::from(&wood("Aroeira", "Myracrodruon urundeuva")));
        model.kiln_drying_days = Some(-1.0);
        let id = model.id;
        let mut db = db;
        Wood::create()
            .id(model.id)
            .name(model.name)
            .scientific_name(model.scientific_name)
            .family(model.family)
            .collection_location(model.collection_location)
            .common_names(model.common_names)
            .source_species_id(model.source_species_id)
            .source_page_url(model.source_page_url)
            .tree(model.tree)
            .general_characteristics(model.general_characteristics)
            .description(model.description)
            .kiln_drying_schedule(model.kiln_drying_schedule)
            .kiln_drying_days(model.kiln_drying_days)
            .images(model.images)
            .physical_properties(model.physical_properties)
            .mechanical_properties(model.mechanical_properties)
            .workabilities(model.workabilities)
            .exec(&mut db)
            .await
            .unwrap();

        let by_id = repository
            .find_by_id(domain::WoodId::from_uuid(id))
            .await
            .unwrap_err();
        let listed = repository.list(0, 10).await.unwrap_err();

        assert!(matches!(by_id, RepositoryError::InvalidData(_)));
        assert!(matches!(listed, RepositoryError::InvalidData(_)));
    }
}
