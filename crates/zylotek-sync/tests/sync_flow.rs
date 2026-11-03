//! The ingest flow end to end: listing pages, then every species page, over real HTTP.

use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path, query_param},
};
use zylotek_sync::{Scrape, Syncronizer};

const SPECIES_HTML: &str = include_str!("fixtures/species.html");

fn listing(server: &MockServer, species_ids: &[u64]) -> String {
    let rows: String = species_ids
        .iter()
        .map(|id| {
            format!(
                r#"<tr><td><a href="{}/especie?especieestudadaid={id}">Genus species{id}</a></td><td>Madeira {id}</td><td>Pará</td></tr>"#,
                server.uri()
            )
        })
        .collect();

    format!(
        r#"<html><body><table id="caracteristicaList"><tbody>{rows}</tbody></table></body></html>"#
    )
}

async fn mount_listing(server: &MockServer, start: u32, body: String) {
    Mock::given(method("GET"))
        .and(path("/madeiras"))
        .and(query_param("start", start.to_string()))
        .and(header("accept", "text/html"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .expect(1)
        .mount(server)
        .await;
}

async fn mount_species(server: &MockServer, id: u64, response: ResponseTemplate) {
    Mock::given(method("GET"))
        .and(path("/especie"))
        .and(query_param("especieestudadaid", id.to_string()))
        .respond_with(response)
        .expect(1)
        .mount(server)
        .await;
}

async fn run(server: &MockServer, last_page: usize) -> Scrape {
    let runner = Syncronizer::new()
        .with_listing_url(format!("{}/madeiras", server.uri()))
        .with_total_page_number(last_page)
        .build();

    tokio::task::spawn_blocking(move || runner.run().unwrap())
        .await
        .unwrap()
}

/// The reasons of the failures, one string each, with the page they name.
fn failures(scrape: &Scrape) -> Vec<String> {
    scrape
        .failures
        .iter()
        .map(|failure| format!("{}: {}", failure.url, failure.reason))
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn scrapes_every_listing_page_and_every_species() {
    let server = MockServer::start().await;
    mount_listing(&server, 0, listing(&server, &[1, 2])).await;
    mount_listing(&server, 20, listing(&server, &[3])).await;
    for id in [1, 2, 3] {
        mount_species(
            &server,
            id,
            ResponseTemplate::new(200).set_body_string(SPECIES_HTML),
        )
        .await;
    }

    let scrape = run(&server, 1).await;
    assert_eq!(scrape.woods.len(), 3);
    assert!(scrape.failures.is_empty());

    // MockServer verifies each `.expect(1)` on drop; 2 listings + 3 species.
    assert_eq!(server.received_requests().await.unwrap().len(), 5);
}

#[tokio::test(flavor = "multi_thread")]
async fn empty_listing_pages_are_fine() {
    let server = MockServer::start().await;
    mount_listing(&server, 0, listing(&server, &[])).await;

    let scrape = run(&server, 0).await;
    assert!(scrape.woods.is_empty() && scrape.failures.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failing_listing_page_is_reported_and_the_others_still_scraped() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/madeiras"))
        .and(query_param("start", "0"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    mount_listing(&server, 20, listing(&server, &[3])).await;
    mount_species(
        &server,
        3,
        ResponseTemplate::new(200).set_body_string(SPECIES_HTML),
    )
    .await;

    let scrape = run(&server, 1).await;

    assert_eq!(scrape.woods.len(), 1);
    let failures = failures(&scrape);
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert!(failures[0].contains("start=0") && failures[0].contains("503"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failing_species_page_is_reported_and_its_neighbours_kept() {
    let server = MockServer::start().await;
    mount_listing(&server, 0, listing(&server, &[1, 2])).await;
    mount_species(&server, 1, ResponseTemplate::new(500)).await;
    mount_species(
        &server,
        2,
        ResponseTemplate::new(200).set_body_string(SPECIES_HTML),
    )
    .await;

    let scrape = run(&server, 0).await;

    assert_eq!(scrape.woods.len(), 1);
    let failures = failures(&scrape);
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert!(failures[0].contains("especieestudadaid=1") && failures[0].contains("500"));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unparsable_species_page_is_reported_naming_the_page() {
    let server = MockServer::start().await;
    mount_listing(&server, 0, listing(&server, &[1])).await;
    mount_species(
        &server,
        1,
        ResponseTemplate::new(200).set_body_string("<html><body>nada</body></html>"),
    )
    .await;

    let scrape = run(&server, 0).await;

    assert!(scrape.woods.is_empty());
    let failures = failures(&scrape);
    assert!(
        failures[0].contains("failed to parse the page"),
        "{failures:?}"
    );
    assert!(failures[0].contains("especieestudadaid=1"), "{failures:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_listing_row_without_a_link_is_reported() {
    let server = MockServer::start().await;
    let body =
        r#"<table id="caracteristicaList"><tbody><tr><td>sem link</td></tr></tbody></table>"#;
    mount_listing(&server, 0, body.to_owned()).await;

    let scrape = run(&server, 0).await;

    assert!(failures(&scrape)[0].contains("no species link"));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unreachable_site_is_reported_not_raised() {
    // Nothing listens on port 1. (A dropped MockServer's port could be reused by a
    // parallel test, so it is not a safe "dead" address.)
    let runner = Syncronizer::new()
        .with_listing_url("http://127.0.0.1:1/madeiras")
        .with_total_page_number(0)
        .build();

    let scrape = tokio::task::spawn_blocking(move || runner.run().unwrap())
        .await
        .unwrap();

    assert!(scrape.woods.is_empty());
    assert_eq!(scrape.failures.len(), 1);
}

#[test]
fn parse_wood_is_public_and_parses_the_fixture() {
    let wood = zylotek_sync::parse_wood(
        SPECIES_HTML,
        "https://lpf.florestal.gov.br/especie?especieestudadaid=7",
    )
    .unwrap();

    assert_eq!(wood.scientific_name.as_str(), "Astronium lecointei");
    assert_eq!(wood.details.source.unwrap().species_id, 7);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_real_csv_is_completed_by_a_scraped_page() {
    let csv = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../toasty/fixtures/dados_abertos_banco_de_dados_de_madeiras_brasileiras_lpfsfb.csv"
    ))
    .unwrap();
    let open_data = zylotek_sync::parse_open_data(&csv).unwrap();
    let server = MockServer::start().await;
    // The fixture page is Astronium lecointei, which the CSV lists on two rows.
    mount_listing(&server, 0, listing(&server, &[21])).await;
    mount_species(
        &server,
        21,
        ResponseTemplate::new(200).set_body_string(SPECIES_HTML),
    )
    .await;
    let scrape = run(&server, 0).await;

    let merged = zylotek_sync::merge(open_data.woods, scrape.woods);

    assert_eq!((merged.enriched, merged.web_only), (1, 0));
    assert_eq!(merged.woods.len(), 269, "every CSV row is still there");
    let lots: Vec<_> = merged
        .woods
        .iter()
        .filter(|w| w.scientific_name.as_str() == "Astronium lecointei")
        .collect();
    assert_eq!(lots.len(), 2);
    // The first lot keeps the CSV's name and location but gains what only the page has.
    assert_eq!(lots[0].name.as_str(), "Muiracatiara-rajada");
    assert_eq!(
        lots[0].details.collection_location.as_deref(),
        Some("Belém, PA")
    );
    assert!(lots[0].details.images.main.is_some());
    assert!(!lots[0].details.common_names.is_empty());
    assert!(lots[0].details.physical_properties.density.basic.is_some());
    assert!(lots[1].details.images.main.is_none());
}
