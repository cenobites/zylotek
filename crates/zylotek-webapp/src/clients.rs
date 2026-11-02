//! The two services the web app depends on, over HTTP, using the shared contract types.

use std::time::Duration;

use reqwest::RequestBuilder;
use serde::de::DeserializeOwned;
use zylotek_contracts::{
    ErrorResponse,
    assistant::{AskRequest, AskResponse},
    woods::{WoodPageResponse, WoodResponse},
};

use crate::error::BackendError;

const API: &str = "the wood API";
const ASSISTANT: &str = "the assistant";
/// The assistant calls a model twice; give it room.
const ASSISTANT_TIMEOUT: Duration = Duration::from_secs(240);
const API_TIMEOUT: Duration = Duration::from_secs(10);

fn client(timeout: Duration) -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .expect("the HTTP client builds with a timeout")
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
        Err(error) => {
            return Err(BackendError::Unreachable {
                service,
                detail: error.without_url().to_string(),
            });
        }
    };
    if !(200..300).contains(&status) {
        let message = serde_json::from_str::<ErrorResponse>(&text)
            .map(|body| body.message)
            .unwrap_or_else(|_| text.chars().take(200).collect());

        return Err(BackendError::Status {
            service,
            status,
            message,
        });
    }

    serde_json::from_str(&text).map_err(|error| BackendError::Malformed {
        service,
        detail: error.to_string(),
    })
}

pub struct ApiClient {
    client: reqwest::Client,
    base_url: String,
}

impl ApiClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            client: client(API_TIMEOUT),
            base_url: base_url.trim_end_matches('/').to_owned(),
        }
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
            endpoint: format!("{}/v1/ask", base_url.trim_end_matches('/')),
            token,
        }
    }

    pub async fn ask(&self, question: &str) -> Result<AskResponse, BackendError> {
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

        call(ASSISTANT, request).await
    }
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

    #[tokio::test]
    async fn asking_posts_the_question_with_the_token() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/ask"))
            .and(header("authorization", "Bearer t0ken"))
            .and(body_json(json!({"question": "a table"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "summary": "s", "criteria": [], "recommendations": [],
                "explanation": null, "catalogue_size": 3,
            })))
            .expect(1)
            .mount(&server)
            .await;
        let assistant = AssistantClient::new(&format!("{}/", server.uri()), Some("t0ken".into()));

        let answer = assistant.ask("a table").await.unwrap();

        assert_eq!(answer.catalogue_size, 3);
    }

    #[tokio::test]
    async fn the_assistants_own_error_message_is_passed_on() {
        let server = MockServer::start().await;
        Mock::given(path("/v1/ask"))
            .respond_with(
                ResponseTemplate::new(503).set_body_json(
                    json!({"code": "CATALOGUE_NOT_READY", "message": "not synced yet"}),
                ),
            )
            .mount(&server)
            .await;

        let error = AssistantClient::new(&server.uri(), None)
            .ask("x")
            .await
            .unwrap_err();

        assert!(matches!(
            &error,
            BackendError::Status { status: 503, message, .. } if message == "not synced yet"
        ));
    }
}
