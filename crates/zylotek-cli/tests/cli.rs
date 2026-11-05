//! The compiled binary, driven as a user would.

use std::fs;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

const TOASTY_CONFIG: &str = include_str!("../../../Toasty.toml");
const WORKSPACE_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

fn cli() -> Command {
    Command::cargo_bin("zylotek-cli").unwrap()
}

#[test]
fn prints_its_version() {
    cli()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::starts_with("zylotek-cli 0.1.0"));
}

#[test]
fn lists_the_subcommands_in_its_help() {
    cli()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage:"))
        .stdout(predicate::str::contains("db"))
        .stdout(predicate::str::contains("sync"))
        .stdout(predicate::str::contains("auth"));
}

#[test]
fn prints_help_when_run_without_arguments() {
    cli()
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage:"));
}

#[test]
fn nested_help_describes_each_command() {
    cli()
        .args(["db", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("migrate"));
    cli()
        .args(["sync", "run", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--config"));
}

#[test]
fn reports_unknown_commands_that_have_no_executable() {
    cli()
        .arg("frobnicate")
        .env("PATH", "")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("unknown command `frobnicate`"))
        .stderr(predicate::str::contains("zylotek-frobnicate"));
}

#[test]
fn refuses_subcommand_names_that_look_like_paths() {
    cli()
        .arg("../evil")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("invalid subcommand"));
}

#[cfg(unix)]
#[test]
fn unknown_commands_run_the_zylotek_executable_with_the_rest_of_the_arguments() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new().unwrap();
    let script = dir.path().join("zylotek-fake");
    fs::write(&script, "#!/bin/sh\necho \"fake: $*\"\nexit 7\n").unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();

    cli()
        .env("PATH", dir.path())
        .args(["fake", "--listen", "0.0.0.0:1"])
        .assert()
        .code(7)
        .stdout("fake: --listen 0.0.0.0:1\n");
}

fn project_with_config() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("Toasty.toml"), TOASTY_CONFIG).unwrap();
    dir
}

#[test]
fn db_setup_generates_a_migration() {
    let dir = project_with_config();

    cli()
        .current_dir(dir.path())
        .args(["db", "setup"])
        .assert()
        .success();

    let migrations: Vec<_> = fs::read_dir(dir.path().join("toasty/migrations"))
        .expect("the migrations directory is created")
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        migrations
            .iter()
            .any(|name| name.starts_with("0000_") && name.ends_with(".sql")),
        "{migrations:?}"
    );
    assert!(dir.path().join("toasty/history.toml").exists());
}

#[test]
fn db_migrate_applies_the_generated_migration() {
    let dir = project_with_config();
    cli()
        .current_dir(dir.path())
        .args(["db", "setup"])
        .assert()
        .success();

    cli()
        .current_dir(dir.path())
        .args(["db", "migrate"])
        .assert()
        .success();
}

// A missing Toasty.toml must fail the command, not exit 0.
#[test]
fn db_commands_fail_without_a_toasty_config() {
    let dir = TempDir::new().unwrap();

    cli()
        .current_dir(dir.path())
        .args(["db", "migrate"])
        .assert()
        .failure();
}

// --- sync -----------------------------------------------------------------------------

use zylotek_domain::woods::{
    entities::Wood, repositories::WoodRepository, value_objects::Measurement,
};
use zylotek_infrastructure::persistence::{build_db, woods::repositories::PostgresWoodRepository};

fn database_in(dir: &TempDir) -> String {
    format!("sqlite:{}", dir.path().join("zylotek.db").display())
}

fn stored_woods(database_url: &str) -> Vec<Wood> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let repository = PostgresWoodRepository::new(build_db(database_url).await.unwrap());
            repository.list(0, 1000).await.unwrap()
        })
}

fn sync_online(database_url: &str) -> Command {
    let mut command = cli();
    command
        .current_dir(WORKSPACE_ROOT)
        .env_remove("ZYLOTEK_DATABASE_URL")
        .args(["--database-url", database_url, "sync", "run"]);
    command
}

