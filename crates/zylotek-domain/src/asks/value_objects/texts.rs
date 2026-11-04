use crate::asks::errors::AskError;

pub const MAX_QUESTION_CHARS: usize = 1000;
/// An explanation is a short text; anything past this is dropped rather than stored.
pub const MAX_EXPLANATION_CHARS: usize = 20_000;
pub const MAX_FAILURE_CHARS: usize = 500;

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

/// The explanation written so far, at most `MAX_EXPLANATION_CHARS` characters.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Explanation(String);

impl Explanation {
    /// Text past the limit is dropped.
    pub fn new(text: &str) -> Self {
        Self::default().followed_by(text)
    }

    /// This explanation with `text` added at the end, still within the limit.
    pub fn followed_by(mut self, text: &str) -> Self {
        let room = MAX_EXPLANATION_CHARS.saturating_sub(self.0.chars().count());
        self.0.extend(text.chars().take(room));
        self
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Why an ask was given up: trimmed, at most `MAX_FAILURE_CHARS` characters, never blank.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureReason(String);

impl FailureReason {
    pub fn new(reason: &str) -> Self {
        let reason: String = reason.trim().chars().take(MAX_FAILURE_CHARS).collect();
        if reason.is_empty() {
            return Self("unknown failure".to_owned());
        }

        Self(reason)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn questions_must_be_present_and_short() {
        assert_eq!(Question::new("  a table  ").unwrap().as_str(), "a table");
        assert!(Question::new(&"x".repeat(MAX_QUESTION_CHARS)).is_ok());
        for bad in ["", "   ", &"x".repeat(MAX_QUESTION_CHARS + 1)] {
            let error = Question::new(bad).unwrap_err();
            assert_eq!(
                error.to_string(),
                "a question must have between 1 and 1000 characters"
            );
        }
    }

    #[test]
    fn a_ranking_must_not_be_blank() {
        assert!(matches!(
            Ranking::new(" ".into()),
            Err(AskError::EmptyRanking)
        ));
        assert_eq!(Ranking::new("{}".into()).unwrap().as_str(), "{}");
    }

    #[test]
    fn the_explanation_stops_growing_at_its_limit() {
        let explanation =
            Explanation::new(&"é".repeat(MAX_EXPLANATION_CHARS - 1)).followed_by("abc");

        assert_eq!(explanation.as_str().chars().count(), MAX_EXPLANATION_CHARS);
        assert!(explanation.as_str().ends_with("éa"));
        assert_eq!(Explanation::default().as_str(), "");
        assert_eq!(
            Explanation::new(&"x".repeat(MAX_EXPLANATION_CHARS + 5))
                .as_str()
                .len(),
            MAX_EXPLANATION_CHARS
        );
    }

    #[test]
    fn a_failure_reason_is_short_and_never_blank() {
        assert_eq!(
            FailureReason::new(&"r".repeat(MAX_FAILURE_CHARS + 50))
                .as_str()
                .len(),
            MAX_FAILURE_CHARS
        );
        assert_eq!(FailureReason::new("  down ").as_str(), "down");
        assert_eq!(FailureReason::new("  ").as_str(), "unknown failure");
    }
}
