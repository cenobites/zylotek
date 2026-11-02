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
    if TcpListener::bind(("0.0.0.0", 8080)).is_err() {
        eprintln!("skipping: port 8080 is in use");
        return;
    }
    let output = TempDir::new().unwrap();
    let stdout = fs::File::create(output.path().join("stdout")).unwrap();
    let mut child = Process::new(assert_cmd::cargo::cargo_bin("zylotek-cli"))
        .args(["api", "run"])
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
