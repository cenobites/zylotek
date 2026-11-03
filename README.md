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
# Load woods into the database: the bundled open-data CSV, completed from the LPF web site
cargo run -p zylotek-cli -- sync run
cargo run -p zylotek-cli -- sync run --file other.csv      # another export of the same format
cargo run -p zylotek-cli -- sync run --offline             # the CSV alone, without visiting the site

# Generate / apply database migrations (Toasty)
cargo run -p zylotek-cli -- db setup
cargo run -p zylotek-cli -- db migrate      # or: make migrate

# Run the HTTP API on 0.0.0.0:8080
cargo run -p zylotek-cli -- api run
```

`sync` and `api` share one database: `sqlite:zylotek.db` in the working directory unless you
pass `--database-url <url>` or set `ZYLOTEK_DATABASE_URL` (`sqlite::memory:` for a throwaway
one). Run `sync run` first and `api run` then serves what was stored. Syncing is an upsert
keyed on the scientific name: a wood that is not stored is created, one that is gets
updated, and one that would not change is left alone, so repeating a run is harmless.

#### What `sync run` does

One command builds each wood from two sources and stores the result.

1. It reads the LPF open-data export. Each row there is a test *lot*, a species collected at
   one place, and 269 rows cover only 225 distinct species, so the first lot in the file is
   stored for each species and the rest are reported as skipped. Values are converted from
   the file's SI units (MPa, N) to the kgf-based units the API serves; see
   `crates/zylotek-sync/src/open_data.rs` for the column mapping.
2. It scrapes the LPF site and merges each species page into the CSV wood of the same
   scientific name (`crates/zylotek-sync/src/merge.rs`). **The CSV wins wherever it has a
   value**; the page only supplies what the CSV lacks: common names, images, workabilities,
   end uses and the other free-text notes, the page URL, and any cell a row left blank.
   Species that only the web has are stored as scraped.

Storing is an upsert: an existing wood takes the new name and values, but keeps anything
it already has that the new data lacks. So a run that could not reach the site (or a page
that failed) never erases what an earlier run fetched, and a later run completes it.
A page that cannot be fetched or parsed is listed on stderr (`web page failed <url>: ...`)
and the run exits 1, but every other wood is still stored. `--offline` skips step 2;
`--listing-url` points step 2 at another listing.

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

### Assistant

`zylotek-assistant` answers questions in plain language ("a table for 12 with Japanese
joinery") from the catalogue of `zylotek-api`. A language model only reads the question and
explains the answer; the ranking is arithmetic over the database. Run any OpenAI-compatible
server on the machine with the GPU, for example llama.cpp. The model must fit in the GPU's
memory (see the comment below for the reference machine: an MX150 with 4 GB of VRAM):

```bash
# On the GPU machine (4 GB VRAM, 16 GB RAM): an instruction-tuned 3B model, 4-bit quantized.
# It is about 2 GB, so `-ngl 99` keeps all of it on the GPU. Larger models do not fit in 4 GB.
pip install -U huggingface_hub
hf auth login
hf download Qwen/Qwen2.5-3B-Instruct-GGUF qwen2.5-3b-instruct-q4_k_m.gguf

llama-server -m qwen2.5-3b-instruct-q4_k_m.gguf --port 8081 --host 0.0.0.0 -ngl 99

# Anywhere that can reach the API and the model server
cargo run -p zylotek-assistant -- \
  --api-url http://localhost:8080 --model-url http://gpu-box:8081 \
  --listen 0.0.0.0:8090 --token "$(openssl rand -hex 24)"

curl -s localhost:8090/v1/ask -H "Authorization: Bearer $TOKEN" -H 'content-type: application/json' \
  -d '{"question": "I want to build a table for 12 seats using Japanese joinery"}'
```

Every option has an environment variable (`ZYLOTEK_API_URL`, `ZYLOTEK_MODEL_URL`,
`ZYLOTEK_MODEL_NAME`, `ZYLOTEK_MODEL_API_KEY`, `ZYLOTEK_ASSISTANT_TOKEN`,
`ZYLOTEK_ASSISTANT_LISTEN`, `ZYLOTEK_ASSISTANT_REFRESH_SECS`). `GET /health` reports how many
woods are loaded. It starts without the API and fills in once the API is reachable.

### Website

`zylotek-webapp` is the site visitors use: ask in plain words which wood suits a project,
browse the catalogue, open a wood, and compare two to four side by side. It needs the API and
the assistant running:

```bash
cargo run -p zylotek-cli -- api run                     # :8080
cargo run -p zylotek-assistant -- --model-url http://gpu-box:8081   # :8090
cargo run -p zylotek-webapp -- --assistant-token "$TOKEN"           # :3000
```

Options and environment variables: `--api-url` (`ZYLOTEK_API_URL`), `--assistant-url`
(`ZYLOTEK_ASSISTANT_URL`), `--assistant-token` (`ZYLOTEK_ASSISTANT_TOKEN`), `--listen`
(`ZYLOTEK_WEBAPP_LISTEN`, default `127.0.0.1:3000`). Browsing works even when the assistant is
down, and asking works only when it is up. The ranking appears at once and the explanation
writes itself in as the model produces it.

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

- The website has no accounts or search box. It is plain HTTP; put TLS in front before
  exposing it, and make sure the proxy does not buffer responses, or the explanation will
  appear all at once (the site sends `X-Accel-Buffering: no` for nginx).

- The assistant ranks on six properties computed from the catalogue; it does not read the
  free-text descriptions (natural durability, end uses), and the explanation is a few sentences, not a conversation (there are no follow-up questions).

- OAuth has no refresh tokens or revocation (tokens simply expire after an hour), one signing
  key with no rotation, and no rate limit on the token endpoint. Put a reverse proxy with TLS
  and rate limiting in front before exposing the API beyond a trusted network: tokens and
  client secrets travel in clear text over plain HTTP.

- Only the first test lot of each species is stored; the other lots in the CSV (different
  collection sites and measurements) are skipped, because a scientific name is registered once.
- The merged `sync run` has not been run against the live site: its tests use a stand-in
  server. A page that fails to scrape leaves that wood with CSV data only until a later run
  succeeds.
- `sync run` never deletes a wood, and an update cannot clear a value: a field the new data
  leaves blank keeps its stored value.
- Migration `0001` adds `NOT NULL` columns without defaults; it applies to an empty
  `woods` table only.
- The root `src/main.rs` is a placeholder.
