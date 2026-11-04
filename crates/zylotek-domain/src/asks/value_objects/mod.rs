pub mod state;
pub mod texts;

pub use state::AskState;
pub use texts::{
    Explanation, FailureReason, MAX_EXPLANATION_CHARS, MAX_FAILURE_CHARS, MAX_QUESTION_CHARS,
    Question, Ranking,
};

/// Milliseconds since the Unix epoch. The domain is told the time; it never reads a clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Timestamp(pub u64);
