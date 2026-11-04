use std::{fmt, str::FromStr};

use uuid::Uuid;

use crate::asks::{
    errors::AskError,
    events::AskEvent,
    value_objects::{AskState, Explanation, FailureReason, Question, Ranking, Timestamp},
};

/// Aggregate root: one question and its answer as far as it has got. Fields are private;
/// every change goes through a method that checks the state machine and returns the
/// `AskEvent` it implies.
#[derive(Debug, Clone)]
pub struct Ask {
    id: AskId,
    question: Question,
    state: AskState,
    ranking: Option<Ranking>,
    explanation: Explanation,
    /// False when the explanation was cut short; meaningful once answered.
    explanation_complete: bool,
    failure: Option<FailureReason>,
    created_at: Timestamp,
    updated_at: Timestamp,
}

/// Everything stored about an ask, for rebuilding it.
pub struct StoredAsk {
    pub id: AskId,
    pub question: Question,
    pub state: AskState,
    pub ranking: Option<Ranking>,
    pub explanation: Explanation,
    pub explanation_complete: bool,
    pub failure: Option<FailureReason>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl Ask {
    /// A new ask, waiting to be worked on.
    pub fn create(question: Question, now: Timestamp) -> (Self, AskEvent) {
        let ask = Self {
            id: AskId::new(),
            question,
            state: AskState::Queued,
            ranking: None,
            explanation: Explanation::default(),
            explanation_complete: false,
            failure: None,
            created_at: now,
            updated_at: now,
        };
        let event = AskEvent::AskOpened { ask_id: ask.id };

        (ask, event)
    }

    /// Rebuilds an ask from storage, without checking how it got there or emitting events.
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
    pub fn start(&mut self, now: Timestamp) -> Result<AskEvent, AskError> {
        self.become_(AskState::Understanding, now)?;

        Ok(AskEvent::AskStarted { ask_id: self.id })
    }

    /// The woods are ranked.
    pub fn rank(&mut self, ranking: Ranking, now: Timestamp) -> Result<AskEvent, AskError> {
        self.become_(AskState::Ranked, now)?;
        self.ranking = Some(ranking);

        Ok(AskEvent::AskRanked { ask_id: self.id })
    }

    /// The model has started writing the explanation.
    pub fn start_explaining(&mut self, now: Timestamp) -> Result<AskEvent, AskError> {
        self.become_(AskState::Explaining, now)?;

        Ok(AskEvent::AskExplaining { ask_id: self.id })
    }

    /// More of the explanation, kept within its limit.
    pub fn explain(&mut self, text: &str, now: Timestamp) -> Result<AskEvent, AskError> {
        self.become_(AskState::Explaining, now)?;
        self.explanation = std::mem::take(&mut self.explanation).followed_by(text);

        Ok(AskEvent::AskExplained { ask_id: self.id })
    }

    /// Done. `complete` is false when the explanation was cut short; the ranking stands.
    pub fn answer(&mut self, complete: bool, now: Timestamp) -> Result<AskEvent, AskError> {
        self.become_(AskState::Answered, now)?;
        self.explanation_complete = complete;

        Ok(AskEvent::AskAnswered {
            ask_id: self.id,
            complete,
        })
    }

    /// Given up, with the reason kept.
    pub fn fail(&mut self, reason: FailureReason, now: Timestamp) -> Result<AskEvent, AskError> {
        self.become_(AskState::Failed, now)?;
        self.failure = Some(reason);

        Ok(AskEvent::AskFailed { ask_id: self.id })
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

    pub fn explanation(&self) -> &Explanation {
        &self.explanation
    }

    pub fn explanation_complete(&self) -> bool {
        self.explanation_complete
    }

    pub fn failure(&self) -> Option<&FailureReason> {
        self.failure.as_ref()
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
        Ask::create(Question::new("  a table for 12  ").unwrap(), T0).0
    }

    fn ranking() -> Ranking {
        Ranking::new(r#"{"recommendations": []}"#.into()).unwrap()
    }

    fn reason(text: &str) -> FailureReason {
        FailureReason::new(text)
    }

    #[test]
    fn a_new_ask_is_queued_with_the_trimmed_question_and_says_it_was_opened() {
        let (ask, event) = Ask::create(Question::new("  a table for 12  ").unwrap(), T0);

        assert_eq!(event, AskEvent::AskOpened { ask_id: ask.id() });
        assert_eq!(ask.state(), AskState::Queued);
        assert_eq!(ask.question().as_str(), "a table for 12");
        assert_eq!((ask.created_at(), ask.updated_at()), (T0, T0));
        assert_eq!(ask.ranking(), None);
        assert_eq!(
            (
                ask.explanation().as_str(),
                ask.explanation_complete(),
                ask.failure()
            ),
            ("", false, None)
        );
        assert_ne!(ask.id(), self::ask().id());
    }

    #[test]
    fn the_happy_path_walks_every_state_and_emits_an_event_per_step() {
        let mut ask = ask();
        let ask_id = ask.id();

        let events = [
            ask.start(Timestamp(2_000)).unwrap(),
            ask.rank(ranking(), Timestamp(3_000)).unwrap(),
            ask.start_explaining(Timestamp(3_500)).unwrap(),
            ask.explain("Cedro is ", Timestamp(4_000)).unwrap(),
            ask.explain("steady.", Timestamp(4_500)).unwrap(),
            ask.answer(true, Timestamp(5_000)).unwrap(),
        ];

        assert_eq!(
            events,
            [
                AskEvent::AskStarted { ask_id },
                AskEvent::AskRanked { ask_id },
                AskEvent::AskExplaining { ask_id },
                AskEvent::AskExplained { ask_id },
                AskEvent::AskExplained { ask_id },
                AskEvent::AskAnswered {
                    ask_id,
                    complete: true
                },
            ]
        );
        assert_eq!(ask.state(), AskState::Answered);
        assert_eq!(ask.ranking(), Some(&ranking()));
        assert_eq!(ask.explanation().as_str(), "Cedro is steady.");
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

        let event = ask.answer(false, T0).unwrap();

        assert!(matches!(
            event,
            AskEvent::AskAnswered {
                complete: false,
                ..
            }
        ));
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
            answered.fail(reason("late"), T0),
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
    fn any_unfinished_ask_can_fail_and_keeps_the_reason() {
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

            let event = ask.fail(reason("model down"), T0).unwrap();

            assert_eq!(event, AskEvent::AskFailed { ask_id: ask.id() });
            assert_eq!(ask.state(), AskState::Failed);
            assert_eq!(ask.failure().unwrap().as_str(), "model down");
            assert!(ask.fail(reason("again"), T0).is_err(), "failed is final");
        }
    }

    #[test]
    fn time_never_runs_backwards() {
        let mut ask = ask();

        ask.start(Timestamp(500)).unwrap();

        assert_eq!(ask.updated_at(), T0);
    }

    #[test]
    fn a_stored_ask_comes_back_as_it_was() {
        let id = AskId::new();
        let ask = Ask::restore(StoredAsk {
            id,
            question: Question::new("q").unwrap(),
            state: AskState::Explaining,
            ranking: Some(ranking()),
            explanation: Explanation::new("half"),
            explanation_complete: false,
            failure: None,
            created_at: T0,
            updated_at: Timestamp(2_000),
        });

        assert_eq!(
            (ask.id(), ask.state(), ask.explanation().as_str()),
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
