//! The pages: ask a question, browse woods, read one, compare several. Rendered on the
//! server; the browser needs no script, and the scripts there are only add the history sidebar
//! and the comparison tray.
//!
//! Asking opens an ask that is answered in the background (`asks.rs`) and sends the visitor to
//! its page, `/asks/{id}`. While this process is answering it, that page streams: a thinking
//! panel at once, the ranking when it is ready, then the explanation as it is written, and CSS
//! `:has()` hides each progress note once what it waits for has arrived. Later, or from another
//! process, the page shows what is stored, and refreshes itself until the ask is finished.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use askama::Template;
use axum::{
    Form, Router,
    body::Body,
    extract::{FromRequestParts, Path, Query, RawQuery, Request, State},
    http::{StatusCode, header, request::Parts},
    middleware::Next,
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use futures::Stream;
use serde::Deserialize;
use std::convert::Infallible;
use tokio::sync::watch;
use tower::limit::ConcurrencyLimitLayer;
use tower_http::{
    compression::CompressionLayer, limit::RequestBodyLimitLayer, timeout::TimeoutLayer,
    trace::TraceLayer,
};
use zylotek_contracts::{
    asks::{AskRecordResponse, AskStateSchema},
    assistant::AskResponse,
};

use crate::{
    asks::{Asks, Snapshot},
    browse::{Entry, Query as BrowseQuery},
    clients::{ApiClient, is_id},
    error::BackendError,
    i18n::{Lang, Strings},
    view::{self, BrowseView, CardView, CompareColumn, CompareRow, WoodView},
};

pub const MIN_COMPARE: usize = 2;
pub const MAX_COMPARE: usize = 4;
const MAX_QUESTION_CHARS: usize = 1000;

#[derive(Clone)]
pub struct AppState {
    pub api: Arc<ApiClient>,
    pub asks: Arc<Asks>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(home))
        .route("/ask", post(ask))
        .route("/asks/{id}", get(ask_page))
        .route("/woods", get(woods))
        .route("/woods/{id}", get(wood))
        .route("/compare", get(compare))
        .route("/static/history.js", get(history_script))
        .route("/static/browse.js", get(browse_script))
        .route("/static/app.css", get(stylesheet))
        .route("/health", get(|| async { "OK" }))
        .fallback(|| async { PageError::NotFound(Subject::Page) })
        .layer(axum::middleware::from_fn(logging))
        .layer(axum::middleware::from_fn(security_headers))
        .layer(axum::middleware::from_fn(localize))
        .layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new())
        .layer(TimeoutLayer::with_status_code(
            axum::http::StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(5),
        ))
        .layer(RequestBodyLimitLayer::new(1024 * 1024)) // 1MB
        .layer(ConcurrencyLimitLayer::new(100)) // 100 concurrent requests
        .with_state(state)
}

pub async fn logging(req: Request, next: Next) -> Response {
    let time = Instant::now();
    let method = req.method().clone();
    let uri = req.uri().clone();
    let response = next.run(req).await;
    log::info!(
        "{} {} -> {} ({:?} ms)",
        method,
        uri,
        response.status(),
        time.elapsed().as_millis()
    );
    response
}