/// The CSV alone: the tests must not depend on the live LPF site.
fn sync(database_url: &str) -> Command {
    let mut command = sync_online(database_url);
    command.arg("--offline");
    command
}

/// A stand-in for the LPF site, served from its own thread so the tests can keep using
/// blocking code. It stops when dropped.
struct Site {
    listing_url: String,
    _stop: std::sync::mpsc::Sender<()>,
}

impl Site {
    /// Lists `species` (id and page) on the first listing page and nothing on the others.
    fn with_species(species: &[(u64, &'static str)]) -> Self {
        Self::start(species.to_vec(), 200)
    }

    /// Answers every request with `status`.
    fn failing(status: u16) -> Self {
        Self::start(Vec::new(), status)
    }

    fn start(species: Vec<(u64, &'static str)>, status: u16) -> Self {
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers};

        let (uri_sender, uri) = std::sync::mpsc::channel();
        let (stop, stopped) = std::sync::mpsc::channel::<()>();
        std::thread::spawn(move || {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    let server = MockServer::start().await;
                    let rows: String = species
                        .iter()
                        .map(|(id, _)| {
                            format!(
                                r#"<tr><td><a href="{}/especie?especieestudadaid={id}">Genus species{id}</a></td></tr>"#,
                                server.uri()
                            )
                        })
                        .collect();
                    let table = |rows: &str| {
                        format!(r#"<table id="caracteristicaList"><tbody>{rows}</tbody></table>"#)
                    };
                    let html = |body: String| {
                        ResponseTemplate::new(status).set_body_string(body)
                    };

                    // The first mock that matches wins: page 0, then every other page.
                    Mock::given(matchers::path("/madeiras"))
                        .and(matchers::query_param("start", "0"))
                        .respond_with(html(table(&rows)))
                        .mount(&server)
                        .await;
                    Mock::given(matchers::path("/madeiras"))
                        .respond_with(html(table("")))
                        .mount(&server)
                        .await;
                    for (id, page) in species {
                        Mock::given(matchers::path("/especie"))
                            .and(matchers::query_param("especieestudadaid", id.to_string()))
                            .respond_with(html(page.to_owned()))
                            .mount(&server)
                            .await;
                    }

                    uri_sender.send(server.uri()).unwrap();
                    tokio::task::spawn_blocking(move || stopped.recv()).await.unwrap().ok();
                });
        });

        Self {
            listing_url: format!("{}/madeiras", uri.recv().unwrap()),
            _stop: stop,
        }
    }
}

/// The species page the sync crate's tests use: Astronium lecointei.
const SPECIES_PAGE: &str = include_str!("../../zylotek-sync/tests/fixtures/species.html");

/// A CSV with only the columns this test needs; the importer accepts any subset.
const TWO_SPECIES_CSV: &str = "id_especie;nome_cientifico;nome_popular_1;familia;densidade_basica\r\n\
     21;Astronium lecointei;MUIRACATIARA-RAJADA;Anacardiaceae;0,71\r\n\
     30;Cedrela fissilis;CEDRO;Meliaceae;0,40\r\n";

fn two_species_csv(dir: &TempDir) -> std::path::PathBuf {
    let csv = dir.path().join("two.csv");
    fs::write(&csv, TWO_SPECIES_CSV).unwrap();
    csv
}

#[test]
fn sync_reads_the_bundled_csv_by_default_and_stores_one_wood_per_species() {
    let dir = TempDir::new().unwrap();
    let url = database_in(&dir);

    sync(&url)
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "Read 269 rows from toasty/fixtures/",
        ))
        .stderr(predicate::str::contains("Stored 225 new woods"))
        .stderr(predicate::str::contains(
            "updated 0, left 0 as they were, skipped 44 further lots of a species already read",
        ));

    let woods = stored_woods(&url);
    assert_eq!(woods.len(), 225);
    let first_lot = woods
        .iter()
        .find(|w| w.scientific_name().as_str() == "Astronium lecointei")
        .expect("the species is stored");
    // Two lots share this name; the first in the file (id 21, Belém) wins.
    assert_eq!(first_lot.name().as_str(), "Muiracatiara-rajada");
    assert_eq!(first_lot.details().source.as_ref().unwrap().species_id, 21);
    assert_eq!(
        first_lot.details().collection_location.as_deref(),
        Some("Belém, PA")
    );
}

