# Flame in Freefall

NASA Space Apps Challenge 2026 — *Flame in Freefall: SI-Powered Fire Safety Insights from Microgravity Combustion Data*.

A dashboard that summarizes, ranks and compares NASA microgravity combustion experiments for crewed-habitat fire safety.

## Layout

- `crates/`, `migrations/`, `Cargo.toml` — the `fire-intel` backend by MonsterLXW (copied from
  https://github.com/MonsterLXW/Fireset at commit `f4622a8`): a Rust API over Postgres that
  ingests NASA NTRS papers, extracts test conditions and findings with a self-hosted model,
  and answers questions with citations. It serves `/api/v1/*` on port 8080.
- `web/` — the dashboard: a Rust server (Axum + Askama templates, Tailwind CSS). All of its data
  and AI come from the backend. It keeps its own `Cargo.lock` and is built separately.

## Run the backend

It needs a Postgres database with the `vector` and `pg_trgm` extensions, and these settings
(environment variables or a `.env` file in this folder):

```
DATABASE_URL=postgres://fire:fire@localhost:5435/fire
API_AUTH_TOKEN=<at least 24 characters>
VLLM_BASE_URL=<address of the model server>
```

```sh
cargo run -p cli -- init-db     # create tables and seed the 13 experiments
cargo run -p api                # http://localhost:8080
```

## Run the dashboard

1. Copy `web/.env.example` to `web/.env` and fill it in:

   ```
   API_BASE_URL=http://127.0.0.1:8080
   API_AUTH_TOKEN=<token>
   ```

2. Start it:

   ```sh
   cd web
   cargo run            # http://localhost:3100
   ```

The token stays on this server; it is never sent to the browser.

Pages: `/` Habitat Risk · `/explorer` · `/experiments/<id>` · `/compare` · `/papers` Literature · `/ask`

## Where each page gets its data

| Page | Backend calls |
|---|---|
| Explorer, experiment pages, Compare table, Habitat evidence | `GET /experiments`, `GET /papers`, then `GET /papers/{id}/extractions` per paper (4 at a time), rolled up per experiment and cached for 5 minutes. "Refresh" on the Explorer rebuilds it |
| Habitat Risk → AI risk assessment | `POST /ai/habitat/analyze` |
| Compare → AI comparison | `POST /ai/compare` |
| Experiment → AI synthesis | `POST /ai/summarize/{acronym}` |
| Literature, paper pages | `GET /papers`, `GET /papers/{id}/extractions`, `GET /pipeline/status` |

Category, fuel phase and hardware per experiment are not in the database; they are a small
table in `web/src/catalog.rs`, taken from the NASA challenge brief. Rejected conditions and
findings are left out of the roll-ups.

| Ask the Data (streaming chat) | `POST /ai/chat` with `stream: true`, relayed through `/ask/stream` so the token never reaches the browser |
| Pipeline | `GET /ai/health`, `GET /pipeline/status`, `POST /ai/test`, `POST /pipeline/ingest`, `POST /pipeline/sheet/import`, `POST /ai/extract/batch` |
| Paper page actions | `POST /ai/extract/{id}`, `PATCH /conditions/{id}`, `PATCH /findings/{id}` (verify, reject, edit) |

## Before putting it on the public internet

The Pipeline page and the verify/reject/edit buttons change the database, and this app has no
login. Anyone who can open the site can use them. For a public deployment set:

```
READ_ONLY=true
```

That turns off ingestion, imports, extraction and editing. Browsing, AI answers and chat still work.

Optional settings (env var or `.env`): `PORT` (default 3100), `STATIC_DIR`, `READ_ONLY`.

## Changing the styling

`static/app.css` is generated. After editing templates or `styles/input.css`:

```sh
tailwindcss -c tailwind.config.js -i styles/input.css -o static/app.css --minify
```

## Deploy

`web/Dockerfile` builds a container that runs anywhere Docker does (Render, Fly.io, Railway).
