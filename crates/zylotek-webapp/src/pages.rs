//! The pages: ask a question, browse woods, read one, compare several. Rendered on the
//! server; the browser needs no script.

use std::sync::Arc;

use askama::Template;
use axum::{
    Form, Router,
    body::Body,
    extract::{Path, Query, RawQuery, State},
    http::{StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use futures::Stream;
use serde::Deserialize;
use std::convert::Infallible;

use crate::{
    clients::{ApiClient, AssistantClient, LiveAnswer, Piece, is_id},
    error::BackendError,
    view::{self, CardView, Fact, WoodRow, WoodView},
};

pub const PAGE_SIZE: usize = 24;
pub const MIN_COMPARE: usize = 2;
pub const MAX_COMPARE: usize = 4;
const MAX_QUESTION_CHARS: usize = 1000;

#[derive(Clone)]
pub struct AppState {
    pub api: Arc<ApiClient>,
    pub assistant: Arc<AssistantClient>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(home))
        .route("/ask", post(ask))
        .route("/woods", get(woods))
        .route("/woods/{id}", get(wood))
        .route("/compare", get(compare))
        .route("/health", get(|| async { "OK" }))
        .fallback(|| async { PageError::NotFound("that page") })
        .layer(axum::middleware::from_fn(security_headers))
        .with_state(state)
}

/// No scripts, no framing, images only over HTTPS: the pages need nothing more.
async fn security_headers(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    for (name, value) in [
        (
            "content-security-policy",
            "default-src 'none'; style-src 'unsafe-inline'; img-src https: data:; form-action 'self'; base-uri 'none'; frame-ancestors 'none'",
        ),
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "no-referrer"),
    ] {
        headers.insert(name, axum::http::HeaderValue::from_static(value));
    }

    response
}

// ---- templates ------------------------------------------------------------------------

#[derive(Template)]
#[template(path = "home.html")]
struct HomePage {
    question: String,
    error: Option<String>,
}

/// Everything up to the opening of the box the explanation streams into.
#[derive(Template)]
#[template(path = "answer_top.html")]
struct AnswerTop {
    question: String,
    summary: String,
    criteria: Vec<(&'static str, u8)>,
    cards: Vec<CardView>,
    can_compare: bool,
}

/// What closes the page once the explanation is done.
#[derive(Template)]
#[template(path = "answer_bottom.html")]
struct AnswerBottom {
    has_explanation_box: bool,
    catalogue_size: usize,
}

#[derive(Template)]
#[template(path = "woods.html")]
struct WoodsPage {
    rows: Vec<WoodRow>,
    total: u64,
    first: usize,
    last: usize,
    previous: Option<usize>,
    next: Option<usize>,
}

#[derive(Template)]
#[template(path = "wood.html")]
struct WoodPage {
    wood: WoodView,
}

#[derive(Template)]
#[template(path = "compare.html")]
struct ComparePage {
    names: Vec<(String, String, String)>,
    rows: Vec<CompareRow>,
}

struct CompareRow {
    label: &'static str,
    values: Vec<String>,
}

#[derive(Template)]
#[template(path = "error.html")]
struct ErrorPage {
    status: u16,
    title: &'static str,
    message: String,
}

fn render<T: Template>(template: T) -> Result<Html<String>, PageError> {
    template
        .render()
        .map(Html)
        .map_err(|error| PageError::Internal(error.to_string()))
}

// ---- errors ---------------------------------------------------------------------------

pub enum PageError {
    NotFound(&'static str),
    BadRequest(String),
    Backend(BackendError),
    Internal(String),
}

impl From<BackendError> for PageError {
    fn from(error: BackendError) -> Self {
        match error {
            BackendError::NotFound => PageError::NotFound("that wood"),
            other => PageError::Backend(other),
        }
    }
}

impl IntoResponse for PageError {
    fn into_response(self) -> Response {
        let (status, title, message) = match self {
            PageError::NotFound(what) => (
                StatusCode::NOT_FOUND,
                "Not found",
                format!("We could not find {what}."),
            ),
            PageError::BadRequest(message) => {
                (StatusCode::BAD_REQUEST, "Check your request", message)
            }
            PageError::Backend(error) => {
                tracing::warn!(%error, "a backend failed");
                (
                    StatusCode::BAD_GATEWAY,
                    "Service unavailable",
                    friendly(&error),
                )
            }
            PageError::Internal(detail) => {
                tracing::error!(%detail, "could not render a page");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Something went wrong",
                    "The page could not be built. Please try again.".to_owned(),
                )
            }
        };
        let page = ErrorPage {
            status: status.as_u16(),
            title,
            message,
        };

