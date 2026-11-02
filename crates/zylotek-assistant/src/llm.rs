//! The language model behind a small trait, and an adapter for any server that speaks the
//! OpenAI chat-completions protocol — llama.cpp's `llama-server`, vLLM, Ollama, LM Studio.

use std::{pin::Pin, time::Duration};

use async_trait::async_trait;
use futures::{Stream, stream};
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

/// The pieces of a reply as the model writes them.
pub type TextStream = Pin<Box<dyn Stream<Item = Result<String, ModelError>> + Send>>;

/// Something that completes a chat. With a schema, the reply must be JSON that fits it.
#[async_trait]
pub trait Model: Send + Sync {
    async fn complete(
        &self,
        messages: &[Message],
        schema: Option<&Value>,
    ) -> Result<String, ModelError>;

    /// Like `complete` without a schema, but hands back the text piece by piece. Failing to
    /// start is an `Err` here; a failure midway is an `Err` item in the stream.
    async fn stream(&self, messages: &[Message]) -> Result<TextStream, ModelError>;
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

    fn request(&self, body: Value) -> reqwest::RequestBuilder {
        let request = self
            .client
            .post(&self.endpoint)
            .header("content-type", "application/json")
            .body(body.to_string());

        match &self.api_key {
            Some(key) => request.bearer_auth(key),
            None => request,
        }
    }

    fn body(&self, messages: &[Message], schema: Option<&Value>, stream: bool) -> Value {
        let mut body = json!({
            "model": self.model,
            "messages": messages.iter().map(Message::to_json).collect::<Vec<_>>(),
            "temperature": TEMPERATURE,
            "max_tokens": MAX_TOKENS,
            "stream": stream,
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
        let request = self.request(self.body(messages, schema, false));

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

    async fn stream(&self, messages: &[Message]) -> Result<TextStream, ModelError> {
        let response = self
            .request(self.body(messages, None, true))
            .send()
            .await
            .map_err(transport)?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.map_err(transport)?;

            return Err(ModelError::Status {
                status: status.as_u16(),
                body: body.chars().take(300).collect(),
            });
        }
        let reader = SseReader {
            response,
            buffer: Vec::new(),
            finished: false,
        };

        Ok(Box::pin(stream::unfold(reader, SseReader::next)))
    }
}

/// What one line of a server-sent-events body means.
#[derive(Debug, PartialEq, Eq)]
enum Line {
    Delta(String),
    Done,
    Skip,
}

fn parse_line(line: &str) -> Result<Line, ModelError> {
    let Some(data) = line.trim_end_matches('\r').strip_prefix("data:") else {
        return Ok(Line::Skip);
    };
    let data = data.trim();
    if data == "[DONE]" {
        return Ok(Line::Done);
    }
    let chunk: StreamChunk =
        serde_json::from_str(data).map_err(|error| ModelError::Malformed(error.to_string()))?;
    let text = chunk
        .choices
        .into_iter()
        .next()
        .and_then(|choice| choice.delta.content)
        .unwrap_or_default();

    Ok(if text.is_empty() {
        Line::Skip
    } else {
        Line::Delta(text)
    })
}

#[derive(Deserialize)]
struct StreamChunk {
    choices: Vec<StreamChoice>,
}

#[derive(Deserialize)]
struct StreamChoice {
    delta: Delta,
}

#[derive(Deserialize)]
struct Delta {
    content: Option<String>,
}

/// Reads a chunked response and hands out complete lines, however the network cut them.
struct SseReader {
    response: reqwest::Response,
    buffer: Vec<u8>,
    finished: bool,
}

impl SseReader {
    /// The next piece of text, or `None` at the end (also after an error was handed out).
    async fn next(mut self) -> Option<(Result<String, ModelError>, Self)> {
        while !self.finished {
            let Some(end) = self.buffer.iter().position(|byte| *byte == b'\n') else {
                match self.response.chunk().await {
                    Ok(Some(bytes)) => self.buffer.extend_from_slice(&bytes),
                    Ok(None) if self.buffer.is_empty() => self.finished = true,
                    Ok(None) => self.buffer.push(b'\n'),
                    Err(error) => {
                        self.finished = true;
                        return Some((Err(transport(error)), self));
                    }
                }
                continue;
            };
            let line: Vec<u8> = self.buffer.drain(..=end).collect();
            match parse_line(&String::from_utf8_lossy(&line)) {
                Ok(Line::Delta(text)) => return Some((Ok(text), self)),
                Ok(Line::Skip) => {}
                Ok(Line::Done) => self.finished = true,
                Err(error) => {
                    self.finished = true;
                    return Some((Err(error), self));
                }
            }
        }

        None
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

    // ---- streaming ----------------------------------------------------------------------

    fn delta(text: &str) -> String {
        format!(
            "data: {}\n\n",
            json!({"choices": [{"delta": {"content": text}}]})
        )
    }

    async fn collect(stream: TextStream) -> Vec<Result<String, ModelError>> {
        use futures::StreamExt;

        stream.collect().await
    }

    fn texts(items: Vec<Result<String, ModelError>>) -> Vec<String> {
        items.into_iter().map(Result::unwrap).collect()
    }

    /// Serves one chunked response, writing each part as its own chunk with a pause between,
    /// so lines and even characters arrive cut in pieces. Without `finish` the connection is
    /// dropped mid-body.
    async fn chunked_server(parts: Vec<Vec<u8>>, finish: bool) -> String {
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
            if finish {
                socket.write_all(b"0\r\n\r\n").await.unwrap();
            }
        });

        url
    }

    #[test]
    fn each_kind_of_stream_line_is_understood() {
        let delta_line = r#"data: {"choices":[{"delta":{"content":"Hi"}}]}"#;

        assert_eq!(parse_line(delta_line).unwrap(), Line::Delta("Hi".into()));
        assert_eq!(
            parse_line(&format!("{delta_line}\r")).unwrap(),
            Line::Delta("Hi".into())
        );
        assert_eq!(parse_line("data:[DONE]").unwrap(), Line::Done);
        assert_eq!(parse_line("data: [DONE]").unwrap(), Line::Done);
        for skipped in [
            "",
            ": keep-alive",
            "event: message",
            r#"data: {"choices":[{"delta":{"role":"assistant"}}]}"#,
            r#"data: {"choices":[{"delta":{"content":""}}]}"#,
            r#"data: {"choices":[{"delta":{"content":null}}]}"#,
            r#"data: {"choices":[]}"#,
        ] {
            assert_eq!(parse_line(skipped).unwrap(), Line::Skip, "{skipped:?}");
        }
        assert!(matches!(
            parse_line("data: {oops"),
            Err(ModelError::Malformed(_))
        ));
    }

    #[tokio::test]
    async fn a_stream_yields_the_pieces_in_order_and_asks_for_streaming() {
        let server = MockServer::start().await;
        let body = format!(
            "{}{}: ping\n\n{}data: [DONE]\n\n",
            delta("Hel"),
            delta("lo, "),
            delta("world")
        );
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(body_partial_json(
                json!({"stream": true, "messages": [{"role": "user", "content": "hi"}]}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .expect(1)
            .mount(&server)
            .await;
        let model = OpenAiCompatible::new(&server.uri(), "m", Some("k".into()));

        let stream = model.stream(&[Message::user("hi")]).await.unwrap();

        assert_eq!(texts(collect(stream).await), ["Hel", "lo, ", "world"]);
    }

    #[tokio::test]
    async fn text_after_done_is_ignored_and_a_missing_final_newline_is_tolerated() {
        let server = MockServer::start().await;
        Mock::given(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(format!(
                "{}data: [DONE]\n\n{}",
                delta("a"),
                delta("never")
            )))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                delta("x").trim_end().to_owned()
                    + "\ndata: {\"choices\":[{\"delta\":{\"content\":\"y\"}}]}",
            ))
            .mount(&server)
            .await;
        let model = OpenAiCompatible::new(&server.uri(), "m", None);

        let done = model.stream(&[Message::user("hi")]).await.unwrap();
        let unterminated = model.stream(&[Message::user("hi")]).await.unwrap();

        assert_eq!(texts(collect(done).await), ["a"]);
        assert_eq!(texts(collect(unterminated).await), ["x", "y"]);
    }

    #[tokio::test]
    async fn a_garbled_line_ends_the_stream_with_one_error() {
        let server = MockServer::start().await;
        Mock::given(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(format!(
                "{}data: {{broken\n\n{}",
                delta("ok"),
                delta("after")
            )))
            .mount(&server)
            .await;
        let model = OpenAiCompatible::new(&server.uri(), "m", None);

        let items = collect(model.stream(&[Message::user("hi")]).await.unwrap()).await;

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].as_ref().unwrap(), "ok");
        assert!(matches!(items[1], Err(ModelError::Malformed(_))));
    }

