//! The two services the web app depends on, over HTTP, using the shared contract types.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use reqwest::RequestBuilder;
use serde::de::DeserializeOwned;
use zylotek_contracts::{
    ErrorResponse,
    asks::{AskProgressRequest, AskRecordResponse, CreateAskRequest},
    assistant::{AskRequest, AskResponse, events},
    oauth::{CLIENT_CREDENTIALS, TokenResponse},
    woods::{WoodPageResponse, WoodResponse},
};

use crate::error::BackendError;

const API: &str = "the wood API";
const ASSISTANT: &str = "the assistant";
/// The assistant calls a model twice; give it room.
const ASSISTANT_TIMEOUT: Duration = Duration::from_secs(240);
const API_TIMEOUT: Duration = Duration::from_secs(10);
/// How long the whole catalogue is reused before it is fetched again.
const CATALOGUE_TTL: Duration = Duration::from_secs(60);
/// The API's largest page.
const CATALOGUE_PAGE: usize = 100;
/// A guard against an API that never stops saying there is more.
const CATALOGUE_MAX_PAGES: usize = 100;

fn client(timeout: Duration) -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .expect("the HTTP client builds with a timeout")
}

fn unreachable(service: &'static str, error: reqwest::Error) -> BackendError {
    BackendError::Unreachable {
        service,
        detail: error.without_url().to_string(),
    }
}

fn malformed(service: &'static str, detail: impl ToString) -> BackendError {
    BackendError::Malformed {
        service,
        detail: detail.to_string(),
    }
}

/// A non-2xx reply: the service's own `ErrorResponse` message when it sent one.
fn status_error(service: &'static str, status: u16, text: &str) -> BackendError {
    let message = serde_json::from_str::<ErrorResponse>(text)
        .map(|body| body.message)
        .unwrap_or_else(|_| text.chars().take(200).collect());

    BackendError::Status {
        service,
        status,
        message,
    }
}

/// A request body. The contract types are plain data, so writing them out cannot fail.
fn json(value: &impl serde::Serialize) -> String {
    serde_json::to_string(value).expect("contract types always serialize")
}

/// The status and body of the reply, or why none came.
async fn exchange(request: RequestBuilder) -> Result<(u16, String), reqwest::Error> {
    let response = request.send().await?;
    let status = response.status().as_u16();

    Ok((status, response.text().await?))
}

/// Sends the request and reads a JSON body of type `T`; non-2xx becomes `Status` carrying
/// the service's own `ErrorResponse` message when it sent one.
async fn call<T: DeserializeOwned>(
    service: &'static str,
    request: RequestBuilder,
) -> Result<T, BackendError> {
    let (status, text) = match exchange(request).await {
        Ok(reply) => reply,
        Err(error) => return Err(unreachable(service, error)),
    };
    if !(200..300).contains(&status) {
        return Err(status_error(service, status, &text));
    }

    serde_json::from_str(&text).map_err(|error| malformed(service, error))
}

/// The scope the web app asks for: opening asks and recording their steps, nothing else.
const ASKS_SCOPE: &str = "asks:write";
/// A token is replaced this long before it expires, so a request never carries a stale one.
const TOKEN_MARGIN: Duration = Duration::from_secs(60);

pub struct ApiClient {
    client: reqwest::Client,
    base_url: String,
    catalogue: tokio::sync::Mutex<Option<(Instant, Arc<Vec<WoodResponse>>)>>,
    credentials: Option<(String, String)>,
    token: tokio::sync::Mutex<Option<(String, Instant)>>,
}

