//! The website over real sockets, in front of the real wood API and assistant.

mod common;

use assert_cmd::Command;
use common::{Site, api_with, ask, assistant, get, ids_in, model, site, slow_model};
use predicates::prelude::*;

const WOODS: [(&str, f64); 3] = [("Soft", 300.0), ("Hard", 1500.0), ("Middle", 900.0)];

async fn running() -> (common::Api, wiremock::MockServer, common::Assistant, Site) {
    let api = api_with(&WOODS).await;
    let model = model().await;
    let assistant = assistant(&api.url, &model.uri()).await;
    let site = site(&api.url, &assistant.url).await;

    (api, model, assistant, site)
}

#[tokio::test]
async fn a_visitor_asks_a_question_and_follows_the_answer_to_a_comparison() {
    let (_api, _model, _assistant, site) = running().await;

    let (status, answer) = ask(&site, "I need a hard table").await;

    assert_eq!(status, 200, "{answer}");
    assert!(answer.contains("a hard table"));
    assert!(answer.contains("The hardest wood suits a table."));
    let hard = answer
        .find("<strong>Hard</strong>")
        .expect("Hard is listed");
    let middle = answer
        .find("<strong>Middle</strong>")
        .expect("Middle is listed");
    let soft = answer
        .find("<strong>Soft</strong>")
        .expect("Soft is listed");
    assert!(hard < middle && middle < soft, "best first");
    assert!(answer.contains("Hardness: 1500 kgf"));

    let ids = ids_in(&answer);
    assert_eq!(ids.len(), 3, "a details link per card");
    let (status, compare) = get(&site, &format!("/compare?id={}&id={}", ids[0], ids[1])).await;
    assert_eq!(status, 200);
    assert!(compare.contains("Hard") && compare.contains("Middle"));
    assert!(compare.contains("1500 kgf") && compare.contains("900 kgf"));
}

#[tokio::test]
async fn the_catalogue_can_be_browsed_and_each_wood_opened() {
    let (_api, _model, _assistant, site) = running().await;

    let (status, list) = get(&site, "/woods").await;
    let ids = ids_in(&list);
    let (wood_status, wood) = get(&site, &format!("/woods/{}", ids[0])).await;
    let (missing, _) = get(&site, "/woods/00000000-0000-0000-0000-000000000000").await;

    assert_eq!(status, 200);
    assert!(list.contains("3 woods"));
    assert_eq!(ids.len(), 3);
    assert_eq!(wood_status, 200);
    assert!(wood.contains("Key figures") && wood.contains("Janka hardness"));
    assert_eq!(missing, 404);
}

#[tokio::test]
async fn the_site_stays_up_when_the_assistant_is_down() {
    let api = api_with(&WOODS).await;
    let site = site(&api.url, "http://127.0.0.1:1").await;

    let (asked, html) = ask(&site, "a table").await;
    let (browse, _) = get(&site, "/woods").await;
    let (home, _) = get(&site, "/").await;

    assert_eq!(asked, 200, "the thinking panel was already sent");
    assert!(html.contains("could not reach the assistant"));
    assert_eq!((browse, home), (200, 200));
}

#[tokio::test]
async fn the_site_stays_up_when_the_api_is_down() {
    let site = site("http://127.0.0.1:1", "http://127.0.0.1:1").await;

    let (list, html) = get(&site, "/woods").await;
    let (home, _) = get(&site, "/").await;

    assert_eq!(list, 502);
    assert!(html.contains("could not reach the wood API"));
    assert_eq!(home, 200);
}

#[test]
fn the_binary_documents_itself_and_rejects_bad_options() {
    Command::cargo_bin("zylotek-webapp")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .stdout(
            predicate::str::contains("--assistant-url").and(predicate::str::contains("--api-url")),
        );
    Command::cargo_bin("zylotek-webapp")
        .unwrap()
        .args(["--listen", "nowhere"])
        .assert()
        .failure();
}

