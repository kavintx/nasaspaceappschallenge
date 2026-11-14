# Flame in Freefall

NASA Space Apps Challenge 2026 — *Flame in Freefall: SI-Powered Fire Safety Insights from Microgravity Combustion Data*.

A dashboard that summarizes, ranks and compares NASA microgravity combustion experiments for crewed-habitat fire safety.

## Layout

- `web/` — the app: a Rust server (Axum + Askama templates, Tailwind CSS). All data and AI come
  from the `fire-intel` research backend (`/api/v1/*`).

## Run it

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
| Ask the Data | `POST /ai/query` |
| Habitat Risk → AI risk assessment | `POST /ai/habitat/analyze` |
| Compare → AI comparison | `POST /ai/compare` |
| Experiment → AI synthesis | `POST /ai/summarize/{acronym}` |
| Literature, paper pages | `GET /papers`, `GET /papers/{id}/extractions`, `GET /pipeline/status` |

Category, fuel phase and hardware per experiment are not in the database; they are a small
table in `web/src/catalog.rs`, taken from the NASA challenge brief. Rejected conditions and
findings are left out of the roll-ups.

Not built into this app: ingestion, the Google Sheets import, running extraction, editing or
verifying conditions and findings, and streaming chat. Those stay as direct API calls.

Optional settings (env var or `.env`): `PORT` (default 3100), `STATIC_DIR`.

## Changing the styling

`static/app.css` is generated. After editing templates or `styles/input.css`:

```sh
tailwindcss -c tailwind.config.js -i styles/input.css -o static/app.css --minify
```

## Deploy

`web/Dockerfile` builds a container that runs anywhere Docker does (Render, Fly.io, Railway).