impl ApiClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            client: client(API_TIMEOUT),
            base_url: base_url.trim_end_matches('/').to_owned(),
            catalogue: tokio::sync::Mutex::new(None),
            credentials: None,
            token: tokio::sync::Mutex::new(None),
        }
    }

    /// The OAuth client the web app writes asks with.
    pub fn with_credentials(mut self, client_id: String, client_secret: String) -> Self {
        self.credentials = Some((client_id, client_secret));
        self
    }

    /// A bearer token for the asks routes, from the client-credentials grant. Reused until a
    /// minute before it expires.
    async fn token(&self) -> Result<String, BackendError> {
        let Some((client_id, client_secret)) = &self.credentials else {
            return Err(BackendError::Status {
                service: API,
                status: 401,
                message: "the web app has no API client credentials".to_owned(),
            });
        };
        let mut cached = self.token.lock().await;
        if let Some((token, _)) = cached.as_ref().filter(|(_, until)| Instant::now() < *until) {
            return Ok(token.clone());
        }

        let body = form_urlencoded::Serializer::new(String::new())
            .append_pair("grant_type", CLIENT_CREDENTIALS)
            .append_pair("client_id", client_id)
            .append_pair("client_secret", client_secret)
            .append_pair("scope", ASKS_SCOPE)
            .finish();
        let request = self
            .client
            .post(format!("{}/oauth/token", self.base_url))
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .body(body);
        let granted: TokenResponse = call(API, request).await?;
        let lifetime = Duration::from_secs(granted.expires_in).saturating_sub(TOKEN_MARGIN);
        *cached = Some((granted.access_token.clone(), Instant::now() + lifetime));

        Ok(granted.access_token)
    }

    /// Opens an ask: stored, queued, and given its id.
    pub async fn create_ask(&self, question: &str) -> Result<AskRecordResponse, BackendError> {
        let token = self.token().await?;
        let request = self
            .client
            .post(format!("{}/api/v1/asks", self.base_url))
            .bearer_auth(token)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(json(&CreateAskRequest {
                question: question.to_owned(),
            }));

        call(API, request).await
    }

    /// An ask as stored. Reading needs no token.
    pub async fn ask(&self, id: &str) -> Result<AskRecordResponse, BackendError> {
        if !is_id(id) {
            return Err(BackendError::NotFound);
        }
        let request = self
            .client
            .get(format!("{}/api/v1/asks/{id}", self.base_url));

        match call(API, request).await {
            Err(BackendError::Status { status: 404, .. }) => Err(BackendError::NotFound),
            other => other,
        }
    }

    /// Records one step of answering an ask.
    pub async fn record(
        &self,
        id: &str,
        progress: &AskProgressRequest,
    ) -> Result<AskRecordResponse, BackendError> {
        let token = self.token().await?;
        let request = self
            .client
            .post(format!("{}/api/v1/asks/{id}/progress", self.base_url))
            .bearer_auth(token)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(json(progress));

        call(API, request).await
    }

    /// Every wood, for searching and filtering in memory. Fetched page by page, then reused for
    /// a minute so that browsing does not ask the API on every click. A failed fetch is not
    /// cached.
    pub async fn all(&self) -> Result<Arc<Vec<WoodResponse>>, BackendError> {
        let mut cached = self.catalogue.lock().await;
        if let Some((_, woods)) = cached
            .as_ref()
            .filter(|(at, _)| at.elapsed() < CATALOGUE_TTL)
        {
            return Ok(woods.clone());
        }

        let mut woods = Vec::new();
        for _ in 0..CATALOGUE_MAX_PAGES {
            let page = self.list(woods.len(), CATALOGUE_PAGE).await?;
            let empty = page.items.is_empty();
            woods.extend(page.items);
            if empty || woods.len() as u64 >= page.total {
                break;
            }
        }
        let woods = Arc::new(woods);
        *cached = Some((Instant::now(), woods.clone()));

        Ok(woods)
    }

    pub async fn list(
        &self,
        offset: usize,
        limit: usize,
    ) -> Result<WoodPageResponse, BackendError> {
        let url = format!(
            "{}/api/v1/woods?offset={offset}&limit={limit}",
            self.base_url
        );

        call(API, self.client.get(url)).await
    }

    pub async fn get(&self, id: &str) -> Result<WoodResponse, BackendError> {
        if !is_id(id) {
            return Err(BackendError::NotFound);
        }
        let url = format!("{}/api/v1/woods/{id}", self.base_url);

        match call(API, self.client.get(url)).await {
            Err(BackendError::Status { status: 404, .. }) => Err(BackendError::NotFound),
            other => other,
        }
    }
}

