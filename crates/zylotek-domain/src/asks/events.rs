use crate::asks::entities::AskId;

/// What happened to an ask; one per state change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AskEvent {
    AskOpened {
        ask_id: AskId,
    },
    AskStarted {
        ask_id: AskId,
    },
    AskRanked {
        ask_id: AskId,
    },
    AskExplaining {
        ask_id: AskId,
    },
    /// More of the explanation was written.
    AskExplained {
        ask_id: AskId,
    },
    AskAnswered {
        ask_id: AskId,
        complete: bool,
    },
    AskFailed {
        ask_id: AskId,
    },
}