        // The template is checked at compile time; an empty body beats a second failure.
        (status, Html(page.render().unwrap_or_default())).into_response()
    }
}

/// What a visitor is told; the technical detail goes to the log only.
fn friendly(error: &BackendError) -> String {
    match error {
        BackendError::Unreachable { service, .. } => {
            format!("We could not reach {service}. Please try again in a moment.")
        }
        BackendError::Status {
            service,
            status: 400,
            message,
        } => format!("{service} could not use that: {message}"),
        BackendError::Status {
            service,
            status: 503,
            ..
        } => format!("{service} is still starting up. Please try again in a moment."),
        BackendError::Status { service, .. } | BackendError::Malformed { service, .. } => {
            format!("{service} had a problem answering. Please try again.")
        }
        BackendError::NotFound => "We could not find that wood.".to_owned(),
    }
}

// ---- handlers -------------------------------------------------------------------------

async fn home() -> Result<Html<String>, PageError> {
    render(HomePage {
        question: String::new(),
        error: None,
    })
}

#[derive(Deserialize)]
struct AskForm {
    question: String,
}

async fn ask(
    State(state): State<AppState>,
    Form(form): Form<AskForm>,
) -> Result<Response, PageError> {
    let question = form.question.trim().to_owned();
    if question.is_empty() || question.chars().count() > MAX_QUESTION_CHARS {
        // Back to the form with the text kept, rather than an error page.
        let error = format!("Please write between 1 and {MAX_QUESTION_CHARS} characters.");
        return Ok(render(HomePage {
            question: form.question,
            error: Some(error),
        })?
        .into_response());
    }

    // Wait only for the ranking. Anything that goes wrong before it is an ordinary error page;
    // once the page has started, the explanation is a courtesy that may be cut short.
    let live = state.assistant.ask_stream(&question).await?;
    let cards: Vec<CardView> = live.answer.recommendations.iter().map(view::card).collect();
    let has_explanation_box = !cards.is_empty();
    let top = AnswerTop {
        question,
        summary: live.answer.summary.clone(),
        criteria: live
            .answer
            .criteria
            .iter()
            .map(|c| (view::label(c.property), c.weight))
            .collect(),
        can_compare: cards.len() >= MIN_COMPARE,
        cards,
    }
    .render()
    .map_err(|error| PageError::Internal(error.to_string()))?;
    let bottom = AnswerBottom {
        has_explanation_box,
        catalogue_size: live.answer.catalogue_size,
    }
    .render()
    .map_err(|error| PageError::Internal(error.to_string()))?;

    Ok((
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
            (header::HeaderName::from_static("x-accel-buffering"), "no"),
        ],
        Body::from_stream(page_stream(top, bottom, live)),
    )
        .into_response())
}

/// The page as it is sent: the top at once, then the explanation piece by piece, then the
/// bottom. A broken explanation ends with a note instead of an error.
fn page_stream(
    top: String,
    bottom: String,
    live: LiveAnswer,
) -> impl Stream<Item = Result<String, Infallible>> {
    enum Stage {
        Top(String),
        Words,
        Bottom,
        Over,
    }

    futures::stream::unfold(
        (Stage::Top(top), live, bottom),
        |(stage, mut live, bottom)| async move {
            let (chunk, next) = match stage {
                Stage::Top(top) => (top, Stage::Words),
                Stage::Words => match live.next_piece().await {
                    Some(Piece::Text(text)) => (escape(&text), Stage::Words),
                    Some(Piece::Cut) => (
                        "\n<p class=\"err\">The explanation was cut short.</p>".to_owned(),
                        Stage::Bottom,
                    ),
                    None => (String::new(), Stage::Bottom),
                },
                Stage::Bottom => (bottom.clone(), Stage::Over),
                Stage::Over => return None,
            };

            Some((Ok(chunk), (next, live, bottom)))
        },
    )
}

