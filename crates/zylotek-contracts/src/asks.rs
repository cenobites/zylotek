//! The wire format of `/api/v1/asks`: a question kept as a record while it is answered in the
//! background. The web app opens an ask, records each step as it drives the assistant, and
//! reads it back to show it; the record is what the browser's history links to.

use serde::{Deserialize, Serialize};

use crate::assistant::AskResponse;

/// Body of `POST /api/v1/asks`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateAskRequest {
    pub question: String,
}

/// Where an ask is. It only moves forward: queued → understanding → ranked → explaining →
/// answered, or to failed from any state before the end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AskStateSchema {
    Queued,
    Understanding,
    Ranked,
    Explaining,
    Answered,
    Failed,
}

impl AskStateSchema {
    pub fn is_final(self) -> bool {
        matches!(self, AskStateSchema::Answered | AskStateSchema::Failed)
    }
}

/// An ask as stored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AskRecordResponse {
    pub id: String,
    pub question: String,
    pub state: AskStateSchema,
    /// The ranking (its `explanation` is `None`), from `ranked` on.
    pub answer: Option<AskResponse>,
    /// As much of the explanation as has been written.
    pub explanation: String,
    /// False when the explanation was cut short; meaningful once answered.
    pub explanation_complete: bool,
    /// Why it failed, for a failed ask.
    pub failure: Option<String>,
    /// Milliseconds since the Unix epoch.
    pub created_at: u64,
    pub updated_at: u64,
}

/// Body of `POST /api/v1/asks/{id}/progress`: one step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum AskProgressRequest {
    Started,
    Ranked {
        answer: AskResponse,
    },
    Explaining,
    /// More of the explanation, appended to what is there.
    Explained {
        text: String,
    },
    Answered {
        complete: bool,
    },
    Failed {
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn steps_are_tagged_and_states_are_snake_case() {
        let answered = AskProgressRequest::Answered { complete: false };
        let wire = serde_json::to_value(&answered).unwrap();

        assert_eq!(wire, json!({"step": "answered", "complete": false}));
        assert_eq!(
            serde_json::from_value::<AskProgressRequest>(json!({"step": "started"})).unwrap(),
            AskProgressRequest::Started
        );
        assert_eq!(
            serde_json::to_value(AskStateSchema::Understanding).unwrap(),
            json!("understanding")
        );
        assert!(serde_json::from_value::<AskProgressRequest>(json!({"step": "skipped"})).is_err());
    }

    #[test]
    fn only_answered_and_failed_are_final() {
        use AskStateSchema::*;

        let finals: Vec<_> = [Queued, Understanding, Ranked, Explaining, Answered, Failed]
            .into_iter()
            .filter(|state| state.is_final())
            .collect();

        assert_eq!(finals, [Answered, Failed]);
    }

    #[test]
    fn a_record_round_trips() {
        let wire = json!({
            "id": "6f1b6c4e-0000-4000-8000-000000000001", "question": "a table",
            "state": "answered",
            "answer": {"summary": "A table", "criteria": [], "recommendations": [],
                       "explanation": null, "catalogue_size": 225},
            "explanation": "Cedro.", "explanation_complete": true, "failure": null,
            "created_at": 1, "updated_at": 2,
        });

        let record: AskRecordResponse = serde_json::from_value(wire.clone()).unwrap();

        assert_eq!(serde_json::to_value(&record).unwrap(), wire);
        assert_eq!(
            serde_json::to_value(CreateAskRequest {
                question: "q".into()
            })
            .unwrap(),
            json!({"question": "q"})
        );
    }
}
