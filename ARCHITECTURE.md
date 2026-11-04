# Architecture

This document describes the high-level structure of Zylotek: what each crate is for,
where the important boundaries are, and which invariants hold across the code. It is
meant to be read once, before diving into the source. Details that change often are
deliberately left out; search for the named types instead.

## Bird's Eye View

Zylotek collects data about Brazilian wood species and serves it. There are two flows:

1. **Ingest.** `zylotek-sync` turns the LPF data into domain value objects (`ScrapedWood`),
   either by parsing the open-data CSV (the default) or by fetching the catalogue pages over
   HTTP. The CLI then stores them through the `ImportWoods` use case.
2. **Serve.** `zylotek-webapp` shows it to visitors, with `zylotek-assistant` answering
   questions; `zylotek-api` exposes CRUD on `/api/v1/woods`. Requests are validated by the
   domain's value objects, stored through Toasty (SQLite), and every change emits a
   `WoodEvent` that an asynchronous handler consumes.

The code follows Domain-Driven Design in a hexagonal layout: the model in the middle,
I/O at the edges, dependencies pointing inward.

```
zylotek-api ──► zylotek-application ──► zylotek-domain ◄── zylotek-infrastructure
  (axum)           (use cases)          (model, ports)       (Toasty, event bus)
     │                    │                   ▲                      ▲
     └────────────────────┴── also depend on ─┼──────────────────────┘
                              (api: build_db, │repositories, event bus;
                               application: event-bus traits)
                                              │
zylotek-cli ──► zylotek-sync ─────────────────┘  (value objects only)
            └─► zylotek-infrastructure           (db migrations)
```

## Code Map

### `crates/zylotek-domain`

The model. Organised per aggregate (only `woods/` today):

- `entities` — the `Wood` aggregate root and `WoodId`. A `Wood` is its identity
  (`WoodName`, `ScientificName`) plus a `WoodDetails` value object. Fields are private;
  state changes go through `create`, `update` and `delete`, which return the `WoodEvent`
  they imply. `restore` rebuilds a wood from storage without an event.
- `value_objects/` — immutable, self-validating types with public fields:
  - `names`: `WoodName`, `ScientificName` (genus + epithet, exposes `genus()`),
    `BotanicalFamily`, `CommonNames` (trimmed, de-duplicated).
  - `measurements`: the `Measurement` trait and unit types — `GramsPerCubicCentimeter`,
    `Percentage`, `KgfPerSquareCentimeter`, `KgfPerCentimeter`, `Kgf`, `Days`.
  - `physical_properties`, `mechanical_properties` (per `MoistureCondition`),
    `workabilities` (rating enums), and `details` (`WoodDetails`, `WoodDescription`,
    `KilnDrying`, `WoodImages`, `SourceReference`, `Attributes`, `WebUrl`).
- `services` — `WoodCreationService` and `WoodUpdateService`, which enforce the one rule
  that needs the repository: a scientific name is registered once.
- `repositories` — the `WoodRepository` port (find, list, count, save, update, delete).
- `events`, `errors`.

**Architecture invariant:** this crate performs no I/O and depends only on `async-trait`,
`thiserror` and `uuid`. Any value that reaches a `Wood` has passed through a value-object
constructor, so an invalid wood cannot be built — not from HTTP, storage, or scraping.

### `crates/zylotek-application`

Use cases, one struct per operation with a single `execute`: `CreateWood`, `GetWood`,
`ListWoods` (clamps `limit` to 1..=100), `UpdateWood` (full replacement), `DeleteWood`.
Each holds its collaborators as trait objects (`Arc<dyn WoodRepository>`,
`Arc<dyn EventPublisher<WoodEvent>>`). Commands carry raw names plus an already-built
`WoodDetails`; the use case turns the names into value objects.

`ImportWoods` upserts a batch of woods keyed on the scientific name
(`WoodRepository::find_by_scientific_name`): an unknown name goes through `CreateWood`, a
stored one through `UpdateWood` with the stored details filling whatever the new ones lack
(`WoodDetails::fill_gaps_from`, in the domain). A wood that would not change is left
alone, so importing twice is harmless; a name repeated within the batch keeps its first
occurrence; any other failure is recorded without stopping the batch. It returns an
`ImportReport` (created, updated, unchanged, duplicates, failed).

