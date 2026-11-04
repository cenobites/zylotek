use toasty::Model;
use uuid::Uuid;

/// The `asks` table: one row per question, rewritten at each step of answering it.
#[derive(Debug, Model)]
pub struct Ask {
    #[key]
    pub id: Uuid,

    pub question: String,

    /// `AskState::as_str`.
    pub state: String,

    /// The assistant's ranking document (JSON), once there is one.
    #[column(type = text)]
    pub ranking: Option<String>,

    #[column(type = text)]
    pub explanation: String,

    pub explanation_complete: bool,

    pub failure: Option<String>,

    /// Milliseconds since the Unix epoch.
    pub created_at: u64,

    pub updated_at: u64,
}
