//! Answers a question about wood in steps: the model reads the need, plain code ranks the
//! catalogue, the model names the question's language (so the summary can be translated and the
//! explanation written in it), the model explains the result. The model never picks a wood by itself, so
//! it cannot invent one, and a flaky explanation does not lose the ranking.

use std::sync::Arc;

use zylotek_contracts::assistant::{AskResponse, Recommendation};

use crate::{
    catalogue::Catalogue,
    error::{AskError, MAX_QUESTION_CHARS},
    llm::{Message, Model, TextStream},
    ranking::describe,
    requirements::{self, MAX_SUMMARY_CHARS, Requirements},
};

/// How many woods an answer lists.
pub const DEFAULT_TOP: usize = 5;

/// The most model calls one ask makes: read the need, name the language, translate the summary,
/// explain. The HTTP timeouts are sized from it.
pub const MODEL_CALLS: u64 = 4;

/// Naming the language is its own plain call: asked together with reading the need, the model
/// called a Portuguese question "English"; asked alone it gets it right.
const LANGUAGE_INSTRUCTIONS: &str = "\
Name the language the user's text is written in. Reply with the English name of the language only, \
such as English, Portuguese, Spanish, French or German.";

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

        let mut requirements = self.understand(question).await?;
        let recommendations = snapshot.ranker.rank(&requirements, self.top);

        // The language only matters for the words: the summary and the explanation.
        let language = if requirements.summary.is_empty() && recommendations.is_empty() {
            None
        } else {
            self.language_of(question).await
        };
        if let Some(language) = language.as_deref().filter(|name| !is_english(name))
            && !requirements.summary.is_empty()
        {
            requirements.summary = self
                .translate(question, &requirements.summary, language)
                .await;
        }

        Ok(Prepared {
            question: question.to_owned(),
            requirements,
            recommendations,
            language,
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

    /// The English name of the language the question is written in, if the model gives one. A
    /// failure is logged and dropped: the words then follow the question without a name.
    async fn language_of(&self, question: &str) -> Option<String> {
        let messages = [
            Message::system(LANGUAGE_INSTRUCTIONS),
            Message::user(question),
        ];
        match self.model.complete(&messages, None).await {
            Ok(text) => {
                let name = first_line(&text).trim_end_matches('.');

                // A name is a word or two; anything longer is the model explaining itself.
                (!name.is_empty()
                    && name.chars().count() <= 40
                    && name.split_whitespace().count() <= 3)
                    .then(|| name.to_owned())
            }
            Err(error) => {
                tracing::warn!(%error, "the model could not name the language of the question");
                None
            }
        }
    }

    /// The summary in `language`. The question rides along so the words the person used for the
    /// project survive: translated from English alone, a "bancada" came back as a "banqueta".
    /// A failure keeps the summary as it was: it is a courtesy, like the explanation.
    async fn translate(&self, question: &str, summary: &str, language: &str) -> String {
        let messages = [
            Message::system(format!(
                "You translate one sentence into {language}. Use the words the question uses for \
                 the project. If the sentence is already in {language}, repeat it unchanged. \
                 Reply with the translation only: no quotes, no notes."
            )),
            Message::user(format!("Question: {question}\n\nSentence: {summary}")),
        ];
        match self.model.complete(&messages, None).await {
            Ok(text) => {
                let translated: String =
                    first_line(&text).chars().take(MAX_SUMMARY_CHARS).collect();

                if translated.is_empty() {
                    summary.to_owned()
                } else {
                    translated
                }
            }
            Err(error) => {
                tracing::warn!(%error, "the model could not translate the summary");
                summary.to_owned()
            }
        }
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

/// The first non-blank line of a reply, without the quotes a model likes to wrap it in.
fn first_line(text: &str) -> &str {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .trim_matches(|c| matches!(c, '"' | '\'' | '“' | '”' | '«' | '»'))
}

fn is_english(language: &str) -> bool {
    language.eq_ignore_ascii_case("english")
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
    /// The English name of the question's language, when the model gave one.
    language: Option<String>,
    catalogue_size: usize,
}

impl Prepared {
    /// What to ask the model to explain, if there is anything to explain.
    fn explanation_messages(&self) -> Option<[Message; 2]> {
        if self.recommendations.is_empty() {
            return None;
        }
        // The facts are numbered and labelled: given bare JSON, a small model reordered the
        // woods and read a density as a "lightness score".
        let facts = self
            .recommendations
            .iter()
            .enumerate()
            .map(|(index, wood)| {
                let reasons = wood
                    .reasons
                    .iter()
                    .map(|reason| {
                        format!(
                            "   - {}; fit {:.2}",
                            describe(reason.property, reason.value),
                            reason.fit
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");

                format!(
                    "{}. {} ({}), overall {:.0}%\n{reasons}",
                    index + 1,
                    wood.name,
                    wood.scientific_name,
                    wood.score * 100.0
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        // Small models drift into English unless the language is named, and named last.
        let language = match self.language.as_deref() {
            None => "the language the question is written in".to_owned(),
            Some(named) => format!("{named}, the language the question is written in"),
        };

        Some([
            Message::system(format!(
                "You are the Project Wood Recommender, an expert woodworker advising on which \
                 Brazilian species to use for a project. Write your whole answer in {language}. \
                 Using only the facts given, say in at most 120 words why the top woods suit the \
                 project, the tradeoffs between them, which one is the better alternative for \
                 which case, and what to watch for. The woods are numbered best first: number 1 \
                 is the best overall match, so never call a lower-ranked wood the best overall. \
                 'fit' is 0 (worst) to 1 (best) among all Brazilian woods in the database. Do not \
                 mention woods that are not listed and do not invent figures."
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

    /// What the model answers when asked to put the summary in Portuguese.
    const TRANSLATED: &str = "uma mesa dura";

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
    async fn it_understands_ranks_translates_and_explains() {
        let model = ScriptedModel::new(vec![
            Ok(NEED.into()),
            Ok("Portuguese".into()),
            Ok(TRANSLATED.into()),
            Ok("Hard is hardest.".into()),
        ]);
        let assistant = assistant(model.clone(), 2);

        let answer = assistant.ask("  quero uma mesa dura ").await.unwrap();

        assert_eq!(answer.summary, TRANSLATED);
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
    async fn an_english_question_is_not_translated() {
        let model = ScriptedModel::new(vec![
            Ok(NEED.into()),
            Ok(" \"English.\" ".into()),
            Ok("ok".into()),
        ]);

        let answer = assistant(model.clone(), 5)
            .ask("a hard table")
            .await
            .unwrap();

        assert_eq!(answer.summary, "a hard table");
        let calls = model.calls.lock().unwrap();
        assert_eq!(calls.len(), 3, "read the need, name the language, explain");
        let (extract, schema) = &calls[0];
        assert!(schema, "extraction is constrained by the schema");
        assert_eq!(extract[1].content, "a hard table");
        let (naming, schema) = &calls[1];
        assert!(!schema);
        assert_eq!(naming[1].content, "a hard table");
        let (explain, schema) = &calls[2];
        assert!(!schema);
        assert!(explain[1].content.contains("Hard scientificus"));
        assert!(explain[1].content.contains("a hard table"));
        assert!(
            explain[0]
                .content
                .contains("Write your whole answer in English, the language")
        );
    }

    #[tokio::test]
    async fn the_summary_and_the_explanation_follow_the_language_the_model_names() {
        // The model wrote the summary in English for a Portuguese question.
        let model = ScriptedModel::new(vec![
            Ok(NEED.into()),
            Ok("Portuguese.\n".into()),
            Ok("  \"Uma mesa dura\"\n(translation of the sentence)".into()),
            Ok("ok".into()),
        ]);

        let answer = assistant(model.clone(), 5)
            .ask("quero uma mesa dura")
            .await
            .unwrap();

        assert_eq!(answer.summary, "Uma mesa dura", "first line, unquoted");
        let calls = model.calls.lock().unwrap();
        assert_eq!(calls[1].0[1].content, "quero uma mesa dura");
        let (system, user) = (&calls[2].0[0].content, &calls[2].0[1].content);
        assert!(system.contains("translate one sentence into Portuguese"));
        assert_eq!(
            user, "Question: quero uma mesa dura\n\nSentence: a hard table",
            "the question rides along so its own words survive"
        );
        let (system, user) = (&calls[3].0[0].content, &calls[3].0[1].content);
        assert!(system.contains(
            "Write your whole answer in Portuguese, the language the question is written in."
        ));
        assert!(user.ends_with("Answer in Portuguese, the language the question is written in."));
    }

    #[tokio::test]
    async fn without_a_language_name_the_summary_stays_and_the_explanation_follows_the_question() {
        let failed = ScriptedModel::new(vec![
            Ok(NEED.into()),
            Err(ModelError::Empty),
            Ok("ok".into()),
        ]);
        let rambling = ScriptedModel::new(vec![
            Ok(NEED.into()),
            Ok("The text is written in Portuguese, I believe.".into()),
            Ok("ok".into()),
        ]);

        let failed_answer = assistant(failed.clone(), 5)
            .ask("a hard table")
            .await
            .unwrap();
        let rambling_answer = assistant(rambling.clone(), 5)
            .ask("a hard table")
            .await
            .unwrap();

        for (answer, model) in [(failed_answer, failed), (rambling_answer, rambling)] {
            assert_eq!(answer.summary, "a hard table");
            assert_eq!(answer.explanation.as_deref(), Some("ok"));
            let calls = model.calls.lock().unwrap();
            assert_eq!(calls.len(), 3, "no translation without a language");
            assert!(
                calls[2].0[1]
                    .content
                    .ends_with("Answer in the language the question is written in."),
                "without a name the model is still told to follow the question"
            );
        }
    }

    #[tokio::test]
    async fn a_failed_or_empty_translation_keeps_the_summary_as_it_was() {
        let failed = ScriptedModel::new(vec![
            Ok(NEED.into()),
            Ok("Portuguese".into()),
            Err(ModelError::Empty),
            Ok("ok".into()),
        ]);
        let blank = ScriptedModel::new(vec![
            Ok(NEED.into()),
            Ok("Portuguese".into()),
            Ok("  \n ".into()),
            Ok("ok".into()),
        ]);

        let failed = assistant(failed, 5).ask("quero uma mesa").await.unwrap();
        let blank = assistant(blank, 5).ask("quero uma mesa").await.unwrap();

        assert_eq!(failed.summary, "a hard table");
        assert_eq!(failed.explanation.as_deref(), Some("ok"));
        assert_eq!(blank.summary, "a hard table");
    }

    #[tokio::test]
    async fn the_explanation_is_given_numbered_labelled_facts_best_first() {
        let need = r#"{"priorities": [{"property": "hardness", "weight": 3}]}"#;
        let model =
            ScriptedModel::new(vec![Ok(need.into()), Ok("English".into()), Ok("ok".into())]);

        assistant(model.clone(), 2)
            .ask("a hard table")
            .await
            .unwrap();

        let calls = model.calls.lock().unwrap();
        let facts = &calls[2].0[1].content;
        assert!(facts.contains(
            "1. Hard (Hard scientificus), overall 83%\n   - hardness (Janka) 1500, higher is harder; fit 0.83"
        ));
        assert!(facts.find("1. Hard").unwrap() < facts.find("2. Middle").unwrap());
        assert!(
            calls[2].0[0]
                .content
                .contains("never call a lower-ranked wood the best overall")
        );
    }

    #[tokio::test]
    async fn a_failed_explanation_still_returns_the_ranking() {
        let model = ScriptedModel::new(vec![
            Ok(NEED.into()),
            Ok("English".into()),
            Err(ModelError::Empty),
        ]);

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
            Ok("English".into()),
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
        assert_eq!(calls.len(), 3);
        assert!(calls[2].0[1].content.contains("a hard table"));
    }

    #[tokio::test]
    async fn nothing_to_explain_or_a_model_that_cannot_start_leaves_no_stream() {
        let none = r#"{"priorities": [{"property": "lightness", "weight": 2}]}"#;
        let nothing = ScriptedModel::new(vec![Ok(none.into())]);
        let busy = ScriptedModel::new(vec![
            Ok(NEED.into()),
            Ok("English".into()),
            Err(ModelError::Empty),
        ]);

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
