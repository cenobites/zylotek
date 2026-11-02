//! The language model behind a small trait, and an adapter for any server that speaks the
//! OpenAI chat-completions protocol — llama.cpp's `llama-server`, vLLM, Ollama, LM Studio.

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::ModelError;

/// Generation is slow on a laptop-class GPU; this is generous on purpose.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
/// Low, so the same question reads the same way twice.
const TEMPERATURE: f64 = 0.2;
const MAX_TOKENS: u32 = 700;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    System,
    User,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }

    fn to_json(&self) -> Value {
        let role = match self.role {
            Role::System => "system",
            Role::User => "user",
        };
        json!({"role": role, "content": self.content})
    }
}

/// Something that completes a chat. With a schema, the reply must be JSON that fits it.
#[async_trait]
pub trait Model: Send + Sync {
    async fn complete(
        &self,
        messages: &[Message],
        schema: Option<&Value>,
    ) -> Result<String, ModelError>;
}

pub struct OpenAiCompatible {
    client: reqwest::Client,
    endpoint: String,
    model: String,
    api_key: Option<String>,
}

impl OpenAiCompatible {
    /// `base_url` is the server root, such as `http://gpu-box:8080`; the `/v1/chat/completions`
    /// path is added here.
    pub fn new(base_url: &str, model: &str, api_key: Option<String>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .expect("the HTTP client builds with a timeout");

        Self {
            client,
            endpoint: format!("{}/v1/chat/completions", base_url.trim_end_matches('/')),
            model: model.to_owned(),
            api_key,
        }
    }

    fn body(&self, messages: &[Message], schema: Option<&Value>) -> Value {
        let mut body = json!({
            "model": self.model,
            "messages": messages.iter().map(Message::to_json).collect::<Vec<_>>(),
            "temperature": TEMPERATURE,
            "max_tokens": MAX_TOKENS,
            "stream": false,
        });
        if let Some(schema) = schema {
            body["response_format"] = json!({
                "type": "json_schema",
                "json_schema": {"name": "answer", "strict": true, "schema": schema},
            });
        }

        body
    }
}

fn transport(error: reqwest::Error) -> ModelError {
    ModelError::Transport(error.to_string())
}

#[derive(Deserialize)]
struct Completion {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: ReplyMessage,
}

#[derive(Deserialize)]
struct ReplyMessage {
    content: Option<String>,
}

#[async_trait]
impl Model for OpenAiCompatible {
    async fn complete(
        &self,
        messages: &[Message],
        schema: Option<&Value>,
    ) -> Result<String, ModelError> {
        let mut request = self
            .client
            .post(&self.endpoint)
            .header("content-type", "application/json")
            .body(self.body(messages, schema).to_string());
        if let Some(key) = &self.api_key {
            request = request.bearer_auth(key);
        }

        let response = request.send().await.map_err(transport)?;
        let status = response.status();
        let text = response.text().await.map_err(transport)?;
        if !status.is_success() {
            return Err(ModelError::Status {
                status: status.as_u16(),
                body: text.chars().take(300).collect(),
            });
        }

        let completion: Completion = serde_json::from_str(&text)
            .map_err(|error| ModelError::Malformed(error.to_string()))?;
        completion
            .choices
            .into_iter()
            .next()
            .and_then(|choice| choice.message.content)
            .map(|content| content.trim().to_owned())
            .filter(|content| !content.is_empty())
            .ok_or(ModelError::Empty)
    }
}

#[cfg(test)]
mod tests {
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_partial_json, header, method, path},
    };

    use super::*;

    fn reply(content: Value) -> ResponseTemplate {
        ResponseTemplate::new(200)
            .set_body_json(json!({"choices": [{"message": {"content": content}}]}))
    }

    #[tokio::test]
    async fn it_posts_a_chat_completion_and_returns_the_trimmed_text() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(body_partial_json(json!({
                "model": "qwen",
                "messages": [
                    {"role": "system", "content": "be brief"},
                    {"role": "user", "content": "hi"},
                ],
                "stream": false,
            })))
            .respond_with(reply(json!("  hello \n")))
            .expect(1)
            .mount(&server)
            .await;
        let model = OpenAiCompatible::new(&format!("{}/", server.uri()), "qwen", None);

        let text = model
            .complete(&[Message::system("be brief"), Message::user("hi")], None)
            .await
            .unwrap();

        assert_eq!(text, "hello");
    }

    #[tokio::test]
    async fn a_schema_becomes_a_response_format_and_a_key_becomes_a_bearer() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(header("authorization", "Bearer sk-local"))
            .and(body_partial_json(json!({
                "response_format": {
                    "type": "json_schema",
                    "json_schema": {"strict": true, "schema": {"type": "object"}},
                },
            })))
            .respond_with(reply(json!("{}")))
            .expect(1)
            .mount(&server)
            .await;
        let model = OpenAiCompatible::new(&server.uri(), "m", Some("sk-local".into()));

        let text = model
            .complete(&[Message::user("x")], Some(&json!({"type": "object"})))
            .await
            .unwrap();

        assert_eq!(text, "{}");
    }

    #[tokio::test]
    async fn every_way_the_server_can_fail_is_a_distinct_error() {
        let server = MockServer::start().await;
        Mock::given(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(503).set_body_string("loading model"))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(path("/v1/chat/completions"))
            .respond_with(reply(json!("   ")))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(path("/v1/chat/completions"))
            .respond_with(reply(Value::Null))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"choices": []})))
            .mount(&server)
            .await;
        let model = OpenAiCompatible::new(&server.uri(), "m", None);
        let messages = [Message::user("x")];
        let ask = || model.complete(&messages, None);

        assert!(matches!(
            ask().await,
            Err(ModelError::Status { status: 503, body }) if body == "loading model"
        ));
        assert!(matches!(ask().await, Err(ModelError::Malformed(_))));
        assert!(matches!(ask().await, Err(ModelError::Empty)), "blank text");
        assert!(matches!(ask().await, Err(ModelError::Empty)), "null text");
        assert!(matches!(ask().await, Err(ModelError::Empty)), "no choices");
    }

    #[tokio::test]
    async fn an_unreachable_server_is_a_transport_error() {
        let model = OpenAiCompatible::new("http://127.0.0.1:1", "m", None);

        let error = model
            .complete(&[Message::user("x")], None)
            .await
            .unwrap_err();

        assert!(matches!(error, ModelError::Transport(_)));
        assert!(error.to_string().contains("could not reach"));
    }

    #[tokio::test]
    async fn a_long_error_body_is_cut_short() {
        let server = MockServer::start().await;
        Mock::given(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(500).set_body_string("x".repeat(5000)))
            .mount(&server)
            .await;
        let model = OpenAiCompatible::new(&server.uri(), "m", None);

        let error = model
            .complete(&[Message::user("x")], None)
            .await
            .unwrap_err();

        assert!(matches!(error, ModelError::Status { body, .. } if body.len() == 300));
    }
}