#[test]
fn syncing_twice_stores_nothing_new() {
    let dir = TempDir::new().unwrap();
    let url = database_in(&dir);
    sync(&url).assert().success();

    sync(&url)
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "updated 0, left 225 as they were, skipped 44",
        ));

    assert_eq!(stored_woods(&url).len(), 225);
}

#[test]
fn bad_rows_are_reported_and_fail_the_run_but_good_rows_are_still_stored() {
    let dir = TempDir::new().unwrap();
    let csv = dir.path().join("woods.csv");
    fs::write(
        &csv,
        "id_especie;nome_cientifico;nome_popular_1;familia\r\n\
         1;Astronium lecointei;MUIRACATIARA;Anacardiaceae\r\n\
         2;Solo;SEM EPITETO;Fabaceae\r\n",
    )
    .unwrap();
    let url = database_in(&dir);

    sync(&url)
        .arg("--file")
        .arg(&csv)
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("Stored 1 new woods"))
        .stderr(predicate::str::contains("rejected line 3 (id 2)"))
        .stderr(predicate::str::contains(
            "1 rows or pages could not be imported",
        ));

    assert_eq!(stored_woods(&url).len(), 1);
}

#[test]
fn a_missing_or_unusable_file_fails_without_touching_the_database() {
    let dir = TempDir::new().unwrap();
    let url = database_in(&dir);
    let not_a_dataset = dir.path().join("other.csv");
    fs::write(&not_a_dataset, "a;b\r\n1;2\r\n").unwrap();

    sync(&url)
        .args(["--file", "nowhere.csv"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("cannot read nowhere.csv"));
    sync(&url)
        .arg("--file")
        .arg(&not_a_dataset)
        .assert()
        .code(1)
        .stderr(predicate::str::contains("lacks the required columns"));

    assert!(!dir.path().join("zylotek.db").exists());
}

#[test]
fn the_old_source_option_is_gone() {
    let dir = TempDir::new().unwrap();

    for source in ["file", "web"] {
        sync(&database_in(&dir))
            .args(["--source", source])
            .assert()
            .code(2)
            .stderr(predicate::str::contains("unexpected argument '--source'"));
    }
}

#[test]
fn offline_and_a_listing_url_contradict_each_other() {
    let dir = TempDir::new().unwrap();

    sync(&database_in(&dir))
        .args(["--listing-url", "http://localhost:1/madeiras"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("cannot be used with"));
}

#[test]
fn the_web_fills_in_what_the_csv_lacks_and_never_overrides_it() {
    let dir = TempDir::new().unwrap();
    let url = database_in(&dir);
    let site = Site::with_species(&[(21, SPECIES_PAGE)]);

    sync_online(&url)
        .arg("--file")
        .arg(two_species_csv(&dir))
        .args(["--listing-url", &site.listing_url])
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "Scraped 1 species pages from the LPF web site (0 failed)",
        ))
        .stderr(predicate::str::contains(
            "Completed 1 species from their web pages; 0 more exist only on the web",
        ))
        .stderr(predicate::str::contains("Stored 2 new woods"));

    let woods = stored_woods(&url);
    let find = |name: &str| {
        woods
            .iter()
            .find(|w| w.scientific_name().as_str() == name)
            .unwrap()
    };
    let astronium = find("Astronium lecointei").details();
    // From the CSV: the row's own name, id and density.
    assert_eq!(
        find("Astronium lecointei").name().as_str(),
        "Muiracatiara-rajada"
    );
    assert_eq!(
        astronium.physical_properties.density.basic.unwrap().value(),
        0.71
    );
    // From the page: what the CSV has no column for.
    assert!(astronium.images.main.is_some());
    assert!(!astronium.common_names.is_empty());
    assert_eq!(
        astronium
            .source
            .as_ref()
            .unwrap()
            .page_url
            .as_str()
            .split('?')
            .next()
            .unwrap(),
        format!("{}/especie", site.listing_url.trim_end_matches("/madeiras"))
    );
    // A species the page does not cover stays as the CSV has it.
    let cedrela = find("Cedrela fissilis").details();
    assert!(cedrela.images.main.is_none());
    assert_eq!(cedrela.source.as_ref().unwrap().species_id, 30);
}

