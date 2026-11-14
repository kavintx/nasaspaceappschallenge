# Flame in Freefall

NASA Space Apps Challenge 2026 — *Flame in Freefall: SI-Powered Fire Safety Insights from Microgravity Combustion Data*.

A dashboard that summarizes, ranks and compares NASA microgravity combustion experiments for crewed-habitat fire safety.

## How it fits together

```
NASA NTRS ──> backend (crates/) ──writes──> Supabase database <──reads── dashboard (web/)
                    ^                                                        │
                    └──────────── AI answers, pipeline, edits ───────────────┘
```

- **Supabase** holds the research database: experiments, papers, extracted test conditions and findings.
- **`web/`** — the dashboard (Rust: Axum + Askama templates, Tailwind CSS). It reads the database
  straight from Supabase with the publishable key, so browsing works with nothing else running.
  AI answers, the Pipeline page and verify/edit go through the backend.
- **`crates/`, `migrations/`, `Cargo.toml`** — the `fire-intel` backend by MonsterLXW (copied from
  https://github.com/MonsterLXW/Fireset at commit `f4622a8`). It ingests NASA NTRS papers, extracts
  conditions and findings with a self-hosted model, and answers questions with citations.
- **`db/supabase_setup.sql`** — creates the backend's tables in Supabase and sets public access to read-only.

## Run the dashboard

```sh
cd web
cargo run            # http://localhost:3100 (or the next free port)
```

That is enough to browse Habitat Risk, Explorer, Compare and Literature. To turn on AI answers, chat,
the Pipeline page and verify/edit, copy `web/.env.example` to `web/.env` and fill in the backend:

```
API_BASE_URL=http://<backend address>:8080
API_AUTH_TOKEN=<token>
```

The token stays on this server; it is never sent to the browser.

## Run the backend against Supabase

Settings (environment variables or a `.env` file in this folder):

```
DATABASE_URL=<Supabase "Session pooler" connection string, from the project's Connect button>
API_AUTH_TOKEN=<at least 24 characters>
VLLM_BASE_URL=<address of the model server>
```

```sh
cargo run -p api                # http://localhost:8080
```

Use the Session pooler string (port 5432), not the Transaction pooler: the backend uses prepared
statements. The tables already exist in Supabase (`db/supabase_setup.sql` records migration 1 as
applied), so the backend starts without re-creating them.

To bring over data from a local fire-intel database (untested here; ids must match, so experiments are replaced too):

```sh
psql "$SUPABASE_URL" -c "truncate experiments cascade"
pg_dump "$LOCAL_URL" --data-only --exclude-table=_sqlx_migrations | psql "$SUPABASE_URL"
```

## What the public key can do

The publishable key in `web/src/store.rs` can only read `experiments`, `experiment_papers`,
`conditions`, `findings`, and `papers` without its full text. It cannot write anything, and cannot
read chat history, job logs or the other tables. The backend connects as the database owner.

## Where each page gets its data

| Page | Source |
|---|---|
| Explorer, experiment pages, Compare table, Habitat evidence, Literature, paper pages | Supabase (rolled up per experiment, cached 5 minutes; "Refresh" on the Explorer rebuilds it) |
| Ask the Data (streaming chat) | backend `POST /ai/chat`, relayed through `/ask/stream` |
| Habitat Risk → AI risk assessment | backend `POST /ai/habitat/analyze` |
| Compare → AI comparison | backend `POST /ai/compare` |
| Experiment → AI synthesis | backend `POST /ai/summarize/{acronym}` |
| Pipeline | backend `GET /ai/health`, `GET /pipeline/status`, `POST /ai/test`, `POST /pipeline/ingest`, `POST /pipeline/sheet/import`, `POST /ai/extract/batch` |
| Paper page actions | backend `POST /ai/extract/{id}`, `PATCH /conditions/{id}`, `PATCH /findings/{id}` |

Category, fuel phase, hardware, and the names and one-line descriptions of the 13 known experiments
come from a small table in `web/src/catalog.rs`, taken from the NASA challenge brief. Rejected
conditions and findings are left out of the roll-ups.

## Before putting it on the public internet

The Pipeline page and the verify/reject/edit buttons change the database, and this app has no
login. Anyone who can open the site can use them when the backend is connected. For a public
deployment set:

```
READ_ONLY=true
```

That turns off ingestion, imports, extraction and editing. Browsing, AI answers and chat still work.

Optional settings (env var or `.env`): `PORT`, `STATIC_DIR`, `READ_ONLY`, `SUPABASE_URL`, `SUPABASE_KEY`
(publishable key only).

## Changing the styling

`static/app.css` is generated. After editing templates or `styles/input.css`:

```sh
tailwindcss -c tailwind.config.js -i styles/input.css -o static/app.css --minify
```

## Deploy

`web/Dockerfile` builds a container that runs anywhere Docker does (Render, Fly.io, Railway).
