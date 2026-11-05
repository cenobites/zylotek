# Testing

Four layers, each with a `make` target. `make help` lists them all.

| Layer       | What it proves                                         | Where                                  | Run                     |
|-------------|--------------------------------------------------------|----------------------------------------|-------------------------|
| Unit        | Each function and type, in isolation, no I/O           | `#[cfg(test)] mod tests` beside code   | `make test-unit`        |
| Integration | Real SQLite, real sockets, the compiled CLI            | `crates/*/tests/`                      | `make test-integration` |
| Benchmarks  | The hot paths don't regress                            | `crates/*/benches/` (criterion)        | `make bench`            |
| Fuzz        | The flows that take untrusted input never panic        | `fuzz/fuzz_targets/` (cargo-fuzz)      | `make fuzz`             |

## Coverage policy

`make cov` enforces two gates with `cargo-llvm-cov` on stable Rust:

- **Unit: 100% of lines** (`make cov-unit`), measured from `--lib` tests only. One line is
  allowed to stay uncovered: the unreachable `else` in `basic_information.rs` (a
  `div > a > img` always has an element grandparent).
- **Integration: at least 80% of lines** (`make cov-integration`), measured from `tests/`
  only. It sits around 93%. The CLI's web step is tested against a stand-in LPF site
  (`Site` in `crates/zylotek-cli/tests/cli.rs`), never the live one; the other `sync` tests
  pass `--offline`.

Files excluded from the unit gate, and why (the regex lives in the `Makefile`):

| File                                  | Reason                                                       |
|---------------------------------------|--------------------------------------------------------------|
| `testing.rs`, `test_support.rs`       | Test support, not product code                               |
| `persistence/woods/models.rs`         | Derive-generated Toasty schema, no logic                     |
| `zylotek-api/src/server.rs`           | Binds port 8080 and waits for OS signals                     |
| `persistence/migrations.rs`           | Needs a real `Toasty.toml` in the working directory          |
| `zylotek-cli`, `src/main.rs`          | Thin binaries, tested through the compiled binary            |

`server.rs` and `migrations.rs` are *measured* by the integration gate. Keep entry-point
code in those files so the unit gate stays honest: don't add logic to them.

## Test support

Crates share fixtures through `testing` modules, compiled only for tests or behind a
`testing` cargo feature (never enable it in a production dependency):

- `zylotek_domain::testing`: `full_details()` (every field populated, so round-trip tests
  touch everything), `wood()`, and `InMemoryWoodRepository` with per-operation fault
  injection (`.fail_on(Operation::Save)`).
- `zylotek_api::auth::testing` (feature `testing`): an enabled `Auth` built once per process
  from a freshly generated key, plus `write_token()`, `read_only_token()` and the writer
  client's credentials. No key is ever committed.
- `zylotek_infrastructure::testing`: `RecordingEventPublisher`, `FailingEventPublisher`,
  `memory_db()`.
- `toasty/fixtures/*.csv`: the real open-data export. `crates/zylotek-sync/tests/open_data.rs`
  parses all 269 rows and checks facts measured independently with Python (225 distinct
  species, 52 synonyms, SI to kgf conversions of a known row); `crates/zylotek-cli/tests/cli.rs`
  imports it into a temp SQLite file and reads the database back.
- `crates/zylotek-sync/tests/fixtures/species.html`: a species page in the markup the
  parsers expect. Tests mutate it (`mutated(from, to)`) to build failure cases. It is
  hand-written, not captured from the site; if the LPF markup changes, update it from a
  real page.

## Conventions

- Unit tests use the in-memory repository, so they never touch SQLite or the network.
- Integration tests use the real stack: `build_state` + the SQLite repository, wiremock for
  HTTP, `assert_cmd` for the binary. Don't bind fixed ports in them (the `zylotek-api` binary
  tests in `crates/zylotek-api/tests/binary.rs` are the one exception and skip themselves if
  8080 is busy), and don't reuse a dropped `MockServer`'s
  address as a "dead" address: a parallel test can take the port.
- Response-shape contracts use `insta` snapshots in `crates/zylotek-api/tests/snapshots/`.
  Review changes with `cargo insta review`.
- A test for a known bug is `#[ignore = "known bug: …"]` and asserts the *correct*
  behaviour, so fixing the bug turns it green. Remove the `#[ignore]` then.

## Fuzzing

Needs nightly (`rustup toolchain install nightly`) and `cargo install cargo-fuzz`.

| Target              | Property                                                              |
|---------------------|-----------------------------------------------------------------------|
| `parse_wood`        | Any HTML yields `Ok` or `Err`, never a panic                          |
| `parse_open_data`   | Any bytes yield a parse result or error; parsed woods are valid       |
| `wood_request_json` | Any JSON body is rejected or converts to values that re-validate      |
| `value_objects`     | Constructors never panic; accepted values are idempotent              |
| `http_requests`     | Any request is answered, never with a 5xx, and no write succeeds without a valid token |
| `oauth_requests`    | Junk `Authorization` headers and token requests are never accepted    |
| `webapp_input`      | Any compare query or wood JSON is handled; accepted ids and links are safe; escaping is lossless |
| `model_reply`       | Any model reply parses within bounds; any figures rank with scores in 0..1, in order |

```bash
make fuzz                                   # every target, 60s each
make fuzz FUZZ_TIME=300 FUZZ_TARGETS=parse_wood
cd fuzz && cargo +nightly fuzz run http_requests
```

Crashes land in `fuzz/artifacts/` (git-ignored). Minimize one with
`cargo +nightly fuzz tmin <target> <file>`, turn it into a regular test, and keep the input
under `fuzz/known-crashes/` while the bug is open. Hand-written seed inputs live in
`fuzz/seeds/` (`make fuzz` passes them as extra corpus); the fuzzer's own `fuzz/corpus/` is
git-ignored.

## Known bugs

Found while writing these tests. None is open now. An open one gets an `#[ignore]`d test
(`make test-known-bugs` runs them, and they fail until the bug is fixed).

1. ~~**The scraper panics on malformed pages.**~~ Fixed: a page with no `<table>`, a heading
   without its table, a short row or an `<img>` without `src` is now an error naming what is
   missing, never a panic (`crates/zylotek-sync/tests/malformed_pages.rs`, plus unit tests next
   to each parser). Found by fuzzing; the crashing page is now a seed in
   `fuzz/seeds/parse_wood/`.
2. ~~**`persistence::cli` swallows errors.**~~ Fixed: it returns the spawned task's `Result`, so
   `db setup` / `db migrate` fail when `Toasty.toml` is missing or a migration fails.
3. ~~`build_db` can't reopen a database.~~ Fixed: it now pushes the schema only when the
   tables are missing (`tests/file_database.rs`).
4. ~~**Duplicate-name races return 500.**~~ Fixed: a create or update that loses the race for a
   scientific name is `RepositoryError::Duplicate`, answered with 422 like the checked case.
5. ~~**The main image can be wrong.**~~ Fixed: the main image is the page's own picture (not a
   gallery photo wrapped in a link), and a page without one is an error instead of silently
   reusing the tree photo.