#[test]
fn syncing_again_updates_a_stored_wood_and_never_forgets_what_an_earlier_run_learned() {
    let dir = TempDir::new().unwrap();
    let url = database_in(&dir);
    let csv = two_species_csv(&dir);
    let image_of = |name: &str| {
        stored_woods(&url)
            .iter()
            .find(|w| w.scientific_name().as_str() == name)
            .unwrap()
            .details()
            .images
            .main
            .clone()
    };

    // First the CSV alone: no images.
    sync(&url).arg("--file").arg(&csv).assert().success();
    assert!(image_of("Astronium lecointei").is_none());

    // A later run with the site completes the stored wood instead of skipping it.
    let site = Site::with_species(&[(21, SPECIES_PAGE)]);
    sync_online(&url)
        .arg("--file")
        .arg(&csv)
        .args(["--listing-url", &site.listing_url])
        .assert()
        .success()
        .stderr(predicate::str::contains("updated 1, left 1 as they were"));
    let image = image_of("Astronium lecointei");
    assert!(image.is_some());
    assert_eq!(stored_woods(&url).len(), 2);

    // And a run that cannot reach the site does not wipe it.
    sync(&url).arg("--file").arg(&csv).assert().success();
    assert_eq!(image_of("Astronium lecointei"), image);
}

#[test]
fn changed_csv_values_replace_the_stored_ones() {
    let dir = TempDir::new().unwrap();
    let url = database_in(&dir);
    let csv = two_species_csv(&dir);
    sync(&url).arg("--file").arg(&csv).assert().success();
    fs::write(&csv, TWO_SPECIES_CSV.replace("0,71", "0,95")).unwrap();

    sync(&url)
        .arg("--file")
        .arg(&csv)
        .assert()
        .success()
        .stderr(predicate::str::contains("updated 1, left 1 as they were"));

    let woods = stored_woods(&url);
    let astronium = woods
        .iter()
        .find(|w| w.scientific_name().as_str() == "Astronium lecointei")
        .unwrap();
    assert_eq!(
        astronium
            .details()
            .physical_properties
            .density
            .basic
            .unwrap()
            .value(),
        0.95
    );
}

#[test]
fn species_only_the_web_has_are_stored_too() {
    let dir = TempDir::new().unwrap();
    let url = database_in(&dir);
    let csv = dir.path().join("cedro.csv");
    fs::write(
        &csv,
        "id_especie;nome_cientifico;nome_popular_1\r\n30;Cedrela fissilis;CEDRO\r\n",
    )
    .unwrap();
    let site = Site::with_species(&[(21, SPECIES_PAGE)]);

    sync_online(&url)
        .arg("--file")
        .arg(&csv)
        .args(["--listing-url", &site.listing_url])
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "Completed 0 species from their web pages; 1 more exist only on the web",
        ));

    let names: Vec<_> = stored_woods(&url)
        .iter()
        .map(|w| w.scientific_name().to_string())
        .collect();
    assert_eq!(names.len(), 2, "{names:?}");
    assert!(names.contains(&"Astronium lecointei".to_owned()));
}

#[test]
fn a_site_that_fails_is_reported_but_the_csv_is_still_stored() {
    let dir = TempDir::new().unwrap();
    let url = database_in(&dir);
    let site = Site::failing(503);

    sync_online(&url)
        .arg("--file")
        .arg(two_species_csv(&dir))
        .args(["--listing-url", &site.listing_url])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("Stored 2 new woods"))
        .stderr(predicate::str::contains("web page failed"))
        .stderr(predicate::str::contains("503"))
        .stderr(predicate::str::contains("could not be imported"));

    let woods = stored_woods(&url);
    assert_eq!(woods.len(), 2);
    assert!(woods.iter().all(|w| w.details().images.main.is_none()));
}

