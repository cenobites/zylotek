# Zylotek

A database of Brazilian wood species. Zylotek scrapes the
[LPF / Serviço Florestal Brasileiro](https://lpf.florestal.gov.br/pt-br/madeiras-brasileiras)
catalogue (basic information, physical and mechanical properties, workability) and serves
wood records over an HTTP CRUD API.

> **Status: early development.** Woods are loaded from the open-data CSV (or scraped from the
> site) into a SQLite database and served over the API. See
> [Current limitations](#current-limitations).

## Requirements

- Rust **1.95 or newer** (Toasty 0.11 requires it) with edition 2024 support
- OpenSSL development headers (`reqwest` is built with `native-tls`)
- Optional: `make setup` installs `prek`, `sccache`, `cargo-llvm-cov` and other dev tools

## Usage

```bash
# Load woods into the database (default: the bundled open-data CSV)
cargo run -p zylotek-cli -- sync run
cargo run -p zylotek-cli -- sync run --file other.csv      # another export of the same format
cargo run -p zylotek-cli -- sync run --source web          # scrape the LPF web site instead

# Generate / apply database migrations (Toasty)
cargo run -p zylotek-cli -- db setup
cargo run -p zylotek-cli -- db migrate      # or: make migrate

# Run the HTTP API on 0.0.0.0:8080
cargo run -p zylotek-cli -- api run
```

`sync` and `api` share one database: `sqlite:zylotek.db` in the working directory unless you
pass `--database-url <url>` or set `ZYLOTEK_DATABASE_URL` (`sqlite::memory:` for a throwaway
one). Run `sync run` first and `api run` then serves what was stored. Syncing is repeatable:
a wood whose scientific name is already stored is skipped, so a second run adds nothing.

#### The CSV import

`sync run --source file` (the default) reads the LPF open-data export. Each row there is a
test *lot*, a species collected at one place, and 269 rows cover only 225 distinct species,
so the first lot in the file is stored for each species and the rest are reported as
skipped. Values are converted from the file's SI units (MPa, N) to the kgf-based units the
API serves; see `crates/zylotek-sync/src/open_data.rs` for the column mapping.

### API

Base path: `/api/v1/woods` (no trailing slash).

| Method | Path                 | Success                               | Errors                                          |
| ------ | -------------------- | ------------------------------------- | ----------------------------------------------- |
| POST   | `/api/v1/woods`      | `201` with the wood                   | `422` invalid data or duplicate scientific name |
| GET    | `/api/v1/woods`      | `200` `{items, total, offset, limit}` | —                                               |
| GET    | `/api/v1/woods/{id}` | `200` with the wood                   | `400` bad id, `404`                             |
| PUT    | `/api/v1/woods/{id}` | `200` with the wood                   | `400`, `404`, `422`                             |
| DELETE | `/api/v1/woods/{id}` | `204`                                 | `400`, `404`                                    |

- `GET /api/v1/woods` takes `offset` (default 0) and `limit` (default 20, max 100), ordered
  by scientific name.
- `PUT` replaces the whole record: fields left out are cleared.
- Only `name` and `scientific_name` (genus + epithet) are required. Everything else is optional:
  `family`, `collection_location`, `common_names`, `source`, `description`, `kiln_drying`,
  `images`, `physical_properties`, `mechanical_properties`, `workabilities`.
- Units: density g/cm³, shrinkage %, stresses kgf/cm², cleavage kgf/cm, Janka hardness and
  nail extraction kgf. Values must be non-negative; percentages at most 100.
- Ratings are snake_case strings, e.g. `"very_good"`, `"does_not_crack"`, `"not_applicable"`.
- Responses add `id`, `genus` and the computed `physical_properties.shrinkage.tangential_radial_ratio`.
- Errors are `{"code": "...", "message": "..."}`.

```bash
curl -X POST localhost:8080/api/v1/woods \
  -H 'content-type: application/json' \
  -d '{
    "name": "Muiracatiara-rajada",
    "scientific_name": "Astronium lecointei",
    "family": "Anacardiaceae",
    "physical_properties": {
      "density": {"basic": 0.8},
      "shrinkage": {"tangential": 7.5, "radial": 5.0}
    },
    "workabilities": {"surface_finishing": {"planer": "very_good"}}
  }'
```

## Development

```bash
make build                                              # build the CLI
cargo test --workspace                                  # tests (none yet)
cargo clippy --workspace --all-targets -- -D warnings   # lint gate
prek install                                            # install git hooks
prek run --all-files                                    # fmt --check, clippy, file hygiene
```

CI (`.github/workflows/rust.yml`) runs `cargo build` and `cargo test` on pushes and pull
requests to `master`.

See [ARCHITECTURE.md](ARCHITECTURE.md) for how the code is organised.

## Repository layout

| Path          | Contents                                                          |
| ------------- | ----------------------------------------------------------------- |
| `crates/`     | The workspace crates (see ARCHITECTURE.md)                        |
| `toasty/`     | Toasty migrations, schema snapshots and the open-data CSV fixture |
| `docs/`       | PDF metadata describing the Brazilian wood database               |
| `Toasty.toml` | Migration configuration                                           |

## Current limitations

- Only the first test lot of each species is stored; the other lots in the CSV (different
  collection sites and measurements) are skipped, because a scientific name is registered once.
- `sync run --source web` has not been run against the live site since the import was added,
  and a malformed page can still abort it (see `TESTING.md`, "Known bugs").
- The CSV has no workability data or images, so those fields stay empty for imported woods.
- Migration `0001` adds `NOT NULL` columns without defaults; it applies to an empty
  `woods` table only.
- The root `src/main.rs` is a placeholder.
