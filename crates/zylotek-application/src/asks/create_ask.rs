use std::sync::Arc;

use zylotek_domain::asks::{
    entities::Ask, events::AskEvent, repositories::AskRepository, services::AskWorkflowService,
};
use zylotek_infrastructure::event_bus::EventPublisher;

use crate::asks::{clock::Clock, errors::AskUseCaseError};

pub struct CreateAsk {
    event_publisher: Arc<dyn EventPublisher<AskEvent>>,
    ask_repository: Arc<dyn AskRepository>,
    ask_workflow_service: AskWorkflowService,
    clock: Arc<dyn Clock>,
}

impl CreateAsk {
    pub fn new(
        event_publisher: Arc<dyn EventPublisher<AskEvent>>,
        ask_repository: Arc<dyn AskRepository>,
        ask_workflow_service: AskWorkflowService,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            event_publisher,
            ask_repository,
            ask_workflow_service,
            clock,
        }
    }

    /// Stores a new, queued ask and publishes `AskOpened`.
    pub async fn execute(&self, command: CreateAskCommand) -> Result<Ask, AskUseCaseError> {
        let (ask, event) = self
            .ask_workflow_service
            .open(&command.question, self.clock.now())?;

        let ask = self
            .ask_repository
            .save(&ask)
            .await
            .map_err(AskUseCaseError::Persistence)?;

        self.event_publisher
            .publish(event)
            .await
            .map_err(AskUseCaseError::Event)?;

        Ok(ask)
    }
}

pub struct CreateAskCommand {
    pub question: String,
}

#[cfg(test)]
mod tests {
    use zylotek_domain::{
        asks::{
            errors::AskError,
            value_objects::{AskState, Timestamp},
        },
        errors::RepositoryError,
        testing::Operation,
    };

    use super::*;
    use crate::asks::support::{Harness, question};

    #[tokio::test]
    async fn stores_a_queued_ask_and_publishes_ask_opened() {
        let harness = Harness::new();

        let created = harness.create.execute(question(" a table ")).await.unwrap();

        assert_eq!(created.state(), AskState::Queued);
        assert_eq!(created.question().as_str(), "a table");
        assert_eq!(created.created_at(), Timestamp(1));
        assert_eq!(
            harness.events.events(),
            [AskEvent::AskOpened {
                ask_id: created.id()
            }]
        );
    }

    #[tokio::test]
    async fn a_blank_question_is_refused_before_anything_is_stored() {
        let harness = Harness::new();

        let error = harness.create.execute(question("  ")).await.unwrap_err();

        assert!(matches!(
            error,
            AskUseCaseError::Domain(AskError::InvalidQuestion { .. })
        ));
        assert!(harness.events.events().is_empty());
    }

    #[tokio::test]
    async fn storage_and_publishing_failures_are_reported() {
        let save = Harness::failing_on(Operation::Save)
            .create
            .execute(question("a table"))
            .await
            .unwrap_err();
        let publish = Harness::without_subscribers()
            .create
            .execute(question("a table"))
            .await
            .unwrap_err();

        assert!(matches!(
            save,
            AskUseCaseError::Persistence(RepositoryError::Unavailable)
        ));
        assert!(matches!(publish, AskUseCaseError::Event(_)));
    }
}
