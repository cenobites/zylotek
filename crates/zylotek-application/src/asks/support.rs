//! Wiring shared by the ask use-case tests.

use std::sync::{Arc, Mutex};

use zylotek_domain::{
    asks::{events::AskEvent, services::AskWorkflowService, value_objects::Timestamp},
    testing::{InMemoryAskRepository, Operation},
};
use zylotek_infrastructure::{
    event_bus::EventPublisher,
    testing::{FailingEventPublisher, RecordingEventPublisher},
};

use crate::asks::{
    clock::Clock,
    create_ask::{CreateAsk, CreateAskCommand},
    facade::AskFacade,
    get_ask::GetAsk,
    record_ask_progress::RecordAskProgress,
};

/// A clock that ticks one millisecond per reading, starting at 1.
pub(crate) struct Ticking(Mutex<u64>);

impl Clock for Ticking {
    fn now(&self) -> Timestamp {
        let mut now = self.0.lock().unwrap();
        *now += 1;
        Timestamp(*now)
    }
}

/// An in-memory repository, a recording bus, a ticking clock, and the use cases wired to them.
pub(crate) struct Harness {
    pub repository: Arc<InMemoryAskRepository>,
    pub events: Arc<RecordingEventPublisher<AskEvent>>,
    pub create: CreateAsk,
    pub get: GetAsk,
    pub record: RecordAskProgress,
}

impl Harness {
    pub fn new() -> Self {
        Self::build(InMemoryAskRepository::new(), None)
    }

    /// Every repository call that touches `operation` fails.
    pub fn failing_on(operation: Operation) -> Self {
        Self::build(InMemoryAskRepository::new().fail_on(operation), None)
    }

    /// Publishing fails, as when no handler is subscribed.
    pub fn without_subscribers() -> Self {
        Self::build(
            InMemoryAskRepository::new(),
            Some(Arc::new(FailingEventPublisher)),
        )
    }

    fn build(
        repository: InMemoryAskRepository,
        publisher: Option<Arc<dyn EventPublisher<AskEvent>>>,
    ) -> Self {
        let repository = Arc::new(repository);
        let events = Arc::new(RecordingEventPublisher::new());
        let publisher = publisher.unwrap_or_else(|| events.clone());
        let clock: Arc<dyn Clock> = Arc::new(Ticking(Mutex::new(0)));
        let service = AskWorkflowService::new();

        Self {
            create: CreateAsk::new(
                publisher.clone(),
                repository.clone(),
                service,
                clock.clone(),
            ),
            get: GetAsk::new(repository.clone()),
            record: RecordAskProgress::new(publisher, repository.clone(), service, clock),
            repository,
            events,
        }
    }

    pub fn facade(self) -> AskFacade {
        AskFacade::new(self.create, self.get, self.record)
    }
}

pub(crate) fn question(text: &str) -> CreateAskCommand {
    CreateAskCommand {
        question: text.to_owned(),
    }
}
