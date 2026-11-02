//! The ingest flow end to end: listing pages, then every species page, over real HTTP.

use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path, query_param},
};
use zylotek_sync::Syncronizer;

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

async fn run(server: &MockServer, last_page: usize) -> Result<usize, String> {
    let runner = Syncronizer::new()
        .with_listing_url(format!("{}/madeiras", server.uri()))
        .with_total_page_number(last_page)
        .build();

    tokio::task::spawn_blocking(move || {
        runner
            .run()
            .map(|woods| woods.len())
            .map_err(|error| error.to_string())
    })
    .await
    .unwrap()
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

    assert_eq!(run(&server, 1).await.unwrap(), 3);

    // MockServer verifies each `.expect(1)` on drop; 2 listings + 3 species.
    assert_eq!(server.received_requests().await.unwrap().len(), 5);
}

#[tokio::test(flavor = "multi_thread")]
async fn empty_listing_pages_are_fine() {
    let server = MockServer::start().await;
    mount_listing(&server, 0, listing(&server, &[])).await;

    assert_eq!(run(&server, 0).await.unwrap(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failing_listing_page_fails_the_run() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/madeiras"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;

    let error = run(&server, 0).await.unwrap_err();

    assert!(error.contains("503"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failing_species_page_fails_the_run() {
    let server = MockServer::start().await;
    mount_listing(&server, 0, listing(&server, &[1])).await;
    mount_species(&server, 1, ResponseTemplate::new(500)).await;

    let error = run(&server, 0).await.unwrap_err();

    assert!(error.contains("500"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unparsable_species_page_fails_the_run_naming_the_page() {
    let server = MockServer::start().await;
    mount_listing(&server, 0, listing(&server, &[1])).await;
    mount_species(
        &server,
        1,
        ResponseTemplate::new(200).set_body_string("<html><body>nada</body></html>"),
    )
    .await;

    let error = run(&server, 0).await.unwrap_err();

    assert!(error.contains("failed to parse the page"), "{error}");
    assert!(error.contains("especieestudadaid=1"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_listing_row_without_a_link_fails_the_run() {
    let server = MockServer::start().await;
    let body =
        r#"<table id="caracteristicaList"><tbody><tr><td>sem link</td></tr></tbody></table>"#;
    mount_listing(&server, 0, body.to_owned()).await;

    let error = run(&server, 0).await.unwrap_err();

    assert!(error.contains("no species link"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unreachable_site_fails_the_run() {
    // Nothing listens on port 1. (A dropped MockServer's port could be reused by a
    // parallel test, so it is not a safe "dead" address.)
    let runner = Syncronizer::new()
        .with_listing_url("http://127.0.0.1:1/madeiras")
        .with_total_page_number(0)
        .build();

    let result = tokio::task::spawn_blocking(move || runner.run())
        .await
        .unwrap();

    assert!(result.is_err());
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
