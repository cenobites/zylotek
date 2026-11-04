//! Answers a question about wood in three steps: the model reads the need, plain code ranks
//! the catalogue, the model explains the result. The model never picks a wood by itself, so
//! it cannot invent one, and a flaky explanation does not lose the ranking.

use std::sync::Arc;

use serde_json::json;
use zylotek_contracts::assistant::{AskResponse, Recommendation};

use crate::{
    catalogue::Catalogue,
    error::{AskError, MAX_QUESTION_CHARS},
    llm::{Message, Model, TextStream},
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
        let prepared = self.prepare(question).await?;
        let explanation = match prepared.explanation_messages() {
            Some(messages) => self.explain(&messages).await,
            None => None,
        };

        Ok(prepared.into_answer(explanation))
    }

    /// Like `ask`, but the explanation is handed back as a stream so a caller can show it as
    /// it is written. The ranking is already in `answer` when this returns.
    pub async fn ask_stream(&self, question: &str) -> Result<StreamedAnswer, AskError> {
        let prepared = self.prepare(question).await?;
        let explanation = match prepared.explanation_messages() {
            Some(messages) => match self.model.stream(&messages).await {
                Ok(stream) => Some(stream),
                Err(error) => {
                    tracing::warn!(%error, "the model could not start explaining the ranking");
                    None
                }
            },
            None => None,
        };

        Ok(StreamedAnswer {
            answer: prepared.into_answer(None),
            explanation,
        })
    }

    /// Validates the question, reads the need out of it and ranks the catalogue.
    async fn prepare(&self, question: &str) -> Result<Prepared, AskError> {
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

        Ok(Prepared {
            question: question.to_owned(),
            requirements,
            recommendations,
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
    async fn explain(&self, messages: &[Message]) -> Option<String> {
        match self.model.complete(messages, None).await {
            Ok(text) => Some(text),
            Err(error) => {
                tracing::warn!(%error, "the model could not explain the ranking");
                None
            }
        }
    }
}

/// An answer whose explanation is still being written.
pub struct StreamedAnswer {
    /// Everything but the explanation, which is `None` here.
    pub answer: AskResponse,
    /// `None` when there is nothing to explain or the model could not start.
    pub explanation: Option<TextStream>,
}

struct Prepared {
    question: String,
    requirements: Requirements,
    recommendations: Vec<Recommendation>,
    catalogue_size: usize,
}

impl Prepared {
    /// What to ask the model to explain, if there is anything to explain.
    fn explanation_messages(&self) -> Option<[Message; 2]> {
        if self.recommendations.is_empty() {
            return None;
        }
        let facts = json!(self.recommendations);
        // Small models drift into English unless the language is named, and named last.
        let language = match self.requirements.language.as_str() {
            "" => "the language the question is written in".to_owned(),
            named => format!("{named}, the language the question is written in"),
        };

        Some([
            Message::system(format!(
                "You are the Project Wood Recommender, an expert woodworker advising on which \
                 Brazilian species to use for a project. Write your whole answer in {language}. \
                 Using only the facts given, say in at most 120 words why the top woods suit the \
                 project, the tradeoffs between them, which one is the better alternative for \
                 which case, and what to watch for. 'fit' is 0 (worst) to 1 (best) among all \
                 Brazilian woods in the database. Do not mention woods that are not listed and \
                 do not invent figures."
            )),
            Message::user(format!(
                "Question: {}\n\nWoods, best first:\n{facts}\n\nAnswer in {language}.",
                self.question
            )),
        ])
    }

    fn into_answer(self, explanation: Option<String>) -> AskResponse {
        AskResponse {
            summary: self.requirements.summary,
            criteria: self.requirements.criteria,
            recommendations: self.recommendations,
            explanation,
            catalogue_size: self.catalogue_size,
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

        /// Replays the next reply word by word. A reply holding `<<cut>>` ends there with
        /// an error item, as if the connection had dropped.
        async fn stream(&self, messages: &[Message]) -> Result<TextStream, ModelError> {
            self.calls.lock().unwrap().push((messages.to_vec(), false));
            let reply = self
                .replies
                .lock()
                .unwrap()
                .pop()
                .unwrap_or(Err(ModelError::Empty))?;
            let (text, cut) = match reply.split_once("<<cut>>") {
                Some((before, _)) => (before.to_owned(), true),
                None => (reply, false),
            };
            let mut items: Vec<Result<String, ModelError>> = text
                .split_inclusive(' ')
                .map(|word| Ok(word.to_owned()))
                .collect();
            if cut {
                items.push(Err(ModelError::Empty));
            }

            Ok(Box::pin(futures::stream::iter(items)))
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
    async fn the_explanation_is_asked_for_in_the_language_of_the_question() {
        let need = r#"{"language": "Brazilian Portuguese", "summary": "uma mesa dura", "priorities": [{"property": "hardness", "weight": 3}]}"#;
        let named = ScriptedModel::new(vec![Ok(need.into()), Ok("ok".into())]);
        let unnamed = ScriptedModel::new(vec![Ok(NEED.into()), Ok("ok".into())]);

        let answer = assistant(named.clone(), 5)
            .ask("quero uma mesa dura")
            .await
            .unwrap();
        assistant(unnamed.clone(), 5)
            .ask("a hard table")
            .await
            .unwrap();

        assert_eq!(answer.summary, "uma mesa dura");
        let calls = named.calls.lock().unwrap();
        let (system, user) = (&calls[1].0[0].content, &calls[1].0[1].content);
        assert!(system.contains("Write your whole answer in Brazilian Portuguese"));
        assert!(
            user.ends_with(
                "Answer in Brazilian Portuguese, the language the question is written in."
            )
        );
        let calls = unnamed.calls.lock().unwrap();
        assert!(
            calls[1].0[1]
                .content
                .ends_with("Answer in the language the question is written in."),
            "without a name the model is still told to follow the question"
        );
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

    async fn written(stream: TextStream) -> String {
        use futures::StreamExt;

        stream
            .map(Result::unwrap)
            .collect::<Vec<_>>()
            .await
            .concat()
    }

    #[tokio::test]
    async fn a_streamed_answer_has_the_ranking_at_once_and_the_words_as_they_come() {
        let model = ScriptedModel::new(vec![
            Ok(NEED.into()),
            Ok("Hard is the hardest wood.".into()),
        ]);

        let streamed = assistant(model.clone(), 2)
            .ask_stream(" a hard table ")
            .await
            .unwrap();

        assert_eq!(streamed.answer.explanation, None);
        assert_eq!(streamed.answer.recommendations[0].name, "Hard");
        assert_eq!(streamed.answer.catalogue_size, 3);
        assert_eq!(
            written(streamed.explanation.unwrap()).await,
            "Hard is the hardest wood."
        );
        let calls = model.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert!(calls[1].0[1].content.contains("a hard table"));
    }

    #[tokio::test]
    async fn nothing_to_explain_or_a_model_that_cannot_start_leaves_no_stream() {
        let none = r#"{"priorities": [{"property": "lightness", "weight": 2}]}"#;
        let nothing = ScriptedModel::new(vec![Ok(none.into())]);
        let busy = ScriptedModel::new(vec![Ok(NEED.into()), Err(ModelError::Empty)]);

        let nothing = assistant(nothing, 5)
            .ask_stream("a light box")
            .await
            .unwrap();
        let busy = assistant(busy, 5).ask_stream("a hard table").await.unwrap();

        assert!(nothing.answer.recommendations.is_empty() && nothing.explanation.is_none());
        assert_eq!(busy.answer.recommendations.len(), 3, "the ranking survives");
        assert!(busy.explanation.is_none());
    }

    #[tokio::test]
    async fn streaming_refuses_the_same_questions_asking_does() {
        let model = ScriptedModel::new(vec![]);
        let unsynced = Assistant::new(model.clone(), Arc::new(Catalogue::new()), 5);

        assert!(matches!(
            assistant(model.clone(), 5).ask_stream(" ").await,
            Err(AskError::EmptyQuestion)
        ));
        assert!(matches!(
            unsynced.ask_stream("x").await,
            Err(AskError::EmptyCatalogue)
        ));
        assert!(matches!(
            assistant(model, 5).ask_stream("x").await,
            Err(AskError::Model(_))
        ));
    }
}
