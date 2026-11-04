use crate::asks::{
    entities::Ask,
    errors::AskError,
    events::AskEvent,
    value_objects::{FailureReason, Question, Ranking, Timestamp},
};

/// One step of answering an ask, as reported by whoever answers it (the web app, driving the
/// assistant). Raw values: the service turns them into value objects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AskStep {
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

/// Opens asks and moves them along their state machine, one reported step at a time.
#[derive(Debug, Default, Clone, Copy)]
pub struct AskWorkflowService;

impl AskWorkflowService {
    pub fn new() -> Self {
        Self
    }

    /// A new ask for `question`, queued.
    pub fn open(&self, question: &str, now: Timestamp) -> Result<(Ask, AskEvent), AskError> {
        Ok(Ask::create(Question::new(question)?, now))
    }

    /// Applies `step` to `ask`. A step the state machine forbids, or a blank ranking, is an
    /// error and changes nothing.
    pub fn advance(
        &self,
        ask: &mut Ask,
        step: AskStep,
        now: Timestamp,
    ) -> Result<AskEvent, AskError> {
        match step {
            AskStep::Started => ask.start(now),
            AskStep::Ranked(document) => ask.rank(Ranking::new(document)?, now),
            AskStep::Explaining => ask.start_explaining(now),
            AskStep::Explained(text) => ask.explain(&text, now),
            AskStep::Answered { complete } => ask.answer(complete, now),
            AskStep::Failed(reason) => ask.fail(FailureReason::new(&reason), now),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asks::value_objects::AskState;

    const T0: Timestamp = Timestamp(1);

    #[test]
    fn open_validates_the_question() {
        let service = AskWorkflowService::new();

        let (ask, event) = service.open(" a table ", T0).unwrap();
        let blank = service.open("  ", T0).unwrap_err();

        assert_eq!(ask.question().as_str(), "a table");
        assert_eq!(event, AskEvent::AskOpened { ask_id: ask.id() });
        assert!(matches!(blank, AskError::InvalidQuestion { .. }));
    }

    #[test]
    fn every_step_maps_to_its_transition_and_event() {
        let service = AskWorkflowService;
        let (mut ask, _) = service.open("a table", T0).unwrap();
        let ask_id = ask.id();

        let events: Vec<_> = [
            AskStep::Started,
            AskStep::Ranked("{}".into()),
            AskStep::Explaining,
            AskStep::Explained("Cedro".into()),
            AskStep::Answered { complete: true },
        ]
        .into_iter()
        .map(|step| service.advance(&mut ask, step, T0).unwrap())
        .collect();

        assert_eq!(
            events,
            [
                AskEvent::AskStarted { ask_id },
                AskEvent::AskRanked { ask_id },
                AskEvent::AskExplaining { ask_id },
                AskEvent::AskExplained { ask_id },
                AskEvent::AskAnswered {
                    ask_id,
                    complete: true
                },
            ]
        );
        assert_eq!(ask.explanation().as_str(), "Cedro");
    }

    #[test]
    fn a_failure_keeps_its_reason_and_a_bad_step_changes_nothing() {
        let service = AskWorkflowService::new();
        let (mut ask, _) = service.open("a table", T0).unwrap();

        let early = service
            .advance(&mut ask, AskStep::Answered { complete: true }, T0)
            .unwrap_err();
        service.advance(&mut ask, AskStep::Started, T0).unwrap();
        let blank = service
            .advance(&mut ask, AskStep::Ranked(" ".into()), T0)
            .unwrap_err();
        let failed = service
            .advance(&mut ask, AskStep::Failed(" model down ".into()), T0)
            .unwrap();

        assert!(matches!(early, AskError::InvalidTransition { .. }));
        assert!(matches!(blank, AskError::EmptyRanking));
        assert_eq!(failed, AskEvent::AskFailed { ask_id: ask.id() });
        assert_eq!(ask.state(), AskState::Failed);
        assert_eq!(ask.failure().unwrap().as_str(), "model down");
    }
}
