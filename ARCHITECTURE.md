# Architecture

This document describes the high-level structure of Zylotek: what each crate is for,
where the important boundaries are, and which invariants hold across the code. It is
meant to be read once, before diving into the source. Details that change often are
deliberately left out; search for the named types instead.

## Bird's Eye View

Zylotek collects data about Brazilian wood species and serves it. There are two flows:

1. **Ingest (prototype).** `zylotek-sync` fetches the LPF catalogue pages over HTTP and
   translates the HTML into domain value objects (`ScrapedWood`). It logs them; it does
   not persist anything yet.
2. **Serve.** `zylotek-api` exposes CRUD on `/api/v1/woods`. Requests are validated by the
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

**Boundary to note:** the event-bus *traits* live in infrastructure, not domain, so
`zylotek-application` depends on `zylotek-infrastructure`.

### `crates/zylotek-api`

The axum HTTP binary and the **composition root**: `main.rs` is the only place where
concrete types are chosen and wired together. `woods/schemas.rs` holds the JSON shapes
(serde mirrors of the value objects, enums as snake_case) and the conversions to and
from the domain. `woods/handlers.rs` holds `From<WoodUseCaseError> for ApiError`, where
errors become status codes: `NotFound` → 404, invalid id → 400, other domain errors → 422,
persistence/event failures → 500.

### `crates/zylotek-sync`

The scraper, acting as an anti-corruption layer between the LPF HTML and the domain.
`SyncronizerRunner::run` fetches listing pages (`TOTAL_PAGE_NUMBER`, 20 rows each)
concurrently, then each wood's detail page. `parsers/` has one module per page section;
each extracts text by label or table header and returns domain value objects.
`parse_wood` assembles them into `ScrapedWood { name, scientific_name, details }`.
Portuguese labels and ratings (`"muito bom"`, `"não trinca"`) are mapped to domain enums
here and nowhere else.

**Architecture invariant:** sync depends on `zylotek-domain` only, never on
infrastructure or application. Storing scraped woods is future work: it should go
through `CreateWood`, using `SourceReference::species_id` to avoid duplicates.

### `crates/zylotek-cli`

A thin clap binary: `db setup|migrate` delegate to `zylotek_infrastructure::persistence::cli`;
`sync run` constructs and runs the synchronizer. No logic lives here.

### `crates/zylotek-contracts`, root `src/main.rs`

Empty placeholders. `zylotek-contracts` is intended for shared API contracts.

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

**Persistence and migrations.** `build_db` calls `push_schema()` on every start, and the
database URI is hard-coded to `sqlite::memory:` in the API and the CLI helper, so
migrations are not yet meaningful at runtime. Uniqueness of `scientific_name` is checked
by the domain service and backed by a unique index. The SQLite repository is still named
`PostgresWoodRepository`.

**Configuration & dependencies.** There is no runtime configuration layer yet (port,
database URI and scraper URL are constants). Dependency versions live only in the root
`[workspace.dependencies]`. Toasty 0.11 requires rustc 1.95+.

**Testing & quality gates.** See `TESTING.md`: unit tests beside the code (100% line
coverage gate), integration tests under `crates/*/tests/` (80% gate), criterion benchmarks
and cargo-fuzz targets. `prek` runs `cargo check`, `rustfmt --check` and `clippy -D warnings`
over `--workspace --all-targets`; CI additionally runs both test suites, the coverage gates,
a benchmark smoke run and a short fuzz run.
