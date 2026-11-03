use crate::{
    errors::RepositoryError,
    woods::entities::{Wood, WoodId},
};

#[async_trait::async_trait]
pub trait WoodRepository: Send + Sync {
    async fn exists_by_scientific_name(
        &self,
        scientific_name: &str,
    ) -> Result<bool, RepositoryError>;

    /// The wood registered under exactly this scientific name, if any.
    async fn find_by_scientific_name(
        &self,
        scientific_name: &str,
    ) -> Result<Option<Wood>, RepositoryError>;

    async fn find_by_id(&self, id: WoodId) -> Result<Option<Wood>, RepositoryError>;

    /// Woods ordered by scientific name.
    async fn list(&self, offset: usize, limit: usize) -> Result<Vec<Wood>, RepositoryError>;

    async fn count(&self) -> Result<u64, RepositoryError>;

    async fn save(&self, wood: &Wood) -> Result<Wood, RepositoryError>;

    async fn update(&self, wood: &Wood) -> Result<Wood, RepositoryError>;

    async fn delete(&self, id: WoodId) -> Result<(), RepositoryError>;
}
