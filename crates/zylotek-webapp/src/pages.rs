//! The pages: ask a question, browse woods, read one, compare several. Rendered on the
//! server; the browser needs no script.

use std::sync::Arc;

use askama::Template;
use axum::{
    Form, Router,
    body::Body,
    extract::{FromRequestParts, Path, Query, RawQuery, State},
    http::{StatusCode, header, request::Parts},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use futures::Stream;
use serde::Deserialize;
use std::convert::Infallible;

use crate::{
    clients::{ApiClient, AssistantClient, LiveAnswer, Piece, is_id},
    error::BackendError,
    i18n::{Lang, Strings},
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
        .fallback(|| async { PageError::NotFound(Subject::Page) })
        .layer(axum::middleware::from_fn(security_headers))
        .layer(axum::middleware::from_fn(localize))
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

// ---- language -------------------------------------------------------------------------

tokio::task_local! {
    /// The language of the request being served, for code that has no request to ask, such as
    /// turning an error into a page.
    static CURRENT: Lang;
}

fn current() -> Lang {
    CURRENT.try_with(|lang| *lang).unwrap_or_default()
}

/// Picks the visitor's language (see [`Lang::detect`]) and remembers an explicit choice.
async fn localize(mut request: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let (lang, chosen) = Lang::detect(request.uri().query(), request.headers());
    request.extensions_mut().insert(lang);
    let mut response = CURRENT.scope(lang, next.run(request)).await;
    let headers = response.headers_mut();
    if let Some(chosen) = chosen
        && let Ok(cookie) = axum::http::HeaderValue::from_str(&chosen.cookie())
    {
        headers.append(header::SET_COOKIE, cookie);
    }
    // The same address answers in the language the request asks for.
    headers.append(
        header::VARY,
        axum::http::HeaderValue::from_static("accept-language, cookie"),
    );

    response
}

impl<S: Send + Sync> FromRequestParts<S> for Lang {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        Ok(parts.extensions.get::<Lang>().copied().unwrap_or_default())
    }
}

// ---- templates ------------------------------------------------------------------------

#[derive(Template)]
#[template(path = "home.html")]
struct HomePage {
    t: &'static Strings,
    question: String,
    error: Option<String>,
}

/// Everything up to the opening of the box the explanation streams into.
#[derive(Template)]
#[template(path = "answer_top.html")]
struct AnswerTop {
    t: &'static Strings,
    question: String,
    summary: String,
    /// "Hardness · essential", one chip each.
    criteria: Vec<String>,
    cards: Vec<CardView>,
    can_compare: bool,
}

/// What closes the page once the explanation is done.
#[derive(Template)]
#[template(path = "answer_bottom.html")]
struct AnswerBottom {
    t: &'static Strings,
    has_explanation_box: bool,
    ranked: String,
}

#[derive(Template)]
#[template(path = "woods.html")]
struct WoodsPage {
    t: &'static Strings,
    showing: String,
    rows: Vec<WoodRow>,
    previous: Option<usize>,
    next: Option<usize>,
}

#[derive(Template)]
#[template(path = "wood.html")]
struct WoodPage {
    t: &'static Strings,
    wood: WoodView,
}

#[derive(Template)]
#[template(path = "compare.html")]
struct ComparePage {
    t: &'static Strings,
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
    t: &'static Strings,
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

/// What could not be found.
#[derive(Clone, Copy, Debug)]
pub enum Subject {
    Page,
    Wood,
}

/// Why a comparison cannot be made; the wording depends on the visitor's language.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompareError {
    InvalidId,
    TooFew,
    TooMany,
}

impl CompareError {
    pub fn message(self, text: &Strings) -> String {
        match self {
            CompareError::InvalidId => text.invalid_wood.to_owned(),
            CompareError::TooFew => text.select_at_least(MIN_COMPARE),
            CompareError::TooMany => text.compare_at_most(MAX_COMPARE),
        }
    }
}

pub enum PageError {
    NotFound(Subject),
    BadRequest(String),
    Backend(BackendError),
    Internal(String),
}

impl From<BackendError> for PageError {
    fn from(error: BackendError) -> Self {
        match error {
            BackendError::NotFound => PageError::NotFound(Subject::Wood),
            other => PageError::Backend(other),
        }
    }
}

impl IntoResponse for PageError {
    fn into_response(self) -> Response {
        let text = current().strings();
        let (status, title, message) = match self {
            PageError::NotFound(what) => (
                StatusCode::NOT_FOUND,
                text.title_not_found,
                text.could_not_find(match what {
                    Subject::Page => text.that_page,
                    Subject::Wood => text.that_wood,
                }),
            ),
            PageError::BadRequest(message) => {
                (StatusCode::BAD_REQUEST, text.title_bad_request, message)
            }
            PageError::Backend(error) => {
                tracing::warn!(%error, "a backend failed");
                (
                    StatusCode::BAD_GATEWAY,
                    text.title_unavailable,
                    friendly(&error, text),
                )
            }
            PageError::Internal(detail) => {
                tracing::error!(%detail, "could not render a page");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    text.title_internal,
                    text.message_internal.to_owned(),
                )
            }
        };
        let page = ErrorPage {
            t: text,
            status: status.as_u16(),
            title,
            message,
        };

        // The template is checked at compile time; an empty body beats a second failure.
        (status, Html(page.render().unwrap_or_default())).into_response()
    }
}

