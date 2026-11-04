# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Overview

Zylotek is a Rust (edition 2024) Cargo workspace for a Brazilian wood-species database. Data comes from the LPF/Serviço Florestal Brasileiro open-data CSV, completed with what the CSV lacks from the site (https://lpf.florestal.gov.br/pt-br/madeiras-brasileiras) by `zylotek-sync`; the CSV wins where both have a value. Reference material: `docs/*.pdf` (database metadata) and `toasty/fixtures/*.csv` (open data export). The root `src/main.rs` is a placeholder "Hello, world" stub.

## Commands

```bash
make setup                          # install prek, sccache, cargo-llvm-cov, etc.
cargo run -p zylotek-cli -- db setup     # generate a migration (toasty migration generate)
cargo run -p zylotek-cli -- sync run     # import the bundled CSV, completed from the LPF site, into sqlite:zylotek.db
cargo run -p zylotek-cli -- sync run --offline   # the CSV alone (no network)
cargo run -p zylotek-cli -- auth keygen  # OAuth signing key (then `auth client create <id>`)
make test                                # unit + integration + doc tests
make cov                                 # coverage gates: unit 100%, integration 80%
make bench-check                         # run each criterion benchmark once
make fuzz FUZZ_TIME=30                   # cargo-fuzz targets (needs nightly)
cargo run -p zylotek-assistant -- --model-url http://gpu-box:11434   # question answering over the API's catalogue
cargo run -p zylotek-webapp                    # the website on :3000 (needs the API and the assistant)
cargo run -p zylotek-api                       # the HTTP API on :8080 (also: zylotek-cli api)
zylotek-cli api | webapp | assistant ...  # git-style: runs the zylotek-<name> binary found beside the CLI or on PATH
```

Test layout, coverage policy and the bugs the suite has found are in `TESTING.md`.

Toasty 0.11 requires rustc ≥ 1.95; if the default toolchain is older, use `cargo +nightly …`.

## Architecture

Layered / hexagonal workspace, with dependencies pointing inward (`api` → `application` → `domain`; `infrastructure` → `domain`). Per-crate tour: see `ARCHITECTURE.md`.

A domain type is mirrored three times: domain value object, persistence model (`models.rs` + `mappers.rs`) and API wire type (`zylotek-contracts/src/woods.rs`, converted by `zylotek-api/src/woods/mappers.rs`). Adding a field to the wood means touching all three plus the sync parsers, then regenerating the migration with `db setup`. Clients (web app, assistant) read the contracts crate, never the domain.

Dependencies are declared once in the root `[workspace.dependencies]`; crates reference them with `{ workspace = true }`.

## Gotchas

- Tests that document an open bug are `#[ignore]`d and assert the *correct* behaviour
  (`make test-known-bugs`). Don't "fix" them by weakening the assertion.
- Every handler that changes data must take `WriteAccess` as its **first** parameter: extractors run in order, so authentication then happens before the body is parsed. Reads are deliberately public. Tests that call the API use `auth::testing` (feature `testing`), which shares one fresh key per process and a ready-made write token.
- Keep process entry points (`zylotek-api/src/server.rs`, `persistence/migrations.rs`) free of
  logic: the unit coverage gate excludes them by filename.

- `sync` and `zylotek-api` share the database from `--database-url` / `ZYLOTEK_DATABASE_URL` (default `sqlite:zylotek.db`, git-ignored). The `db` migration commands still use an in-memory database.
- The CSV has one row per test lot, not per species (269 rows, 225 species), and its units are SI (the MOE columns are labelled MPa but hold GPa). `open_data.rs` documents both; first lot wins on import.
- The SQLite-backed repository is named `PostgresWoodRepository`.
- Toasty migrations (`toasty/migrations`, `snapshots`, `history.toml`) are generated; use the CLI rather than editing them by hand.
