//! A real wood API on a socket (SQLite, OAuth and all), a stand-in for the model server, and
//! the assistant itself, either in-process or as the compiled binary.

#![allow(dead_code)]

use std::{
    net::{SocketAddr, TcpListener as StdListener},
    process::{Child, Command, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use tokio::{net::TcpListener, task::JoinHandle};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_string_contains, method, path},
};
use zylotek_api::{auth::testing, build_state, router::router};
use zylotek_domain::testing::full_details;
use zylotek_infrastructure::{
    persistence::woods::repositories::PostgresWoodRepository, testing::memory_db,
};

pub const NEED: &str = r#"{"summary": "a hard table", "priorities": [{"property": "hardness", "weight": 3}], "avoid": []}"#;

pub struct Api {
    pub url: String,
    task: JoinHandle<std::io::Result<()>>,
}

impl Drop for Api {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// The real API with the given woods created through its write endpoint: `(name, janka)`.
pub async fn api_with(woods: &[(&str, f64)]) -> Api {
    let db = memory_db().await.unwrap();
    let app = router(build_state(
        Arc::new(PostgresWoodRepository::new(db)),
        testing::auth(),
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, app).await });

    let client = reqwest::Client::new();
    for (name, janka) in woods {
        let details: zylotek_contracts::woods::WoodDetailsSchema = {
            use zylotek_api::woods::mappers::Convert;
            (&full_details()).convert()
        };
        let mut body = serde_json::to_value(details).unwrap();
        body["name"] = json!(name);
        body["scientific_name"] = json!(format!("{name} scientificus"));
        body["mechanical_properties"]["dry"]["janka_hardness"]["parallel_to_grain"] = json!(janka);
        let response = client
            .post(format!("{url}/api/v1/woods"))
            .bearer_auth(testing::write_token())
            .header("content-type", "application/json")
            .body(body.to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 201, "seeding {name}");
    }

    Api { url, task }
}

/// A model server that reads the need as `NEED` and then explains it in one sentence.
pub async fn model() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_string_contains("response_format"))
        .respond_with(completion(NEED))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_string_contains(r#""stream":true"#))
        .respond_with(streamed(&["The hardest ", "wood suits ", "a table."]))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(completion("The hardest wood suits a table."))
        .mount(&server)
        .await;

    server
}

/// A streamed chat completion, one server-sent event per piece.
pub fn streamed(pieces: &[&str]) -> ResponseTemplate {
    let mut body = String::new();
    for piece in pieces {
        body += &format!(
            "data: {}\n\n",
            json!({"choices": [{"delta": {"content": piece}}]})
        );
    }
    body += "data: [DONE]\n\n";

    ResponseTemplate::new(200)
        .insert_header("content-type", "text/event-stream")
        .set_body_string(body)
}

pub fn completion(content: &str) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .set_body_json(json!({"choices": [{"message": {"content": content}}]}))
}

/// The compiled binary, killed when dropped.
pub struct Running {
    pub url: String,
    child: Child,
}

impl Drop for Running {
    /// Asks the process to stop the way a service manager would, so it exits cleanly and its
    /// coverage data is written; kills it if it has not gone in five seconds.
    fn drop(&mut self) {
        let _ = Command::new("kill")
            .args(["-TERM", &self.child.id().to_string()])
            .status();
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn free_port() -> SocketAddr {
    StdListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
}

pub fn spawn(api_url: &str, model_url: &str, extra: &[&str]) -> Running {
    let listen = free_port();
    let child = Command::new(assert_cmd::cargo::cargo_bin("zylotek-assistant"))
        .args(["--api-url", api_url, "--model-url", model_url])
        .args(["--listen", &listen.to_string()])
        .args(extra)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    Running {
        url: format!("http://{listen}"),
        child,
    }
}

/// Polls `/health` until the assistant answers; returns its body.
pub async fn health(assistant: &Running) -> Value {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Ok(response) = reqwest::get(format!("{}/health", assistant.url)).await {
            return serde_json::from_str(&response.text().await.unwrap()).unwrap();
        }
        assert!(Instant::now() < deadline, "the assistant never came up");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Waits until the catalogue has `woods` entries.
pub async fn until_synced(assistant: &Running, woods: u64) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while health(assistant).await["woods"] != woods {
        assert!(Instant::now() < deadline, "the catalogue never synced");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

pub async fn ask(assistant: &Running, token: Option<&str>, question: &str) -> (u16, Value) {
    let mut request = reqwest::Client::new()
        .post(format!("{}/v1/ask", assistant.url))
        .header("content-type", "application/json")
        .body(json!({"question": question}).to_string());
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    let response = request.send().await.unwrap();

    let status = response.status().as_u16();
    let text = response.text().await.unwrap();

    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}

/// `(event, data)` pairs from `POST /v1/ask/stream`, plus the HTTP status.
pub async fn ask_stream(
    assistant: &Running,
    token: Option<&str>,
    question: &str,
) -> (u16, Vec<(String, String)>) {
    let mut request = reqwest::Client::new()
        .post(format!("{}/v1/ask/stream", assistant.url))
        .header("content-type", "application/json")
        .body(json!({"question": question}).to_string());
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    let response = request.send().await.unwrap();
    let status = response.status().as_u16();
    let text = response.text().await.unwrap();
    let events = text
        .split("\n\n")
        .filter(|block| block.contains("event:"))
        .map(|block| {
            let field = |name: &str| {
                block
                    .lines()
                    .find_map(|line| line.strip_prefix(name))
                    .unwrap_or_default()
                    .trim()
                    .to_owned()
            };

            (field("event:"), field("data:"))
        })
        .collect();

    (status, events)
}
