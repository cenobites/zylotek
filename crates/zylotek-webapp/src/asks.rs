//! Asking in the background. Opening an ask stores it through the API and starts a job that
//! drives the assistant, recording each step (understanding, ranked, explaining, answered or
//! failed) so that the answer outlives the visitor's page. While the job runs, a page in this
//! process can follow it live through a snapshot; afterwards, or from elsewhere, the stored
//! record is all there is.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use tokio::sync::watch;
use zylotek_contracts::{
    asks::{AskProgressRequest, AskRecordResponse, AskStateSchema},
    assistant::AskResponse,
};

use crate::{
    clients::{ApiClient, AssistantClient, LiveAnswer, Piece},
    error::BackendError,
    i18n::Lang,
};

/// How often the explanation written so far is saved while it is being written.
const FLUSH_EVERY: Duration = Duration::from_millis(500);
/// Saved sooner when this much has piled up.
const FLUSH_CHARS: usize = 400;

/// Where asks are kept. The API in production; a map in tests.
#[async_trait::async_trait]
pub trait AskStore: Send + Sync {
    async fn create(&self, question: &str) -> Result<AskRecordResponse, BackendError>;

    async fn get(&self, id: &str) -> Result<AskRecordResponse, BackendError>;

    async fn record(
        &self,
        id: &str,
        progress: AskProgressRequest,
    ) -> Result<AskRecordResponse, BackendError>;
}

#[async_trait::async_trait]
impl AskStore for ApiClient {
    async fn create(&self, question: &str) -> Result<AskRecordResponse, BackendError> {
        self.create_ask(question).await
    }

    async fn get(&self, id: &str) -> Result<AskRecordResponse, BackendError> {
        self.ask(id).await
    }

    async fn record(
        &self,
        id: &str,
        progress: AskProgressRequest,
    ) -> Result<AskRecordResponse, BackendError> {
        ApiClient::record(self, id, &progress).await
    }
}

/// An ask as its job knows it, a step ahead of what is stored.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub question: String,
    pub state: AskStateSchema,
    pub answer: Option<AskResponse>,
    pub explanation: String,
    pub complete: bool,
    pub failure: Option<String>,
}

type Running = Arc<Mutex<HashMap<String, watch::Receiver<Snapshot>>>>;

pub struct Asks {
    store: Arc<dyn AskStore>,
    assistant: Arc<AssistantClient>,
    /// The asks this process is working on now; each entry goes when its job ends.
    running: Running,
}