/// Wood ids are UUIDs; anything else never reaches a URL.
pub fn is_id(id: &str) -> bool {
    (1..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

pub struct AssistantClient {
    client: reqwest::Client,
    endpoint: String,
    token: Option<String>,
}

impl AssistantClient {
    pub fn new(base_url: &str, token: Option<String>) -> Self {
        Self {
            client: client(ASSISTANT_TIMEOUT),
            endpoint: format!("{}/v1/ask/stream", base_url.trim_end_matches('/')),
            token,
        }
    }

    /// Asks and waits only for the ranking; the explanation keeps arriving through the
    /// returned `LiveAnswer`.
    pub async fn ask_stream(&self, question: &str) -> Result<LiveAnswer, BackendError> {
        let body = serde_json::to_string(&AskRequest {
            question: question.to_owned(),
        })
        .expect("a question serializes");
        let mut request = self
            .client
            .post(&self.endpoint)
            .header("content-type", "application/json")
            .body(body);
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }

        let response = match request.send().await {
            Ok(response) => response,
            Err(error) => return Err(unreachable(ASSISTANT, error)),
        };
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            let text = response.text().await.unwrap_or_default();

            return Err(status_error(ASSISTANT, status, &text));
        }

        let mut reader = EventReader::new(response);
        let (name, data) = match reader.next().await {
            Ok(Some(event)) => event,
            Ok(None) => return Err(malformed(ASSISTANT, "the stream ended before the answer")),
            Err(error) => return Err(unreachable(ASSISTANT, error)),
        };
        if name != events::ANSWER {
            return Err(malformed(
                ASSISTANT,
                format!("expected the answer first, got `{name}`"),
            ));
        }
        let answer = serde_json::from_str(&data).map_err(|error| malformed(ASSISTANT, error))?;

        Ok(LiveAnswer {
            answer,
            reader,
            finished: false,
        })
    }
}

/// One step of the explanation as it is written.
#[derive(Debug, PartialEq, Eq)]
pub enum Piece {
    Text(String),
    /// The stream broke or the model gave up: what came before is all there will be.
    Cut,
}

pub struct LiveAnswer {
    /// The ranking, complete; its `explanation` is `None`.
    pub answer: AskResponse,
    reader: EventReader,
    finished: bool,
}

impl LiveAnswer {
    /// The next piece of the explanation; `None` when it is complete (or after a `Cut`).
    pub async fn next_piece(&mut self) -> Option<Piece> {
        while !self.finished {
            let event = match self.reader.next().await {
                Ok(Some(event)) => event,
                // The connection ended or broke without a closing `done`.
                Ok(None) | Err(_) => {
                    self.finished = true;
                    return Some(Piece::Cut);
                }
            };
            match (event.0.as_str(), serde_json::from_str::<String>(&event.1)) {
                (events::TOKEN, Ok(text)) => return Some(Piece::Text(text)),
                (events::DONE, _) => self.finished = true,
                (events::TOKEN | events::FAILED, _) => {
                    self.finished = true;
                    return Some(Piece::Cut);
                }
                _ => {} // an event from a newer assistant: not for us
            }
        }

        None
    }
}

/// Reads server-sent events off a response as they arrive, however the network cut them.
struct EventReader {
    response: reqwest::Response,
    buffer: Vec<u8>,
}

impl EventReader {
    fn new(response: reqwest::Response) -> Self {
        Self {
            response,
            buffer: Vec::new(),
        }
    }

    /// The next `(event, data)` pair; comments and keep-alives are skipped.
    async fn next(&mut self) -> Result<Option<(String, String)>, reqwest::Error> {
        loop {
            if let Some(end) = self.buffer.windows(2).position(|pair| pair == b"\n\n") {
                let block: Vec<u8> = self.buffer.drain(..end + 2).collect();
                if let Some(event) = parse_block(&String::from_utf8_lossy(&block)) {
                    return Ok(Some(event));
                }
                continue;
            }
            match self.response.chunk().await? {
                // CRLF line ends are folded to LF; JSON text never holds a raw CR.
                Some(bytes) => self
                    .buffer
                    .extend(bytes.iter().copied().filter(|byte| *byte != b'\r')),
                None => return Ok(None),
            }
        }
    }
}

