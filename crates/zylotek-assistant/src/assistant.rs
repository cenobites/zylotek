//! Answers a question about wood in three steps: the model reads the need, plain code ranks
//! the catalogue, the model explains the result. The model never picks a wood by itself, so
//! it cannot invent one, and a flaky explanation does not lose the ranking.

use std::sync::Arc;

use serde_json::json;
use zylotek_contracts::assistant::{AskResponse, Recommendation};

use crate::{
    catalogue::Catalogue,
    error::{AskError, MAX_QUESTION_CHARS},
    llm::{Message, Model},
    requirements::{self, Requirements},
};

/// How many woods an answer lists.
pub const DEFAULT_TOP: usize = 5;

pub struct Assistant {
    model: Arc<dyn Model>,
    catalogue: Arc<Catalogue>,
    top: usize,
}

impl Assistant {
    pub fn new(model: Arc<dyn Model>, catalogue: Arc<Catalogue>, top: usize) -> Self {
        Self {
            model,
            catalogue,
            top,
        }
    }

    pub fn catalogue(&self) -> &Catalogue {
        &self.catalogue
    }

    pub async fn ask(&self, question: &str) -> Result<AskResponse, AskError> {
        let question = question.trim();
        if question.is_empty() {
            return Err(AskError::EmptyQuestion);
        }
        if question.chars().count() > MAX_QUESTION_CHARS {
            return Err(AskError::QuestionTooLong);
        }
        let snapshot = self
            .catalogue
            .snapshot()
            .filter(|snapshot| !snapshot.ranker.is_empty())
            .ok_or(AskError::EmptyCatalogue)?;

        let requirements = self.understand(question).await?;
        let recommendations = snapshot.ranker.rank(&requirements, self.top);
        let explanation = self.explain(question, &recommendations).await;

        Ok(AskResponse {
            summary: requirements.summary,
            criteria: requirements.criteria,
            recommendations,
            explanation,
            catalogue_size: snapshot.ranker.len(),
        })
    }

    async fn understand(&self, question: &str) -> Result<Requirements, AskError> {
        let messages = [
            Message::system(requirements::INSTRUCTIONS),
            Message::user(question),
        ];
        let reply = self
            .model
            .complete(&messages, Some(&requirements::schema()))
            .await?;

        Requirements::parse(&reply)
    }

    /// A failure here is logged and dropped: the ranking is the answer, the words are a courtesy.
    async fn explain(&self, question: &str, recommendations: &[Recommendation]) -> Option<String> {
        if recommendations.is_empty() {
            return None;
        }
        let facts = json!(recommendations);
        let messages = [
            Message::system(
                "You advise on choosing wood. Using only the facts given, in the language of the \
                 question, say in at most 120 words why the top woods suit the need and what to \
                 watch for. 'fit' is 0 (worst) to 1 (best) among all Brazilian woods in the \
                 database. Do not mention woods that are not listed and do not invent figures.",
            ),
            Message::user(format!(
                "Question: {question}\n\nWoods, best first:\n{facts}"
            )),
        ];

        match self.model.complete(&messages, None).await {
            Ok(text) => Some(text),
            Err(error) => {
                tracing::warn!(%error, "the model could not explain the ranking");
                None
            }
        }
    }
}

#[cfg(test)]
pub mod testing {
    use std::sync::Mutex;

    use async_trait::async_trait;
    use serde_json::Value;

    use crate::{error::ModelError, llm::Model};

    use super::*;

    /// Plays back canned replies in order and remembers what it was asked.
    pub struct ScriptedModel {
        replies: Mutex<Vec<Result<String, ModelError>>>,
        pub calls: Mutex<Vec<(Vec<Message>, bool)>>,
    }

    impl ScriptedModel {
        pub fn new(replies: Vec<Result<String, ModelError>>) -> Arc<Self> {
            let mut replies = replies;
            replies.reverse();

            Arc::new(Self {
                replies: Mutex::new(replies),
                calls: Mutex::new(Vec::new()),
            })
        }
    }

