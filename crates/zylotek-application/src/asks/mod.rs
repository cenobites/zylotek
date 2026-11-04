//! Use cases for asks: open one, read one, and record each step of answering it. The
//! answering itself happens elsewhere (the web app drives the assistant); these only keep
//! the record and refuse steps the state machine does not allow.

use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use zylotek_domain::{
    asks::{
        entities::{Ask, AskId, Question, Ranking, Timestamp},
        errors::AskError,
        repositories::AskRepository,
    },
    errors::RepositoryError,
};

#[derive(Debug, thiserror::Error)]
pub enum AskUseCaseError {
    #[error("ask {0} not found")]
    NotFound(AskId),

    #[error(transparent)]
    Domain(#[from] AskError),

    #[error("failed to persist ask")]
    Persistence(#[source] RepositoryError),
}

/// Where the use cases get the time from, so tests can fix it.
pub trait Clock: Send + Sync {
    fn now(&self) -> Timestamp;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis());

        Timestamp(u64::try_from(millis).unwrap_or(u64::MAX))
    }
}

/// One step of answering an ask.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    Started,
    /// The assistant's ranking document.
    Ranked(String),
    Explaining,
    /// More of the explanation.
    Explained(String),
    Answered {
        complete: bool,
    },
    Failed(String),
}

/// Everything the adapters need, built once.
pub struct AskFacade {
    repository: Arc<dyn AskRepository>,
    clock: Arc<dyn Clock>,
}

impl AskFacade {
    pub fn new(repository: Arc<dyn AskRepository>, clock: Arc<dyn Clock>) -> Self {
        Self { repository, clock }
    }

    /// A new ask, queued.
    pub async fn create(&self, question: &str) -> Result<Ask, AskUseCaseError> {
        let ask = Ask::create(Question::new(question)?, self.clock.now());

        self.repository
            .save(&ask)
            .await
            .map_err(AskUseCaseError::Persistence)
    }

    pub async fn get(&self, id: AskId) -> Result<Ask, AskUseCaseError> {
        self.repository
            .find_by_id(id)
            .await
            .map_err(AskUseCaseError::Persistence)?
            .ok_or(AskUseCaseError::NotFound(id))
    }

    /// Applies one step and stores the result. A step the state machine forbids is an error
    /// and changes nothing.
    pub async fn record(&self, id: AskId, progress: Progress) -> Result<Ask, AskUseCaseError> {
        let mut ask = self.get(id).await?;
        let now = self.clock.now();
        match progress {
            Progress::Started => ask.start(now)?,
            Progress::Ranked(document) => ask.rank(Ranking::new(document)?, now)?,
            Progress::Explaining => ask.start_explaining(now)?,
            Progress::Explained(text) => ask.explain(&text, now)?,
            Progress::Answered { complete } => ask.answer(complete, now)?,
            Progress::Failed(reason) => ask.fail(&reason, now)?,
        }

        self.repository
            .update(&ask)
            .await
            .map_err(AskUseCaseError::Persistence)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{
            Mutex,
            atomic::{AtomicBool, Ordering},
        },
    };

    use zylotek_domain::asks::entities::AskState;

    use super::*;

    #[derive(Default)]
    struct Memory {
        asks: Mutex<HashMap<AskId, Ask>>,
        broken: AtomicBool,
    }

    impl Memory {
        fn check(&self) -> Result<(), RepositoryError> {
            if self.broken.load(Ordering::SeqCst) {
                Err(RepositoryError::Database)
            } else {
                Ok(())
            }
        }
    }

    #[async_trait::async_trait]
    impl AskRepository for Memory {
        async fn find_by_id(&self, id: AskId) -> Result<Option<Ask>, RepositoryError> {
            self.check()?;
            Ok(self.asks.lock().unwrap().get(&id).cloned())
        }

        async fn save(&self, ask: &Ask) -> Result<Ask, RepositoryError> {
            self.check()?;
            self.asks.lock().unwrap().insert(ask.id(), ask.clone());
            Ok(ask.clone())
        }

        async fn update(&self, ask: &Ask) -> Result<Ask, RepositoryError> {
            self.save(ask).await
        }
    }

    struct Fixed(Mutex<u64>);

