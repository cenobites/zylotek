use crate::{
    asks::entities::{Ask, AskId},
    errors::RepositoryError,
};

#[async_trait::async_trait]
pub trait AskRepository: Send + Sync {
    async fn find_by_id(&self, id: AskId) -> Result<Option<Ask>, RepositoryError>;

    async fn save(&self, ask: &Ask) -> Result<Ask, RepositoryError>;

    async fn update(&self, ask: &Ask) -> Result<Ask, RepositoryError>;
}
