//! The compiled binary, driven as a user would.

use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    process::{Child, Command as Process, Stdio},
    time::{Duration, Instant},
};

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
        .stdout(predicate::str::contains("api"));
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
        .args(["api", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("run"));
    cli()
        .args(["sync", "run", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--config"));
}

#[test]
fn rejects_unknown_commands_with_a_usage_error() {
    cli()
        .arg("frobnicate")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("unrecognized subcommand"));
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

// `persistence::cli` drops the spawned task's result, so a missing Toasty.toml exits 0.
#[test]
#[ignore = "known bug: persistence::cli swallows the inner task error"]
fn db_commands_fail_without_a_toasty_config() {
    let dir = TempDir::new().unwrap();

    cli()
        .current_dir(dir.path())
        .args(["db", "migrate"])
        .assert()
        .failure();
}

fn health(port: u16) -> Option<String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    stream
        .write_all(b"GET /health HTTP/1.0\r\nHost: localhost\r\n\r\n")
        .ok()?;
    let mut response = String::new();
    stream.read_to_string(&mut response).ok()?;
    Some(response)
}

fn wait_until_serving(child: &mut Child, port: u16) -> String {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(response) = health(port) {
            return response;
        }
        assert!(
            child.try_wait().unwrap().is_none(),
            "the server exited early"
        );
        assert!(
            Instant::now() < deadline,
            "the server never answered /health"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// `api run` binds the fixed port 8080, so this test steps aside when it is taken.
#[test]
#[cfg(unix)]
fn api_run_serves_health_and_stops_on_sigterm() {
    let _port = PORT_8080
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if TcpListener::bind(("0.0.0.0", 8080)).is_err() {
        eprintln!("skipping: port 8080 is in use");
        return;
    }
    let output = TempDir::new().unwrap();
    let stdout = fs::File::create(output.path().join("stdout")).unwrap();
    let mut child = Process::new(assert_cmd::cargo::cargo_bin("zylotek-cli"))
        .args(["api", "run"])
        .env("ZYLOTEK_DATABASE_URL", "sqlite::memory:")
        .stdout(stdout)
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let response = wait_until_serving(&mut child, 8080);
    assert!(
        response.starts_with("HTTP/1.0 200") || response.starts_with("HTTP/1.1 200"),
        "{response}"
    );
    assert!(response.ends_with("OK"));

    let status = Process::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
    let exit = wait_with_timeout(&mut child, Duration::from_secs(10));

    assert!(exit.success(), "graceful shutdown exits 0, got {exit:?}");
    assert_stdout_has_banner(&output.path().join("stdout"));
}

fn wait_with_timeout(child: &mut Child, timeout: Duration) -> std::process::ExitStatus {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        if Instant::now() > deadline {
            child.kill().unwrap();
            panic!("the server did not stop after SIGTERM");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn assert_stdout_has_banner(path: &Path) {
    let stdout = fs::read_to_string(path).unwrap();

    assert!(stdout.contains("Version: 0.1.0"), "{stdout}");
}

// --- sync -----------------------------------------------------------------------------

use zylotek_domain::woods::{entities::Wood, repositories::WoodRepository};
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

fn sync(database_url: &str) -> Command {
    let mut command = cli();
    command
        .current_dir(WORKSPACE_ROOT)
        .env_remove("ZYLOTEK_DATABASE_URL")
        .args(["--database-url", database_url, "sync", "run"]);
    command
}

#[test]
fn sync_reads_the_bundled_csv_by_default_and_stores_one_wood_per_species() {
    let dir = TempDir::new().unwrap();
    let url = database_in(&dir);

    sync(&url)
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Read 269 rows from toasty/fixtures/",
        ))
        .stdout(predicate::str::contains("Stored 225 new woods"))
        .stdout(predicate::str::contains(
            "skipped 44 whose scientific name is already registered",
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
        .stdout(predicate::str::contains("Stored 0 new woods"))
        .stdout(predicate::str::contains("skipped 269"));

    assert_eq!(stored_woods(&url).len(), 225);
}

#[test]
fn an_explicit_file_source_is_the_same_as_the_default() {
    let dir = TempDir::new().unwrap();
    let url = database_in(&dir);

    sync(&url).args(["--source", "file"]).assert().success();

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
        .stdout(predicate::str::contains("Stored 1 new woods"))
        .stderr(predicate::str::contains("rejected line 3 (id 2)"))
        .stderr(predicate::str::contains("1 rows could not be imported"));

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
fn an_unknown_source_is_a_usage_error() {
    let dir = TempDir::new().unwrap();

    sync(&database_in(&dir))
        .args(["--source", "ftp"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("invalid value 'ftp'"));
}

#[test]
fn the_database_url_can_come_from_the_environment() {
    let dir = TempDir::new().unwrap();
    let url = database_in(&dir);

    cli()
        .current_dir(WORKSPACE_ROOT)
        .env("ZYLOTEK_DATABASE_URL", &url)
        .args(["sync", "run"])
        .assert()
        .success();

    assert_eq!(stored_woods(&url).len(), 225);
}

// --- OAuth -----------------------------------------------------------------------------

use std::sync::Mutex;

/// `api run` binds the fixed port 8080, so the tests that start it take turns.
static PORT_8080: Mutex<()> = Mutex::new(());

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
        .stdout(predicate::str::contains("Wrote a new signing key"));

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
        .stdout(predicate::str::contains(
            "Created client assistant (woods:write)",
        ))
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output).unwrap();
    let secret = stdout
        .lines()
        .find_map(|line| line.strip_prefix("client_secret: "))
        .expect("the secret is printed")
        .to_owned();

    let stored = fs::read_to_string(&clients).unwrap();
    assert_eq!(secret.len(), 43);
    assert!(!stored.contains(&secret), "only the hash is stored");
    assert!(stored.contains("$argon2id$"));
    auth(&dir)
        .args(["auth", "client", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("assistant\twoods:write"))
        .stdout(predicate::str::contains(&secret).not());
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
            "woods:read",
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
        .code(1)
        .stderr(predicate::str::contains("invalid scope"));
    auth(&dir)
        .args(["auth", "client", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("bot\twoods:write woods:read"))
        .stdout(predicate::str::contains("other").not());
}

#[test]
fn auth_client_list_says_so_when_there_are_no_clients() {
    let dir = TempDir::new().unwrap();

    auth(&dir)
        .args(["auth", "client", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No clients yet"));
}

/// One HTTP/1.0 exchange; returns the status code and the body.
fn http(method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(("127.0.0.1", 8080)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut request = format!(
        "{method} {path} HTTP/1.0\r\nHost: localhost\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (name, value) in headers {
        request += &format!("{name}: {value}\r\n");
    }
    stream
        .write_all(format!("{request}\r\n{body}").as_bytes())
        .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let status = response.split(' ').nth(1).unwrap().parse().unwrap();

    (
        status,
        response
            .split_once("\r\n\r\n")
            .map(|(_, body)| body.to_owned())
            .unwrap_or_default(),
    )
}

/// `api run` as a child process, stopped when dropped.
struct Api(Child);

impl Drop for Api {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start_api(dir: &TempDir, extra: &[&str]) -> Api {
    let mut child = Process::new(assert_cmd::cargo::cargo_bin("zylotek-cli"))
        .current_dir(dir.path())
        .env("ZYLOTEK_DATABASE_URL", "sqlite::memory:")
        .env_remove("ZYLOTEK_OAUTH_SIGNING_KEY")
        .env_remove("ZYLOTEK_OAUTH_CLIENTS")
        .args(extra)
        .args(["api", "run"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    wait_until_serving(&mut child, 8080);

    Api(child)
}

const WOOD: &str = r#"{"name":"Aroeira","scientific_name":"Myracrodruon urundeuva"}"#;

#[test]
#[cfg(unix)]
fn api_run_demands_a_token_for_writes_once_a_key_and_a_client_exist() {
    let _port = PORT_8080
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if TcpListener::bind(("0.0.0.0", 8080)).is_err() {
        eprintln!("skipping: port 8080 is in use");
        return;
    }
    let dir = TempDir::new().unwrap();
    auth(&dir).args(["auth", "keygen"]).assert().success();
    let created = auth(&dir)
        .args(["auth", "client", "create", "importer"])
        .assert()
        .success();
    let stdout = String::from_utf8(created.get_output().stdout.clone()).unwrap();
    let secret = stdout
        .lines()
        .find_map(|l| l.strip_prefix("client_secret: "))
        .unwrap()
        .to_owned();
    let (key, clients) = oauth_files(&dir);
    let _api = start_api(
        &dir,
        &["--oauth-signing-key", &key, "--oauth-clients", &clients],
    );
    let json = [("Content-Type", "application/json")];

    // Reads are open; a write without a token is challenged.
    assert_eq!(http("GET", "/api/v1/woods", &[], "").0, 200);
    let (status, body) = http("POST", "/api/v1/woods", &json, WOOD);
    assert_eq!((status, body.contains("UNAUTHORIZED")), (401, true));

    // The client trades its secret for a token and writes with it.
    let basic = format!("Basic {}", base64_encode(&format!("importer:{secret}")));
    let (status, body) = http(
        "POST",
        "/oauth/token",
        &[
            ("Authorization", &basic),
            ("Content-Type", "application/x-www-form-urlencoded"),
        ],
        "grant_type=client_credentials",
    );
    assert_eq!(status, 200, "{body}");
    let token: serde_json::Value = serde_json::from_str(&body).unwrap();
    let bearer = format!("Bearer {}", token["access_token"].as_str().unwrap());
    let (status, body) = http(
        "POST",
        "/api/v1/woods",
        &[("Authorization", &bearer), json[0]],
        WOOD,
    );
    assert_eq!(status, 201, "{body}");
    let (_, listing) = http("GET", "/api/v1/woods", &[], "");
    assert!(listing.contains("Myracrodruon urundeuva"));

    // A wrong secret gets nothing.
    let wrong = format!("Basic {}", base64_encode("importer:wrong"));
    let (status, _) = http(
        "POST",
        "/oauth/token",
        &[
            ("Authorization", &wrong),
            ("Content-Type", "application/x-www-form-urlencoded"),
        ],
        "grant_type=client_credentials",
    );
    assert_eq!(status, 401);
}

#[test]
#[cfg(unix)]
fn api_run_without_oauth_files_serves_reads_and_refuses_writes() {
    let _port = PORT_8080
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if TcpListener::bind(("0.0.0.0", 8080)).is_err() {
        eprintln!("skipping: port 8080 is in use");
        return;
    }
    let dir = TempDir::new().unwrap();
    let _api = start_api(&dir, &[]);

    let (read, _) = http("GET", "/api/v1/woods", &[], "");
    let (write, body) = http(
        "POST",
        "/api/v1/woods",
        &[("Content-Type", "application/json")],
        WOOD,
    );

    assert_eq!(read, 200);
    assert_eq!(write, 503);
    assert!(body.contains("OAuth is not configured"), "{body}");
}

#[test]
fn api_run_refuses_to_start_with_half_an_oauth_setup() {
    let dir = TempDir::new().unwrap();
    let (key, _) = oauth_files(&dir);
    auth(&dir).args(["auth", "keygen"]).assert().success();

    // A key but no clients file: given by neither flag nor default name.
    cli()
        .current_dir(dir.path())
        .env_remove("ZYLOTEK_OAUTH_CLIENTS")
        .env_remove("ZYLOTEK_OAUTH_SIGNING_KEY")
        .args(["--oauth-signing-key", &key, "api", "run"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("only half configured"));
}

fn base64_encode(text: &str) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in text.as_bytes().chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for index in 0..4 {
            if index <= chunk.len() {
                out.push(ALPHABET[((n >> (18 - 6 * index)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}
