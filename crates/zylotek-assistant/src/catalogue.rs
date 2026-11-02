//! The assistant's copy of the wood catalogue, pulled from the API. Reads are public, so no
//! credentials are needed. A sync builds a whole new snapshot and swaps it in, so a question
//! never sees half a catalogue, and a failed sync leaves the old one in place.

use std::{
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};

use zylotek_contracts::woods::{WoodPageResponse, WoodResponse};

use crate::{error::SyncError, ranking::Ranker};

/// The API's page size ceiling.
const PAGE_SIZE: usize = 100;
/// Stops a misbehaving server from keeping a sync running forever.
const MAX_PAGES: usize = 1000;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

pub struct Snapshot {
    pub ranker: Ranker,
    pub fetched_at: Instant,
}

#[derive(Default)]
pub struct Catalogue {
    current: RwLock<Option<Arc<Snapshot>>>,
}

impl Catalogue {
    pub fn new() -> Self {
        Self::default()
    }

    /// The latest snapshot, or `None` before the first successful sync.
    pub fn snapshot(&self) -> Option<Arc<Snapshot>> {
        self.current
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn replace(&self, woods: Vec<WoodResponse>) {
        let snapshot = Arc::new(Snapshot {
            ranker: Ranker::new(woods),
            fetched_at: Instant::now(),
        });
        *self
            .current
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(snapshot);
    }
}

pub struct ApiClient {
    client: reqwest::Client,
    base_url: String,
}

impl ApiClient {
    pub fn new(base_url: &str) -> Self {
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .expect("the HTTP client builds with a timeout");

        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_owned(),
        }
    }

    /// Every wood the API lists, page by page.
    pub async fn fetch_all(&self) -> Result<Vec<WoodResponse>, SyncError> {
        let mut woods = Vec::new();
        for _ in 0..MAX_PAGES {
            let url = format!(
                "{}/api/v1/woods?offset={}&limit={PAGE_SIZE}",
                self.base_url,
                woods.len()
            );
            let page = self.page(&url).await?;
            let fetched = page.items.len();
            woods.extend(page.items);
            if fetched == 0 || woods.len() as u64 >= page.total {
                break;
            }
        }

        Ok(woods)
    }

    async fn page(&self, url: &str) -> Result<WoodPageResponse, SyncError> {
        let response = self.client.get(url).send().await.map_err(transport)?;
        let status = response.status();
        if !status.is_success() {
            return Err(SyncError::Status {
                status: status.as_u16(),
                url: url.to_owned(),
            });
        }
        let text = response.text().await.map_err(transport)?;

        serde_json::from_str(&text).map_err(|error| SyncError::Malformed(error.to_string()))
    }
}

fn transport(error: reqwest::Error) -> SyncError {
    SyncError::Transport(error.to_string())
}

/// Fetches the whole catalogue and swaps it in; returns how many woods it holds now.
pub async fn sync(api: &ApiClient, catalogue: &Catalogue) -> Result<usize, SyncError> {
    let woods = api.fetch_all().await?;
    let count = woods.len();
    catalogue.replace(woods);

    Ok(count)
}

