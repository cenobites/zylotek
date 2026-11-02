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
  only. It sits around 93%. The deliberate hole is `sync run --source web`, which would hit
  the live LPF site; the CSV source is covered end to end.

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
  HTTP, `assert_cmd` for the binary. Don't bind fixed ports in them (`api run` is the one
  exception and skips itself if 8080 is busy), and don't reuse a dropped `MockServer`'s
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
| `http_requests`     | Any request to the router is answered, never with a 5xx               |

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

Found while writing these tests. Each open one has an `#[ignore]`d test
(`make test-known-bugs` runs them, and they fail until the bug is fixed).

1. **The scraper panics on malformed pages.** `parse_wood` unwraps or indexes page content in
   `workabilities.rs` (lines 106, 171, 190) and `selectors.rs` (111): a page with no
   `<table>`, a heading without its table, or a short row aborts the task. Found by
   fuzzing; see `crates/zylotek-sync/tests/malformed_pages.rs`.
2. **`persistence::cli` swallows errors.** It discards the spawned task's `Result`, so
   `db setup` / `db migrate` exit 0 when `Toasty.toml` is missing or a migration fails.
3. ~~`build_db` can't reopen a database.~~ Fixed: it now pushes the schema only when the
   tables are missing (`tests/file_database.rs`).
4. **Duplicate-name races return 500.** Two concurrent creates of one scientific name: the
   loser hits the unique index and gets `RepositoryError::Database` → 500. The unused
   `RepositoryError::Duplicate` looks intended for this. Exactly one wood is stored.
5. **The main image can be wrong.** The parser takes the first `<img>` in the page as the
   main image, so a page without one silently reuses the tree photo. Pinned by
   `the_main_image_is_simply_the_first_image_on_the_page` as a characterization test.