fn parse_block(block: &str) -> Option<(String, String)> {
    let mut name = None;
    let mut data = Vec::new();
    for line in block.lines() {
        if let Some(value) = line.strip_prefix("event:") {
            name = Some(value.trim().to_owned());
        } else if let Some(value) = line.strip_prefix("data:") {
            data.push(value.trim_start().to_owned());
        }
    }

    name.filter(|name| !name.is_empty())
        .map(|name| (name, data.join("\n")))
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, header, method, path, query_param},
    };

    use super::*;

    fn wood(id: &str) -> serde_json::Value {
        json!({"id": id, "name": "Cedro", "scientific_name": "Cedrela fissilis", "genus": "Cedrela"})
    }

    #[test]
    fn only_uuid_shaped_ids_are_allowed_into_urls() {
        assert!(is_id("0b9c4f1e-aaaa-4bbb-8ccc-123456789abc"));
        assert!(is_id("7"));
        for bad in ["", "../etc", "a/b", "a b", "a?b", "é", &"x".repeat(65)] {
            assert!(!is_id(bad), "{bad:?}");
        }
    }

    #[tokio::test]
    async fn it_lists_and_gets_woods() {
        let server = MockServer::start().await;
        Mock::given(path("/api/v1/woods"))
            .and(query_param("offset", "20"))
            .and(query_param("limit", "10"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"items": [wood("a")], "total": 21, "offset": 20, "limit": 10}),
            ))
            .mount(&server)
            .await;
        Mock::given(path("/api/v1/woods/a"))
            .respond_with(ResponseTemplate::new(200).set_body_json(wood("a")))
            .mount(&server)
            .await;
        let api = ApiClient::new(&format!("{}/", server.uri()));

        let page = api.list(20, 10).await.unwrap();
        let one = api.get("a").await.unwrap();

        assert_eq!((page.total, page.items.len()), (21, 1));
        assert_eq!(one.name, "Cedro");
    }

    fn ask_json(id: &str, state: &str) -> serde_json::Value {
        json!({"id": id, "question": "a table", "state": state, "answer": null,
               "explanation": "", "explanation_complete": false, "failure": null,
               "created_at": 1, "updated_at": 1})
    }

    #[tokio::test]
    async fn asks_are_written_with_one_reused_client_credentials_token() {
        let server = MockServer::start().await;
        Mock::given(path("/oauth/token"))
            .and(wiremock::matchers::body_string_contains("grant_type=client_credentials"))
            .and(wiremock::matchers::body_string_contains("client_id=webapp"))
            .and(wiremock::matchers::body_string_contains("scope=asks%3Awrite"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "tok", "token_type": "Bearer", "expires_in": 3600, "scope": "asks:write",
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(path("/api/v1/asks"))
            .and(wiremock::matchers::header("authorization", "Bearer tok"))
            .and(wiremock::matchers::body_string_contains("a table"))
            .respond_with(ResponseTemplate::new(201).set_body_json(ask_json("a", "queued")))
            .mount(&server)
            .await;
        Mock::given(path("/api/v1/asks/a/progress"))
            .and(wiremock::matchers::header("authorization", "Bearer tok"))
            .and(wiremock::matchers::body_string_contains(
                r#""step":"started""#,
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(ask_json("a", "understanding")))
            .mount(&server)
            .await;
        Mock::given(path("/api/v1/asks/a"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ask_json("a", "understanding")))
            .mount(&server)
            .await;
        let api = ApiClient::new(&server.uri()).with_credentials("webapp".into(), "s3cret".into());

        let created = api.create_ask("a table").await.unwrap();
        let started = api.record("a", &AskProgressRequest::Started).await.unwrap();
        let read = api.ask("a").await.unwrap();

        assert_eq!(created.id, "a");
        assert_eq!(
            started.state,
            zylotek_contracts::asks::AskStateSchema::Understanding
        );
        assert_eq!(read, started);
    }

    #[tokio::test]
    async fn without_credentials_or_with_a_refused_token_nothing_is_written() {
        let server = MockServer::start().await;
        Mock::given(path("/oauth/token"))
            .respond_with(ResponseTemplate::new(401).set_body_json(
                json!({"error": "invalid_client", "error_description": "unknown client"}),
            ))
            .mount(&server)
            .await;
        Mock::given(path("/api/v1/asks/gone"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let anonymous = ApiClient::new(&server.uri());
        let refused = ApiClient::new(&server.uri()).with_credentials("x".into(), "y".into());

        let none = anonymous.create_ask("a table").await.unwrap_err();
        let bad = refused.create_ask("a table").await.unwrap_err();
        let gone = anonymous.ask("gone").await.unwrap_err();
        let malformed = anonymous.ask("../x").await.unwrap_err();

        assert!(
            matches!(none, BackendError::Status { status: 401, ref message, .. } if message.contains("credentials"))
        );
        assert!(matches!(bad, BackendError::Status { status: 401, .. }));
        assert!(matches!(gone, BackendError::NotFound));
        assert!(matches!(malformed, BackendError::NotFound));
        assert_eq!(
            server.received_requests().await.unwrap().len(),
            2,
            "one refused token request, one read; nothing for the bad id"
        );
    }

    #[tokio::test]
    async fn the_whole_catalogue_is_fetched_page_by_page_and_reused() {
        let server = MockServer::start().await;
        for (offset, ids) in [("0", vec!["a", "b"]), ("2", vec!["c"])] {
            Mock::given(path("/api/v1/woods"))
                .and(query_param("offset", offset))
                .and(query_param("limit", "100"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "items": ids.iter().map(|id| wood(id)).collect::<Vec<_>>(),
                    "total": 3, "offset": 0, "limit": 100,
                })))
                .mount(&server)
                .await;
        }
        let api = ApiClient::new(&server.uri());

        let first = api.all().await.unwrap();
        let again = api.all().await.unwrap();

        assert_eq!(
            first.iter().map(|w| w.id.as_str()).collect::<Vec<_>>(),
            ["a", "b", "c"]
        );
        assert!(
            Arc::ptr_eq(&first, &again),
            "the second look reuses the first"
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn the_catalogue_stops_at_an_empty_page_and_a_failure_is_not_remembered() {
        let server = MockServer::start().await;
        Mock::given(path("/api/v1/woods"))
            .respond_with(
                ResponseTemplate::new(500)
                    .set_body_json(json!({"code": "INTERNAL", "message": "down"})),
            )
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(path("/api/v1/woods"))
            .and(query_param("offset", "0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"items": [wood("a")], "total": 5, "offset": 0, "limit": 100}),
            ))
            .mount(&server)
            .await;
        Mock::given(path("/api/v1/woods"))
            .and(query_param("offset", "1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"items": [], "total": 5, "offset": 1, "limit": 100})),
            )
            .mount(&server)
            .await;
        let api = ApiClient::new(&server.uri());

        let failed = api.all().await;
        let woods = api.all().await.unwrap();

        assert!(matches!(
            failed,
            Err(BackendError::Status { status: 500, .. })
        ));
        assert_eq!(woods.len(), 1, "a short catalogue does not loop forever");
    }

    #[tokio::test]
    async fn a_missing_wood_and_a_bad_id_are_not_found_without_asking_twice() {
        let server = MockServer::start().await;
        Mock::given(path("/api/v1/woods/gone"))
            .respond_with(
                ResponseTemplate::new(404)
                    .set_body_json(json!({"code": "NOT_FOUND", "message": "wood gone not found"})),
            )
            .mount(&server)
            .await;
        let api = ApiClient::new(&server.uri());

        assert!(matches!(api.get("gone").await, Err(BackendError::NotFound)));
        assert!(matches!(api.get("../x").await, Err(BackendError::NotFound)));
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn failures_say_which_service_and_what_it_said() {
        let server = MockServer::start().await;
        Mock::given(path("/api/v1/woods"))
            .and(query_param("offset", "0"))
            .respond_with(
                ResponseTemplate::new(500)
                    .set_body_json(json!({"code": "INTERNAL", "message": "database is down"})),
            )
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(path("/api/v1/woods"))
            .and(query_param("offset", "1"))
            .respond_with(ResponseTemplate::new(502).set_body_string("<html>bad gateway</html>"))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(path("/api/v1/woods"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&server)
            .await;
        let api = ApiClient::new(&server.uri());

        let json = api.list(0, 5).await.unwrap_err();
        let html = api.list(1, 5).await.unwrap_err();
        let garbage = api.list(2, 5).await.unwrap_err();
        let down = ApiClient::new("http://127.0.0.1:1")
            .list(0, 5)
            .await
            .unwrap_err();

        assert_eq!(
            json.to_string(),
            "the wood API answered 500: database is down"
        );
        assert!(html.to_string().contains("<html>bad gateway"));
        assert!(matches!(garbage, BackendError::Malformed { .. }));
        assert!(garbage.to_string().contains("could not be read"));
        assert!(matches!(down, BackendError::Unreachable { .. }));
        assert!(
            !down.to_string().contains("127.0.0.1:1/"),
            "no URL leaks into the message"
        );
    }

    fn sse(events: &[(&str, serde_json::Value)]) -> String {
        events
            .iter()
            .map(|(name, data)| format!("event: {name}\ndata: {data}\n\n"))
            .collect()
    }

    fn answer_json() -> serde_json::Value {
        json!({"summary": "s", "criteria": [], "recommendations": [],
            "explanation": null, "catalogue_size": 3})
    }

    async fn pieces(live: &mut LiveAnswer) -> Vec<Piece> {
        let mut pieces = Vec::new();
        while let Some(piece) = live.next_piece().await {
            pieces.push(piece);
        }

        pieces
    }

    fn text(piece: &str) -> Piece {
        Piece::Text(piece.to_owned())
    }

    async fn serve_sse(body: String) -> (MockServer, AssistantClient) {
        let server = MockServer::start().await;
        Mock::given(path("/v1/ask/stream"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;
        let client = AssistantClient::new(&server.uri(), None);

        (server, client)
    }

    #[tokio::test]
    async fn asking_posts_the_question_with_the_token_and_reads_answer_then_words() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/ask/stream"))
            .and(header("authorization", "Bearer t0ken"))
            .and(body_json(json!({"question": "a table"})))
            .respond_with(ResponseTemplate::new(200).set_body_string(sse(&[
                ("answer", answer_json()),
                ("token", json!("Hello, ")),
                ("token", json!("wo\nrld")),
                ("done", json!({})),
            ])))
            .expect(1)
            .mount(&server)
            .await;
        let assistant = AssistantClient::new(&format!("{}/", server.uri()), Some("t0ken".into()));

        let mut live = assistant.ask_stream("a table").await.unwrap();

        assert_eq!(live.answer.catalogue_size, 3);
        assert_eq!(pieces(&mut live).await, [text("Hello, "), text("wo\nrld")]);
        assert_eq!(live.next_piece().await, None, "and it stays finished");
    }

    #[tokio::test]
    async fn a_failed_event_cuts_the_explanation_and_unknown_events_are_ignored() {
        let (_server, client) = serve_sse(
            sse(&[
                ("answer", answer_json()),
                ("progress", json!(0.5)),
                ("token", json!("Half ")),
                ("failed", json!("the explanation was cut short")),
                ("token", json!("never seen")),
                ("done", json!({})),
            ])
            .replace("event: answer", ": hello\n\nevent: answer"),
        )
        .await;

        let mut live = client.ask_stream("x").await.unwrap();

        assert_eq!(pieces(&mut live).await, [text("Half "), Piece::Cut]);
    }

    #[tokio::test]
    async fn a_stream_that_just_ends_or_sends_a_bad_token_is_a_cut() {
        let (_a, ended) =
            serve_sse(sse(&[("answer", answer_json()), ("token", json!("Hi"))])).await;
        let (_b, garbled) =
            serve_sse(sse(&[("answer", answer_json()), ("token", json!(42))])).await;
        let (_c, bare) = serve_sse(sse(&[("answer", answer_json())])).await;

        let mut ended = ended.ask_stream("x").await.unwrap();
        let mut garbled = garbled.ask_stream("x").await.unwrap();
        let mut bare = bare.ask_stream("x").await.unwrap();

        assert_eq!(pieces(&mut ended).await, [text("Hi"), Piece::Cut]);
        assert_eq!(pieces(&mut garbled).await, [Piece::Cut]);
        assert_eq!(pieces(&mut bare).await, [Piece::Cut]);
    }

    #[tokio::test]
    async fn crlf_line_ends_and_multi_line_data_are_understood() {
        let body = "event: answer\r\ndata: {\"summary\":\r\ndata: \"s\", \"criteria\": [], \"recommendations\": [], \"explanation\": null, \"catalogue_size\": 1}\r\n\r\nevent: done\r\ndata: {}\r\n\r\n";
        let (_server, client) = serve_sse(body.to_owned()).await;

        let mut live = client.ask_stream("x").await.unwrap();

        assert_eq!(live.answer.summary, "s");
        assert!(pieces(&mut live).await.is_empty());
    }

    #[tokio::test]
    async fn a_stream_that_does_not_start_with_the_answer_is_malformed() {
        let (_a, empty) = serve_sse(String::new()).await;
        let (_b, wrong) = serve_sse(sse(&[("token", json!("x"))])).await;
        let (_c, junk) = serve_sse(sse(&[("answer", json!("not an answer"))])).await;

        for client in [empty, wrong, junk] {
            let error = client.ask_stream("x").await.err().unwrap();

            assert!(matches!(error, BackendError::Malformed { .. }), "{error}");
        }
    }

    #[tokio::test]
    async fn the_assistants_own_error_message_is_passed_on() {
        let server = MockServer::start().await;
        Mock::given(path("/v1/ask/stream"))
            .respond_with(
                ResponseTemplate::new(503).set_body_json(
                    json!({"code": "CATALOGUE_NOT_READY", "message": "not synced yet"}),
                ),
            )
            .mount(&server)
            .await;

        let error = AssistantClient::new(&server.uri(), None)
            .ask_stream("x")
            .await
            .err()
            .unwrap();
        let down = AssistantClient::new("http://127.0.0.1:1", None)
            .ask_stream("x")
            .await
            .err()
            .unwrap();

        assert!(matches!(
            &error,
            BackendError::Status { status: 503, message, .. } if message == "not synced yet"
        ));
        assert!(matches!(down, BackendError::Unreachable { .. }));
    }

    /// Serves one chunked response, one part per chunk with a pause between, so events
    /// arrive cut in pieces. The connection is dropped afterwards, mid-body.
    async fn chunked_server(parts: Vec<Vec<u8>>) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 4096];
            let _ = socket.read(&mut request).await;
            let head = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n";
            socket.write_all(head.as_bytes()).await.unwrap();
            for part in parts {
                socket
                    .write_all(format!("{:x}\r\n", part.len()).as_bytes())
                    .await
                    .unwrap();
                socket.write_all(&part).await.unwrap();
                socket.write_all(b"\r\n").await.unwrap();
                socket.flush().await.unwrap();
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
        });

        url
    }

    #[tokio::test]
    async fn events_cut_by_the_network_are_put_back_together() {
        let whole = sse(&[
            ("answer", answer_json()),
            ("token", json!("café ")),
            ("token", json!("au lait")),
            ("done", json!({})),
        ]);
        let bytes = whole.as_bytes();
        let accent = whole.find('é').unwrap() + 1; // between the two bytes of é
        let parts = vec![
            bytes[..20].to_vec(),
            bytes[20..accent].to_vec(),
            bytes[accent..].to_vec(),
        ];
        let url = chunked_server(parts).await;

        let mut live = AssistantClient::new(&url, None)
            .ask_stream("x")
            .await
            .unwrap();

        assert_eq!(pieces(&mut live).await, [text("café "), text("au lait")]);
    }

    #[tokio::test]
    async fn a_connection_that_dies_midway_cuts_the_explanation() {
        let early = chunked_server(vec![b"event: ans".to_vec()]).await;
        let late = chunked_server(vec![
            sse(&[("answer", answer_json()), ("token", json!("Hi "))]).into_bytes(),
        ])
        .await;

        let before_the_answer = AssistantClient::new(&early, None)
            .ask_stream("x")
            .await
            .err()
            .unwrap();
        let mut live = AssistantClient::new(&late, None)
            .ask_stream("x")
            .await
            .unwrap();

        assert!(matches!(
            before_the_answer,
            BackendError::Unreachable { .. }
        ));
        assert_eq!(pieces(&mut live).await, [text("Hi "), Piece::Cut]);
    }
}