    #[tokio::test]
    async fn lines_and_characters_cut_by_the_network_are_put_back_together() {
        let first = delta("caf");
        let second = delta("é ao");
        let middle = second.find('é').unwrap() + 1; // inside the two bytes of é
        let parts = vec![
            first.as_bytes()[..10].to_vec(),
            first.as_bytes()[10..].to_vec(),
            second.as_bytes()[..middle].to_vec(),
            second.as_bytes()[middle..].to_vec(),
        ];
        let url = chunked_server(parts, true).await;
        let model = OpenAiCompatible::new(&url, "m", None);

        let items = collect(model.stream(&[Message::user("hi")]).await.unwrap()).await;

        assert_eq!(texts(items).concat(), "café ao");
    }

    #[tokio::test]
    async fn a_connection_that_dies_midway_is_an_error_item_then_the_end() {
        let url = chunked_server(vec![delta("partial").into_bytes()], false).await;
        let model = OpenAiCompatible::new(&url, "m", None);

        let items = collect(model.stream(&[Message::user("hi")]).await.unwrap()).await;

        assert_eq!(items[0].as_ref().unwrap(), "partial");
        assert!(matches!(items.last(), Some(Err(ModelError::Transport(_)))));
        assert_eq!(items.len(), 2);
    }

    #[tokio::test]
    async fn a_stream_that_cannot_start_is_an_error_up_front() {
        let server = MockServer::start().await;
        Mock::given(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(503).set_body_string("y".repeat(1000)))
            .mount(&server)
            .await;
        let busy = OpenAiCompatible::new(&server.uri(), "m", None);
        let down = OpenAiCompatible::new("http://127.0.0.1:1", "m", None);

        let status = busy.stream(&[Message::user("hi")]).await.err().unwrap();
        let transport = down.stream(&[Message::user("hi")]).await.err().unwrap();

        assert!(matches!(status, ModelError::Status { status: 503, body } if body.len() == 300));
        assert!(matches!(transport, ModelError::Transport(_)));
    }
}
