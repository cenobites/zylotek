mod merge;
mod open_data;
mod parsers;

pub use merge::{Merged, merge};
pub use open_data::{DATASET_URL, OpenData, RejectedRow, parse_open_data};
pub use parsers::parse_wood;

use std::hash::{DefaultHasher, Hash, Hasher};

use anyhow::{Context, Result};
use scraper::{Html, Selector};
use tokio::{self, task::JoinHandle};
use zylotek_domain::woods::value_objects::{ScientificName, WoodDetails, WoodName};

const TOTAL_PAGE_NUMBER: usize = 13;
const LISTING_URL: &str = "https://lpf.florestal.gov.br/pt-br/madeiras-brasileiras";

type Error = Box<dyn std::error::Error + Send + Sync>;

pub struct Syncronizer {
    listing_url: String,
    total_page_number: usize,
}

impl Syncronizer {
    pub fn new() -> Self {
        Self {
            listing_url: LISTING_URL.to_owned(),
            total_page_number: TOTAL_PAGE_NUMBER,
        }
    }

    /// Scrapes `listing_url` instead of the LPF site.
    pub fn with_listing_url(mut self, listing_url: impl Into<String>) -> Self {
        self.listing_url = listing_url.into();
        self
    }

    /// Fetches listing pages `0..=total_page_number`.
    pub fn with_total_page_number(mut self, total_page_number: usize) -> Self {
        self.total_page_number = total_page_number;
        self
    }

    pub fn build(self) -> SyncronizerRunner {
        SyncronizerRunner {
            listing_url: self.listing_url,
            total_page_number: self.total_page_number,
        }
    }
}

impl Default for Syncronizer {
    fn default() -> Self {
        Self::new()
    }
}

pub struct SyncronizerRunner {
    listing_url: String,
    total_page_number: usize,
}

/// What a scrape brought back. A page that cannot be fetched or parsed is recorded in
/// `failures` instead of aborting the run, so one bad page never costs the others.
#[derive(Debug, Default)]
pub struct Scrape {
    pub woods: Vec<ScrapedWood>,
    pub failures: Vec<ScrapeFailure>,
}

/// A listing or species page that could not be scraped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScrapeFailure {
    pub url: String,
    pub reason: String,
}

impl ScrapeFailure {
    fn new(url: &str, error: impl std::fmt::Display) -> Self {
        Self {
            url: url.to_owned(),
            reason: error.to_string(),
        }
    }
}

impl SyncronizerRunner {
    /// Scrapes every listing page and every species page. Fails only when the HTTP client
    /// cannot be set up; page problems are reported in [`Scrape::failures`].
    pub fn run(self) -> Result<Scrape, Error> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;

        let client = reqwest::Client::builder()
            .connection_verbose(false)
            .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:156.0) Gecko/20100101 Firefox/156.0")
            .build()?;

        let mut handles = Vec::new();
        for i in 0..=self.total_page_number {
            let client = client.clone();
            let page_url = format!("{}?start={}", self.listing_url, i * 20);

            handles.push((
                page_url.clone(),
                rt.spawn(async move { scrape_page(&client, &page_url).await }),
            ));
        }

        let mut scrape = Scrape::default();
        for (page_url, handle) in handles {
            let page = page_or_failure(&page_url, rt.block_on(handle));
            log::debug!("scraped {} woods", page.woods.len());
            scrape.woods.extend(page.woods);
            scrape.failures.extend(page.failures);
        }

        Ok(scrape)
    }
}

/// One listing page and the species pages it links to.
async fn scrape_page(client: &reqwest::Client, page_url: &str) -> Scrape {
    match fetch_woods(client, page_url).await {
        Ok(tasks) => collect(tasks).await,
        Err(error) => Scrape {
            woods: Vec::new(),
            failures: vec![ScrapeFailure::new(page_url, format!("{error:#}"))],
        },
    }
}

/// Waits for the species downloads, which are already running concurrently.
async fn collect(tasks: Vec<(String, WoodFetcherTask)>) -> Scrape {
    let mut scrape = Scrape::default();

    for (wood_url, task) in tasks {
        match task.await {
            Ok(Ok(wood)) => scrape.woods.push(wood),
            Ok(Err(error)) => scrape
                .failures
                .push(ScrapeFailure::new(&wood_url, format!("{error:#}"))),
            Err(error) => scrape.failures.push(ScrapeFailure::new(&wood_url, error)),
        }
    }

    scrape
}

