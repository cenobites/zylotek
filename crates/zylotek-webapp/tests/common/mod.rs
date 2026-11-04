//! The real chain behind the website: the wood API (SQLite, OAuth), the assistant library
//! with a stand-in model server, and the compiled website binary.

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
use zylotek_api::{auth::testing, build_state, router::router, woods::mappers::Convert};
use zylotek_assistant::server::{Config as AssistantConfig, run as run_assistant};
use zylotek_domain::testing::full_details;
use zylotek_infrastructure::{
    persistence::woods::repositories::PostgresWoodRepository, testing::memory_db,
};

const NEED: &str = r#"{"summary": "a hard table", "priorities": [{"property": "hardness", "weight": 3}], "avoid": []}"#;

pub struct Task(JoinHandle<()>);

impl Drop for Task {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub struct Api {
    pub url: String,
    _task: Task,
}

/// The real API with woods created through its write endpoint: `(name, janka)`.
pub async fn api_with(woods: &[(&str, f64)]) -> Api {
    let db = memory_db().await.unwrap();
    let app = router(build_state(
        Arc::new(PostgresWoodRepository::new(db)),
        testing::auth(),
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let client = reqwest::Client::new();
    for (name, janka) in woods {
        let details: zylotek_contracts::woods::WoodDetailsSchema = (&full_details()).convert();
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

    Api {
        url,
        _task: Task(task),
    }
}

pub async fn model() -> MockServer {
    let server = MockServer::start().await;
    let completion = |content: &str| {
        ResponseTemplate::new(200)
            .set_body_json(json!({"choices": [{"message": {"content": content}}]}))
    };
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_string_contains("response_format"))
        .respond_with(completion(NEED))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_string_contains(r#""stream":true"#))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(format!(
                    "{}{}data: [DONE]\n\n",
                    delta("The hardest "),
                    delta("wood suits a table.")
                )),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(completion("The hardest wood suits a table."))
        .mount(&server)
        .await;

    server
}

fn free_port() -> SocketAddr {
    StdListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
}

/// The assistant, in this process, listening on a free port.
pub struct Assistant {
    pub url: String,
    _task: Task,
}

pub async fn assistant(api_url: &str, model_url: &str) -> Assistant {
    let listen = free_port();
    let task = tokio::spawn({
        let config = AssistantConfig {
            api_url: api_url.to_owned(),
            model_url: model_url.to_owned(),
            model_name: "local".into(),
            model_api_key: None,
            token: None,
            listen,
            refresh: Duration::from_secs(60),
            top: 5,
        };
        async move {
            let _ = run_assistant(config).await;
        }
    });
    let url = format!("http://{listen}");
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Ok(response) = reqwest::get(format!("{url}/health")).await {
            let body: Value = serde_json::from_str(&response.text().await.unwrap()).unwrap();
            if body["woods"].as_u64().unwrap_or(0) > 0 {
                break;
            }
        }
        assert!(Instant::now() < deadline, "the assistant never synced");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    Assistant {
        url,
        _task: Task(task),
    }
}

/// The compiled website, stopped with SIGTERM so its coverage data is written.
pub struct Site {
    pub url: String,
    child: Child,
}

impl Drop for Site {
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

pub async fn site(api_url: &str, assistant_url: &str) -> Site {
    let listen = free_port();
    let child = Command::new(assert_cmd::cargo::cargo_bin("zylotek-webapp"))
        .args(["--api-url", api_url, "--assistant-url", assistant_url])
        .args(["--listen", &listen.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let site = Site {
        url: format!("http://{listen}"),
        child,
    };
    let deadline = Instant::now() + Duration::from_secs(20);
    while reqwest::get(format!("{}/health", site.url)).await.is_err() {
        assert!(Instant::now() < deadline, "the website never came up");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    site
}

pub async fn get(site: &Site, path: &str) -> (u16, String) {
    let response = reqwest::get(format!("{}{path}", site.url)).await.unwrap();

    (response.status().as_u16(), response.text().await.unwrap())
}

pub async fn ask(site: &Site, question: &str) -> (u16, String) {
    let body = format!("question={}", question.replace(' ', "+"));
    let response = reqwest::Client::new()
        .post(format!("{}/ask", site.url))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .unwrap();

    (response.status().as_u16(), response.text().await.unwrap())
}

/// Ids of the API's woods in listing order, by scraping the site's own list page.
pub fn ids_in(html: &str) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    for id in html
        .split("href=\"/woods/")
        .skip(1)
        .filter_map(|rest| rest.split('"').next())
    {
        // A card links to its wood more than once.
        if !ids.iter().any(|seen| seen == id) {
            ids.push(id.to_owned());
        }
    }

    ids
}

fn delta(text: &str) -> String {
    format!(
        "data: {}\n\n",
        json!({"choices": [{"delta": {"content": text}}]})
    )
}

fn completion(content: &str) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .set_body_json(json!({"choices": [{"message": {"content": content}}]}))
}

/// A model server that writes its explanation in two halves with `pause` between them, so a
/// test can tell whether the website forwards the first half before the second exists.
pub async fn slow_model(pause: Duration) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let mut request = Vec::new();
                let mut buffer = [0u8; 4096];
                loop {
                    let read = socket.read(&mut buffer).await.unwrap_or(0);
                    request.extend_from_slice(&buffer[..read]);
                    if read == 0 || request_is_complete(&request) {
                        break;
                    }
                }
                let request = String::from_utf8_lossy(&request).into_owned();

                if request.contains("response_format") || !request.contains(r#""stream":true"#) {
                    let body = json!({"choices": [{"message": {"content": NEED}}]}).to_string();
                    let reply = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    socket.write_all(reply.as_bytes()).await.unwrap();
                    return;
                }
                let head = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n";
                socket.write_all(head.as_bytes()).await.unwrap();
                for (n, part) in [
                    delta("The hardest "),
                    format!("{}data: [DONE]\n\n", delta("wood suits a table.")),
                ]
                .iter()
                .enumerate()
                {
                    if n == 1 {
                        tokio::time::sleep(pause).await;
                    }
                    let chunk = format!("{:x}\r\n{part}\r\n", part.len());
                    socket.write_all(chunk.as_bytes()).await.unwrap();
                    socket.flush().await.unwrap();
                }
                socket.write_all(b"0\r\n\r\n").await.unwrap();
            });
        }
    });

    url
}

fn request_is_complete(request: &[u8]) -> bool {
    let text = String::from_utf8_lossy(request);
    let Some((head, body)) = text.split_once("\r\n\r\n") else {
        return false;
    };
    let length = head
        .lines()
        .find_map(|line| {
            line.to_lowercase()
                .strip_prefix("content-length:")
                .map(|v| v.trim().parse::<usize>().unwrap_or(0))
        })
        .unwrap_or(0);

    body.len() >= length
}
