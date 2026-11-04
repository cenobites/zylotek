use std::{fmt, str::FromStr};

use uuid::Uuid;

use crate::asks::errors::AskError;

pub const MAX_QUESTION_CHARS: usize = 1000;
/// An explanation is a short text; anything past this is dropped rather than stored.
pub const MAX_EXPLANATION_CHARS: usize = 20_000;
pub const MAX_FAILURE_CHARS: usize = 500;

/// Where an ask is in its life. Each step only moves forward:
///
/// ```text
/// Queued → Understanding → Ranked → Explaining → Answered
///    └──────────┴─────────────┴──────────┴─────→ Failed
/// ```
///
/// `Ranked → Answered` is allowed too, for an answer with nothing to explain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AskState {
    Queued,
    Understanding,
    Ranked,
    Explaining,
    Answered,
    Failed,
}

impl AskState {
    pub const ALL: [AskState; 6] = [
        AskState::Queued,
        AskState::Understanding,
        AskState::Ranked,
        AskState::Explaining,
        AskState::Answered,
        AskState::Failed,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            AskState::Queued => "queued",
            AskState::Understanding => "understanding",
            AskState::Ranked => "ranked",
            AskState::Explaining => "explaining",
            AskState::Answered => "answered",
            AskState::Failed => "failed",
        }
    }

    /// Answered and failed asks never change again.
    pub fn is_final(self) -> bool {
        matches!(self, AskState::Answered | AskState::Failed)
    }

    fn can_become(self, next: AskState) -> bool {
        use AskState::*;
        match (self, next) {
            (Queued, Understanding)
            | (Understanding, Ranked)
            | (Ranked, Explaining)
            | (Ranked, Answered)
            | (Explaining, Explaining)
            | (Explaining, Answered) => true,
            (from, Failed) => !from.is_final(),
            _ => false,
        }
    }
}

impl fmt::Display for AskState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for AskState {
    type Err = AskError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        AskState::ALL
            .into_iter()
            .find(|state| state.as_str() == value)
            .ok_or_else(|| AskError::InvalidState(value.to_owned()))
    }
}

/// What was asked, trimmed, between 1 and `MAX_QUESTION_CHARS` characters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question(String);