/// Scripts from this site only (the ask history; no inline script), framing by this site only
/// (the wood drawer on the answer page), images only over HTTPS: the pages need nothing more.
async fn security_headers(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    for (name, value) in [
        (
            "content-security-policy",
            "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src https: data:; frame-src 'self'; form-action 'self'; base-uri 'none'; frame-ancestors 'self'",
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

/// What is sent at once, before the assistant has answered: the question and the thinking panel.
#[derive(Template)]
#[template(path = "answer_shell.html")]
struct AnswerShell {
    t: &'static Strings,
    id: String,
    question: String,
    /// Reload every few seconds: the ask is being answered somewhere this page cannot follow.
    refresh: bool,
}

/// The ranking, up to the opening of the box the explanation streams into.
#[derive(Template)]
#[template(path = "answer_top.html")]
struct AnswerTop {
    t: &'static Strings,
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
    /// The explanation is over; the page marks itself done so the writing note goes away.
    finished: bool,
}

/// What closes a page whose ask has not been ranked yet.
#[derive(Template)]
#[template(path = "answer_pending.html")]
struct AnswerPending {
    t: &'static Strings,
}

/// What closes the page when the assistant could not answer.
#[derive(Template)]
#[template(path = "answer_failed.html")]
struct AnswerFailed {
    t: &'static Strings,
    message: String,
}

#[derive(Template)]
#[template(path = "woods.html")]
struct WoodsPage {
    t: &'static Strings,
    v: BrowseView,
}

#[derive(Template)]
#[template(path = "wood.html")]
struct WoodPage {
    t: &'static Strings,
    wood: WoodView,
}

/// A wood without the site around it, for the drawer on the answer page.
#[derive(Template)]
#[template(path = "wood_embed.html")]
struct WoodEmbed {
    t: &'static Strings,
    wood: WoodView,
}

#[derive(Template)]
#[template(path = "compare.html")]
struct ComparePage {
    t: &'static Strings,
    columns: Vec<CompareColumn>,
    rows: Vec<CompareRow>,
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
    Ask,
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
                    Subject::Ask => text.that_ask,
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
pub(crate) fn friendly(error: &BackendError, text: &Strings) -> String {
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

    let id = state.asks.open(&question, lang).await?;

    // See Other: reloading the answer page then reads it again instead of asking again.
    Ok(Redirect::to(&format!("/asks/{id}")).into_response())
}

/// An unfinished ask whose record has not moved for this long is shown as interrupted: it is
/// longer than the assistant is given to answer, so whatever was answering it has stopped.
const STALE_AFTER_MS: u64 = 5 * 60 * 1000;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

/// One ask's page. Followed live while this process answers it; otherwise what is stored.
async fn ask_page(
    lang: Lang,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, PageError> {
    if !is_id(&id) {
        return Err(PageError::NotFound(Subject::Ask));
    }
    let text = lang.strings();
    if let Some(live) = state.asks.follow(&id) {
        let question = live.borrow().question.clone();
        let shell = shell(text, &id, question, false)?;

        return Ok((
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                (header::CACHE_CONTROL, "no-store"),
                (header::HeaderName::from_static("x-accel-buffering"), "no"),
            ],
            Body::from_stream(follow(shell, live, lang)),
        )
            .into_response());
    }

    let record = state.asks.get(&id).await.map_err(|error| match error {
        BackendError::NotFound => PageError::NotFound(Subject::Ask),
        other => PageError::Backend(other),
    })?;

    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Html(saved(&record, lang, now_ms())?),
    )
        .into_response())
}

fn shell(
    text: &'static Strings,
    id: &str,
    question: String,
    refresh: bool,
) -> Result<String, PageError> {
    AnswerShell {
        t: text,
        id: id.to_owned(),
        question,
        refresh,
    }
    .render()
    .map_err(|error| PageError::Internal(error.to_string()))
}

/// A stored ask, as one page.
fn saved(record: &AskRecordResponse, lang: Lang, now: u64) -> Result<String, PageError> {
    let text = lang.strings();
    let unfinished = !record.state.is_final();
    let stale = unfinished && now.saturating_sub(record.updated_at) > STALE_AFTER_MS;
    let waiting = unfinished && !stale;
    let mut page = shell(text, &record.id, record.question.clone(), waiting)?;

    match &record.answer {
        Some(answer) => {
            let (top, bottom) = results(answer, lang, !waiting);
            page.push_str(&top);
            page.push_str(&escape(&record.explanation));
            let note = if stale {
                Some(text.ask_interrupted.to_owned())
            } else if record.state == AskStateSchema::Failed {
                record.failure.clone()
            } else if record.state == AskStateSchema::Answered && !record.explanation_complete {
                Some(text.explanation_cut_short.to_owned())
            } else {
                None
            };
            if let Some(note) = note {
                page.push_str(&note_html(&note));
            }
            page.push_str(&bottom);
        }
        None if waiting => page.push_str(&AnswerPending { t: text }.render().unwrap_or_default()),
        None => {
            let reason = match (&record.failure, stale) {
                (Some(reason), false) => reason.clone(),
                _ => text.ask_interrupted.to_owned(),
            };
            page.push_str(&failure(text, reason));
        }
    }

    Ok(page)
}

fn note_html(note: &str) -> String {
    format!("\n<p class=\"text-danger\">{}</p>", escape(note))
}

/// The ranking and the end of the page.
fn results(answer: &AskResponse, lang: Lang, finished: bool) -> (String, String) {
    let text = lang.strings();
    let cards: Vec<CardView> = answer
        .recommendations
        .iter()
        .map(|r| view::card(r, lang))
        .collect();
    let has_explanation_box = !cards.is_empty();
    let top = AnswerTop {
        t: text,
        summary: answer.summary.clone(),
        criteria: answer
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
    };
    let bottom = AnswerBottom {
        t: text,
        has_explanation_box,
        ranked: text.ranked_among(answer.catalogue_size),
        finished,
    };

    // The templates are checked at compile time; a streamed page has already started, so
    // there is no error status left to give.
    (
        top.render().unwrap_or_default(),
        bottom.render().unwrap_or_default(),
    )
}

/// A page following an ask while it is answered: the shell at once, then the ranking (or why
/// there is none), then the explanation as it grows, then the end.
fn follow(
    shell: String,
    live: watch::Receiver<Snapshot>,
    lang: Lang,
) -> impl Stream<Item = Result<String, Infallible>> {
    enum Stage {
        Shell(String),
        Ranking,
        Words { sent: usize, bottom: String },
        Over,
    }
    let text = lang.strings();

    futures::stream::unfold(
        (Stage::Shell(shell), live),
        move |(stage, mut live)| async move {
            let (chunk, next) = match stage {
                Stage::Shell(shell) => (shell, Stage::Ranking),
                Stage::Ranking => loop {
                    let (answer, failed) = {
                        let now = live.borrow_and_update();
                        (now.answer.clone(), now.failure.clone())
                    };
                    if let Some(answer) = answer {
                        let (top, bottom) = results(&answer, lang, true);
                        break (top, Stage::Words { sent: 0, bottom });
                    }
                    if let Some(reason) = failed {
                        break (failure(text, reason), Stage::Over);
                    }
                    if live.changed().await.is_err() {
                        break (failure(text, text.ask_interrupted.to_owned()), Stage::Over);
                    }
                },
                Stage::Words { sent, bottom } => loop {
                    let (more, over, complete) = {
                        let now = live.borrow_and_update();
                        (
                            now.explanation.get(sent..).unwrap_or_default().to_owned(),
                            now.state.is_final(),
                            now.complete,
                        )
                    };
                    if !more.is_empty() {
                        let sent = sent + more.len();
                        break (escape(&more), Stage::Words { sent, bottom });
                    }
                    if over {
                        let note = if complete {
                            String::new()
                        } else {
                            note_html(text.explanation_cut_short)
                        };
                        break (note + &bottom, Stage::Over);
                    }
                    if live.changed().await.is_err() {
                        break (bottom, Stage::Over);
                    }
                },
                Stage::Over => return None,
            };

            Some((Ok(chunk), (next, live)))
        },
    )
}

/// The end of a page whose ask could not be answered.
fn failure(text: &'static Strings, message: String) -> String {
    AnswerFailed { t: text, message }
        .render()
        .unwrap_or_default()
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

async fn woods(
    lang: Lang,
    State(state): State<AppState>,
    RawQuery(raw): RawQuery,
) -> Result<Html<String>, PageError> {
    let all = state.api.all().await?;
    let entries: Vec<Entry> = all.iter().map(Entry::new).collect();
    let query = BrowseQuery::parse(raw.as_deref());
    let results = query.search(&entries);

    render(WoodsPage {
        t: lang.strings(),
        v: view::browse(entries.len(), &query, &results, lang),
    })
}

/// The site's scripts are small enhancements served from its own address; the CSP allows no
/// other. Saves finished answers in the browser and lists them.
async fn history_script() -> Response {
    script(include_str!("../static/history.js"))
}

/// Keeps the woods picked for comparison across pages, and applies filters as they are ticked.
async fn browse_script() -> Response {
    script(include_str!("../static/browse.js"))
}

/// The styles, built from `assets/tailwind.css` by `npm run build` (`make css`) and committed so
/// that cargo alone runs the site. Inline `style` attributes remain for bar widths and delays.
async fn stylesheet() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        include_str!("../static/app.css"),
    )
        .into_response()
}

fn script(source: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        source,
    )
        .into_response()
}