/// HTML-escapes text that is written into the page by hand.
pub fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            other => escaped.push(other),
        }
    }

    escaped
}

#[derive(Deserialize)]
struct Paging {
    offset: Option<usize>,
}

async fn woods(
    State(state): State<AppState>,
    Query(paging): Query<Paging>,
) -> Result<Html<String>, PageError> {
    let offset = paging.offset.unwrap_or(0);
    let page = state.api.list(offset, PAGE_SIZE).await?;
    let shown = page.items.len();

    render(WoodsPage {
        rows: page.items.iter().map(view::row).collect(),
        total: page.total,
        first: if shown == 0 { 0 } else { offset + 1 },
        last: offset + shown,
        previous: (offset > 0).then(|| offset.saturating_sub(PAGE_SIZE)),
        next: ((offset + shown) as u64 > 0 && ((offset + shown) as u64) < page.total)
            .then_some(offset + shown),
    })
}

async fn wood(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Html<String>, PageError> {
    let wood = state.api.get(&id).await?;

    render(WoodPage {
        wood: view::detail(&wood),
    })
}

/// The `id` values of a query string: distinct, well-formed, between 2 and 4 of them.
pub fn compare_ids(query: Option<&str>) -> Result<Vec<String>, String> {
    let mut ids: Vec<String> = Vec::new();
    for (key, value) in form_urlencoded::parse(query.unwrap_or_default().as_bytes()) {
        if key != "id" {
            continue;
        }
        if !is_id(&value) {
            return Err("One of the selected woods is not valid.".to_owned());
        }
        if !ids.iter().any(|seen| *seen == value) {
            ids.push(value.into_owned());
        }
    }
    if ids.len() < MIN_COMPARE {
        return Err(format!("Select at least {MIN_COMPARE} woods to compare."));
    }
    if ids.len() > MAX_COMPARE {
        return Err(format!("Compare at most {MAX_COMPARE} woods at a time."));
    }

    Ok(ids)
}

async fn compare(
    State(state): State<AppState>,
    RawQuery(query): RawQuery,
) -> Result<Html<String>, PageError> {
    let ids = compare_ids(query.as_deref()).map_err(PageError::BadRequest)?;
    let mut woods = Vec::with_capacity(ids.len());
    for id in &ids {
        woods.push(state.api.get(id).await?);
    }
    let columns: Vec<Vec<Fact>> = woods.iter().map(view::facts).collect();
    let rows = columns[0]
        .iter()
        .enumerate()
        .map(|(n, fact)| CompareRow {
            label: fact.label,
            values: columns
                .iter()
                .map(|column| column[n].value.clone())
                .collect(),
        })
        .collect();

    render(ComparePage {
        names: woods
            .iter()
            .map(|w| (w.id.clone(), w.name.clone(), w.scientific_name.clone()))
            .collect(),
        rows,
    })
}

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{Request, header},
    };
    use http_body_util::BodyExt;
    use serde_json::{Value, json};
    use tower::ServiceExt;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path, query_param},
    };

    use super::*;

    struct Backends {
        api: MockServer,
        assistant: MockServer,
    }

    impl Backends {
        async fn start() -> Self {
            Self {
                api: MockServer::start().await,
                assistant: MockServer::start().await,
            }
        }

        fn app(&self) -> Router {
            router(AppState {
                api: Arc::new(ApiClient::new(&self.api.uri())),
                assistant: Arc::new(AssistantClient::new(&self.assistant.uri(), None)),
            })
        }

        async fn wood(&self, id: &str, name: &str) {
            Mock::given(path(format!("/api/v1/woods/{id}")))
                .respond_with(ResponseTemplate::new(200).set_body_json(wood_json(id, name)))
                .mount(&self.api)
                .await;
        }
    }

    fn wood_json(id: &str, name: &str) -> Value {
        json!({
            "id": id, "name": name, "scientific_name": format!("{name} scientificus"),
            "genus": name, "family": "Meliaceae", "common_names": ["Alias"],
            "images": {"main": "https://img.example/x.jpg"},
            "description": {"end_uses": "Furniture", "tree": {"Height": "30 m"}},
            "mechanical_properties": {"dry": {"janka_hardness": {"parallel_to_grain": 900.0}}},
        })
    }

    async fn get(app: &Router, uri: &str) -> (StatusCode, String, axum::http::HeaderMap) {
        send(app, Request::get(uri).body(Body::empty()).unwrap()).await
    }

    async fn send(
        app: &Router,
        request: Request<Body>,
    ) -> (StatusCode, String, axum::http::HeaderMap) {
        let response = app.clone().oneshot(request).await.unwrap();
        let (parts, body) = response.into_parts();
        let bytes = body.collect().await.unwrap().to_bytes();

        (
            parts.status,
            String::from_utf8(bytes.to_vec()).unwrap(),
            parts.headers,
        )
    }

    async fn post_question(app: &Router, question: &str) -> (StatusCode, String) {
        let body: String = form_urlencoded::Serializer::new(String::new())
            .append_pair("question", question)
            .finish();
        let request = Request::post("/ask")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap();
        let (status, html, _) = send(app, request).await;

        (status, html)
    }

    fn events(items: &[(&str, Value)]) -> ResponseTemplate {
        let body: String = items
            .iter()
            .map(|(name, data)| format!("event: {name}\ndata: {data}\n\n"))
            .collect();

        ResponseTemplate::new(200)
            .insert_header("content-type", "text/event-stream")
            .set_body_string(body)
    }

    fn ranking(recommendations: Value) -> Value {
        json!({
            "summary": "A big table",
            "criteria": [{"property": "stability", "weight": 3}, {"property": "hardness", "weight": 2}, {"property": "lightness", "weight": 1}],
            "recommendations": recommendations,
            "explanation": null,
            "catalogue_size": 225,
        })
    }

    /// A finished stream: the ranking, then the explanation in two pieces.
    fn answer(recommendations: Value) -> ResponseTemplate {
        events(&[
            ("answer", ranking(recommendations)),
            ("token", json!("Cedro is steady ")),
            ("token", json!("<b>and</b> easy to work.")),
            ("done", json!({})),
        ])
    }

    fn recommendation(id: &str, name: &str) -> Value {
        json!({"wood_id": id, "name": name, "scientific_name": "X y", "score": 0.91,
            "reasons": [{"property": "hardness", "value": 900.0, "fit": 0.8}]})
    }

    #[tokio::test]
    async fn the_home_page_is_a_form_and_every_page_carries_security_headers() {
        let backends = Backends::start().await;

        let (status, html, headers) = get(&backends.app(), "/").await;

        assert_eq!(status, StatusCode::OK);
        assert!(html.contains(r#"<form method="post" action="/ask">"#));
        assert!(html.contains("<textarea"));
        assert!(
            headers["content-security-policy"]
                .to_str()
                .unwrap()
                .contains("default-src 'none'")
        );
        assert_eq!(headers["x-content-type-options"], "nosniff");
        assert_eq!(headers["referrer-policy"], "no-referrer");
        assert_eq!(get(&backends.app(), "/health").await.1, "OK");
    }

    #[tokio::test]
    async fn an_answer_lists_the_woods_with_reasons_and_escapes_everything() {
        let backends = Backends::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/ask/stream"))
            .respond_with(answer(json!([
                recommendation("a", "Cedro"),
                recommendation("b", "Ipê")
            ])))
            .mount(&backends.assistant)
            .await;

        let (status, html) =
            post_question(&backends.app(), "a <script>alert(1)</script> table").await;

        assert_eq!(status, StatusCode::OK);
        assert!(html.contains("A big table"));
        assert!(html.contains("Dimensional stability · essential"));
        assert!(html.contains("Hardness · important"));
        assert!(
            html.contains("Light weight</span>"),
            "weight 1 has no suffix"
        );
        assert!(html.contains("match <strong>91%</strong>"));
        assert!(html.contains("Hardness: 900 kgf"));
        assert!(html.contains("better than 80% of woods"));
        assert!(html.contains(r#"<input type="checkbox" name="id" value="a">"#));
        assert!(html.contains("Compare selected"));
        assert!(html.contains("Ranked among 225 woods"));
        assert!(!html.contains("<script>"), "the question is escaped");
        assert!(html.contains("&#60;script&#62;"));
        assert!(
            !html.contains("<b>and</b>"),
            "the model's text is escaped too"
        );
        assert!(html.contains("Cedro is steady &lt;b&gt;and&lt;/b&gt; easy to work."));
        assert!(html.contains("<h2>In short</h2>"));
        assert!(html.trim_end().ends_with("</html>"), "the page is complete");
    }

    #[tokio::test]
    async fn one_recommendation_cannot_be_compared_and_none_says_so() {
        let backends = Backends::start().await;
        Mock::given(path("/v1/ask/stream"))
            .and(wiremock::matchers::body_string_contains("one"))
            .respond_with(answer(json!([recommendation("a", "Cedro")])))
            .mount(&backends.assistant)
            .await;
        Mock::given(path("/v1/ask/stream"))
            .respond_with(answer(json!([])))
            .mount(&backends.assistant)
            .await;

        let (_, one) = post_question(&backends.app(), "one wood").await;
        let (_, none) = post_question(&backends.app(), "nothing").await;

        assert!(!one.contains("Compare selected") && !one.contains(r#"type="checkbox""#));
        assert!(none.contains("No wood in the database has the figures"));
    }

    #[tokio::test]
    async fn a_bad_question_returns_to_the_form_with_the_text_kept() {
        let backends = Backends::start().await;

        let (status, blank) = post_question(&backends.app(), "   ").await;
        let (_, long) = post_question(&backends.app(), &"x".repeat(MAX_QUESTION_CHARS + 1)).await;

        assert_eq!(status, StatusCode::OK);
        assert!(blank.contains("Please write between 1 and 1000 characters."));
        assert!(long.contains("Please write between 1 and 1000 characters."));
        assert!(long.contains(&"x".repeat(50)), "the text is kept");
        assert!(
            backends
                .assistant
                .received_requests()
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn assistant_trouble_becomes_a_friendly_page_without_internals() {
        let backends = Backends::start().await;
        Mock::given(path("/v1/ask/stream"))
            .and(wiremock::matchers::body_string_contains("starting"))
            .respond_with(
                ResponseTemplate::new(503).set_body_json(
                    json!({"code": "CATALOGUE_NOT_READY", "message": "secret detail"}),
                ),
            )
            .mount(&backends.assistant)
            .await;
        Mock::given(path("/v1/ask/stream"))
            .and(wiremock::matchers::body_string_contains("rude"))
            .respond_with(ResponseTemplate::new(400).set_body_json(
                json!({"code": "INVALID_QUESTION", "message": "the question is empty"}),
            ))
            .mount(&backends.assistant)
            .await;
        Mock::given(path("/v1/ask/stream"))
            .respond_with(ResponseTemplate::new(502).set_body_json(
                json!({"code": "MODEL_UNAVAILABLE", "message": "http://gpu-box:8081 refused"}),
            ))
            .mount(&backends.assistant)
            .await;

        let (starting, html_starting) = post_question(&backends.app(), "starting").await;
        let (rude, html_rude) = post_question(&backends.app(), "rude").await;
        let (broken, html_broken) = post_question(&backends.app(), "other").await;

        assert_eq!(starting, StatusCode::BAD_GATEWAY);
        assert!(html_starting.contains("still starting up"));
        assert!(!html_starting.contains("secret detail"));
        assert_eq!(rude, StatusCode::BAD_GATEWAY);
        assert!(html_rude.contains("the question is empty"));
        assert_eq!(broken, StatusCode::BAD_GATEWAY);
        assert!(html_broken.contains("had a problem answering"));
        assert!(
            !html_broken.contains("gpu-box"),
            "internal addresses stay in the log"
        );
    }

    #[tokio::test]
    async fn an_unreachable_assistant_is_reported_as_such() {
        let backends = Backends::start().await;
        let app = router(AppState {
            api: Arc::new(ApiClient::new(&backends.api.uri())),
            assistant: Arc::new(AssistantClient::new("http://127.0.0.1:1", None)),
        });

        let (status, html) = post_question(&app, "a table").await;

        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert!(html.contains("could not reach the assistant"));
    }

    #[tokio::test]
    async fn the_list_pages_through_the_catalogue() {
        let backends = Backends::start().await;
        let items = |names: &[&str]| -> Vec<Value> {
            names
                .iter()
                .map(|n| wood_json(&format!("id-{n}"), n))
                .collect()
        };
        Mock::given(path("/api/v1/woods"))
            .and(query_param("offset", "0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"items": items(&["Ipê", "Cedro"]), "total": 50, "offset": 0, "limit": 24}),
            ))
            .mount(&backends.api)
            .await;
        Mock::given(path("/api/v1/woods"))
            .and(query_param("offset", "24"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"items": items(&["Pinus"]), "total": 25, "offset": 24, "limit": 24}),
            ))
            .mount(&backends.api)
            .await;
        Mock::given(path("/api/v1/woods"))
            .and(query_param("offset", "100"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"items": [], "total": 25, "offset": 100, "limit": 24})),
            )
            .mount(&backends.api)
            .await;
        let app = backends.app();

        let (_, first, _) = get(&app, "/woods").await;
        let (_, last, _) = get(&app, "/woods?offset=24").await;
        let (_, past, _) = get(&app, "/woods?offset=100").await;

        assert!(first.contains("Showing 1–2 of 50"));
        assert!(
            first.contains(r#"href="/woods?offset=2""#),
            "next starts after what was shown"
        );
        assert!(!first.contains("Previous"));
        assert!(first.contains(r#"href="/woods/id-Ipê""#));
        assert!(first.contains("900 kgf"));
        assert!(last.contains("Showing 25–25 of 25"));
        assert!(last.contains(r#"href="/woods?offset=0""#) && !last.contains("Next"));
        assert!(past.contains("Showing 0–100 of 25") && past.contains("Previous"));
        assert_eq!(
            get(&app, "/woods?offset=-1").await.0,
            StatusCode::BAD_REQUEST
        );
    }

    #[tokio::test]
    async fn a_wood_page_shows_figures_text_and_a_safe_image() {
        let backends = Backends::start().await;
        backends.wood("cedro", "Cedro").await;
        Mock::given(path("/api/v1/woods/gone"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&backends.api)
            .await;
        let app = backends.app();

        let (status, html, _) = get(&app, "/woods/cedro").await;
        let (missing, missing_html, _) = get(&app, "/woods/gone").await;
        let (bad, _, _) = get(&app, "/woods/..%2Fsecrets").await;

        assert_eq!(status, StatusCode::OK);
        assert!(html.contains("<h1>Cedro</h1>"));
        assert!(html.contains("also called Alias"));
        assert!(html.contains(r#"src="https://img.example/x.jpg""#));
        assert!(html.contains("Janka hardness") && html.contains("900 kgf"));
        assert!(html.contains("Furniture") && html.contains("Height"));
        assert_eq!(missing, StatusCode::NOT_FOUND);
        assert!(missing_html.contains("could not find that wood"));
        assert_eq!(bad, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn comparing_puts_the_woods_side_by_side() {
        let backends = Backends::start().await;
        backends.wood("a", "Cedro").await;
        backends.wood("b", "Ipê").await;
        let app = backends.app();

        let (status, html, _) = get(&app, "/compare?id=a&id=b&id=a").await;

        assert_eq!(status, StatusCode::OK);
        assert!(html.contains("Cedro") && html.contains("Ipê"));
        assert!(html.contains("<th>Janka hardness</th><td>900 kgf</td><td>900 kgf</td>"));
        assert_eq!(
            backends.api.received_requests().await.unwrap().len(),
            2,
            "duplicates fetched once"
        );
    }

    #[tokio::test]
    async fn bad_comparisons_are_explained_and_a_missing_wood_is_a_404() {
        let backends = Backends::start().await;
        backends.wood("a", "Cedro").await;
        Mock::given(path("/api/v1/woods/gone"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&backends.api)
            .await;
        let app = backends.app();

        let (none, none_html, _) = get(&app, "/compare").await;
        let (one, _, _) = get(&app, "/compare?id=a").await;
        let (many, many_html, _) = get(&app, "/compare?id=1&id=2&id=3&id=4&id=5").await;
        let (invalid, invalid_html, _) = get(&app, "/compare?id=a&id=../x").await;
        let (gone, _, _) = get(&app, "/compare?id=a&id=gone").await;

        assert_eq!(
            (none, one, many, invalid),
            (
                StatusCode::BAD_REQUEST,
                StatusCode::BAD_REQUEST,
                StatusCode::BAD_REQUEST,
                StatusCode::BAD_REQUEST
            )
        );
        assert!(none_html.contains("Select at least 2 woods"));
        assert!(many_html.contains("at most 4"));
        assert!(invalid_html.contains("not valid"));
        assert_eq!(gone, StatusCode::NOT_FOUND);
    }

    #[test]
    fn compare_ids_reads_only_id_keys_and_dedupes() {
        assert_eq!(compare_ids(Some("id=a&x=1&id=b&id=a")).unwrap(), ["a", "b"]);
        assert_eq!(compare_ids(Some("id=a%2Db&id=c")).unwrap(), ["a-b", "c"]);
        assert!(compare_ids(None).is_err());
        assert!(compare_ids(Some("")).is_err());
        assert!(compare_ids(Some("id=a&id=b&id=c&id=d")).is_ok());
    }

    #[tokio::test]
    async fn unknown_pages_are_a_friendly_404() {
        let backends = Backends::start().await;

        let (status, html, _) = get(&backends.app(), "/nope").await;

        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(html.contains("could not find that page"));
        assert!(html.contains("Error 404"));
    }

    #[test]
    fn every_backend_failure_has_a_visitor_friendly_message() {
        let cases = [
            (BackendError::NotFound, "could not find that wood"),
            (
                BackendError::Malformed {
                    service: "the wood API",
                    detail: "x".into(),
                },
                "had a problem answering",
            ),
            (
                BackendError::Unreachable {
                    service: "the wood API",
                    detail: "x".into(),
                },
                "could not reach the wood API",
            ),
        ];

        for (error, expected) in cases {
            assert!(friendly(&error).contains(expected), "{error}");
        }
    }

    #[tokio::test]
    async fn a_page_that_cannot_render_is_a_500_without_details() {
        let response = PageError::Internal("template exploded".into()).into_response();
        let status = response.status();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let html = String::from_utf8(body.to_vec()).unwrap();

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(html.contains("Something went wrong"));
        assert!(!html.contains("exploded"));
        assert_eq!(
            PageError::from(BackendError::NotFound)
                .into_response()
                .status(),
            StatusCode::NOT_FOUND
        );
    }

    async fn raw_answer(
        app: &Router,
        question: &str,
    ) -> (StatusCode, axum::http::HeaderMap, String) {
        let body: String = form_urlencoded::Serializer::new(String::new())
            .append_pair("question", question)
            .finish();
        let request = Request::post("/ask")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap();
        let (status, html, headers) = send(app, request).await;

        (status, headers, html)
    }

    #[tokio::test]
    async fn the_answer_page_is_html_that_must_not_be_cached_or_buffered() {
        let backends = Backends::start().await;
        Mock::given(path("/v1/ask/stream"))
            .respond_with(answer(json!([recommendation("a", "Cedro")])))
            .mount(&backends.assistant)
            .await;

        let (status, headers, _) = raw_answer(&backends.app(), "a table").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers[header::CONTENT_TYPE], "text/html; charset=utf-8");
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
        assert_eq!(headers["x-accel-buffering"], "no");
        assert!(
            headers.contains_key("content-security-policy"),
            "streamed pages are covered too"
        );
    }

    #[tokio::test]
    async fn a_cut_explanation_ends_with_a_note_and_a_complete_page() {
        let backends = Backends::start().await;
        Mock::given(path("/v1/ask/stream"))
            .respond_with(events(&[
                (
                    "answer",
                    ranking(json!([
                        recommendation("a", "Cedro"),
                        recommendation("b", "Ipê")
                    ])),
                ),
                ("token", json!("Cedro is ")),
                ("failed", json!("the explanation was cut short")),
                ("done", json!({})),
            ]))
            .mount(&backends.assistant)
            .await;

        let (status, _, html) = raw_answer(&backends.app(), "a table").await;

        assert_eq!(status, StatusCode::OK, "the ranking was already shown");
        assert!(html.contains("Cedro is "));
        assert!(html.contains("The explanation was cut short."));
        assert!(html.contains("Ranked among 225 woods"));
        assert!(html.trim_end().ends_with("</html>"));
    }

    #[test]
    fn text_written_by_hand_is_escaped() {
        assert_eq!(
            escape("a & b < c > d \" e ' f"),
            "a &amp; b &lt; c &gt; d &quot; e &#39; f"
        );
        assert_eq!(escape("plain café"), "plain café");
    }
}