impl Question {
    pub fn new(text: &str) -> Result<Self, AskError> {
        let text = text.trim();
        if text.is_empty() || text.chars().count() > MAX_QUESTION_CHARS {
            return Err(AskError::InvalidQuestion {
                max: MAX_QUESTION_CHARS,
            });
        }

        Ok(Self(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The ranked woods and how the question was understood, as the assistant wrote them. The
/// domain keeps the document whole; its shape belongs to the assistant's contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ranking(String);

impl Ranking {
    pub fn new(document: String) -> Result<Self, AskError> {
        if document.trim().is_empty() {
            return Err(AskError::EmptyRanking);
        }

        Ok(Self(document))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Milliseconds since the Unix epoch. The domain is told the time; it never reads a clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Timestamp(pub u64);

/// Aggregate root: one question and its answer as far as it has got.
#[derive(Debug, Clone)]
pub struct Ask {
    id: AskId,
    question: Question,
    state: AskState,
    ranking: Option<Ranking>,
    explanation: String,
    /// False when the explanation was cut short; meaningful once answered.
    explanation_complete: bool,
    failure: Option<String>,
    created_at: Timestamp,
    updated_at: Timestamp,
}

/// Everything stored about an ask, for rebuilding it.
pub struct StoredAsk {
    pub id: AskId,
    pub question: Question,
    pub state: AskState,
    pub ranking: Option<Ranking>,
    pub explanation: String,
    pub explanation_complete: bool,
    pub failure: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl Ask {
    /// A new ask, waiting to be worked on.
    pub fn create(question: Question, now: Timestamp) -> Self {
        Self {
            id: AskId::new(),
            question,
            state: AskState::Queued,
            ranking: None,
            explanation: String::new(),
            explanation_complete: false,
            failure: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Rebuilds an ask from storage, without checking how it got there.
    pub fn restore(stored: StoredAsk) -> Self {
        Self {
            id: stored.id,
            question: stored.question,
            state: stored.state,
            ranking: stored.ranking,
            explanation: stored.explanation,
            explanation_complete: stored.explanation_complete,
            failure: stored.failure,
            created_at: stored.created_at,
            updated_at: stored.updated_at,
        }
    }

    fn become_(&mut self, next: AskState, now: Timestamp) -> Result<(), AskError> {
        if !self.state.can_become(next) {
            return Err(AskError::InvalidTransition {
                from: self.state,
                to: next,
            });
        }
        self.state = next;
        self.updated_at = self.updated_at.max(now);

        Ok(())
    }

    /// Work has started: the model is reading the question.
    pub fn start(&mut self, now: Timestamp) -> Result<(), AskError> {
        self.become_(AskState::Understanding, now)
    }

    /// The woods are ranked.
    pub fn rank(&mut self, ranking: Ranking, now: Timestamp) -> Result<(), AskError> {
        self.become_(AskState::Ranked, now)?;
        self.ranking = Some(ranking);

        Ok(())
    }

    /// The model has started writing the explanation.
    pub fn start_explaining(&mut self, now: Timestamp) -> Result<(), AskError> {
        self.become_(AskState::Explaining, now)
    }

    /// More of the explanation. Text past `MAX_EXPLANATION_CHARS` is dropped.
    pub fn explain(&mut self, text: &str, now: Timestamp) -> Result<(), AskError> {
        self.become_(AskState::Explaining, now)?;
        let room = MAX_EXPLANATION_CHARS.saturating_sub(self.explanation.chars().count());
        self.explanation.extend(text.chars().take(room));

        Ok(())
    }

    /// Done. `complete` is false when the explanation was cut short; the ranking stands.
    pub fn answer(&mut self, complete: bool, now: Timestamp) -> Result<(), AskError> {
        self.become_(AskState::Answered, now)?;
        self.explanation_complete = complete;

        Ok(())
    }

    /// Given up: the reason is kept, cut to `MAX_FAILURE_CHARS`.
    pub fn fail(&mut self, reason: &str, now: Timestamp) -> Result<(), AskError> {
        self.become_(AskState::Failed, now)?;
        let reason: String = reason.trim().chars().take(MAX_FAILURE_CHARS).collect();
        self.failure = Some(if reason.is_empty() {
            "unknown failure".to_owned()
        } else {
            reason
        });

        Ok(())
    }

    pub fn id(&self) -> AskId {
        self.id
    }

    pub fn question(&self) -> &Question {
        &self.question
    }

    pub fn state(&self) -> AskState {
        self.state
    }

    pub fn ranking(&self) -> Option<&Ranking> {
        self.ranking.as_ref()
    }

    pub fn explanation(&self) -> &str {
        &self.explanation
    }

    pub fn explanation_complete(&self) -> bool {
        self.explanation_complete
    }

    pub fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }

    pub fn created_at(&self) -> Timestamp {
        self.created_at
    }

    pub fn updated_at(&self) -> Timestamp {
        self.updated_at
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AskId(Uuid);

impl AskId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn from_uuid(value: Uuid) -> Self {
        Self(value)
    }

    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for AskId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for AskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for AskId {
    type Err = AskError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value)
            .map(Self)
            .map_err(|_| AskError::InvalidId(value.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::RepositoryError;

    const T0: Timestamp = Timestamp(1_000);

    fn ask() -> Ask {
        Ask::create(Question::new("  a table for 12  ").unwrap(), T0)
    }

    fn ranking() -> Ranking {
        Ranking::new(r#"{"recommendations": []}"#.into()).unwrap()
    }

    #[test]
    fn a_new_ask_is_queued_with_the_trimmed_question() {
        let ask = ask();

        assert_eq!(ask.state(), AskState::Queued);
        assert_eq!(ask.question().as_str(), "a table for 12");
        assert_eq!((ask.created_at(), ask.updated_at()), (T0, T0));
        assert_eq!(ask.ranking(), None);
        assert_eq!(
            (ask.explanation(), ask.explanation_complete(), ask.failure()),
            ("", false, None)
        );
        assert_ne!(ask.id(), Ask::create(Question::new("x").unwrap(), T0).id());
    }

    #[test]
    fn questions_must_be_present_and_short() {
        assert!(Question::new(&"x".repeat(MAX_QUESTION_CHARS)).is_ok());
        for bad in ["", "   ", &"x".repeat(MAX_QUESTION_CHARS + 1)] {
            let error = Question::new(bad).unwrap_err();
            assert_eq!(
                error.to_string(),
                "a question must have between 1 and 1000 characters"
            );
        }
        assert!(matches!(
            Ranking::new(" ".into()),
            Err(AskError::EmptyRanking)
        ));
        assert_eq!(ranking().as_str(), r#"{"recommendations": []}"#);
    }

    #[test]
    fn the_happy_path_walks_every_state_and_keeps_what_each_step_brings() {
        let mut ask = ask();

        ask.start(Timestamp(2_000)).unwrap();
        assert_eq!(ask.state(), AskState::Understanding);
        ask.rank(ranking(), Timestamp(3_000)).unwrap();
        assert_eq!(
            (ask.state(), ask.ranking()),
            (AskState::Ranked, Some(&ranking()))
        );
        ask.start_explaining(Timestamp(3_500)).unwrap();
        ask.explain("Cedro is ", Timestamp(4_000)).unwrap();
        ask.explain("steady.", Timestamp(4_500)).unwrap();
        assert_eq!(
            (ask.state(), ask.explanation()),
            (AskState::Explaining, "Cedro is steady.")
        );
        ask.answer(true, Timestamp(5_000)).unwrap();

        assert_eq!(ask.state(), AskState::Answered);
        assert!(ask.explanation_complete());
        assert_eq!(ask.updated_at(), Timestamp(5_000));
        assert_eq!(ask.created_at(), T0, "creation time never moves");
    }

    #[test]
    fn an_answer_with_nothing_to_explain_skips_explaining() {
        let mut ask = ask();
        ask.start(T0).unwrap();
        ask.rank(ranking(), T0).unwrap();

        ask.answer(true, T0).unwrap();

        assert_eq!(ask.state(), AskState::Answered);
    }

    #[test]
    fn a_cut_explanation_is_still_an_answer_but_says_so() {
        let mut ask = ask();
        ask.start(T0).unwrap();
        ask.rank(ranking(), T0).unwrap();
        ask.explain("Half", T0).unwrap();

        ask.answer(false, T0).unwrap();

        assert_eq!(
            (ask.state(), ask.explanation_complete()),
            (AskState::Answered, false)
        );
    }

    #[test]
    fn steps_cannot_be_skipped_repeated_or_undone() {
        let mut queued = ask();
        let error = queued.rank(ranking(), T0).unwrap_err();
        assert_eq!(error.to_string(), "an ask cannot go from queued to ranked");
        assert!(queued.explain("x", T0).is_err());
        assert!(queued.answer(true, T0).is_err());
        assert!(queued.start_explaining(T0).is_err());

        let mut answered = ask();
        answered.start(T0).unwrap();
        answered.rank(ranking(), T0).unwrap();
        answered.answer(true, T0).unwrap();
        for result in [
            answered.start(T0),
            answered.rank(ranking(), T0),
            answered.explain("x", T0),
            answered.answer(true, T0),
            answered.fail("late", T0),
        ] {
            assert!(matches!(
                result,
                Err(AskError::InvalidTransition {
                    from: AskState::Answered,
                    ..
                })
            ));
        }
        assert_eq!(
            answered.state(),
            AskState::Answered,
            "a rejected step changes nothing"
        );

        let mut understanding = ask();
        understanding.start(T0).unwrap();
        assert!(understanding.start(T0).is_err(), "no going round twice");
    }

    #[test]
    fn any_unfinished_ask_can_fail_and_the_reason_is_kept_short() {
        for steps in 0..4 {
            let mut ask = ask();
            if steps > 0 {
                ask.start(T0).unwrap();
            }
            if steps > 1 {
                ask.rank(ranking(), T0).unwrap();
            }
            if steps > 2 {
                ask.explain("x", T0).unwrap();
            }

            ask.fail(&"r".repeat(MAX_FAILURE_CHARS + 50), T0).unwrap();

            assert_eq!(ask.state(), AskState::Failed);
            assert_eq!(ask.failure().unwrap().len(), MAX_FAILURE_CHARS);
            assert!(ask.fail("again", T0).is_err(), "failed is final");
        }
        let mut blank = ask();
        blank.fail("  ", T0).unwrap();
        assert_eq!(blank.failure(), Some("unknown failure"));
    }

    #[test]
    fn the_explanation_stops_growing_at_its_limit() {
        let mut ask = ask();
        ask.start(T0).unwrap();
        ask.rank(ranking(), T0).unwrap();

        ask.explain(&"é".repeat(MAX_EXPLANATION_CHARS - 1), T0)
            .unwrap();
        ask.explain("abc", T0).unwrap();

        assert_eq!(ask.explanation().chars().count(), MAX_EXPLANATION_CHARS);
        assert!(ask.explanation().ends_with("éa"));
    }

    #[test]
    fn time_never_runs_backwards() {
        let mut ask = ask();

        ask.start(Timestamp(500)).unwrap();

        assert_eq!(ask.updated_at(), T0);
    }

    #[test]
    fn states_have_names_and_only_the_last_two_are_final() {
        for state in AskState::ALL {
            assert_eq!(state.as_str().parse::<AskState>().unwrap(), state);
            assert_eq!(state.to_string(), state.as_str());
            assert_eq!(
                state.is_final(),
                matches!(state, AskState::Answered | AskState::Failed)
            );
        }
        assert_eq!(
            "done".parse::<AskState>().unwrap_err().to_string(),
            "invalid ask state 'done'"
        );
    }

    #[test]
    fn a_stored_ask_comes_back_as_it_was() {
        let id = AskId::new();
        let ask = Ask::restore(StoredAsk {
            id,
            question: Question::new("q").unwrap(),
            state: AskState::Explaining,
            ranking: Some(ranking()),
            explanation: "half".into(),
            explanation_complete: false,
            failure: None,
            created_at: T0,
            updated_at: Timestamp(2_000),
        });

        assert_eq!(
            (ask.id(), ask.state(), ask.explanation()),
            (id, AskState::Explaining, "half")
        );
        assert_eq!(ask.updated_at(), Timestamp(2_000));
    }

    #[test]
    fn ids_parse_and_print() {
        let id = AskId::default();

        assert_eq!(id.to_string().parse::<AskId>().unwrap(), id);
        assert_eq!(AskId::from_uuid(id.as_uuid()), id);
        assert_eq!(
            "nope".parse::<AskId>().unwrap_err().to_string(),
            "invalid ask id 'nope'"
        );
        assert_eq!(
            AskError::from(RepositoryError::Database).to_string(),
            "repository error"
        );
    }
}
