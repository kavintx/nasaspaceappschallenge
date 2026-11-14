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

Pages: `/` Habitat Risk · `/explorer` · `/experiments/<id>` · `/compare` · `/ask`

Settings (environment variables, all optional):

| Variable | What it does |
|---|---|
| `PORT` | Port to listen on (default 3100) |
| `SUPABASE_URL`, `SUPABASE_KEY` | Override the Supabase project. Only ever use the publishable (read-only) key here |
| `ASK_API_URL` | The AI endpoint for Ask the Data. The app POSTs `{"question": "..."}` and expects `{"answer": "...", "sources": [{"title", "url", "experiment_id"}]}`. Without it, Ask falls back to keyword matches |
| `DATA_FIXTURE` | Path to a JSON snapshot (e.g. `data/fixture.json`) to run offline without Supabase |

## Changing the styling

`static/app.css` is generated. After editing templates or `styles/input.css`:

```sh
tailwindcss -c tailwind.config.js -i styles/input.css -o static/app.css --minify
```

## Deploy

`web/Dockerfile` builds a container that runs anywhere Docker does (Render, Fly.io, Railway).