#[test]
fn a_port_that_is_taken_is_a_clean_failure() {
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();

    Command::cargo_bin("zylotek-webapp")
        .unwrap()
        .args(["--listen", &taken.local_addr().unwrap().to_string()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("error:"));
}

#[tokio::test]
async fn the_ranking_reaches_the_browser_before_the_explanation_is_written() {
    use std::time::{Duration, Instant};

    let pause = Duration::from_millis(1500);
    let api = api_with(&WOODS).await;
    let model = slow_model(pause).await;
    let assistant = assistant(&api.url, &model).await;
    let site = site(&api.url, &assistant.url).await;

    let started = Instant::now();
    let mut response = reqwest::Client::new()
        .post(format!("{}/ask", site.url))
        .header("content-type", "application/x-www-form-urlencoded")
        .body("question=a+hard+table")
        .send()
        .await
        .unwrap();
    // The page start may come in more than one network chunk.
    let mut first = String::new();
    while !first.contains("Thinking…") {
        first += &String::from_utf8_lossy(&response.chunk().await.unwrap().unwrap());
    }
    let mut ranked = first.clone();
    while !ranked.contains("<strong>Hard</strong>") {
        ranked += &String::from_utf8_lossy(&response.chunk().await.unwrap().unwrap());
    }
    let ranked_at = started.elapsed();
    let mut rest = String::new();
    while let Some(chunk) = response.chunk().await.unwrap() {
        rest += &String::from_utf8_lossy(&chunk);
    }

    assert!(
        first.contains("Thinking…") && !first.contains("<strong>Hard</strong>"),
        "the thinking panel comes first, on its own"
    );
    assert!(ranked_at < pause, "the ranking took {ranked_at:?}");
    assert!(!ranked.contains("wood suits a table."));
    assert!(started.elapsed() >= pause, "the rest waited for the model");
    assert!(rest.contains("wood suits a table."));
    assert!(rest.trim_end().ends_with("</html>"));
}

#[tokio::test]
async fn an_ask_is_answered_and_stored_even_when_nobody_waits_for_it() {
    use std::time::{Duration, Instant};

    let (api, _model, _assistant, site) = running().await;
    let no_redirects = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();

    // Ask, and leave before the answer page is even opened.
    let asked = no_redirects
        .post(format!("{}/ask", site.url))
        .header("content-type", "application/x-www-form-urlencoded")
        .body("question=a+hard+table")
        .send()
        .await
        .unwrap();
    assert_eq!(asked.status().as_u16(), 303);
    let location = asked.headers()["location"].to_str().unwrap().to_owned();
    let id = location.strip_prefix("/asks/").unwrap().to_owned();

    // The API's record moves on by itself until it is answered.
    let deadline = Instant::now() + Duration::from_secs(20);
    let record = loop {
        let text = reqwest::get(format!("{}/api/v1/asks/{id}", api.url))
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        let record: serde_json::Value = serde_json::from_str(&text).unwrap();
        if record["state"] == "answered" {
            break record;
        }
        assert!(
            Instant::now() < deadline,
            "the ask never finished: {record}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let (status, page) = get(&site, &location).await;

    assert_eq!(record["answer"]["recommendations"][0]["name"], "Hard");
    assert!(
        record["explanation"]
            .as_str()
            .unwrap()
            .contains("wood suits a table.")
    );
    assert_eq!(status, 200);
    assert!(page.contains("<strong>Hard</strong>") && page.contains("wood suits a table."));
    assert!(page.contains(&format!(r#"data-ask-id="{id}""#)));
}

#[test]
fn a_config_file_that_is_missing_or_malformed_stops_the_binary() {
    let dir = std::env::temp_dir();
    Command::cargo_bin("zylotek-webapp")
        .unwrap()
        .args([
            "--config-file",
            dir.join("zylotek-no-such.toml").to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot read config file"));
}
