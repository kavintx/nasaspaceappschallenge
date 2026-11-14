# Flame in Freefall

NASA Space Apps Challenge 2026 — *Flame in Freefall: SI-Powered Fire Safety Insights from Microgravity Combustion Data*.

A dashboard that summarizes, ranks and compares NASA microgravity combustion experiments for crewed-habitat fire safety.

## Layout

- `web/` — the app: a Rust server (Axum + Askama templates, Tailwind CSS) that reads from Supabase
- `db/schema.sql` — the Supabase tables (`experiments`, `papers`, `habitat_scores`)

## Run it

```sh
cd web
cargo run            # http://localhost:3100
```

Pages: `/` Habitat Risk · `/explorer` · `/experiments/<id>` · `/compare` · `/papers` Literature · `/ask`

## Connecting the research backend

The AI features and the Literature pages come from the `fire-intel` backend (`/api/v1/*`).
Copy `web/.env.example` to `web/.env`, paste the token, and restart:

```
API_BASE_URL=http://127.0.0.1:8080
API_AUTH_TOKEN=<token from Gus>
```

The token stays on this server; it is never sent to the browser. Without it the app still
runs, and the AI sections say the backend isn't connected.

What uses the backend:

| Page | Endpoint |
|---|---|
| Ask the Data | `POST /ai/query` |
| Habitat Risk → AI risk assessment | `POST /ai/habitat/analyze` |
| Compare → AI comparison | `POST /ai/compare` |
| Experiment → AI synthesis | `POST /ai/summarize/{id}` |
| Literature, paper pages | `GET /papers`, `GET /papers/{id}/extractions`, `GET /pipeline/status` |

Not built into this app: running ingestion, the Google Sheets import, batch extraction, and
editing/verifying conditions and findings. Those stay as direct API calls.

Other settings (all optional, env var or `.env`):

| Variable | What it does |
|---|---|
| `PORT` | Port to listen on (default 3100) |
| `SUPABASE_URL`, `SUPABASE_KEY` | Override the Supabase project that holds the experiment catalog. Only ever use the publishable (read-only) key here |
| `DATA_FIXTURE` | Path to a JSON snapshot (e.g. `data/fixture.json`) to run without Supabase |

## Changing the styling

`static/app.css` is generated. After editing templates or `styles/input.css`:

```sh
tailwindcss -c tailwind.config.js -i styles/input.css -o static/app.css --minify
```

## Deploy

`web/Dockerfile` builds a container that runs anywhere Docker does (Render, Fly.io, Railway).
