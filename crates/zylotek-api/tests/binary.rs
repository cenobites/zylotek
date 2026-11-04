//! The compiled `zylotek-api` binary, driven as an operator would.
//!
//! It binds the fixed port 8080, so these tests take turns and step aside when it is taken.

use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    process::{Child, Command, Stdio},
    sync::Mutex,
    time::{Duration, Instant},
};

use tempfile::TempDir;
use zylotek_api::auth::{WRITE_SCOPE, create_client, write_new_signing_key};

const API: &str = env!("CARGO_BIN_EXE_zylotek-api");
const PORT: u16 = 8080;
const WOOD: &str = r#"{"name":"Aroeira","scientific_name":"Myracrodruon urundeuva"}"#;

static PORT_8080: Mutex<()> = Mutex::new(());

/// Holds the port for the test, or `None` when something else already listens on it.
fn take_port() -> Option<std::sync::MutexGuard<'static, ()>> {
    let guard = PORT_8080
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if TcpListener::bind(("0.0.0.0", PORT)).is_err() {
        eprintln!("skipping: port {PORT} is in use");
        return None;
    }
    Some(guard)
}

fn api(dir: &TempDir) -> Command {
    let mut command = Command::new(API);
    command
        .current_dir(dir.path())
        .env("ZYLOTEK_DATABASE_URL", "sqlite::memory:")
        .env_remove("ZYLOTEK_OAUTH_SIGNING_KEY")
        .env_remove("ZYLOTEK_OAUTH_CLIENTS")
        .env_remove("ZYLOTEK_OAUTH_ISSUER");
    command
}

fn health() -> Option<String> {
    let mut stream = TcpStream::connect(("127.0.0.1", PORT)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    stream
        .write_all(b"GET /health HTTP/1.0\r\nHost: localhost\r\n\r\n")
        .ok()?;
    let mut response = String::new();
    stream.read_to_string(&mut response).ok()?;
    Some(response)
}

fn wait_until_serving(child: &mut Child) -> String {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(response) = health() {
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

/// The API as a child process, stopped when dropped.
struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start(dir: &TempDir, extra: &[&str]) -> Running {
    let mut child = api(dir)
        .args(extra)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    wait_until_serving(&mut child);

    Running(child)
}

/// One HTTP/1.0 exchange; returns the status code and the body.
fn http(method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(("127.0.0.1", PORT)).unwrap();
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

#[test]
fn prints_its_version() {
    let output = Command::new(API).arg("--version").output().unwrap();

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).starts_with("zylotek-api 0.1.0"));
}

#[test]
#[cfg(unix)]
fn serves_health_and_stops_on_sigterm() {
    let Some(_port) = take_port() else { return };
    let dir = TempDir::new().unwrap();
    let stdout = fs::File::create(dir.path().join("stdout")).unwrap();
    let mut child = api(&dir)
        .stdout(stdout)
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let response = wait_until_serving(&mut child);
    assert!(
        response.starts_with("HTTP/1.0 200") || response.starts_with("HTTP/1.1 200"),
        "{response}"
    );
    assert!(response.ends_with("OK"));

    let status = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
    let exit = wait_with_timeout(&mut child, Duration::from_secs(10));

    assert!(exit.success(), "graceful shutdown exits 0, got {exit:?}");
    assert_stdout_has_banner(&dir.path().join("stdout"));
}

#[test]
#[cfg(unix)]
fn demands_a_token_for_writes_once_a_key_and_a_client_exist() {
    let Some(_port) = take_port() else { return };
    let dir = TempDir::new().unwrap();
    let key = dir.path().join("key.pem");
    let clients = dir.path().join("clients.json");
    write_new_signing_key(&key).unwrap();
    let secret = create_client(&clients, "importer", &[WRITE_SCOPE.to_owned()]).unwrap();
    let _api = start(
        &dir,
        &[
            "--oauth-signing-key",
            key.to_str().unwrap(),
            "--oauth-clients",
            clients.to_str().unwrap(),
        ],
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
fn without_oauth_files_serves_reads_and_refuses_writes() {
    let Some(_port) = take_port() else { return };
    let dir = TempDir::new().unwrap();
    let _api = start(&dir, &[]);

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
fn refuses_to_start_with_half_an_oauth_setup() {
    let dir = TempDir::new().unwrap();
    let key = dir.path().join("key.pem");
    write_new_signing_key(&key).unwrap();

    // A key but no clients file: given by neither flag nor default name.
    let output = api(&dir)
        .args(["--oauth-signing-key", key.to_str().unwrap()])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("only half configured"));
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

#[test]
fn a_config_file_that_is_missing_or_malformed_stops_the_binary() {
    let dir = TempDir::new().unwrap();
    let bad = dir.path().join("bad.toml");
    fs::write(&bad, "[database\n").unwrap();
    for (path, message) in [
        (dir.path().join("missing.toml"), "cannot read config file"),
        (bad, "invalid config file"),
    ] {
        let out = Command::new(API)
            .arg("--config-file")
            .arg(&path)
            .output()
            .unwrap();
        assert!(!out.status.success());
        assert!(String::from_utf8_lossy(&out.stderr).contains(message));
    }
}
