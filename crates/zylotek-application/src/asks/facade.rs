use std::sync::Arc;

use zylotek_domain::asks::{
    entities::{Ask, AskId},
    services::AskStep,
};

use crate::asks::{
    create_ask::{CreateAsk, CreateAskCommand},
    errors::AskUseCaseError,
    get_ask::GetAsk,
    record_ask_progress::{RecordAskProgress, RecordAskProgressCommand},
};

/// The entry point adapters call for asks.
#[derive(Clone)]
pub struct AskFacade {
    create_ask: Arc<CreateAsk>,
    get_ask: Arc<GetAsk>,
    record_ask_progress: Arc<RecordAskProgress>,
}

impl AskFacade {
    pub fn new(
        create_ask: CreateAsk,
        get_ask: GetAsk,
        record_ask_progress: RecordAskProgress,
    ) -> Self {
        Self {
            create_ask: Arc::new(create_ask),
            get_ask: Arc::new(get_ask),
            record_ask_progress: Arc::new(record_ask_progress),
        }
    }

    pub async fn create_ask(&self, command: CreateAskCommand) -> Result<Ask, AskUseCaseError> {
        self.create_ask.execute(command).await
    }

    pub async fn get_ask(&self, id: AskId) -> Result<Ask, AskUseCaseError> {
        self.get_ask.execute(id).await
    }

    pub async fn record_ask_progress(
        &self,
        id: AskId,
        step: AskStep,
    ) -> Result<Ask, AskUseCaseError> {
        self.record_ask_progress
            .execute(RecordAskProgressCommand { id, step })
            .await
    }
}

#[cfg(test)]
mod tests {
    use zylotek_domain::asks::value_objects::AskState;

    use super::*;
    use crate::asks::support::{Harness, question};

    #[tokio::test]
    async fn drives_an_ask_through_its_whole_lifecycle_and_clones_share_state() {
        let harness = Harness::new();
        let events = harness.events.clone();
        let facade = harness.facade();
        let clone = facade.clone();

        let created = facade.create_ask(question("a table")).await.unwrap();
        clone
            .record_ask_progress(created.id(), AskStep::Started)
            .await
            .unwrap();
        clone
            .record_ask_progress(created.id(), AskStep::Failed("down".into()))
            .await
            .unwrap();
        let fetched = facade.get_ask(created.id()).await.unwrap();

        assert_eq!(fetched.state(), AskState::Failed);
        assert_eq!(events.events().len(), 3);
    }
}