#[derive(Deserialize)]
struct WoodQuery {
    /// Any value: the page goes into the answer page's drawer, without the site around it.
    embed: Option<String>,
}

async fn wood(
    lang: Lang,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<WoodQuery>,
) -> Result<Html<String>, PageError> {
    let wood = state.api.get(&id).await?;
    let (t, wood) = (lang.strings(), view::detail(&wood, lang));

    match query.embed {
        Some(_) => render(WoodEmbed { t, wood }),
        None => render(WoodPage { t, wood }),
    }
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
    let (columns, rows) = view::comparison(&woods, woods.len() > MIN_COMPARE, lang);

    render(ComparePage {
        t: lang.strings(),
        columns,
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
        matchers::{method, path},
    };

    use super::*;
    use crate::{
        asks::testing::{MemoryStore, record},
        clients::AssistantClient,
    };

    struct Backends {
        api: MockServer,
        assistant: MockServer,
        /// Where the asks go; shared by every app built from these backends.
        store: Arc<MemoryStore>,
    }

    impl Backends {
        async fn start() -> Self {
            Self {
                api: MockServer::start().await,
                assistant: MockServer::start().await,
                store: Arc::new(MemoryStore::default()),
            }
        }

        fn app(&self) -> Router {
            self.app_asking(&self.assistant.uri())
        }

        fn app_asking(&self, assistant: &str) -> Router {
            router(AppState {
                api: Arc::new(ApiClient::new(&self.api.uri())),
                asks: Arc::new(Asks::new(
                    self.store.clone(),
                    Arc::new(AssistantClient::new(assistant, None)),
                )),
            })
        }

        /// The API's whole catalogue, in one page.
        async fn catalogue(&self, woods: &[zylotek_contracts::woods::WoodResponse]) {
            Mock::given(path("/api/v1/woods"))
                .respond_with(ResponseTemplate::new(200).set_body_json(
                    json!({"items": woods, "total": woods.len(), "offset": 0, "limit": 100}),
                ))
                .mount(&self.api)
                .await;
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

    /// Asks, and follows the redirect to the ask's page as a browser would; a question the
    /// form refuses comes back as the form.
    async fn post_question(app: &Router, question: &str) -> (StatusCode, String) {
        let (status, headers, html) = raw_answer(app, question).await;
        let _ = headers;

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
        assert!(html.contains(r#"action="/ask" class="ask mt-4">"#));
        assert!(html.contains("<textarea"));
        assert!(
            html.contains(r#"type="submit" name="question" value="I need a stable wood for a wide kitchen countertop that won&#39;t warp.">"#),
            "an example is asked in one click"
        );
        assert!(html.contains(r#"href="/woods""#));
        assert!(
            headers["content-security-policy"]
                .to_str()
                .unwrap()
                .contains("default-src 'none'")
        );
        let policy = headers["content-security-policy"].to_str().unwrap();
        assert!(policy.contains("frame-src 'self'") && policy.contains("frame-ancestors 'self'"));
        assert!(
            policy.contains("script-src 'self';"),
            "this site's own script file only, nothing inline"
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
    async fn the_answer_page_thinks_first_then_ranks_then_writes() {
        let backends = Backends::start().await;
        Mock::given(path("/v1/ask/stream"))
            .respond_with(answer(json!([recommendation("a", "Cedro")])))
            .mount(&backends.assistant)
            .await;

        let (_, html) = post_question(&backends.app(), "a table").await;

        let at = |needle: &str| html.find(needle).unwrap_or_else(|| panic!("no {needle}"));
        assert!(html.contains("Reading your project"));
        assert!(at(r#"class="thinking card "#) < at(r#"class="results""#));
        assert!(at(r#"class="results""#) < at(r#"class="writing text-muted "#));
        assert!(at("Cedro is steady") < at(r#"class="done "#));
    }

    #[tokio::test]
    async fn a_suggested_wood_opens_in_a_drawer_over_the_answer() {
        let backends = Backends::start().await;
        Mock::given(path("/v1/ask/stream"))
            .respond_with(answer(json!([recommendation("a", "Cedro")])))
            .mount(&backends.assistant)
            .await;

        let (_, html) = post_question(&backends.app(), "a table").await;

        assert!(html.contains(r##"href="#peek-a""##));
        assert!(html.contains(r#"id="peek-a" aria-label="Cedro""#));
        assert!(html.contains(r#"src="/woods/a?embed=1" loading="lazy""#));
        assert!(
            html.contains(r##"class="close ml-auto" href="#card-a">"##),
            "closing returns to the card"
        );
        assert!(html.contains(r#"id="card-a""#));
        assert!(html.contains(r#"<a href="/woods/a" target="_blank" rel="noopener">"#));
    }

    #[tokio::test]
    async fn the_history_sidebar_and_its_script_are_served_in_the_visitors_language() {
        let backends = Backends::start().await;

        let (_, page, _) = get(&backends.app(), "/").await;
        let (_, portuguese, _) =
            get_with(&backends.app(), "/", &[("accept-language", "pt-BR")]).await;
        let (script_status, script, headers) = get(&backends.app(), "/static/history.js").await;
        let (gone, _, _) = get(&backends.app(), "/history").await;

        assert!(
            page.contains(r#"id="sidebar" data-sidebar hidden"#),
            "the script shows it; without one there is no history to list"
        );
        assert!(page.contains("+ New question") && page.contains("data-history-list"));
        assert!(page.contains("data-sidebar-toggle") && !page.contains("data-history-answer"));
        assert!(portuguese.contains("Nova pergunta") && portuguese.contains("Limpar histórico"));
        assert_eq!(gone, StatusCode::NOT_FOUND, "no separate history page");
        let (css_status, css, css_headers) = get(&backends.app(), "/static/app.css").await;
        assert_eq!(css_status, StatusCode::OK);
        assert_eq!(css_headers[header::CONTENT_TYPE], "text/css; charset=utf-8");
        assert!(
            css.contains(".btn") && css.contains("border-l-tag-hardness"),
            "the built stylesheet has the shared shapes and the tag colours; run `make css`"
        );
        let (browse_status, browse_script, _) = get(&backends.app(), "/static/browse.js").await;
        assert_eq!(browse_status, StatusCode::OK);
        assert!(browse_script.contains("zylotek.compare.v1"));
        assert_eq!(script_status, StatusCode::OK);
        assert_eq!(
            headers[header::CONTENT_TYPE],
            "text/javascript; charset=utf-8"
        );
        assert!(script.contains("zylotek.history.v1"));
    }

    #[tokio::test]
    async fn every_page_loads_the_history_script_and_answers_carry_their_question() {
        let backends = Backends::start().await;
        Mock::given(path("/v1/ask/stream"))
            .respond_with(answer(json!([recommendation("a", "Cedro")])))
            .mount(&backends.assistant)
            .await;

        let (_, home, _) = get(&backends.app(), "/").await;
        let (_, html) = post_question(&backends.app(), r#"a "big" table"#).await;

        assert!(home.contains(r#"<script src="/static/history.js" defer></script>"#));
        assert!(
            html.contains(
                r#"data-question="a &#34;big&#34; table">
<script src="/static/history.js"></script>"#
            ),
            "on an ask's page the history runs at once, so leaving early still remembers the ask"
        );
        assert!(html.contains(r#"data-question="a &#34;big&#34; table">"#));
        assert!(
            html.contains("data-sidebar"),
            "the answer page has the sidebar too"
        );
        backends.wood("a", "Cedro").await;
        backends.catalogue(&[]).await;
        let app = backends.app();
        for page in ["/woods", "/woods/a", "/compare?id=a&id=a2", "/nowhere"] {
            let (_, other, _) = get(&app, page).await;
            assert!(
                other.contains("data-sidebar-toggle") && other.contains("data-sidebar hidden"),
                "{page} has the hamburger and the sidebar it opens"
            );
        }
        let (_, embedded, _) = get(&app, "/woods/a?embed=1").await;
        assert!(
            !embedded.contains("data-sidebar"),
            "the drawer's page has neither"
        );
    }

    #[tokio::test]
    async fn an_embedded_wood_page_has_no_site_around_it() {
        let backends = Backends::start().await;
        backends.wood("a", "Cedro").await;

        let (status, embedded, _) = get(&backends.app(), "/woods/a?embed=1").await;
        let (_, full, _) = get(&backends.app(), "/woods/a").await;

        assert_eq!(status, StatusCode::OK);
        assert!(embedded.contains("<h1>Cedro</h1>") && embedded.contains(r#"<body class="embed "#));
        assert!(!embedded.contains("<header") && !embedded.contains("All woods"));
        assert!(full.contains("<header") && full.contains("All woods"));
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
    async fn assistant_trouble_ends_the_started_page_with_a_friendly_note_without_internals() {
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

        // The thinking panel was already sent, so the status cannot change any more.
        assert_eq!([starting, rude, broken], [StatusCode::OK; 3]);
        assert!(html_starting.contains("still starting up"));
        assert!(!html_starting.contains("secret detail"));
        assert!(html_rude.contains("the question is empty"));
        assert!(html_broken.contains("had a problem answering"));
        assert!(
            html_broken.contains("results card failed"),
            "it hides the panel"
        );
        assert!(
            html_broken.trim_end().ends_with("</html>"),
            "the page is complete"
        );
        assert!(
            !html_broken.contains("gpu-box"),
            "internal addresses stay in the log"
        );
    }

    #[tokio::test]
    async fn an_unreachable_assistant_is_reported_as_such() {
        let backends = Backends::start().await;
        let app = backends.app_asking("http://127.0.0.1:1");

        let (status, html) = post_question(&app, "a table").await;

        assert_eq!(status, StatusCode::OK);
        assert!(html.contains("could not reach the assistant"));
    }

    #[tokio::test]
    async fn browsing_shows_cards_with_tags_filters_and_counts() {
        let backends = Backends::start().await;
        backends
            .catalogue(&crate::browse::testing::catalogue())
            .await;

        let (status, html, _) = get(&backends.app(), "/woods").await;

        assert_eq!(status, StatusCode::OK);
        assert!(html.contains("5 woods"));
        assert_eq!(html.matches("data-card>").count(), 5);
        assert!(html.contains(r#"href="/woods/id-ipê""#));
        assert!(
            html.contains(
                r#"<a class="tag border-l-tag-hardness" data-tag="hardness" href="/woods?hardness=very-hard">Very hard</a>"#
            )
        );
        assert!(html.contains(r#"<a class="tag border-l-tag-color" data-tag="color" href="/woods?color=marrom">Brown</a>"#));
        assert!(html.contains(
            r#"<a class="tag border-l-tag-region" data-tag="region" href="/woods?region=PA">PA</a>"#
        ));
        assert!(html.contains(
            r#"name="color" value="marrom"> <span class="flex-1">Brown</span> <em class="text-sm text-muted not-italic">3</em>"#
        ));
        assert!(html.contains(r#"<input type="checkbox" name="family" value="Meliaceae">"#));
        assert!(html.contains("<option value=\"name\" selected>Name A–Z</option>"));
        assert!(
            !html.contains(r#"value="relevance""#),
            "no search, no relevance"
        );
        assert!(html.contains(r#"data-compare data-name="Ipê""#));
        assert!(html.contains("1800 kgf") && html.contains("No photo"));
        assert!(html.contains(r#"src="https://img.example/ipe.jpg""#));
        assert!(html.contains("Page 1 of 1") && !html.contains("Clear all"));
        assert!(html.contains(r#"href="/woods" aria-current="true">Cards</a>"#));
        assert!(html.contains(r#"href="/woods?view=list">List</a>"#));
    }

    #[tokio::test]
    async fn searching_and_filtering_show_the_chips_that_undo_them() {
        let backends = Backends::start().await;
        backends
            .catalogue(&crate::browse::testing::catalogue())
            .await;
        let app = backends.app();

        let (_, one, _) = get(&app, "/woods?q=ipe&hardness=very-hard").await;
        let (_, none, _) = get(&app, "/woods?q=zzzz").await;

        assert!(one.contains("1 of 5 woods"));
        assert_eq!(one.matches("data-card>").count(), 1);
        assert!(one.contains(
            r#"<a class="chip" href="/woods?hardness=very-hard" title="Clear all">“ipe” ✕</a>"#
        ));
        assert!(one.contains("Very hard ✕"));
        assert!(one.contains("Clear all"));
        assert!(one.contains("<option value=\"relevance\" selected>Best match</option>"));
        assert!(none.contains("No wood matches this search.") && none.contains("Try fewer words"));
        assert!(none.contains("0 of 5 woods"));
    }

    #[tokio::test]
    async fn every_kind_of_filter_has_a_chip_and_the_list_view_sorts_by_its_columns() {
        let backends = Backends::start().await;
        backends
            .catalogue(&crate::browse::testing::catalogue())
            .await;
        let query = "q=ipe&family=Bignoniaceae&color=marrom&region=PA&hardness=very-hard&density=heavy\
                     &stability=moderate&hardness_min=1000&hardness_max=2000&density_min=0.5&density_max=1.2\
                     &image=1&view=list&sort=hardest";

        let (_, html, _) = get(&backends.app(), &format!("/woods?{query}")).await;

        for chip in [
            "Bignoniaceae ✕",
            "Brown ✕",
            "PA ✕",
            "Very hard ✕",
            "Heavy ✕",
            "Moderately stable ✕",
            "Janka: 1000–2000 ✕",
            "Density: 0.5–1.2 ✕",
            "Only woods with a photo ✕",
        ] {
            assert!(html.contains(chip), "{chip}");
        }
        assert!(html.contains(r#"<table class="woods-list data-table">"#));
        assert!(html.contains(r#"<input type="hidden" name="view" value="list">"#));
        assert!(html.contains(r#"name="hardness_min" min="0" step="any" value="1000""#));
        assert!(html.contains("Hardness ▼"), "the active sort is marked");
        assert!(
            html.contains("sort=softest"),
            "and a second click reverses it"
        );
        assert!(html.contains("sort=heaviest") && html.contains("sort=name"));
        assert!(html.contains("sort=stablest"));
        assert!(html.contains(r#"<option value="hardest" selected>"#));
    }

    #[tokio::test]
    async fn browsing_is_paged_and_a_page_past_the_end_shows_the_last() {
        let backends = Backends::start().await;
        let woods: Vec<_> = (0..30)
            .map(|n| {
                crate::browse::testing::wood(
                    &format!("W{n:02}"),
                    "F",
                    "c",
                    "X, PA",
                    Some(100.0),
                    Some(0.5),
                    Some(5.0),
                )
            })
            .collect();
        backends.catalogue(&woods).await;
        let app = backends.app();

        let (_, first, _) = get(&app, "/woods").await;
        let (_, second, _) = get(&app, "/woods?page=2").await;
        let (_, past, _) = get(&app, "/woods?page=99").await;

        assert!(first.contains("Page 1 of 2") && first.contains(r#"href="/woods?page=2">Next →"#));
        assert!(!first.contains("← Previous"));
        assert_eq!(first.matches("data-card>").count(), 24);
        assert!(second.contains("Page 2 of 2") && second.contains(r#"href="/woods">← Previous"#));
        assert_eq!(second.matches("data-card>").count(), 6);
        assert!(past.contains("Page 2 of 2"));
        assert_eq!(
            backends.api.received_requests().await.unwrap().len(),
            1,
            "the catalogue is fetched once and reused"
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
        assert!(html.contains(
            r#"<th>Janka hardness</th><td>900 kgf<div class="mt-1 h-1.5 min-w-20 overflow-hidden rounded bg-line"><i class="block h-full bg-accent" style="width:100%"></i></div></td><td>900 kgf"#
        ));
        assert!(
            !html.contains(r#"class="best "#),
            "equal figures have no best"
        );
        assert!(
            !html.contains("✕ Remove"),
            "two woods is the least to compare"
        );
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
        if status != StatusCode::SEE_OTHER {
            return (status, headers, html);
        }
        let location = headers[header::LOCATION].to_str().unwrap().to_owned();
        let (status, html, headers) = get(app, &location).await;

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
        assert!(portuguese.contains("Madeira Ideal"));
        assert!(portuguese.contains("Buscar madeiras"));
        assert!(!portuguese.contains("Wood Match"));
        assert!(!headers.contains_key(header::SET_COOKIE));
        assert!(
            headers[header::VARY]
                .to_str()
                .unwrap()
                .contains("accept-language")
        );
        assert!(english.contains("<html lang=\"en\">") && english.contains("Wood Match"));
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

        assert!(html.contains("Madeira Ideal"));
        let cookie = headers[header::SET_COOKIE].to_str().unwrap();
        assert!(cookie.starts_with("lang=pt-br;") && cookie.contains("SameSite=Lax"));
        assert!(
            back.contains("Madeira Ideal"),
            "the cookie beats the browser"
        );
        assert!(!english_headers.contains_key(header::SET_COOKIE));
        assert!(switched.contains("Wood Match"));
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
        backends
            .catalogue(&crate::browse::testing::catalogue())
            .await;

        let (_, html, _) = get_with(
            &backends.app(),
            "/woods?q=ipe",
            &[("accept-language", "pt")],
        )
        .await;

        assert!(html.contains("Madeiras brasileiras"));
        assert!(html.contains("1 de 5 madeiras"));
        assert!(html.contains("Pesquisar") && html.contains("Filtros"));
        assert!(html.contains("Cor do cerne") && html.contains("Marrom"));
        assert!(html.contains("Muito dura") && html.contains("Mais relevante"));
        assert!(html.contains("aria-label=\"Comparar Ipê\""));
        assert!(html.contains("Página 1 de 1") && html.contains("Cartões"));
    }

    #[tokio::test]
    async fn comparing_highlights_the_best_figures_and_lets_a_wood_be_taken_out() {
        let backends = Backends::start().await;
        for (id, name, janka, shrinkage) in [
            ("a", "Cedro", 450.0, 8.0),
            ("b", "Ipê", 1800.0, 12.0),
            ("c", "Pinus", 900.0, 0.0),
        ] {
            let mut body = wood_json(id, name);
            body["mechanical_properties"]["dry"]["janka_hardness"]["parallel_to_grain"] =
                json!(janka);
            body["physical_properties"] = json!({"shrinkage": {"volumetric": shrinkage}});
            Mock::given(path(format!("/api/v1/woods/{id}")))
                .respond_with(ResponseTemplate::new(200).set_body_json(body))
                .mount(&backends.api)
                .await;
        }

        let (_, html, _) = get(&backends.app(), "/compare?id=a&id=b&id=c").await;

        assert!(
            html.contains(r#"<td class="best bg-accent/15 font-bold">1800 kgf"#),
            "the hardest is highlighted"
        );
        assert!(
            html.contains(r#"<td class="best bg-accent/15 font-bold">0.0 %"#),
            "and the least shrinking"
        );
        assert!(html.contains(r#"style="width:25%""#), "450 against 1800");
        assert!(html.contains(r#"href="/compare?id=b&#38;id=c">✕ Remove"#));
        assert!(html.contains("Add another wood"));
        assert!(html.contains(r#"data-tag="hardness" href="/woods?hardness=very-hard">"#));
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

        let app = backends.app();
        let (asked, _, headers) = send(&app, request).await;
        let page = Request::get(headers[header::LOCATION].to_str().unwrap())
            .header("accept-language", "pt-BR")
            .body(Body::empty())
            .unwrap();
        let (status, html, _) = send(&app, page).await;

        assert_eq!(asked, StatusCode::SEE_OTHER);
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

    // ---- asks -------------------------------------------------------------------------

    fn stored_answer(recommendations: Value) -> zylotek_contracts::assistant::AskResponse {
        serde_json::from_value(ranking(recommendations)).unwrap()
    }

    #[tokio::test]
    async fn asking_opens_an_ask_and_sends_the_visitor_to_its_page() {
        let backends = Backends::start().await;
        Mock::given(path("/v1/ask/stream"))
            .respond_with(answer(json!([recommendation("a", "Cedro")])))
            .mount(&backends.assistant)
            .await;
        let body = "question=+a+table+";
        let request = Request::post("/ask")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap();

        let (status, _, headers) = send(&backends.app(), request).await;

        assert_eq!(status, StatusCode::SEE_OTHER);
        assert_eq!(headers[header::LOCATION], "/asks/ask-1");
        let stored = backends.store.asks.lock().unwrap()["ask-1"].clone();
        assert_eq!(stored.question, "a table");
    }

    #[tokio::test]
    async fn an_api_that_cannot_open_the_ask_is_an_error_page() {
        let backends = Backends::start().await;
        backends
            .store
            .down
            .store(true, std::sync::atomic::Ordering::SeqCst);

        let (status, html) = post_question(&backends.app(), "a table").await;

        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert!(html.contains("still starting up"));
    }

    #[tokio::test]
    async fn a_finished_ask_is_shown_from_what_is_stored() {
        let backends = Backends::start().await;
        let mut done = record("done-1", AskStateSchema::Answered);
        done.answer = Some(stored_answer(json!([recommendation("a", "Cedro")])));
        done.explanation = "Cedro <is> steady.".into();
        done.explanation_complete = true;
        backends.store.put(done);

        let (status, html, headers) = get(&backends.app(), "/asks/done-1").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
        assert!(html.contains(r#"data-ask-id="done-1""#));
        assert!(html.contains("<strong>Cedro</strong>"));
        assert!(
            html.contains("Cedro &lt;is&gt; steady."),
            "the stored text is escaped"
        );
        assert!(html.contains(r#"class="done "#), "the page is marked done");
        assert!(
            !html.contains("http-equiv=\"refresh\""),
            "a finished ask does not refresh"
        );
        assert!(!html.contains("cut short"));
        assert!(html.trim_end().ends_with("</html>"));
    }

    #[tokio::test]
    async fn a_stored_ask_still_being_answered_elsewhere_refreshes_itself() {
        let backends = Backends::start().await;
        let now = now_ms();
        let mut queued = record("queued-1", AskStateSchema::Understanding);
        queued.updated_at = now;
        let mut writing = record("writing-1", AskStateSchema::Explaining);
        writing.answer = Some(stored_answer(json!([recommendation("a", "Cedro")])));
        writing.explanation = "Cedro is".into();
        writing.updated_at = now;
        backends.store.put(queued);
        backends.store.put(writing);
        let app = backends.app();

        let (_, waiting, _) = get(&app, "/asks/queued-1").await;
        let (_, partial, _) = get(&app, "/asks/writing-1").await;

        assert!(waiting.contains(r#"<meta http-equiv="refresh" content="3">"#));
        assert!(waiting.contains("Thinking…") && !waiting.contains(r#"class="results""#));
        assert!(waiting.trim_end().ends_with("</html>"));
        assert!(partial.contains(r#"<meta http-equiv="refresh" content="3">"#));
        assert!(partial.contains("Cedro is") && !partial.contains(r#"class="done "#));
    }

    #[tokio::test]
    async fn an_ask_that_stopped_moving_is_shown_as_interrupted() {
        let backends = Backends::start().await;
        let mut ranked = record("stuck-1", AskStateSchema::Explaining);
        ranked.answer = Some(stored_answer(json!([recommendation("a", "Cedro")])));
        ranked.explanation = "Half".into();
        backends.store.put(ranked);
        backends
            .store
            .put(record("stuck-2", AskStateSchema::Queued));
        let app = backends.app();

        let (_, partial, _) = get(&app, "/asks/stuck-1").await;
        let (_, nothing, _) = get(&app, "/asks/stuck-2").await;

        for page in [&partial, &nothing] {
            assert!(page.contains("stopped before it was finished"));
            assert!(!page.contains("http-equiv=\"refresh\""), "no point waiting");
        }
        assert!(partial.contains("<strong>Cedro</strong>") && partial.contains("Half"));
        assert!(nothing.contains(r#"results card failed"#));
    }

    #[tokio::test]
    async fn failed_and_cut_short_asks_say_so() {
        let backends = Backends::start().await;
        let mut failed = record("failed-1", AskStateSchema::Failed);
        failed.failure = Some("The assistant had a problem answering.".into());
        let mut failed_late = record("failed-2", AskStateSchema::Failed);
        failed_late.answer = Some(stored_answer(json!([recommendation("a", "Cedro")])));
        failed_late.failure = Some("Gave up.".into());
        let mut cut = record("cut-1", AskStateSchema::Answered);
        cut.answer = Some(stored_answer(json!([recommendation("a", "Cedro")])));
        cut.explanation = "Half".into();
        let mut none = record("none-1", AskStateSchema::Answered);
        none.answer = Some(stored_answer(json!([])));
        none.explanation_complete = true;
        for ask in [failed, failed_late, cut, none] {
            backends.store.put(ask);
        }
        let app = backends.app();

        let (_, failed, _) = get(&app, "/asks/failed-1").await;
        let (_, failed_late, _) = get(&app, "/asks/failed-2").await;
        let (_, cut, _) = get(&app, "/asks/cut-1").await;
        let (_, none, _) = get(&app, "/asks/none-1").await;

        assert!(failed.contains("The assistant had a problem answering."));
        assert!(failed.contains(r#"results card failed"#));
        assert!(failed_late.contains("<strong>Cedro</strong>") && failed_late.contains("Gave up."));
        assert!(cut.contains("The explanation was cut short."));
        assert!(none.contains("No wood in the database has the figures"));
    }

    #[tokio::test]
    async fn an_unknown_or_malformed_ask_is_a_404_and_a_failing_api_a_502() {
        let backends = Backends::start().await;
        Mock::given(path("/api/v1/asks/abc"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&backends.api)
            .await;
        let app = backends.app();
        let broken = router(AppState {
            api: Arc::new(ApiClient::new(&backends.api.uri())),
            asks: Arc::new(Asks::new(
                Arc::new(ApiClient::new(&backends.api.uri())),
                Arc::new(AssistantClient::new(&backends.assistant.uri(), None)),
            )),
        });

        let (missing, html, _) = get(&app, "/asks/nobody").await;
        let (malformed, _, _) = get(&app, "/asks/..%2Fx").await;
        let (down, _, _) = get(&broken, "/asks/abc").await;

        assert_eq!(missing, StatusCode::NOT_FOUND);
        assert!(html.contains("that question"));
        assert_eq!(malformed, StatusCode::NOT_FOUND);
        assert_eq!(down, StatusCode::BAD_GATEWAY);
    }

    #[tokio::test]
    async fn a_page_following_an_ask_ends_cleanly_if_the_job_goes_away() {
        let snapshot = Snapshot {
            question: "q".into(),
            state: AskStateSchema::Understanding,
            answer: None,
            explanation: String::new(),
            complete: false,
            failure: None,
        };
        let collect = |live: watch::Receiver<Snapshot>| async move {
            use futures::StreamExt;
            follow("SHELL".into(), live, Lang::En)
                .map(Result::unwrap)
                .collect::<Vec<_>>()
                .await
                .concat()
        };

        let (gone, receiver) = watch::channel(snapshot.clone());
        drop(gone);
        let before_ranking = collect(receiver).await;
        let (sender, receiver) = watch::channel(Snapshot {
            answer: Some(stored_answer(json!([recommendation("a", "Cedro")]))),
            explanation: "Half".into(),
            ..snapshot
        });
        drop(sender);
        let while_writing = collect(receiver).await;

        // Words that arrive while the page waits are sent as they come.
        let (sender, receiver) = watch::channel(Snapshot {
            answer: Some(stored_answer(json!([recommendation("a", "Cedro")]))),
            state: AskStateSchema::Explaining,
            ..Snapshot {
                question: "q".into(),
                state: AskStateSchema::Queued,
                answer: None,
                explanation: String::new(),
                complete: false,
                failure: None,
            }
        });
        let writer = tokio::spawn(async move {
            for word in ["Cedro ", "is ", "steady."] {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                sender.send_modify(|s| s.explanation.push_str(word));
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            sender.send_modify(|s| {
                s.state = AskStateSchema::Answered;
                s.complete = true;
            });
        });
        let streamed = collect(receiver).await;
        writer.await.unwrap();

        assert!(streamed.contains("Cedro is steady.") && !streamed.contains("cut short"));
        assert!(before_ranking.starts_with("SHELL"));
        assert!(before_ranking.contains("stopped before it was finished"));
        assert!(while_writing.contains("<strong>Cedro</strong>") && while_writing.contains("Half"));
        assert!(while_writing.trim_end().ends_with("</html>"));
    }
}
