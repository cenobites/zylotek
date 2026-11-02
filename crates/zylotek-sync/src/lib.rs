mod open_data;
mod parsers;

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

impl SyncronizerRunner {
    /// Scrapes every listing page and every species page, returning what was scraped.
    pub fn run(self) -> Result<Vec<ScrapedWood>, Error> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;

        let mut handles: Vec<tokio::task::JoinHandle<Result<_, Error>>> = Vec::new();
        let client = reqwest::Client::builder()
            .connection_verbose(false)
            .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:156.0) Gecko/20100101 Firefox/156.0")
            .build()?;
        let url = self.listing_url;

        let num_pages = self.total_page_number;
        for i in 0..=num_pages {
            let client = client.clone();
            let next_page = format!("{}?start={}", url, i * 20);

            handles.push(rt.spawn(async move {
                let wood_tasks = fetch_woods(&client, &next_page).await?;
                let woods: Vec<ScrapedWood> = futures::future::try_join_all(wood_tasks)
                    .await?
                    .into_iter()
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(woods)
            }));
        }

        let mut scraped = Vec::new();
        for handle in handles {
            let woods = rt
                .block_on(handle)
                .map_err(|e| Error::from(e.to_string()))??;
            log::debug!("scraped {} woods", woods.len());
            scraped.extend(woods);
        }

        Ok(scraped)
    }
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

async fn fetch_woods(client: &reqwest::Client, url: &str) -> Result<Vec<WoodFetcherTask>> {
    log::info!("Fetching page: {url}");
    let response = client
        .get(url)
        .header("Accept", "text/html")
        .send()
        .await?
        .error_for_status()?;
    let body = response.text().await?;

    let mut tasks = Vec::<WoodFetcherTask>::with_capacity(20);
    for wood_fetcher in parse_listing(&body)? {
        let client = client.clone();
        tasks.push(tokio::spawn(
            async move { wood_fetcher.fetch(&client).await },
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
        for task in tasks {
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
    async fn run_reports_a_failing_listing() {
        let server = server_with("/madeiras", wiremock::ResponseTemplate::new(502)).await;
        let runner = Syncronizer::new()
            .with_listing_url(format!("{}/madeiras", server.uri()))
            .with_total_page_number(0)
            .build();

        let result = tokio::task::spawn_blocking(move || runner.run())
            .await
            .unwrap();

        assert!(result.unwrap_err().to_string().contains("502"));
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

        tokio::task::spawn_blocking(move || runner.run())
            .await
            .unwrap()
            .unwrap();
    }
}