#[test]
fn the_database_url_can_come_from_the_environment() {
    let dir = TempDir::new().unwrap();
    let url = database_in(&dir);

    cli()
        .current_dir(WORKSPACE_ROOT)
        .env("ZYLOTEK_DATABASE_URL", &url)
        .args(["sync", "run", "--offline"])
        .assert()
        .success();

    assert_eq!(stored_woods(&url).len(), 225);
}

// --- OAuth -----------------------------------------------------------------------------

fn oauth_files(dir: &TempDir) -> (String, String) {
    (
        dir.path().join("key.pem").display().to_string(),
        dir.path().join("clients.json").display().to_string(),
    )
}

fn auth(dir: &TempDir) -> Command {
    let (key, clients) = oauth_files(dir);
    let mut command = cli();
    command
        .current_dir(dir.path())
        .env_remove("ZYLOTEK_OAUTH_SIGNING_KEY")
        .env_remove("ZYLOTEK_OAUTH_CLIENTS")
        .args(["--oauth-signing-key", &key, "--oauth-clients", &clients]);
    command
}

#[test]
fn auth_keygen_writes_a_private_key_and_refuses_to_overwrite_it() {
    let dir = TempDir::new().unwrap();
    let (key, _) = oauth_files(&dir);

    auth(&dir)
        .args(["auth", "keygen"])
        .assert()
        .success()
        .stderr(predicate::str::contains("Wrote a new signing key"));

    let pem = fs::read_to_string(&key).unwrap();
    assert!(pem.starts_with("-----BEGIN PRIVATE KEY-----"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&key).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    auth(&dir)
        .args(["auth", "keygen"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("refusing to overwrite"));
    assert_eq!(
        fs::read_to_string(&key).unwrap(),
        pem,
        "the key is untouched"
    );
}

#[test]
fn auth_client_create_shows_the_secret_once_and_stores_only_its_hash() {
    let dir = TempDir::new().unwrap();
    let (_, clients) = oauth_files(&dir);
    let output = auth(&dir)
        .args(["auth", "client", "create", "assistant"])
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "Created client assistant (woods:write)",
        ))
        .get_output()
        .stderr
        .clone();
    let stderr = String::from_utf8(output).unwrap();
    // Each log line carries a timestamp and level before the message.
    let secret = stderr
        .lines()
        .find_map(|line| line.split_once("client_secret: ").map(|(_, rest)| rest))
        .expect("the secret is printed")
        .trim()
        .to_owned();

    let stored = fs::read_to_string(&clients).unwrap();
    assert_eq!(secret.len(), 43);
    assert!(!stored.contains(&secret), "only the hash is stored");
    assert!(stored.contains("$argon2id$"));
    auth(&dir)
        .args(["auth", "client", "list"])
        .assert()
        .success()
        .stderr(predicate::str::contains("assistant\twoods:write"))
        .stderr(predicate::str::contains(&secret).not());
}

#[test]
fn auth_client_create_validates_and_never_replaces_a_client() {
    let dir = TempDir::new().unwrap();
    auth(&dir)
        .args([
            "auth",
            "client",
            "create",
            "bot",
            "-s",
            "woods:write",
            "-s",
            "asks:write",
        ])
        .assert()
        .success();

    auth(&dir)
        .args(["auth", "client", "create", "bot"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("already exists"));
    auth(&dir)
        .args(["auth", "client", "create", "has space"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("invalid client id"));
    auth(&dir)
        .args(["auth", "client", "create", "other", "--scope", "WRITE"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("invalid value 'WRITE'"));
    auth(&dir)
        .args(["auth", "client", "list"])
        .assert()
        .success()
        .stderr(predicate::str::contains("bot\twoods:write asks:write"))
        .stderr(predicate::str::contains("other").not());
}

#[test]
fn auth_client_list_says_so_when_there_are_no_clients() {
    let dir = TempDir::new().unwrap();

    auth(&dir)
        .args(["auth", "client", "list"])
        .assert()
        .success()
        .stderr(predicate::str::contains("No clients yet"));
}