/// Syncs forever, once per `every`; failures are logged and the old snapshot is kept.
pub async fn keep_in_sync(api: Arc<ApiClient>, catalogue: Arc<Catalogue>, every: Duration) {
    let mut ticker = tokio::time::interval(every);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        match sync(&api, &catalogue).await {
            Ok(count) => tracing::info!(count, "catalogue synced"),
            Err(error) => tracing::warn!(%error, "catalogue sync failed; keeping the old copy"),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path, query_param},
    };

    use super::*;
    use crate::ranking::testing::wood;

    fn page(names: &[&str], total: u64, offset: usize) -> ResponseTemplate {
        let items: Vec<_> = names.iter().map(|name| wood(name)).collect();
        ResponseTemplate::new(200).set_body_json(json!({
            "items": items, "total": total, "offset": offset, "limit": 100,
        }))
    }

    async fn serve_pages(server: &MockServer, pages: &[(usize, &[&str], u64)]) {
        for &(offset, names, total) in pages {
            Mock::given(method("GET"))
                .and(path("/api/v1/woods"))
                .and(query_param("offset", offset.to_string()))
                .and(query_param("limit", "100"))
                .respond_with(page(names, total, offset))
                .mount(server)
                .await;
        }
    }

    #[test]
    fn nothing_is_available_before_the_first_sync() {
        let catalogue = Catalogue::new();

        assert!(catalogue.snapshot().is_none());
        catalogue.replace(vec![wood("A")]);
        assert_eq!(catalogue.snapshot().unwrap().ranker.len(), 1);
    }

    #[test]
    fn a_snapshot_in_hand_survives_being_replaced() {
        let catalogue = Catalogue::new();
        catalogue.replace(vec![wood("A")]);
        let old = catalogue.snapshot().unwrap();

        catalogue.replace(vec![wood("A"), wood("B")]);

        assert_eq!(old.ranker.len(), 1);
        assert_eq!(catalogue.snapshot().unwrap().ranker.len(), 2);
    }

    #[test]
    fn a_poisoned_lock_does_not_take_the_assistant_down() {
        let catalogue = Arc::new(Catalogue::new());
        let poisoner = catalogue.clone();
        let _ = std::thread::spawn(move || {
            let _guard = poisoner.current.write().unwrap();
            panic!("poison the lock");
        })
        .join();

        catalogue.replace(vec![wood("A")]);

        assert_eq!(catalogue.snapshot().unwrap().ranker.len(), 1);
    }

    #[tokio::test]
    async fn it_follows_the_pages_until_it_has_everything() {
        let server = MockServer::start().await;
        let first: Vec<String> = (0..100).map(|n| format!("W{n:03}")).collect();
        let first: Vec<&str> = first.iter().map(String::as_str).collect();
        serve_pages(&server, &[(0, &first, 103), (100, &["X", "Y", "Z"], 103)]).await;
        let api = ApiClient::new(&format!("{}/", server.uri()));

        let woods = api.fetch_all().await.unwrap();

        assert_eq!(woods.len(), 103);
        assert_eq!(woods[102].name, "Z");
    }

    #[tokio::test]
    async fn an_empty_catalogue_is_fine_and_a_short_page_does_not_loop() {
        let server = MockServer::start().await;
        serve_pages(&server, &[(0, &[], 0)]).await;
        let empty = ApiClient::new(&server.uri()).fetch_all().await.unwrap();

        let other = MockServer::start().await;
        serve_pages(&other, &[(0, &["A"], 50), (1, &[], 50)]).await;
        let short = ApiClient::new(&other.uri()).fetch_all().await.unwrap();

        assert!(empty.is_empty());
        assert_eq!(
            short.len(),
            1,
            "the API lost woods mid-sync; take what there is"
        );
    }

    #[tokio::test]
    async fn a_server_that_never_ends_is_cut_off() {
        let server = MockServer::start().await;
        Mock::given(path("/api/v1/woods"))
            .respond_with(page(&["A"], u64::MAX, 0))
            .expect(MAX_PAGES as u64)
            .mount(&server)
            .await;

        let woods = ApiClient::new(&server.uri()).fetch_all().await.unwrap();

        assert_eq!(woods.len(), MAX_PAGES);
    }

    #[tokio::test]
    async fn every_failure_is_reported_and_leaves_the_old_catalogue() {
        let server = MockServer::start().await;
        Mock::given(path("/api/v1/woods"))
            .and(query_param("offset", "0"))
            .respond_with(ResponseTemplate::new(500))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(path("/api/v1/woods"))
            .respond_with(ResponseTemplate::new(200).set_body_string("<html>"))
            .mount(&server)
            .await;
        let api = ApiClient::new(&server.uri());
        let catalogue = Catalogue::new();
        catalogue.replace(vec![wood("Kept")]);

        let status = sync(&api, &catalogue).await.unwrap_err();
        let malformed = sync(&api, &catalogue).await.unwrap_err();
        let transport = sync(&ApiClient::new("http://127.0.0.1:1"), &catalogue)
            .await
            .unwrap_err();

        assert!(matches!(status, SyncError::Status { status: 500, .. }));
        assert!(status.to_string().contains("500"));
        assert!(matches!(malformed, SyncError::Malformed(_)));
        assert!(matches!(transport, SyncError::Transport(_)));
        assert_eq!(catalogue.snapshot().unwrap().ranker.len(), 1);
    }

    #[tokio::test]
    async fn the_background_loop_syncs_on_a_schedule_and_survives_failures() {
        let server = MockServer::start().await;
        Mock::given(path("/api/v1/woods"))
            .respond_with(ResponseTemplate::new(500))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        serve_pages(&server, &[(0, &["A", "B"], 2)]).await;
        let catalogue = Arc::new(Catalogue::new());
        let task = tokio::spawn(keep_in_sync(
            Arc::new(ApiClient::new(&server.uri())),
            catalogue.clone(),
            Duration::from_millis(20),
        ));

        for _ in 0..200 {
            if catalogue.snapshot().is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        task.abort();

        assert_eq!(catalogue.snapshot().unwrap().ranker.len(), 2);
    }
}
