//! The whole chain over real sockets: the wood API, the compiled assistant, and a model
//! server. Seeds woods through the API's own write endpoint, so a drift between the API's
//! JSON and the contracts the assistant reads fails here.

mod common;

use assert_cmd::Command;
use common::{NEED, api_with, ask, ask_stream, completion, health, model, spawn, until_synced};
use predicates::prelude::*;
use serde_json::json;
use wiremock::{
    Mock, ResponseTemplate,
    matchers::{method, path},
};

const WOODS: [(&str, f64); 3] = [("Soft", 300.0), ("Hard", 1500.0), ("Middle", 900.0)];

#[tokio::test]
async fn a_question_is_answered_from_the_catalogue_pulled_from_the_api() {
    let api = api_with(&WOODS).await;
    let model = model().await;
    let assistant = spawn(&api.url, &model.uri(), &[]);
    until_synced(&assistant, 3).await;

    let (status, answer) = ask(&assistant, None, "I need a hard table").await;

    assert_eq!(status, 200, "{answer}");
    assert_eq!(answer["summary"], "a hard table");
    assert_eq!(answer["catalogue_size"], 3);
    let names: Vec<_> = answer["recommendations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Hard", "Middle", "Soft"]);
    assert_eq!(
        answer["recommendations"][0]["reasons"][0],
        json!({"property": "hardness", "value": 1500.0, "fit": answer["recommendations"][0]["reasons"][0]["fit"]})
    );
    assert_eq!(answer["explanation"], "The hardest wood suits a table.");
}

#[tokio::test]
async fn the_answer_can_be_streamed_ranking_first_then_the_words() {
    let api = api_with(&WOODS).await;
    let model = model().await;
    let assistant = spawn(&api.url, &model.uri(), &["--token", "s3cret"]);
    until_synced(&assistant, 3).await;

    let (status, events) = ask_stream(&assistant, Some("s3cret"), "a hard table").await;
    let (anonymous, _) = ask_stream(&assistant, None, "a hard table").await;

    assert_eq!((status, anonymous), (200, 401));
    let names: Vec<_> = events.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["answer", "token", "token", "token", "done"]);
    let answer: serde_json::Value = serde_json::from_str(&events[0].1).unwrap();
    assert_eq!(answer["recommendations"][0]["name"], "Hard");
    assert_eq!(answer["explanation"], serde_json::Value::Null);
    let words: String = events[1..4]
        .iter()
        .map(|(_, data)| serde_json::from_str::<String>(data).unwrap())
        .collect();
    assert_eq!(words, "The hardest wood suits a table.");
}

#[tokio::test]
async fn the_top_option_limits_the_list() {
    let api = api_with(&WOODS).await;
    let model = model().await;
    let assistant = spawn(&api.url, &model.uri(), &["--top", "1"]);
    until_synced(&assistant, 3).await;

    let (_, answer) = ask(&assistant, None, "a hard table").await;

    assert_eq!(answer["recommendations"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn the_token_protects_asking_but_not_health() {
    let api = api_with(&WOODS).await;
    let model = model().await;
    let assistant = spawn(&api.url, &model.uri(), &["--token", "s3cret"]);
    until_synced(&assistant, 3).await;

    let (anonymous, _) = ask(&assistant, None, "a hard table").await;
    let (wrong, _) = ask(&assistant, Some("nope"), "a hard table").await;
    let (right, _) = ask(&assistant, Some("s3cret"), "a hard table").await;

    assert_eq!((anonymous, wrong, right), (401, 401, 200));
}

#[tokio::test]
async fn it_starts_without_the_api_and_says_so_until_the_catalogue_arrives() {
    let model = model().await;
    let assistant = spawn("http://127.0.0.1:1", &model.uri(), &["--refresh-secs", "1"]);

    let body = health(&assistant).await;
    let (status, error) = ask(&assistant, None, "a hard table").await;

    assert_eq!(body["status"], "syncing");
    assert_eq!(body["woods"], 0);
    assert_eq!(
        (status, error["code"].as_str()),
        (503, Some("CATALOGUE_NOT_READY"))
    );
}

#[tokio::test]
async fn it_picks_up_woods_added_after_it_started() {
    let api = api_with(&WOODS).await;
    let model = model().await;
    let assistant = spawn(&api.url, &model.uri(), &["--refresh-secs", "1"]);
    until_synced(&assistant, 3).await;

    let created = reqwest::Client::new()
        .post(format!("{}/api/v1/woods", api.url))
        .bearer_auth(zylotek_api::auth::testing::write_token())
        .header("content-type", "application/json")
        .body(json!({"name": "Fresh", "scientific_name": "Fresh novus"}).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), 201);

    until_synced(&assistant, 4).await;
}

#[tokio::test]
async fn a_dead_model_is_a_bad_gateway_not_a_hang() {
    let api = api_with(&WOODS).await;
    let assistant = spawn(&api.url, "http://127.0.0.1:1", &[]);
    until_synced(&assistant, 3).await;

    let (status, error) = ask(&assistant, None, "a hard table").await;

    assert_eq!(
        (status, error["code"].as_str()),
        (502, Some("MODEL_UNAVAILABLE"))
    );
}

#[tokio::test]
async fn a_model_that_cannot_explain_still_yields_the_ranking() {
    let api = api_with(&WOODS).await;
    let model = wiremock::MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(wiremock::matchers::body_string_contains("response_format"))
        .respond_with(completion(NEED))
        .mount(&model)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&model)
        .await;
    let assistant = spawn(&api.url, &model.uri(), &[]);
    until_synced(&assistant, 3).await;

    let (status, answer) = ask(&assistant, None, "a hard table").await;

    assert_eq!(status, 200);
    assert_eq!(answer["explanation"], serde_json::Value::Null);
    assert_eq!(answer["recommendations"][0]["name"], "Hard");
}

#[tokio::test]
async fn the_model_key_is_sent_as_a_bearer() {
    let api = api_with(&WOODS).await;
    let model = wiremock::MockServer::start().await;
    Mock::given(method("POST"))
        .and(wiremock::matchers::header("authorization", "Bearer sk-gpu"))
        .respond_with(completion(NEED))
        .mount(&model)
        .await;
    let assistant = spawn(&api.url, &model.uri(), &["--model-api-key", "sk-gpu"]);
    until_synced(&assistant, 3).await;

    let (status, answer) = ask(&assistant, None, "a hard table").await;

    assert_eq!(status, 200, "{answer}");
}

#[test]
fn the_binary_documents_itself_and_rejects_bad_options() {
    let mut help = Command::cargo_bin("zylotek-assistant").unwrap();
    help.arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("--model-url").and(predicate::str::contains("--token")));

    for args in [
        ["--top", "0"],
        ["--refresh-secs", "0"],
        ["--listen", "nowhere"],
    ] {
        Command::cargo_bin("zylotek-assistant")
            .unwrap()
            .args(args)
            .assert()
            .failure();
    }
}

#[test]
fn a_port_that_is_taken_is_a_clean_failure() {
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();

    Command::cargo_bin("zylotek-assistant")
        .unwrap()
        .args(["--listen", &taken.local_addr().unwrap().to_string()])
        .args(["--api-url", "http://127.0.0.1:1"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("error:"));
}

#[test]
fn a_config_file_that_is_missing_stops_the_binary() {
    let missing = std::env::temp_dir().join("zylotek-no-such.toml");
    Command::cargo_bin("zylotek-assistant")
        .unwrap()
        .args(["--config-file", missing.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot read config file"));
}