impl Asks {
    pub fn new(store: Arc<dyn AskStore>, assistant: Arc<AssistantClient>) -> Self {
        Self {
            store,
            assistant,
            running: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Stores a new ask and starts answering it in the background. Returns its id at once.
    /// `lang` is the language a failure is told in.
    pub async fn open(&self, question: &str, lang: Lang) -> Result<String, BackendError> {
        let record = self.store.create(question).await?;
        let (sender, receiver) = watch::channel(Snapshot {
            question: record.question.clone(),
            state: record.state,
            answer: None,
            explanation: String::new(),
            complete: false,
            failure: None,
        });
        self.running
            .lock()
            .expect("the map is never left poisoned")
            .insert(record.id.clone(), receiver);

        let job = Job {
            store: self.store.clone(),
            assistant: self.assistant.clone(),
            running: self.running.clone(),
            id: record.id.clone(),
            question: record.question,
            lang,
            snapshot: sender,
        };
        tokio::spawn(job.run());

        Ok(record.id)
    }

    /// The live snapshot of an ask this process is answering right now.
    pub fn follow(&self, id: &str) -> Option<watch::Receiver<Snapshot>> {
        self.running
            .lock()
            .expect("the map is never left poisoned")
            .get(id)
            .cloned()
    }

    /// The ask as stored.
    pub async fn get(&self, id: &str) -> Result<AskRecordResponse, BackendError> {
        self.store.get(id).await
    }
}

struct Job {
    store: Arc<dyn AskStore>,
    assistant: Arc<AssistantClient>,
    running: Running,
    id: String,
    question: String,
    lang: Lang,
    snapshot: watch::Sender<Snapshot>,
}

impl Job {
    /// Saves a step. A step that cannot be saved is logged and the job goes on: the visitor
    /// following live still gets the answer, and a record left behind shows as interrupted.
    async fn step(&self, progress: AskProgressRequest) {
        if let Err(error) = self.store.record(&self.id, progress).await {
            tracing::warn!(%error, ask = %self.id, "could not record a step of an ask");
        }
    }

    fn show(&self, change: impl FnOnce(&mut Snapshot)) {
        self.snapshot.send_modify(change);
    }

    async fn run(self) {
        self.step(AskProgressRequest::Started).await;
        self.show(|s| s.state = AskStateSchema::Understanding);

        match self.assistant.ask_stream(&self.question).await {
            Err(error) => {
                tracing::warn!(%error, ask = %self.id, "the assistant could not answer");
                let reason = crate::pages::friendly(&error, self.lang.strings());
                self.step(AskProgressRequest::Failed {
                    reason: reason.clone(),
                })
                .await;
                self.show(|s| {
                    s.state = AskStateSchema::Failed;
                    s.failure = Some(reason);
                });
            }
            Ok(mut live) => {
                let answer = live.answer.clone();
                let nothing_to_explain = answer.recommendations.is_empty();
                self.step(AskProgressRequest::Ranked {
                    answer: answer.clone(),
                })
                .await;
                self.show(|s| {
                    s.state = AskStateSchema::Ranked;
                    s.answer = Some(answer);
                });

                let complete = if nothing_to_explain {
                    true
                } else {
                    self.step(AskProgressRequest::Explaining).await;
                    self.show(|s| s.state = AskStateSchema::Explaining);
                    self.explain(&mut live).await
                };
                self.step(AskProgressRequest::Answered { complete }).await;
                self.show(|s| {
                    s.state = AskStateSchema::Answered;
                    s.complete = complete;
                });
            }
        }

        self.running
            .lock()
            .expect("the map is never left poisoned")
            .remove(&self.id);
    }

    /// Passes the explanation on as it is written, saving it in batches. False when it was
    /// cut short.
    async fn explain(&self, live: &mut LiveAnswer) -> bool {
        let mut pending = String::new();
        let mut saved_at = Instant::now();
        let complete = loop {
            match live.next_piece().await {
                Some(Piece::Text(text)) => {
                    pending.push_str(&text);
                    self.show(|s| s.explanation.push_str(&text));
                    if saved_at.elapsed() >= FLUSH_EVERY || pending.len() >= FLUSH_CHARS {
                        self.step(AskProgressRequest::Explained {
                            text: std::mem::take(&mut pending),
                        })
                        .await;
                        saved_at = Instant::now();
                    }
                }
                Some(Piece::Cut) => break false,
                None => break true,
            }
        };
        if !pending.is_empty() {
            self.step(AskProgressRequest::Explained { text: pending })
                .await;
        }

        complete
    }
}

#[cfg(test)]
pub mod testing {
    use std::sync::atomic::{AtomicBool, Ordering};

    use zylotek_contracts::asks::{AskProgressRequest, AskRecordResponse, AskStateSchema};

    use super::*;

    /// Asks in a map, applying steps the way the API does (without refusing any), and
    /// remembering every step it was sent.
    #[derive(Default)]
    pub struct MemoryStore {
        pub asks: Mutex<HashMap<String, AskRecordResponse>>,
        pub steps: Mutex<Vec<AskProgressRequest>>,
        /// Refuse to open asks, as an API that is down would.
        pub down: AtomicBool,
    }

    impl MemoryStore {
        pub fn put(&self, record: AskRecordResponse) {
            self.asks.lock().unwrap().insert(record.id.clone(), record);
        }
    }

    /// A stored ask with nothing but a question and a state.
    pub fn record(id: &str, state: AskStateSchema) -> AskRecordResponse {
        AskRecordResponse {
            id: id.to_owned(),
            question: "a table".to_owned(),
            state,
            answer: None,
            explanation: String::new(),
            explanation_complete: false,
            failure: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[async_trait::async_trait]
    impl AskStore for MemoryStore {
        async fn create(&self, question: &str) -> Result<AskRecordResponse, BackendError> {
            if self.down.load(Ordering::SeqCst) {
                return Err(BackendError::Status {
                    service: "the wood API",
                    status: 503,
                    message: "down".into(),
                });
            }
            let id = format!("ask-{}", self.asks.lock().unwrap().len() + 1);
            let mut created = record(&id, AskStateSchema::Queued);
            created.question = question.to_owned();
            self.put(created.clone());

            Ok(created)
        }

        async fn get(&self, id: &str) -> Result<AskRecordResponse, BackendError> {
            self.asks
                .lock()
                .unwrap()
                .get(id)
                .cloned()
                .ok_or(BackendError::NotFound)
        }

        async fn record(
            &self,
            id: &str,
            progress: AskProgressRequest,
        ) -> Result<AskRecordResponse, BackendError> {
            self.steps.lock().unwrap().push(progress.clone());
            let mut asks = self.asks.lock().unwrap();
            let ask = asks.get_mut(id).ok_or(BackendError::NotFound)?;
            match progress {
                AskProgressRequest::Started => ask.state = AskStateSchema::Understanding,
                AskProgressRequest::Ranked { answer } => {
                    ask.state = AskStateSchema::Ranked;
                    ask.answer = Some(answer);
                }
                AskProgressRequest::Explaining => ask.state = AskStateSchema::Explaining,
                AskProgressRequest::Explained { text } => ask.explanation.push_str(&text),
                AskProgressRequest::Answered { complete } => {
                    ask.state = AskStateSchema::Answered;
                    ask.explanation_complete = complete;
                }
                AskProgressRequest::Failed { reason } => {
                    ask.state = AskStateSchema::Failed;
                    ask.failure = Some(reason);
                }
            }

            Ok(ask.clone())
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};

    use super::{testing::*, *};

    fn answer(recommendations: Value) -> Value {
        json!({"summary": "A table", "criteria": [{"property": "hardness", "weight": 3}],
               "recommendations": recommendations, "explanation": null, "catalogue_size": 225})
    }

    fn woods() -> Value {
        json!([{"wood_id": "a", "name": "Cedro", "scientific_name": "X y", "score": 0.9, "reasons": []}])
    }

    fn sse(items: &[(&str, Value)]) -> ResponseTemplate {
        let body: String = items
            .iter()
            .map(|(name, data)| format!("event: {name}\ndata: {data}\n\n"))
            .collect();

        ResponseTemplate::new(200)
            .insert_header("content-type", "text/event-stream")
            .set_body_string(body)
    }

    async fn assistant(reply: ResponseTemplate) -> (MockServer, Arc<AssistantClient>) {
        let server = MockServer::start().await;
        Mock::given(path("/v1/ask/stream"))
            .respond_with(reply)
            .mount(&server)
            .await;
        let client = Arc::new(AssistantClient::new(&server.uri(), None));

        (server, client)
    }

    /// Waits for the job to finish: its entry leaves the running map.
    async fn finished(asks: &Asks, id: &str) {
        for _ in 0..500 {
            if asks.follow(id).is_none() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(asks.follow(id).is_none(), "the job did not finish");
    }

    #[tokio::test]
    async fn an_ask_is_answered_in_the_background_and_every_step_is_saved() {
        let (_server, assistant) = assistant(sse(&[
            ("answer", answer(woods())),
            ("token", json!("Cedro is ")),
            ("token", json!("steady.")),
            ("done", json!({})),
        ]))
        .await;
        let store = Arc::new(MemoryStore::default());
        let asks = Asks::new(store.clone(), assistant);

        let id = asks.open("a table", Lang::En).await.unwrap();
        let live = asks.follow(&id).expect("followed while it runs");
        finished(&asks, &id).await;

        let saved = asks.get(&id).await.unwrap();
        assert_eq!(saved.state, AskStateSchema::Answered);
        assert!(saved.explanation_complete);
        assert_eq!(saved.explanation, "Cedro is steady.");
        assert_eq!(saved.answer.unwrap().recommendations[0].name, "Cedro");
        let steps = store.steps.lock().unwrap().clone();
        assert_eq!(steps[0], AskProgressRequest::Started);
        assert!(matches!(steps[1], AskProgressRequest::Ranked { .. }));
        assert_eq!(steps[2], AskProgressRequest::Explaining);
        assert_eq!(
            steps.last(),
            Some(&AskProgressRequest::Answered { complete: true })
        );
        let last = live.borrow().clone();
        assert_eq!(
            (last.state, last.explanation.as_str()),
            (AskStateSchema::Answered, "Cedro is steady.")
        );
    }

    #[tokio::test]
    async fn no_woods_means_answered_without_explaining() {
        let (_server, assistant) =
            assistant(sse(&[("answer", answer(json!([]))), ("done", json!({}))])).await;
        let store = Arc::new(MemoryStore::default());
        let asks = Asks::new(store.clone(), assistant);

        let id = asks.open("a box", Lang::En).await.unwrap();
        finished(&asks, &id).await;

        let steps = store.steps.lock().unwrap().clone();
        assert!(!steps.contains(&AskProgressRequest::Explaining));
        assert_eq!(asks.get(&id).await.unwrap().state, AskStateSchema::Answered);
    }

    #[tokio::test]
    async fn a_cut_explanation_is_saved_as_an_incomplete_answer() {
        let (_server, assistant) = assistant(sse(&[
            ("answer", answer(woods())),
            ("token", json!("Half")),
            ("failed", json!("cut")),
        ]))
        .await;
        let asks = Asks::new(Arc::new(MemoryStore::default()), assistant);

        let id = asks.open("a table", Lang::En).await.unwrap();
        finished(&asks, &id).await;
        let saved = asks.get(&id).await.unwrap();

        assert_eq!(
            (saved.state, saved.explanation_complete),
            (AskStateSchema::Answered, false)
        );
        assert_eq!(saved.explanation, "Half");
    }

    #[tokio::test]
    async fn an_assistant_that_cannot_be_reached_leaves_a_failed_ask_in_the_visitors_language() {
        let assistant = Arc::new(AssistantClient::new("http://127.0.0.1:1", None));
        let asks = Asks::new(Arc::new(MemoryStore::default()), assistant);

        let id = asks.open("uma mesa", Lang::PtBr).await.unwrap();
        finished(&asks, &id).await;
        let saved = asks.get(&id).await.unwrap();

        assert_eq!(saved.state, AskStateSchema::Failed);
        assert_eq!(
            saved.failure.as_deref(),
            Some(Lang::PtBr.strings().unreachable("the assistant").as_str())
        );
    }

    #[tokio::test]
    async fn a_long_explanation_is_saved_in_batches_and_a_lost_ask_only_logs() {
        let tokens: Vec<(&str, Value)> = std::iter::once(("answer", answer(woods())))
            .chain((0..10).map(|_| ("token", json!("x".repeat(100)))))
            .chain(std::iter::once(("done", json!({}))))
            .collect();
        let (_server, assistant) = assistant(sse(&tokens)).await;
        let store = Arc::new(MemoryStore::default());
        let asks = Asks::new(store.clone(), assistant.clone());

        let id = asks.open("a table", Lang::En).await.unwrap();
        finished(&asks, &id).await;

        let batches = store
            .steps
            .lock()
            .unwrap()
            .iter()
            .filter(|step| matches!(step, AskProgressRequest::Explained { .. }))
            .count();
        assert!(
            (2..10).contains(&batches),
            "{batches} batches for 1000 characters"
        );
        assert_eq!(asks.get(&id).await.unwrap().explanation.len(), 1000);

        // A job whose ask the store has lost logs each step it cannot save and still ends.
        let lost = Arc::new(MemoryStore::default());
        let (sender, receiver) = watch::channel(Snapshot {
            question: "q".into(),
            state: AskStateSchema::Queued,
            answer: None,
            explanation: String::new(),
            complete: false,
            failure: None,
        });
        Job {
            store: lost.clone(),
            assistant,
            running: Arc::new(Mutex::new(HashMap::new())),
            id: "gone".into(),
            question: "q".into(),
            lang: Lang::En,
            snapshot: sender,
        }
        .run()
        .await;
        assert_eq!(receiver.borrow().state, AskStateSchema::Answered);
        assert!(!lost.steps.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_explanation_saved_exactly_at_a_batch_leaves_nothing_over() {
        let tokens: Vec<(&str, Value)> = std::iter::once(("answer", answer(woods())))
            .chain((0..4).map(|_| ("token", json!("x".repeat(100)))))
            .chain(std::iter::once(("done", json!({}))))
            .collect();
        let (_server, assistant) = assistant(sse(&tokens)).await;
        let store = Arc::new(MemoryStore::default());
        let asks = Asks::new(store.clone(), assistant);

        let id = asks.open("a table", Lang::En).await.unwrap();
        finished(&asks, &id).await;

        let batches = store
            .steps
            .lock()
            .unwrap()
            .iter()
            .filter(|step| matches!(step, AskProgressRequest::Explained { .. }))
            .count();
        assert_eq!(
            batches, 1,
            "400 characters are one batch, with nothing after it"
        );
    }

    #[tokio::test]
    async fn the_api_client_is_an_ask_store() {
        let server = MockServer::start().await;
        let record = json!({"id": "a", "question": "q", "state": "queued", "answer": null,
            "explanation": "", "explanation_complete": false, "failure": null,
            "created_at": 1, "updated_at": 1});
        Mock::given(path("/oauth/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "t", "token_type": "Bearer", "expires_in": 3600, "scope": "asks:write"})))
            .mount(&server)
            .await;
        for route in ["/api/v1/asks", "/api/v1/asks/a", "/api/v1/asks/a/progress"] {
            Mock::given(path(route))
                .respond_with(ResponseTemplate::new(200).set_body_json(record.clone()))
                .mount(&server)
                .await;
        }
        let store: Arc<dyn AskStore> =
            Arc::new(ApiClient::new(&server.uri()).with_credentials("w".into(), "s".into()));

        assert_eq!(store.create("q").await.unwrap().id, "a");
        assert_eq!(store.get("a").await.unwrap().id, "a");
        assert_eq!(
            store
                .record("a", AskProgressRequest::Started)
                .await
                .unwrap()
                .id,
            "a"
        );
    }

    #[tokio::test]
    async fn an_api_that_cannot_open_the_ask_is_an_error_and_nothing_runs() {
        let store = Arc::new(MemoryStore::default());
        store.down.store(true, std::sync::atomic::Ordering::SeqCst);
        let asks = Asks::new(
            store,
            Arc::new(AssistantClient::new("http://127.0.0.1:1", None)),
        );

        let error = asks.open("a table", Lang::En).await.unwrap_err();

        assert!(matches!(error, BackendError::Status { status: 503, .. }));
        assert!(asks.running.lock().unwrap().is_empty());
    }
}
