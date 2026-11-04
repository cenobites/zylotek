use std::sync::Arc;

use zylotek_domain::asks::{
    entities::{Ask, AskId},
    events::AskEvent,
    repositories::AskRepository,
    services::{AskStep, AskWorkflowService},
};
use zylotek_infrastructure::event_bus::EventPublisher;

use crate::asks::{clock::Clock, errors::AskUseCaseError, get_ask::GetAsk};

pub struct RecordAskProgress {
    event_publisher: Arc<dyn EventPublisher<AskEvent>>,
    ask_repository: Arc<dyn AskRepository>,
    ask_workflow_service: AskWorkflowService,
    clock: Arc<dyn Clock>,
}

impl RecordAskProgress {
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

    /// Applies one step, stores the result and publishes its event. A step the state machine
    /// forbids is an error and changes nothing.
    pub async fn execute(&self, command: RecordAskProgressCommand) -> Result<Ask, AskUseCaseError> {
        let mut ask = GetAsk::new(self.ask_repository.clone())
            .execute(command.id)
            .await?;
        let event = self
            .ask_workflow_service
            .advance(&mut ask, command.step, self.clock.now())?;

        let ask = self
            .ask_repository
            .update(&ask)
            .await
            .map_err(AskUseCaseError::Persistence)?;

        self.event_publisher
            .publish(event)
            .await
            .map_err(AskUseCaseError::Event)?;

        Ok(ask)
    }
}

pub struct RecordAskProgressCommand {
    pub id: AskId,
    pub step: AskStep,
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

    fn step(id: AskId, step: AskStep) -> RecordAskProgressCommand {
        RecordAskProgressCommand { id, step }
    }

    #[tokio::test]
    async fn every_step_is_recorded_in_order_with_its_event() {
        let harness = Harness::new();
        let id = harness
            .create
            .execute(question("a table"))
            .await
            .unwrap()
            .id();

        for next in [
            AskStep::Started,
            AskStep::Ranked("{}".into()),
            AskStep::Explaining,
            AskStep::Explained("Cedro ".into()),
            AskStep::Explained("is steady.".into()),
            AskStep::Answered { complete: true },
        ] {
            harness.record.execute(step(id, next)).await.unwrap();
        }
        let ask = harness.get.execute(id).await.unwrap();

        assert_eq!(ask.state(), AskState::Answered);
        assert_eq!(ask.explanation().as_str(), "Cedro is steady.");
        assert_eq!(ask.ranking().unwrap().as_str(), "{}");
        assert_eq!(ask.updated_at(), Timestamp(7));
        assert_eq!(harness.events.events().len(), 7, "opened + six steps");
        assert_eq!(
            harness.events.events().last(),
            Some(&AskEvent::AskAnswered {
                ask_id: id,
                complete: true
            })
        );
    }

    #[tokio::test]
    async fn a_forbidden_step_is_refused_and_nothing_is_stored_or_published() {
        let harness = Harness::new();
        let id = harness
            .create
            .execute(question("a table"))
            .await
            .unwrap()
            .id();

        let early = harness
            .record
            .execute(step(id, AskStep::Answered { complete: true }))
            .await
            .unwrap_err();
        let failed = harness
            .record
            .execute(step(id, AskStep::Failed("model down".into())))
            .await
            .unwrap();
        let late = harness
            .record
            .execute(step(id, AskStep::Started))
            .await
            .unwrap_err();

        assert!(matches!(
            early,
            AskUseCaseError::Domain(AskError::InvalidTransition { .. })
        ));
        assert!(matches!(
            late,
            AskUseCaseError::Domain(AskError::InvalidTransition { .. })
        ));
        assert_eq!(failed.failure().unwrap().as_str(), "model down");
        assert_eq!(harness.events.events().len(), 2, "opened + failed");
    }

    #[tokio::test]
    async fn unknown_ids_and_storage_failures_are_reported() {
        let missing = AskId::new();
        let unknown = Harness::new()
            .record
            .execute(step(missing, AskStep::Started))
            .await
            .unwrap_err();

        let read_only = Harness::failing_on(Operation::Update);
        let id = read_only.create.execute(question("q")).await.unwrap().id();
        let update = read_only
            .record
            .execute(step(id, AskStep::Started))
            .await
            .unwrap_err();

        assert!(matches!(unknown, AskUseCaseError::NotFound(_)));
        assert!(matches!(
            update,
            AskUseCaseError::Persistence(RepositoryError::Unavailable)
        ));
    }

    #[tokio::test]
    async fn a_step_that_cannot_be_published_is_reported() {
        let harness = Harness::without_subscribers();
        let (ask, _) = AskWorkflowService::new()
            .open("a table", Timestamp(1))
            .unwrap();
        harness.repository.save(&ask).await.unwrap();

        let error = harness
            .record
            .execute(step(ask.id(), AskStep::Started))
            .await
            .unwrap_err();

        assert!(matches!(error, AskUseCaseError::Event(_)));
    }
}
