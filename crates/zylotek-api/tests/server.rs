//! The router served over a real TCP socket, as `api run` serves it.

mod common;

use std::net::SocketAddr;

use common::{app, wood_body};
use serde_json::Value;
use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle};

struct Server {
    address: SocketAddr,
    shutdown: oneshot::Sender<()>,
    task: JoinHandle<std::io::Result<()>>,
}

async fn serve() -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (shutdown, signal) = oneshot::channel::<()>();
    let router = app().await;
    let task = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async {
                let _ = signal.await;
            })
            .await
    });

    Server {
        address,
        shutdown,
        task,
    }
}

#[tokio::test]
async fn serves_requests_over_http() {
    let server = serve().await;
    let client = reqwest::Client::new();
    let base = format!("http://{}", server.address);

    let health = client.get(format!("{base}/health")).send().await.unwrap();
    assert_eq!(health.status(), 200);
    assert_eq!(health.text().await.unwrap(), "OK");

    let created = client
        .post(format!("{base}/api/v1/woods"))
        .header("content-type", "application/json")
        .body(wood_body("Aroeira", "Myracrodruon urundeuva").to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), 201);
    let created: Value = serde_json::from_str(&created.text().await.unwrap()).unwrap();

    let fetched = client
        .get(format!(
            "{base}/api/v1/woods/{}",
            created["id"].as_str().unwrap()
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(fetched.status(), 200);

    let missing = client.get(format!("{base}/nope")).send().await.unwrap();
    assert_eq!(missing.status(), 404);
    assert_eq!(missing.text().await.unwrap(), "404 Not Found");
}

#[tokio::test]
async fn handles_many_concurrent_clients() {
    let server = serve().await;
    let client = reqwest::Client::new();
    let url = format!("http://{}/api/v1/woods", server.address);

    let requests: Vec<_> = (0..64)
        .map(|_| {
            let (client, url) = (client.clone(), url.clone());
            tokio::spawn(async move { client.get(url).send().await.unwrap().status() })
        })
        .collect();

    for request in requests {
        assert_eq!(request.await.unwrap(), 200);
    }
}

#[tokio::test]
async fn shuts_down_gracefully_once_signalled() {
    let server = serve().await;
    let client = reqwest::Client::new();
    let url = format!("http://{}/health", server.address);
    assert_eq!(client.get(&url).send().await.unwrap().status(), 200);

    server.shutdown.send(()).unwrap();
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), server.task)
        .await
        .expect("the server stops after the shutdown signal")
        .unwrap();

    assert!(result.is_ok());
    assert!(
        client.get(&url).send().await.is_err(),
        "the port is closed after shutdown"
    );
}