    #[async_trait]
    impl Model for ScriptedModel {
        async fn complete(
            &self,
            messages: &[Message],
            schema: Option<&Value>,
        ) -> Result<String, ModelError> {
            self.calls
                .lock()
                .unwrap()
                .push((messages.to_vec(), schema.is_some()));

            self.replies
                .lock()
                .unwrap()
                .pop()
                .unwrap_or(Err(ModelError::Empty))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{testing::*, *};
    use crate::{error::ModelError, ranking::testing::hardness};

    const NEED: &str = r#"{"summary": "a hard table", "priorities": [{"property": "hardness", "weight": 3}], "avoid": []}"#;

    fn catalogue() -> Arc<Catalogue> {
        let catalogue = Arc::new(Catalogue::new());
        catalogue.replace(vec![
            hardness("Soft", 300.0),
            hardness("Hard", 1500.0),
            hardness("Middle", 900.0),
        ]);

        catalogue
    }

    fn assistant(model: Arc<ScriptedModel>, top: usize) -> Assistant {
        Assistant::new(model, catalogue(), top)
    }

    #[tokio::test]
    async fn it_understands_ranks_and_explains() {
        let model = ScriptedModel::new(vec![Ok(NEED.into()), Ok("Hard is hardest.".into())]);
        let assistant = assistant(model.clone(), 2);

        let answer = assistant.ask("  I need a hard table ").await.unwrap();

        assert_eq!(answer.summary, "a hard table");
        assert_eq!(answer.catalogue_size, 3);
        assert_eq!(answer.criteria.len(), 1);
        assert_eq!(
            answer
                .recommendations
                .iter()
                .map(|r| r.name.as_str())
                .collect::<Vec<_>>(),
            ["Hard", "Middle"]
        );
        assert_eq!(answer.explanation.as_deref(), Some("Hard is hardest."));
        assert_eq!(assistant.catalogue().snapshot().unwrap().ranker.len(), 3);
    }

    #[tokio::test]
    async fn the_model_is_asked_for_json_first_and_prose_second() {
        let model = ScriptedModel::new(vec![Ok(NEED.into()), Ok("ok".into())]);

        assistant(model.clone(), 5)
            .ask("a hard table")
            .await
            .unwrap();

        let calls = model.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        let (extract, schema) = &calls[0];
        assert!(schema, "extraction is constrained by the schema");
        assert_eq!(extract[1].content, "a hard table");
        let (explain, schema) = &calls[1];
        assert!(!schema);
        assert!(explain[1].content.contains("Hard scientificus"));
        assert!(explain[1].content.contains("a hard table"));
    }

    #[tokio::test]
    async fn a_failed_explanation_still_returns_the_ranking() {
        let model = ScriptedModel::new(vec![Ok(NEED.into()), Err(ModelError::Empty)]);

        let answer = assistant(model, 5).ask("a hard table").await.unwrap();

        assert_eq!(answer.recommendations.len(), 3);
        assert_eq!(answer.explanation, None);
    }

    #[tokio::test]
    async fn no_matching_woods_means_nothing_to_explain() {
        let need = r#"{"priorities": [{"property": "lightness", "weight": 2}]}"#;
        let model = ScriptedModel::new(vec![Ok(need.into())]);

        let answer = assistant(model.clone(), 5)
            .ask("a light box")
            .await
            .unwrap();

        assert!(answer.recommendations.is_empty());
        assert_eq!(answer.explanation, None);
        assert_eq!(model.calls.lock().unwrap().len(), 1, "no second call");
    }

    #[tokio::test]
    async fn bad_questions_never_reach_the_model() {
        let model = ScriptedModel::new(vec![]);
        let assistant = assistant(model.clone(), 5);

        let empty = assistant.ask("   \n").await.unwrap_err();
        let long = assistant
            .ask(&"x".repeat(MAX_QUESTION_CHARS + 1))
            .await
            .unwrap_err();
        let limit = assistant
            .ask(&"x".repeat(MAX_QUESTION_CHARS))
            .await
            .unwrap_err();

        assert!(matches!(empty, AskError::EmptyQuestion));
        assert!(matches!(long, AskError::QuestionTooLong));
        assert!(
            matches!(limit, AskError::Model(_)),
            "exactly the limit is allowed"
        );
        assert_eq!(model.calls.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn without_a_catalogue_there_is_nothing_to_answer_with() {
        let model = ScriptedModel::new(vec![]);
        let unsynced = Assistant::new(model.clone(), Arc::new(Catalogue::new()), 5);
        let empty_catalogue = Arc::new(Catalogue::new());
        empty_catalogue.replace(Vec::new());
        let empty = Assistant::new(model.clone(), empty_catalogue, 5);

        assert!(matches!(
            unsynced.ask("x").await,
            Err(AskError::EmptyCatalogue)
        ));
        assert!(matches!(
            empty.ask("x").await,
            Err(AskError::EmptyCatalogue)
        ));
        assert!(model.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_model_failure_or_nonsense_reading_is_an_error() {
        let down = ScriptedModel::new(vec![Err(ModelError::Transport("refused".into()))]);
        let nonsense = ScriptedModel::new(vec![Ok("I like trees".into())]);

        let down = assistant(down, 5).ask("x").await.unwrap_err();
        let nonsense = assistant(nonsense, 5).ask("x").await.unwrap_err();

        assert!(matches!(down, AskError::Model(ModelError::Transport(_))));
        assert!(matches!(nonsense, AskError::Understanding(_)));
    }
}
