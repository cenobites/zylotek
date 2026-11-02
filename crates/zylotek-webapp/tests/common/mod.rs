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
    html.split("href=\"/woods/")
        .skip(1)
        .filter_map(|rest| rest.split('"').next())
        .map(str::to_owned)
        .collect()
}