/// What a visitor is told; the technical detail goes to the log only.
fn friendly(error: &BackendError, text: &Strings) -> String {
    match error {
        BackendError::Unreachable { service, .. } => text.unreachable(service),
        BackendError::Status {
            service,
            status: 400,
            message,
        } => text.rejected(service, message),
        BackendError::Status {
            service,
            status: 503,
            ..
        } => text.starting_up(service),
        BackendError::Status { service, .. } | BackendError::Malformed { service, .. } => {
            text.had_a_problem(service)
        }
        BackendError::NotFound => text.could_not_find(text.that_wood),
    }
}

// ---- handlers -------------------------------------------------------------------------

async fn home(lang: Lang) -> Result<Html<String>, PageError> {
    render(HomePage {
        t: lang.strings(),
        question: String::new(),
        error: None,
    })
}

#[derive(Deserialize)]
struct AskForm {
    question: String,
}

async fn ask(
    lang: Lang,
    State(state): State<AppState>,
    Form(form): Form<AskForm>,
) -> Result<Response, PageError> {
    let question = form.question.trim().to_owned();
    if question.is_empty() || question.chars().count() > MAX_QUESTION_CHARS {
        // Back to the form with the text kept, rather than an error page.
        let error = lang.strings().question_length(MAX_QUESTION_CHARS);
        return Ok(render(HomePage {
            t: lang.strings(),
            question: form.question,
            error: Some(error),
        })?
        .into_response());
    }

    // Wait only for the ranking. Anything that goes wrong before it is an ordinary error page;
    // once the page has started, the explanation is a courtesy that may be cut short.
    let live = state.assistant.ask_stream(&question).await?;
    let cards: Vec<CardView> = live
        .answer
        .recommendations
        .iter()
        .map(|r| view::card(r, lang))
        .collect();
    let has_explanation_box = !cards.is_empty();
    let text = lang.strings();
    let top = AnswerTop {
        t: text,
        question,
        summary: live.answer.summary.clone(),
        criteria: live
            .answer
            .criteria
            .iter()
            .map(|c| {
                let label = view::label(c.property, lang);
                match c.weight {
                    3 => format!("{label} · {}", text.essential),
                    2 => format!("{label} · {}", text.important),
                    _ => label.to_owned(),
                }
            })
            .collect(),
        can_compare: cards.len() >= MIN_COMPARE,
        cards,
    }
    .render()
    .map_err(|error| PageError::Internal(error.to_string()))?;
    let bottom = AnswerBottom {
        t: text,
        has_explanation_box,
        ranked: text.ranked_among(live.answer.catalogue_size),
    }
    .render()
    .map_err(|error| PageError::Internal(error.to_string()))?;

    Ok((
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
            (header::HeaderName::from_static("x-accel-buffering"), "no"),
        ],
        Body::from_stream(page_stream(top, bottom, live, text)),
    )
        .into_response())
}