    impl Clock for Fixed {
        fn now(&self) -> Timestamp {
            let mut now = self.0.lock().unwrap();
            *now += 1;
            Timestamp(*now)
        }
    }

    fn facade() -> (AskFacade, Arc<Memory>) {
        let memory = Arc::new(Memory::default());
        let facade = AskFacade::new(memory.clone(), Arc::new(Fixed(Mutex::new(0))));

        (facade, memory)
    }

    #[tokio::test]
    async fn an_ask_is_created_queued_and_found_again() {
        let (asks, _) = facade();

        let created = asks.create(" a table ").await.unwrap();
        let found = asks.get(created.id()).await.unwrap();

        assert_eq!(found.state(), AskState::Queued);
        assert_eq!(found.question().as_str(), "a table");
        assert_eq!(found.created_at(), Timestamp(1));
    }

    #[tokio::test]
    async fn every_step_is_recorded_in_order() {
        let (asks, _) = facade();
        let id = asks.create("a table").await.unwrap().id();

        for step in [
            Progress::Started,
            Progress::Ranked("{}".into()),
            Progress::Explaining,
            Progress::Explained("Cedro ".into()),
            Progress::Explained("is steady.".into()),
            Progress::Answered { complete: true },
        ] {
            asks.record(id, step).await.unwrap();
        }
        let ask = asks.get(id).await.unwrap();

        assert_eq!(ask.state(), AskState::Answered);
        assert_eq!(ask.explanation(), "Cedro is steady.");
        assert_eq!(ask.ranking().unwrap().as_str(), "{}");
        assert_eq!(ask.updated_at(), Timestamp(7));
    }

    #[tokio::test]
    async fn a_forbidden_step_is_refused_and_nothing_is_stored() {
        let (asks, _) = facade();
        let id = asks.create("a table").await.unwrap().id();

        let error = asks
            .record(id, Progress::Answered { complete: true })
            .await
            .unwrap_err();
        let failed = asks
            .record(id, Progress::Failed("model down".into()))
            .await
            .unwrap();
        let empty = asks
            .record(id, Progress::Ranked(" ".into()))
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            AskUseCaseError::Domain(AskError::InvalidTransition { .. })
        ));
        assert_eq!(failed.failure(), Some("model down"));
        assert!(matches!(
            empty,
            AskUseCaseError::Domain(AskError::EmptyRanking)
        ));
    }

    #[tokio::test]
    async fn bad_input_unknown_ids_and_storage_failures_are_reported() {
        let (asks, memory) = facade();
        let missing = AskId::new();

        let blank = asks.create("  ").await.unwrap_err();
        let unknown = asks.record(missing, Progress::Started).await.unwrap_err();
        memory.broken.store(true, Ordering::SeqCst);
        let save = asks.create("a table").await.unwrap_err();
        let read = asks.get(missing).await.unwrap_err();

        assert!(matches!(
            blank,
            AskUseCaseError::Domain(AskError::InvalidQuestion { .. })
        ));
        assert_eq!(unknown.to_string(), format!("ask {missing} not found"));
        assert!(matches!(save, AskUseCaseError::Persistence(_)));
        assert_eq!(read.to_string(), "failed to persist ask");
    }

    #[tokio::test]
    async fn a_failed_update_is_reported() {
        struct ReadOnly(Memory);

        #[async_trait::async_trait]
        impl AskRepository for ReadOnly {
            async fn find_by_id(&self, id: AskId) -> Result<Option<Ask>, RepositoryError> {
                self.0.find_by_id(id).await
            }
            async fn save(&self, ask: &Ask) -> Result<Ask, RepositoryError> {
                self.0.save(ask).await
            }
            async fn update(&self, _: &Ask) -> Result<Ask, RepositoryError> {
                Err(RepositoryError::OperationFailed)
            }
        }
        let asks = AskFacade::new(Arc::new(ReadOnly(Memory::default())), Arc::new(SystemClock));
        let id = asks.create("a table").await.unwrap().id();

        let error = asks.record(id, Progress::Started).await.unwrap_err();

        assert!(matches!(
            error,
            AskUseCaseError::Persistence(RepositoryError::OperationFailed)
        ));
    }

    #[test]
    fn the_system_clock_is_after_2020() {
        assert!(SystemClock.now() > Timestamp(1_577_836_800_000));
    }
}
