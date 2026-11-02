use std::sync::Arc;

use zylotek_domain::woods::entities::{Wood, WoodId};

use crate::woods::{
    create_wood::{CreateWood, CreateWoodCommand},
    delete_wood::DeleteWood,
    errors::WoodUseCaseError,
    get_wood::GetWood,
    list_woods::{ListWoods, ListWoodsQuery, WoodPage},
    update_wood::{UpdateWood, UpdateWoodCommand},
};

#[derive(Clone)]
pub struct WoodFacade {
    create_wood: Arc<CreateWood>,
    get_wood: Arc<GetWood>,
    list_woods: Arc<ListWoods>,
    update_wood: Arc<UpdateWood>,
    delete_wood: Arc<DeleteWood>,
}

impl WoodFacade {
    pub fn new(
        create_wood: CreateWood,
        get_wood: GetWood,
        list_woods: ListWoods,
        update_wood: UpdateWood,
        delete_wood: DeleteWood,
    ) -> Self {
        Self {
            create_wood: Arc::new(create_wood),
            get_wood: Arc::new(get_wood),
            list_woods: Arc::new(list_woods),
            update_wood: Arc::new(update_wood),
            delete_wood: Arc::new(delete_wood),
        }
    }

    pub async fn create_wood(&self, command: CreateWoodCommand) -> Result<Wood, WoodUseCaseError> {
        self.create_wood.execute(command).await
    }

    pub async fn get_wood(&self, id: WoodId) -> Result<Wood, WoodUseCaseError> {
        self.get_wood.execute(id).await
    }

    pub async fn list_woods(&self, query: ListWoodsQuery) -> Result<WoodPage, WoodUseCaseError> {
        self.list_woods.execute(query).await
    }

    pub async fn update_wood(&self, command: UpdateWoodCommand) -> Result<Wood, WoodUseCaseError> {
        self.update_wood.execute(command).await
    }

    pub async fn delete_wood(&self, id: WoodId) -> Result<(), WoodUseCaseError> {
        self.delete_wood.execute(id).await
    }
}

#[cfg(test)]
mod tests {
    use zylotek_domain::testing::wood;

    use crate::woods::{
        errors::WoodUseCaseError,
        list_woods::ListWoodsQuery,
        support::{Harness, create_command, update_command},
    };

    #[tokio::test]
    async fn drives_a_wood_through_its_whole_lifecycle() {
        let harness = Harness::new([]);
        let events = harness.events.clone();
        let facade = harness.facade();

        let created = facade
            .create_wood(create_command("Aroeira", "Myracrodruon urundeuva"))
            .await
            .unwrap();
        let fetched = facade.get_wood(created.id()).await.unwrap();
        let page = facade.list_woods(ListWoodsQuery::default()).await.unwrap();
        let updated = facade
            .update_wood(update_command(
                created.id(),
                "Muiracatiara",
                "Astronium lecointei",
            ))
            .await
            .unwrap();
        facade.delete_wood(created.id()).await.unwrap();
        let gone = facade.get_wood(created.id()).await.unwrap_err();

        assert_eq!(fetched.id(), created.id());
        assert_eq!(page.total, 1);
        assert_eq!(updated.name().as_str(), "Muiracatiara");
        assert!(matches!(gone, WoodUseCaseError::NotFound(_)));
        assert_eq!(events.events().len(), 3);
    }

    #[tokio::test]
    async fn clones_share_the_same_state() {
        let stored = wood("Aroeira", "Myracrodruon urundeuva");
        let facade = Harness::new([stored.clone()]).facade();
        let clone = facade.clone();

        clone.delete_wood(stored.id()).await.unwrap();

        assert!(facade.get_wood(stored.id()).await.is_err());
    }
}