`WoodFacade` is the entry point adapters call. `WoodEventHandler` handles every
`WoodEvent`; it currently only logs. All use cases return `WoodUseCaseError`
(`NotFound`, `Domain`, `Persistence`, `Event`).

### `crates/zylotek-infrastructure`

Adapters for the domain's ports and for technical plumbing.

- `persistence/` — Toasty ORM. `woods/models.rs` is the storage model: scalar columns, a
  unique `scientific_name`, JSON text columns for lists and maps (`toasty::Json`), and
  `#[derive(Embed)]` structs/enums flattened into queryable columns
  (e.g. `physical_properties_density_basic`). `mappers.rs` converts domain → `WoodRecord`
  and storage → domain (fallible: stored data is re-validated). `repositories.rs`
  implements `WoodRepository`.
- `event_bus/` — the `EventPublisher` / `EventHandler` traits plus a `tokio::sync::broadcast`
  implementation (`TokioEventPublisher`, `run_event_handler`).
- `logging` — the one place a `tracing` subscriber is built. Each binary's `main` calls
  `logging::init_with_env(LoggingConfig::server())` (JSON lines on stdout; the CLI uses
  `LoggingConfig::cli()`: text on stderr). `RUST_LOG` replaces the filter and
  `ZYLOTEK_LOG_FORMAT=json|pretty` the format. It also routes the `log` macros into tracing.
  Call it from `main`, never from a `run`/`serve` function the tests call more than once per
  process.

The `persistence` feature (on by default) holds the database code and the `testing` support.
The web app and the assistant take the crate with `default-features = false` to use `logging`
without Toasty or the domain.

**Boundary to note:** the event-bus *traits* live in infrastructure, not domain, so
`zylotek-application` depends on `zylotek-infrastructure`.

### `crates/zylotek-api`

The axum HTTP binary and the **composition root**: `main.rs` is the only place where
concrete types are chosen and wired together. `woods/mappers.rs` converts between the
domain and the JSON shapes in `zylotek-contracts` through two local traits, `Convert` and
`TryConvert` (the orphan rule rules out `From` between two foreign types).
`auth/` is the OAuth 2.0 implementation: a token endpoint for the client-credentials grant, the
JWKS and discovery document, and the `WriteAccess` extractor that the handlers which change
data take as their first parameter. Reads are public; writes need a Bearer token with the
`woods:write` scope; with OAuth unconfigured, writes answer `503` (fail closed). Tokens are
EdDSA-signed JWTs (RFC 9068); the verifier fixes the algorithm and looks keys up in the
published JWKS, so an external authorization server could replace the built-in one without
touching the routes. `woods/handlers.rs` holds `From<WoodUseCaseError> for ApiError`, where
errors become status codes: `NotFound` → 404, invalid id → 400, other domain errors → 422,
persistence/event failures → 500.

### `crates/zylotek-sync`

The scraper, acting as an anti-corruption layer between the LPF HTML and the domain.
`SyncronizerRunner::run` fetches listing pages (`TOTAL_PAGE_NUMBER`, 20 rows each)
concurrently, then each wood's detail page. It returns a `Scrape`: the woods plus a
`ScrapeFailure` (page URL and reason) for each page that failed, so a bad page, even a
parser panic, never aborts the run. `parsers/` has one module per page section;
each extracts text by label or table header and returns domain value objects.
`parse_wood` assembles them into `ScrapedWood { name, scientific_name, details }`.
Portuguese labels and ratings (`"muito bom"`, `"não trinca"`) are mapped to domain enums
here and nowhere else.

`open_data.rs` parses the open-data CSV the same way: it maps the Portuguese columns to
domain value objects, converts SI units (MPa, N) to the kgf-based units, and reports rows
it cannot parse in `OpenData::rejected` instead of failing the file. Each CSV row is a test
lot, so one scientific name can occur on several rows; deduplicating is left to
`ImportWoods`, not the parser. Imported woods carry the row's `id_especie` as
`SourceReference::species_id` and the dataset page as their URL.