/// A page task that panicked becomes a failure naming the page.
fn page_or_failure(page_url: &str, joined: Result<Scrape, tokio::task::JoinError>) -> Scrape {
    joined.unwrap_or_else(|error| Scrape {
        woods: Vec::new(),
        failures: vec![ScrapeFailure::new(page_url, error)],
    })
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct WoodFetcher {
    id: u64,
    name: String,
    scientific_name: String,
    collection_location: String,
    source_page_link: String,
}

impl WoodFetcher {
    fn new(
        name: String,
        scientific_name: String,
        collection_location: String,
        source_page_link: String,
    ) -> Self {
        Self {
            id: Self::calculate_id(&scientific_name),
            name,
            scientific_name,
            collection_location,
            source_page_link,
        }
    }

    async fn fetch(&self, client: &reqwest::Client) -> Result<ScrapedWood> {
        fetch_wood(client, &self.source_page_link).await
    }

    fn calculate_id(str: &str) -> u64 {
        let mut hasher = DefaultHasher::new();
        str.hash(&mut hasher);
        hasher.finish()
    }
}

/// A wood as scraped from its LPF species page, expressed in domain terms.
#[derive(Debug, Clone)]
pub struct ScrapedWood {
    pub name: WoodName,
    pub scientific_name: ScientificName,
    pub details: WoodDetails,
}

async fn fetch_wood(client: &reqwest::Client, url: &str) -> Result<ScrapedWood> {
    log::info!(" + Fetching wood {url}");
    let response = client
        .get(url)
        .header("Accept", "text/html")
        .send()
        .await?
        .error_for_status()?;
    let body = response.text().await?;

    parsers::parse_wood(&body, url).with_context(|| format!("failed to parse the page: {}", url))
}

type WoodFetcherTask = JoinHandle<Result<ScrapedWood>>;

/// One download per listed species, with the page it fetches so a failure can name it.
async fn fetch_woods(
    client: &reqwest::Client,
    url: &str,
) -> Result<Vec<(String, WoodFetcherTask)>> {
    log::info!("Fetching page: {url}");
    let response = client
        .get(url)
        .header("Accept", "text/html")
        .send()
        .await?
        .error_for_status()?;
    let body = response.text().await?;

    let mut tasks = Vec::with_capacity(20);
    for wood_fetcher in parse_listing(&body)? {
        let client = client.clone();
        let wood_url = wood_fetcher.source_page_link.clone();
        tasks.push((
            wood_url,
            tokio::spawn(async move { wood_fetcher.fetch(&client).await }),
        ));
    }

    Ok(tasks)
}

/// Reads the species rows of a listing page: scientific name and link, common name and
/// collection location.
fn parse_listing(body: &str) -> Result<Vec<WoodFetcher>> {
    let document = Html::parse_document(body);
    let line_selector = Selector::parse(r#"table[id=caracteristicaList] > tbody > tr"#).unwrap();
    let column_selector = Selector::parse(r#"td"#).unwrap();
    let link_selector = Selector::parse(r#"a"#).unwrap();

    let mut woods = Vec::new();
    for line in document.select(&line_selector) {
        let mut columns = line.select(&column_selector);
        let scientific_name_and_link = columns
            .next()
            .and_then(|column| column.select(&link_selector).next())
            .context("listing row has no species link")?;

        let scientific_name = scientific_name_and_link.inner_html().trim().to_string();
        let link = scientific_name_and_link
            .value()
            .attr("href")
            .context("species link has no href")?
            .trim();
        let name = columns
            .next()
            .map(|el| el.text().collect::<String>())
            .unwrap_or_default()
            .trim()
            .to_string();
        let collection_location = columns
            .next()
            .map(|el| el.text().collect::<String>())
            .unwrap_or_default()
            .trim()
            .to_string();

        woods.push(WoodFetcher::new(
            name,
            scientific_name,
            collection_location,
            link.to_string(),
        ));
    }

    Ok(woods)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LISTING: &str = r#"
        <table id="caracteristicaList"><tbody>
          <tr>
            <td><a href=" https://lpf.example/especie?especieestudadaid=1 "> Astronium lecointei </a></td>
            <td> Muiracatiara </td>
            <td> Pará </td>
          </tr>
          <tr><td><a href="/especie?especieestudadaid=2">Cedrela fissilis</a></td></tr>
        </tbody></table>
        <table id="other"><tbody><tr><td><a href="/ignored">ignored</a></td></tr></tbody></table>
    "#;

    #[test]
    fn parse_listing_reads_every_species_row() {
        let woods = parse_listing(LISTING).unwrap();

        assert_eq!(woods.len(), 2);
        assert_eq!(woods[0].scientific_name, "Astronium lecointei");
        assert_eq!(woods[0].name, "Muiracatiara");
        assert_eq!(woods[0].collection_location, "Pará");
        assert_eq!(
            woods[0].source_page_link,
            "https://lpf.example/especie?especieestudadaid=1"
        );
        assert_eq!(
            (
                woods[1].name.as_str(),
                woods[1].collection_location.as_str()
            ),
            ("", "")
        );
    }

    #[test]
    fn parse_listing_ignores_pages_without_the_species_table() {
        assert!(
            parse_listing("<html><body><p>nada</p></body></html>")
                .unwrap()
                .is_empty()
        );
        assert!(parse_listing("").unwrap().is_empty());
    }

    #[test]
    fn parse_listing_rejects_rows_without_a_link() {
        for row in [
            "<tr></tr>",
            "<tr><td>sem link</td></tr>",
            "<tr><td><a>sem href</a></td></tr>",
        ] {
            let html = format!(r#"<table id="caracteristicaList"><tbody>{row}</tbody></table>"#);

            assert!(parse_listing(&html).is_err(), "{row}");
        }
    }

    #[test]
    fn wood_ids_are_derived_from_the_scientific_name() {
        let wood = |scientific_name: &str| {
            WoodFetcher::new(
                String::new(),
                scientific_name.into(),
                String::new(),
                String::new(),
            )
        };

        assert_eq!(
            wood("Astronium lecointei").id,
            wood("Astronium lecointei").id
        );
        assert_ne!(wood("Astronium lecointei").id, wood("Cedrela fissilis").id);
    }

    #[test]
    fn the_synchronizer_defaults_to_the_lpf_site() {
        let runner = Syncronizer::default().build();

        assert_eq!(runner.listing_url, LISTING_URL);
        assert_eq!(runner.total_page_number, TOTAL_PAGE_NUMBER);
    }

    #[test]
    fn the_synchronizer_can_be_pointed_elsewhere() {
        let runner = Syncronizer::new()
            .with_listing_url("http://localhost:1234/madeiras")
            .with_total_page_number(2)
            .build();

        assert_eq!(runner.listing_url, "http://localhost:1234/madeiras");
        assert_eq!(runner.total_page_number, 2);
    }

    const SPECIES_HTML: &str = include_str!("../tests/fixtures/species.html");

    async fn server_with(
        route: &str,
        response: wiremock::ResponseTemplate,
    ) -> wiremock::MockServer {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path(route))
            .respond_with(response)
            .mount(&server)
            .await;
        server
    }

    fn ok(body: impl Into<String>) -> wiremock::ResponseTemplate {
        wiremock::ResponseTemplate::new(200).set_body_string(body.into())
    }

    #[tokio::test]
    async fn fetch_wood_parses_the_page_it_downloads() {
        let server = server_with("/especie", ok(SPECIES_HTML)).await;
        let url = format!("{}/especie?especieestudadaid=9", server.uri());

        let wood = fetch_wood(&reqwest::Client::new(), &url).await.unwrap();

        assert_eq!(wood.scientific_name.as_str(), "Astronium lecointei");
        assert_eq!(wood.details.source.unwrap().species_id, 9);
    }

    #[tokio::test]
    async fn fetch_wood_names_the_page_it_failed_to_parse() {
        let server = server_with("/especie", ok("<html></html>")).await;
        let url = format!("{}/especie?especieestudadaid=9", server.uri());

        let error = fetch_wood(&reqwest::Client::new(), &url).await.unwrap_err();

        assert!(
            format!("{error:#}").contains("failed to parse the page"),
            "{error:#}"
        );
    }

    #[tokio::test]
    async fn fetch_wood_rejects_http_errors() {
        let server = server_with("/especie", wiremock::ResponseTemplate::new(404)).await;
        let url = format!("{}/especie?especieestudadaid=9", server.uri());

        let error = fetch_wood(&reqwest::Client::new(), &url).await.unwrap_err();

        assert!(error.to_string().contains("404"), "{error}");
    }

    #[tokio::test]
    async fn fetch_woods_starts_one_task_per_listed_species() {
        let server = wiremock::MockServer::start().await;
        let listing = format!(
            r#"<table id="caracteristicaList"><tbody>
               <tr><td><a href="{0}/especie?especieestudadaid=1">A b</a></td></tr>
               <tr><td><a href="{0}/especie?especieestudadaid=2">C d</a></td></tr></tbody></table>"#,
            server.uri()
        );
        for (route, body) in [("/listing", listing), ("/especie", SPECIES_HTML.to_owned())] {
            wiremock::Mock::given(wiremock::matchers::path(route))
                .respond_with(ok(body))
                .mount(&server)
                .await;
        }

        let tasks = fetch_woods(
            &reqwest::Client::new(),
            &format!("{}/listing", server.uri()),
        )
        .await
        .unwrap();

        assert_eq!(tasks.len(), 2);
        for (url, task) in tasks {
            assert!(url.contains("/especie?especieestudadaid="), "{url}");
            assert!(task.await.unwrap().is_ok());
        }
    }

    #[tokio::test]
    async fn fetch_woods_rejects_http_errors_and_broken_rows() {
        let failing = server_with("/listing", wiremock::ResponseTemplate::new(500)).await;
        let broken = server_with(
            "/listing",
            ok(r#"<table id="caracteristicaList"><tbody><tr><td>sem link</td></tr></tbody></table>"#),
        )
        .await;

        let http = fetch_woods(
            &reqwest::Client::new(),
            &format!("{}/listing", failing.uri()),
        )
        .await;
        let row = fetch_woods(
            &reqwest::Client::new(),
            &format!("{}/listing", broken.uri()),
        )
        .await;

        assert!(http.err().unwrap().to_string().contains("500"));
        assert!(row.err().unwrap().to_string().contains("no species link"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn run_reports_a_failing_listing_without_failing() {
        let server = server_with("/madeiras", wiremock::ResponseTemplate::new(502)).await;
        let runner = Syncronizer::new()
            .with_listing_url(format!("{}/madeiras", server.uri()))
            .with_total_page_number(0)
            .build();

        let scrape = tokio::task::spawn_blocking(move || runner.run())
            .await
            .unwrap()
            .unwrap();

        assert!(scrape.woods.is_empty());
        assert_eq!(scrape.failures.len(), 1);
        assert!(scrape.failures[0].url.ends_with("/madeiras?start=0"));
        assert!(scrape.failures[0].reason.contains("502"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn run_scrapes_every_listed_species() {
        let server = wiremock::MockServer::start().await;
        let listing = format!(
            r#"<table id="caracteristicaList"><tbody>
               <tr><td><a href="{}/especie?especieestudadaid=1">A b</a></td></tr></tbody></table>"#,
            server.uri()
        );
        for (route, body) in [
            ("/madeiras", listing),
            ("/especie", SPECIES_HTML.to_owned()),
        ] {
            wiremock::Mock::given(wiremock::matchers::path(route))
                .respond_with(ok(body))
                .expect(1)
                .mount(&server)
                .await;
        }
        let runner = Syncronizer::new()
            .with_listing_url(format!("{}/madeiras", server.uri()))
            .with_total_page_number(0)
            .build();

        let scrape = tokio::task::spawn_blocking(move || runner.run())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(scrape.woods.len(), 1);
        assert!(scrape.failures.is_empty());
    }

    #[tokio::test]
    async fn scrape_page_keeps_the_good_species_and_names_the_bad_ones() {
        let server = wiremock::MockServer::start().await;
        let listing = format!(
            r#"<table id="caracteristicaList"><tbody>
               <tr><td><a href="{0}/especie?especieestudadaid=1">A b</a></td></tr>
               <tr><td><a href="{0}/missing">C d</a></td></tr>
               <tr><td><a href="{0}/broken">E f</a></td></tr></tbody></table>"#,
            server.uri()
        );
        for (route, response) in [
            ("/listing", ok(listing)),
            ("/especie", ok(SPECIES_HTML)),
            ("/missing", wiremock::ResponseTemplate::new(404)),
            ("/broken", ok("<html></html>")),
        ] {
            wiremock::Mock::given(wiremock::matchers::path(route))
                .respond_with(response)
                .mount(&server)
                .await;
        }

        let page = scrape_page(
            &reqwest::Client::new(),
            &format!("{}/listing", server.uri()),
        )
        .await;

        assert_eq!(page.woods.len(), 1);
        let reasons: Vec<_> = page
            .failures
            .iter()
            .map(|f| (f.url.rsplit('/').next().unwrap(), f.reason.as_str()))
            .collect();
        assert!(reasons[0].0 == "missing" && reasons[0].1.contains("404"));
        assert!(reasons[1].0 == "broken" && reasons[1].1.contains("failed to parse the page"));
    }

    #[tokio::test]
    async fn a_panicking_download_is_a_failure_naming_its_page() {
        let panicking: WoodFetcherTask = tokio::spawn(async { panic!("parser bug") });

        let scrape = collect(vec![("https://lpf.example/bad".to_owned(), panicking)]).await;

        assert!(scrape.woods.is_empty());
        assert_eq!(scrape.failures[0].url, "https://lpf.example/bad");
        assert!(scrape.failures[0].reason.contains("panic"));
    }

    #[tokio::test]
    async fn a_panicking_page_task_is_a_failure_naming_the_listing() {
        let panicking: JoinHandle<Scrape> = tokio::spawn(async { panic!("listing bug") });

        let page = page_or_failure("https://lpf.example/madeiras?start=0", panicking.await);

        assert!(page.woods.is_empty());
        assert_eq!(page.failures[0].url, "https://lpf.example/madeiras?start=0");
        assert!(page.failures[0].reason.contains("panic"));
    }
}