/// The page as it is sent: the top at once, then the explanation piece by piece, then the
/// bottom. A broken explanation ends with a note instead of an error.
fn page_stream(
    top: String,
    bottom: String,
    live: LiveAnswer,
    text: &'static Strings,
) -> impl Stream<Item = Result<String, Infallible>> {
    enum Stage {
        Top(String),
        Words,
        Bottom,
        Over,
    }

    futures::stream::unfold(
        (Stage::Top(top), live, bottom),
        move |(stage, mut live, bottom)| async move {
            let (chunk, next) = match stage {
                Stage::Top(top) => (top, Stage::Words),
                Stage::Words => match live.next_piece().await {
                    Some(Piece::Text(text)) => (escape(&text), Stage::Words),
                    Some(Piece::Cut) => (
                        format!(
                            "\n<p class=\"err\">{}</p>",
                            escape(text.explanation_cut_short)
                        ),
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
    lang: Lang,
    State(state): State<AppState>,
    Query(paging): Query<Paging>,
) -> Result<Html<String>, PageError> {
    let offset = paging.offset.unwrap_or(0);
    let page = state.api.list(offset, PAGE_SIZE).await?;
    let shown = page.items.len();

    let first = if shown == 0 { 0 } else { offset + 1 };

    render(WoodsPage {
        t: lang.strings(),
        showing: lang.strings().showing(first, offset + shown, page.total),
        rows: page.items.iter().map(|w| view::row(w, lang)).collect(),
        previous: (offset > 0).then(|| offset.saturating_sub(PAGE_SIZE)),
        next: ((offset + shown) as u64 > 0 && ((offset + shown) as u64) < page.total)
            .then_some(offset + shown),
    })
}

async fn wood(
    lang: Lang,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Html<String>, PageError> {
    let wood = state.api.get(&id).await?;

    render(WoodPage {
        t: lang.strings(),
        wood: view::detail(&wood, lang),
    })
}

/// The `id` values of a query string: distinct, well-formed, between 2 and 4 of them.
pub fn compare_ids(query: Option<&str>) -> Result<Vec<String>, CompareError> {
    let mut ids: Vec<String> = Vec::new();
    for (key, value) in form_urlencoded::parse(query.unwrap_or_default().as_bytes()) {
        if key != "id" {
            continue;
        }
        if !is_id(&value) {
            return Err(CompareError::InvalidId);
        }
        if !ids.iter().any(|seen| *seen == value) {
            ids.push(value.into_owned());
        }
    }
    if ids.len() < MIN_COMPARE {
        return Err(CompareError::TooFew);
    }
    if ids.len() > MAX_COMPARE {
        return Err(CompareError::TooMany);
    }

    Ok(ids)
}

async fn compare(
    lang: Lang,
    State(state): State<AppState>,
    RawQuery(query): RawQuery,
) -> Result<Html<String>, PageError> {
    let ids = compare_ids(query.as_deref())
        .map_err(|error| PageError::BadRequest(error.message(lang.strings())))?;
    let mut woods = Vec::with_capacity(ids.len());
    for id in &ids {
        woods.push(state.api.get(id).await?);
    }
    let columns: Vec<Vec<Fact>> = woods.iter().map(|w| view::facts(w, lang)).collect();
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
        t: lang.strings(),
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
            assert!(
                friendly(&error, Lang::En.strings()).contains(expected),
                "{error}"
            );
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
    async fn get_with(
        app: &Router,
        uri: &str,
        headers: &[(&str, &str)],
    ) -> (StatusCode, String, axum::http::HeaderMap) {
        let mut request = Request::get(uri);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }

        send(app, request.body(Body::empty()).unwrap()).await
    }

    #[tokio::test]
    async fn the_browser_language_picks_the_translation_without_setting_a_cookie() {
        let backends = Backends::start().await;
        let app = backends.app();

        let (_, portuguese, headers) =
            get_with(&app, "/", &[("accept-language", "pt-BR,pt;q=0.9,en;q=0.8")]).await;
        let (_, english, _) = get(&app, "/").await;
        let (_, other, _) = get_with(&app, "/", &[("accept-language", "fr-FR")]).await;

        assert!(portuguese.contains("<html lang=\"pt-BR\">"));
        assert!(portuguese.contains("Qual madeira devo usar?"));
        assert!(portuguese.contains("Buscar madeiras"));
        assert!(!portuguese.contains("Which wood"));
        assert!(!headers.contains_key(header::SET_COOKIE));
        assert!(
            headers[header::VARY]
                .to_str()
                .unwrap()
                .contains("accept-language")
        );
        assert!(english.contains("<html lang=\"en\">") && english.contains("Which wood"));
        assert_eq!(
            other, english,
            "an unsupported language falls back to English"
        );
    }

    #[tokio::test]
    async fn the_lang_parameter_wins_and_is_remembered_in_a_cookie() {
        let backends = Backends::start().await;
        let app = backends.app();

        let (_, html, headers) = get_with(&app, "/?lang=pt-br", &[("accept-language", "en")]).await;
        let (_, back, english_headers) = get_with(
            &app,
            "/",
            &[("accept-language", "en"), ("cookie", "lang=pt-br")],
        )
        .await;
        let (_, switched, switched_headers) =
            get_with(&app, "/?lang=en", &[("cookie", "lang=pt-br")]).await;

        assert!(html.contains("Qual madeira devo usar?"));
        let cookie = headers[header::SET_COOKIE].to_str().unwrap();
        assert!(cookie.starts_with("lang=pt-br;") && cookie.contains("SameSite=Lax"));
        assert!(
            back.contains("Qual madeira devo usar?"),
            "the cookie beats the browser"
        );
        assert!(!english_headers.contains_key(header::SET_COOKIE));
        assert!(switched.contains("Which wood should I use?"));
        assert!(
            switched_headers[header::SET_COOKIE]
                .to_str()
                .unwrap()
                .starts_with("lang=en;")
        );
    }

    #[tokio::test]
    async fn every_page_offers_both_languages() {
        let backends = Backends::start().await;

        let (_, html, _) = get(&backends.app(), "/").await;

        assert!(html.contains("href=\"/?lang=en\"") && html.contains("href=\"/?lang=pt-br\""));
    }

    #[tokio::test]
    async fn errors_are_shown_in_the_visitors_language() {
        let backends = Backends::start().await;
        let app = backends.app();
        let pt = [("accept-language", "pt")];

        let (status, missing, _) = get_with(&app, "/nope", &pt).await;
        let (bad, few, _) = get_with(&app, "/compare?id=a", &pt).await;
        let (_, invalid, _) = get_with(&app, "/compare?id=a&id=%25", &pt).await;
        let (_, many, _) = get_with(&app, "/compare?id=a&id=b&id=c&id=d&id=e", &pt).await;

        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(missing.contains("Não encontramos essa página."));
        assert!(missing.contains("Erro 404") && missing.contains("Voltar ao início"));
        assert_eq!(bad, StatusCode::BAD_REQUEST);
        assert!(few.contains("Selecione pelo menos 2 madeiras para comparar."));
        assert!(invalid.contains("não é válida"));
        assert!(many.contains("Compare no máximo 4 madeiras por vez."));
    }

    #[tokio::test]
    async fn a_backend_failure_page_is_translated_too() {
        let backends = Backends::start().await;

        let (status, html, _) =
            get_with(&backends.app(), "/woods", &[("accept-language", "pt-BR")]).await;

        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert!(html.contains("Serviço indisponível"));
        assert!(html.contains("Tente novamente"));
    }

    #[tokio::test]
    async fn a_wood_page_and_a_comparison_are_translated_with_decimal_commas() {
        let backends = Backends::start().await;
        let (a, b) = (
            "0b9c4f1e-aaaa-4bbb-8ccc-123456789abc",
            "1b9c4f1e-aaaa-4bbb-8ccc-123456789abc",
        );
        backends.wood(a, "Cedro").await;
        backends.wood(b, "Ipê").await;
        let app = backends.app();
        let pt = [("accept-language", "pt-BR")];

        let (_, wood, _) = get_with(&app, &format!("/woods/{a}"), &pt).await;
        let (_, compare, _) = get_with(&app, &format!("/compare?id={a}&id={b}"), &pt).await;

        assert!(wood.contains("Principais características"));
        assert!(wood.contains("Dureza Janka") && wood.contains("900 kgf"));
        assert!(wood.contains("Usos típicos") && wood.contains("também chamada de Alias"));
        assert!(wood.contains("← Todas as madeiras"));
        assert!(compare.contains("Comparar madeiras"));
        assert!(compare.contains("Retração volumétrica"));
        assert!(compare.contains("Maior dureza, resistência e rigidez"));
    }

    #[tokio::test]
    async fn the_catalogue_page_is_translated() {
        let backends = Backends::start().await;
        Mock::given(path("/api/v1/woods"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "items": [wood_json("0b9c4f1e-aaaa-4bbb-8ccc-123456789abc", "Cedro")],
                "total": 30, "limit": 24, "offset": 0,
            })))
            .mount(&backends.api)
            .await;

        let (_, html, _) = get_with(&backends.app(), "/woods", &[("accept-language", "pt")]).await;

        assert!(html.contains("Madeiras brasileiras"));
        assert!(html.contains("Mostrando 1–1 de 30."));
        assert!(html.contains("Nome científico") && html.contains("Família"));
        assert!(html.contains("aria-label=\"Comparar Cedro\""));
        assert!(html.contains("Próxima →"));
    }

    #[tokio::test]
    async fn the_answer_page_is_translated_around_the_explanation() {
        let backends = Backends::start().await;
        Mock::given(path("/v1/ask/stream"))
            .respond_with(answer(json!([
                recommendation("a", "Cedro"),
                recommendation("b", "Ipê")
            ])))
            .mount(&backends.assistant)
            .await;
        let body: String = form_urlencoded::Serializer::new(String::new())
            .append_pair("question", "uma mesa")
            .finish();
        let request = Request::post("/ask")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header("accept-language", "pt-BR")
            .body(Body::from(body))
            .unwrap();

        let (status, html, _) = send(&backends.app(), request).await;

        assert_eq!(status, StatusCode::OK);
        assert!(html.contains("<html lang=\"pt-BR\">"));
        assert!(html.contains("Madeiras para o seu projeto"));
        assert!(html.contains("Entendemos:"));
        assert!(html.contains("Estabilidade dimensional · essencial"));
        assert!(html.contains("Dureza · importante"));
        assert!(html.contains("compatibilidade"));
        assert!(html.contains("Ver detalhes"));
        assert!(html.contains("Comparar selecionadas (2 a 4)"));
        assert!(html.contains("Em resumo"));
        assert!(html.contains("Classificadas entre 225 madeiras."));
        assert!(html.contains("Fazer outra pergunta"));
        assert!(html.contains("melhor que"));
    }

    #[tokio::test]
    async fn a_bad_question_is_answered_in_the_visitors_language() {
        let backends = Backends::start().await;
        let request = Request::post("/ask")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header("cookie", "lang=pt-br")
            .body(Body::from("question=+"))
            .unwrap();

        let (_, html, _) = send(&backends.app(), request).await;

        assert!(html.contains("Escreva entre 1 e 1000 caracteres."));
    }
}