`merge.rs` combines the two sources (the field-by-field rule is `WoodDetails::fill_gaps_from`): `merge(open_data, scraped)` matches by scientific name
(case and spacing ignored) and fills only the gaps of the CSV wood from the page, field by
field, so the CSV's values always win. Only a species' first CSV row is completed, as it is
the one that gets stored. Web-only species are appended. If the page has a source reference
it replaces the CSV's dataset URL, since the id and URL belong together.

**Architecture invariant:** sync depends on `zylotek-domain` only, never on
infrastructure or application. Storing woods goes through `CreateWood` and `UpdateWood`, via `ImportWoods`.

### `crates/zylotek-cli`

A thin clap binary: `db setup|migrate` delegate to `zylotek_infrastructure::persistence::cli`.
`sync run [--file <csv>] [--offline | --listing-url <url>]` parses the CSV, scrapes the site
(unless `--offline`), merges the two with `zylotek_sync::merge` and stores the result with
`ImportWoods`, so the CLI is a second composition root next to
`zylotek-api`. The global `--database-url` (env `ZYLOTEK_DATABASE_URL`) picks the database
for `sync`; `auth keygen|client` manage the OAuth key and clients the API reads.

Any other subcommand is dispatched git-style (`commands/external.rs`): `zylotek-cli api …` or
`zylotek-cli webapp …` runs the `zylotek-api` or `zylotek-webapp` executable with the remaining arguments, found next to the CLI binary
first and then on `PATH` (on Unix the CLI `exec`s it, so signals and exit status pass through).
The webapp and assistant stay separate binaries the CLI does not link, so it needs no web or
LLM dependencies; any `zylotek-<name>` executable you add is reachable the same way.

### `crates/zylotek-contracts`

The API's wire format as plain serde types (`WoodRequest`, `WoodResponse`,
`WoodPageResponse`, `ErrorResponse`, the `*Schema` structs and the snake_case rating
enums). It depends on `serde` alone, so the server and every client — the web app and the
assistant — share one definition and a rename breaks their builds instead of drifting
silently. It knows nothing of the domain; converting is the API's job (`mappers.rs`).
Unknown fields are ignored on read, so the server can add fields without breaking clients.
`oauth` holds the token endpoint's request, response and error shapes (RFC 6749), so a client
such as the assistant can obtain tokens without depending on the API crate.

### `crates/zylotek-assistant`

Answers questions such as "which wood for a table with Japanese joinery for 12 seats?". It
is its own binary, meant to run next to the GPU, and depends on `zylotek-contracts` only: the
catalogue arrives over HTTP from `GET /api/v1/woods` (public, so no token), page by page, and
is swapped in as one immutable snapshot every few minutes (`catalogue.rs`). A question takes
three steps (`assistant.rs`):

1. **Understand.** The model, constrained by a JSON schema, turns the question into weighted
   properties (hardness, strength, stiffness, stability, lightness, workability) and woods to
   avoid. `requirements.rs` validates and clamps whatever comes back.
2. **Rank.** `ranking.rs` is plain arithmetic: each property becomes a percentile among all
   woods that have the figure, and a wood's score is the weighted mean, with a missing figure
   counting as zero. The model never chooses a wood, so it cannot invent one and the same
   question always yields the same list.
3. **Explain.** The model writes a short answer from the ranked facts. If this fails, the
   ranking is returned without it. `ask_stream` hands the explanation back as a stream so a
   caller can show it as it is written; the ranking is already complete by then.

