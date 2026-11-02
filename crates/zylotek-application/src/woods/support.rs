//! Wiring shared by the use-case tests.

use std::sync::Arc;

use zylotek_domain::{
    testing::{InMemoryWoodRepository, Operation, full_details},
    woods::{
        entities::{Wood, WoodId},
        events::WoodEvent,
        services::{WoodCreationService, WoodUpdateService},
    },
};
use zylotek_infrastructure::{
    event_bus::EventPublisher,
    testing::{FailingEventPublisher, RecordingEventPublisher},
};

use crate::woods::{
    create_wood::{CreateWood, CreateWoodCommand},
    delete_wood::DeleteWood,
    facade::WoodFacade,
    get_wood::GetWood,
    list_woods::ListWoods,
    update_wood::{UpdateWood, UpdateWoodCommand},
};

/// A repository seeded with `woods`, a recording bus, and the use cases wired to both.
pub(crate) struct Harness {
    pub repository: Arc<InMemoryWoodRepository>,
    pub events: Arc<RecordingEventPublisher<WoodEvent>>,
    pub create: CreateWood,
    pub get: GetWood,
    pub list: ListWoods,
    pub update: UpdateWood,
    pub delete: DeleteWood,
}

impl Harness {
    pub fn new(woods: impl IntoIterator<Item = Wood>) -> Self {
        Self::build(InMemoryWoodRepository::with_woods(woods), None)
    }

    /// Every repository call that touches `operation` fails.
    pub fn failing_on(operation: Operation, woods: impl IntoIterator<Item = Wood>) -> Self {
        Self::build(
            InMemoryWoodRepository::with_woods(woods).fail_on(operation),
            None,
        )
    }

    /// Publishing fails, as when no handler is subscribed.
    pub fn without_subscribers(woods: impl IntoIterator<Item = Wood>) -> Self {
        Self::build(
            InMemoryWoodRepository::with_woods(woods),
            Some(Arc::new(FailingEventPublisher)),
        )
    }

    fn build(
        repository: InMemoryWoodRepository,
        publisher: Option<Arc<dyn EventPublisher<WoodEvent>>>,
    ) -> Self {
        let repository = Arc::new(repository);
        let events = Arc::new(RecordingEventPublisher::new());
        let publisher = publisher.unwrap_or_else(|| events.clone());

        Self {
            create: CreateWood::new(
                publisher.clone(),
                repository.clone(),
                WoodCreationService::new(repository.clone()),
            ),
            get: GetWood::new(repository.clone()),
            list: ListWoods::new(repository.clone()),
            update: UpdateWood::new(
                publisher.clone(),
                repository.clone(),
                WoodUpdateService::new(repository.clone()),
            ),
            delete: DeleteWood::new(publisher, repository.clone()),
            repository,
            events,
        }
    }

    pub fn facade(self) -> WoodFacade {
        WoodFacade::new(self.create, self.get, self.list, self.update, self.delete)
    }
}

pub(crate) fn create_command(name: &str, scientific_name: &str) -> CreateWoodCommand {
    CreateWoodCommand {
        name: name.to_owned(),
        scientific_name: scientific_name.to_owned(),
        details: full_details(),
    }
}

pub(crate) fn update_command(id: WoodId, name: &str, scientific_name: &str) -> UpdateWoodCommand {
    UpdateWoodCommand {
        id,
        name: name.to_owned(),
        scientific_name: scientific_name.to_owned(),
        details: full_details(),
    }
}
