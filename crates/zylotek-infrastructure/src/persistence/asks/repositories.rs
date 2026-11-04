use zylotek_domain::{
    asks::{
        entities::{self as domain, StoredAsk},
        repositories::AskRepository,
        value_objects::{Explanation, FailureReason, Question, Ranking, Timestamp},
    },
    errors::RepositoryError,
};

use crate::persistence::asks::models::Ask;

pub struct SqliteAskRepository {
    db: toasty::Db,
}

impl SqliteAskRepository {
    pub fn new(db: toasty::Db) -> Self {
        Self { db }
    }
}

#[async_trait::async_trait]
impl AskRepository for SqliteAskRepository {
    async fn find_by_id(&self, id: domain::AskId) -> Result<Option<domain::Ask>, RepositoryError> {
        let mut db = self.db.clone();

        let model: Option<Ask> = Ask::filter_by_id(id.as_uuid())
            .first()
            .exec(&mut db)
            .await
            .map_err(database_error)?;

        model.map(to_domain).transpose()
    }

    async fn save(&self, ask: &domain::Ask) -> Result<domain::Ask, RepositoryError> {
        let mut db = self.db.clone();

        let model = Ask::create()
            .id(ask.id().as_uuid())
            .question(ask.question().as_str().to_owned())
            .state(ask.state().as_str().to_owned())
            .ranking(ask.ranking().map(|r| r.as_str().to_owned()))
            .explanation(ask.explanation().as_str().to_owned())
            .explanation_complete(ask.explanation_complete())
            .failure(ask.failure().map(|f| f.as_str().to_owned()))
            .created_at(ask.created_at().0)
            .updated_at(ask.updated_at().0)
            .exec(&mut db)
            .await
            .map_err(database_error)?;

        to_domain(model)
    }

    async fn update(&self, ask: &domain::Ask) -> Result<domain::Ask, RepositoryError> {
        let mut db = self.db.clone();

        Ask::filter_by_id(ask.id().as_uuid())
            .update()
            .state(ask.state().as_str().to_owned())
            .ranking(ask.ranking().map(|r| r.as_str().to_owned()))
            .explanation(ask.explanation().as_str().to_owned())
            .explanation_complete(ask.explanation_complete())
            .failure(ask.failure().map(|f| f.as_str().to_owned()))
            .updated_at(ask.updated_at().0)
            .exec(&mut db)
            .await
            .map_err(database_error)?;

        self.find_by_id(ask.id())
            .await?
            .ok_or(RepositoryError::OperationFailed)
    }
}

/// Stored rows are checked again on the way out, like woods are.
fn to_domain(model: Ask) -> Result<domain::Ask, RepositoryError> {
    let invalid = |error: zylotek_domain::asks::errors::AskError| {
        RepositoryError::InvalidData(error.to_string())
    };

    Ok(domain::Ask::restore(StoredAsk {
        id: domain::AskId::from_uuid(model.id),
        question: Question::new(&model.question).map_err(invalid)?,
        state: model.state.parse().map_err(invalid)?,
        ranking: model
            .ranking
            .map(Ranking::new)
            .transpose()
            .map_err(invalid)?,
        explanation: Explanation::new(&model.explanation),
        explanation_complete: model.explanation_complete,
        failure: model.failure.as_deref().map(FailureReason::new),
        created_at: Timestamp(model.created_at),
        updated_at: Timestamp(model.updated_at),
    }))
}

fn database_error(err: toasty::Error) -> RepositoryError {
    tracing::error!(error = %err, "ask repository query failed");
    RepositoryError::Database
}

#[cfg(test)]
mod tests {
    use zylotek_domain::asks::value_objects::AskState;

    use super::*;
    use crate::persistence::build_db;

    async fn repository() -> SqliteAskRepository {
        SqliteAskRepository::new(build_db("sqlite::memory:").await.unwrap())
    }

    #[tokio::test]
    async fn an_ask_is_saved_found_and_updated_step_by_step() {
        let repository = repository().await;
        let mut ask = domain::Ask::create(Question::new("a table").unwrap(), Timestamp(1)).0;

        let saved = repository.save(&ask).await.unwrap();
        ask.start(Timestamp(2)).unwrap();
        ask.rank(
            Ranking::new(r#"{"woods": 1}"#.into()).unwrap(),
            Timestamp(3),
        )
        .unwrap();
        ask.explain("Cedro é estável.", Timestamp(4)).unwrap();
        ask.answer(false, Timestamp(5)).unwrap();
        let updated = repository.update(&ask).await.unwrap();

        assert_eq!((saved.id(), saved.state()), (ask.id(), AskState::Queued));
        assert_eq!(updated.state(), AskState::Answered);
        assert_eq!(updated.ranking().unwrap().as_str(), r#"{"woods": 1}"#);
        assert_eq!(updated.explanation().as_str(), "Cedro é estável.");
        assert!(!updated.explanation_complete());
        assert_eq!(
            (updated.created_at(), updated.updated_at()),
            (Timestamp(1), Timestamp(5))
        );
    }

    #[tokio::test]
    async fn a_failure_is_stored_and_an_unknown_id_is_none() {
        let repository = repository().await;
        let mut ask = domain::Ask::create(Question::new("a table").unwrap(), Timestamp(1)).0;
        repository.save(&ask).await.unwrap();
        ask.fail(FailureReason::new("the model is down"), Timestamp(2))
            .unwrap();

        let failed = repository.update(&ask).await.unwrap();
        let missing = repository.find_by_id(domain::AskId::new()).await.unwrap();

        assert_eq!(failed.failure().unwrap().as_str(), "the model is down");
        assert!(missing.is_none());
    }

    #[tokio::test]
    async fn updating_an_ask_that_was_never_saved_fails() {
        let repository = repository().await;
        let ask = domain::Ask::create(Question::new("a table").unwrap(), Timestamp(1)).0;

        let error = repository.update(&ask).await.unwrap_err();

        assert!(matches!(error, RepositoryError::OperationFailed));
    }

    #[tokio::test]
    async fn a_row_that_does_not_make_sense_is_reported_not_returned() {
        let repository = repository().await;
        let mut db = repository.db.clone();
        let id = uuid::Uuid::new_v4();
        Ask::create()
            .id(id)
            .question("a table".to_owned())
            .state("lost".to_owned())
            .ranking(None)
            .explanation(String::new())
            .explanation_complete(false)
            .failure(None)
            .created_at(1)
            .updated_at(1)
            .exec(&mut db)
            .await
            .unwrap();

        let error = repository
            .find_by_id(domain::AskId::from_uuid(id))
            .await
            .unwrap_err();

        assert!(matches!(error, RepositoryError::InvalidData(message) if message.contains("lost")));
    }

    #[tokio::test]
    async fn an_ask_saved_with_its_ranking_keeps_it() {
        let repository = repository().await;
        let mut ask = domain::Ask::create(Question::new("a table").unwrap(), Timestamp(1)).0;
        ask.start(Timestamp(2)).unwrap();
        ask.rank(Ranking::new("{}".into()).unwrap(), Timestamp(3))
            .unwrap();

        let saved = repository.save(&ask).await.unwrap();

        assert_eq!(saved.ranking().unwrap().as_str(), "{}");
    }

    #[tokio::test]
    async fn saving_the_same_ask_twice_is_a_database_error() {
        let repository = repository().await;
        let ask = domain::Ask::create(Question::new("a table").unwrap(), Timestamp(1)).0;
        repository.save(&ask).await.unwrap();

        let error = repository.save(&ask).await.unwrap_err();

        assert!(matches!(error, RepositoryError::Database));
    }
}
