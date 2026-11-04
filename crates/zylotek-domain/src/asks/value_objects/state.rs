use std::{fmt, str::FromStr};

use crate::asks::errors::AskError;

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

    pub fn can_become(self, next: AskState) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn only_forward_moves_are_allowed() {
        use AskState::*;
        assert!(Queued.can_become(Understanding));
        assert!(Ranked.can_become(Answered));
        assert!(Explaining.can_become(Explaining));
        assert!(Explaining.can_become(Failed));
        assert!(!Queued.can_become(Ranked));
        assert!(!Understanding.can_become(Understanding));
        assert!(!Answered.can_become(Failed));
        assert!(!Failed.can_become(Failed));
    }
}