`llm.rs` hides the model behind a `Model` trait; `OpenAiCompatible` speaks the chat-completions
protocol of Ollama (also llama.cpp's `llama-server`, vLLM, LM Studio). `http.rs` serves
`GET /health`, `POST /v1/ask` (one JSON answer) and `POST /v1/ask/stream` (server-sent events:
`answer`, then `token`s, then `done`; see `contracts::assistant::events`), the last two behind
an optional shared bearer token. `llm.rs` streams from the model by reading its SSE body
line by line, whatever way the network cut it.
`server.rs` wires it up and is outside the unit gate. The request and response types live in
`zylotek-contracts::assistant` for the web app.

### `crates/zylotek-webapp`

The website, on its own port (3000). Pages are rendered on the server with Askama templates
(`templates/`) and need no script: the question is a plain form POST, the comparison a plain
GET with repeated `id` parameters. It reads the catalogue from `zylotek-api` and asks
`zylotek-assistant` for recommendations through `clients.rs`, both using the contract types, so
it never touches the database or the model. Asking is streamed: the page's top (understanding,
criteria, ranked cards) is sent as soon as the ranking exists, the explanation follows as the
model writes it, and the closing markup comes last, as one chunked HTML response with no
script; if the explanation breaks, the page ends with a note instead of an error.
`view.rs` turns contract types into display
strings (templates hold no logic), `pages.rs` has the routes. A backend failure becomes a
friendly error page; the technical detail goes to the log only. Askama escapes every value,
including the model's text, and a header middleware sets a CSP that forbids scripts. Wood ids
are checked before they reach a URL, and image and source links must be `http(s)`.

### Root `src/main.rs`

An empty placeholder.

### `toasty/`, `Toasty.toml`

Generated migrations, snapshots and `history.toml` for the Toasty ORM, plus
`fixtures/*.csv`, the open-data export of the wood database (column names are in
Portuguese; `docs/` has the matching metadata PDF). Treat the generated files as
outputs of `zylotek-cli db setup`, not hand-edited sources.

## Request Flow: updating a wood

1. `router` → `update_wood_handler` parses the path id into a `WoodId` (400 if invalid)
   and converts the JSON body's details into `WoodDetails` (422 if a value is invalid).
2. `WoodFacade::update_wood` → `UpdateWood::execute` builds `WoodName` and
   `ScientificName`, then loads the wood (404 if missing).
3. `WoodUpdateService::update` checks that a *changed* scientific name is not taken
   (422), then `Wood::update` replaces the state and returns `WoodUpdated`.
4. The repository writes every column and re-reads the row.
5. The event is published; `run_event_handler` hands it to `WoodEventHandler`.
6. The handler responds `200` with the stored wood.

Create, get, list and delete follow the same path with fewer steps.

## Cross-Cutting Concerns

**Error handling.** Each layer has its own `thiserror` enum and wraps the one below it
(`WoodError` → `WoodServiceError` → `WoodUseCaseError` → `ApiError`). Only the API crate
knows about HTTP status codes. Binaries (`main`) use `anyhow`.

**Events.** Events are produced by the aggregate but published by the use case, after
persistence succeeds. Delivery is in-process and best-effort: `publish` fails with
`NoSubscribers` if nobody listens, and slow subscribers drop events (`Lagged`) rather
than block publishers. Because publishing happens after the write, a publish failure
returns an error to the caller even though the change was stored.

**Persistence and migrations.** `build_db` pushes the schema only when the tables are
missing, so it can reopen an existing database file. The database defaults to
`sqlite:zylotek.db` and is set with `--database-url` / `ZYLOTEK_DATABASE_URL`. The schema is
still created from the models, not by applying the generated migrations (the `db` commands
use a throwaway in-memory database), so migrations are not yet meaningful at runtime. Uniqueness of `scientific_name` is checked
by the domain service and backed by a unique index. The SQLite repository is still named
`PostgresWoodRepository`.

**Configuration & dependencies.** There is no configuration file yet: the
database URL is a CLI option or environment variable, while the port and scraper URL are
constants. Dependency versions live only in the root
`[workspace.dependencies]`. Toasty 0.11 requires rustc 1.95+.

**Testing & quality gates.** See `TESTING.md`: unit tests beside the code (100% line
coverage gate), integration tests under `crates/*/tests/` (80% gate), criterion benchmarks
and cargo-fuzz targets. `prek` runs `cargo check`, `rustfmt --check` and `clippy -D warnings`
over `--workspace --all-targets`; CI additionally runs both test suites, the coverage gates,
a benchmark smoke run and a short fuzz run.
