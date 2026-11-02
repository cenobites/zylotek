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
| POST   | `/api/v1/woods`      | `201` with the wood                   | `401`/`403` token, `422` invalid data or duplicate scientific name |
| GET    | `/api/v1/woods`      | `200` `{items, total, offset, limit}` | —                                               |
| GET    | `/api/v1/woods/{id}` | `200` with the wood                   | `400` bad id, `404`                             |
| PUT    | `/api/v1/woods/{id}` | `200` with the wood                   | `401`/`403`, `400`, `404`, `422`                |
| DELETE | `/api/v1/woods/{id}` | `204`                                 | `401`/`403`, `400`, `404`                       |

**Reading is open; changing woods needs an OAuth 2.0 access token** (see
[Authentication](#authentication)). `GET`, `/health` and the OAuth endpoints need no token.

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

### Authentication

The write routes (`POST`, `PUT`, `DELETE` on woods) require a Bearer token with the
`woods:write` scope. The API issues tokens itself, using the OAuth 2.0 client-credentials
grant (RFC 6749 section 4.4), meant for machines such as an importer or the assistant.

```bash
# One-time setup: a signing key, and a client (its secret is printed once, never stored)
cargo run -p zylotek-cli -- auth keygen
cargo run -p zylotek-cli -- auth client create importer

cargo run -p zylotek-cli -- api run

# Trade the secret for a token (valid for an hour), then write with it
TOKEN=$(curl -s -u importer:CLIENT_SECRET -d grant_type=client_credentials \
  localhost:8080/oauth/token | jq -r .access_token)
curl -X POST localhost:8080/api/v1/woods -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"name": "Aroeira", "scientific_name": "Myracrodruon urundeuva"}'
```

| Endpoint | Purpose |
| -------- | ------- |
| `POST /oauth/token` | Token endpoint. Authenticate with HTTP Basic or with `client_id` and `client_secret` form fields. |
| `GET /.well-known/jwks.json` | The public key that verifies tokens (JWKS). |
| `GET /.well-known/oauth-authorization-server` | Discovery document (RFC 8414). |

- Tokens are EdDSA-signed JWTs (RFC 9068). A token that is missing, malformed, expired or for
  another issuer gets `401` with a `WWW-Authenticate: Bearer` challenge; a valid token without
  the `woods:write` scope gets `403`.
- Authentication is checked before the request body is read.
- With no signing key or clients file, OAuth stays off: reads work and writes answer `503`.
  Having only one of the two files is an error. The key and the clients file are looked up as
  `zylotek-oauth-key.pem` and `zylotek-oauth-clients.json` in the working directory, or set
  with `--oauth-signing-key` / `--oauth-clients` (env `ZYLOTEK_OAUTH_SIGNING_KEY`,
  `ZYLOTEK_OAUTH_CLIENTS`). Both are git-ignored and written readable by you only.
- `--oauth-issuer` (env `ZYLOTEK_OAUTH_ISSUER`, default `http://localhost:8080`) is the
  token's `iss` and the base of the URLs in the discovery document; set it to the address
  clients really use.
- `sync run` writes to the database directly, not through the API, so it needs no token.

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

- OAuth has no refresh tokens or revocation (tokens simply expire after an hour), one signing
  key with no rotation, and no rate limit on the token endpoint. Put a reverse proxy with TLS
  and rate limiting in front before exposing the API beyond a trusted network: tokens and
  client secrets travel in clear text over plain HTTP.

- Only the first test lot of each species is stored; the other lots in the CSV (different
  collection sites and measurements) are skipped, because a scientific name is registered once.
- `sync run --source web` has not been run against the live site since the import was added,
  and a malformed page can still abort it (see `TESTING.md`, "Known bugs").
- The CSV has no workability data or images, so those fields stay empty for imported woods.
- Migration `0001` adds `NOT NULL` columns without defaults; it applies to an empty
  `woods` table only.
- The root `src/main.rs` is a placeholder.
