# Working in this repo (for AI coding assistants and people)

Several people and their AI assistants work here at the same time. Read this first.

## What we're building

**Flame in Freefall** — fire safety for the first Moon base: from the evidence, to the engineer, to the classroom.
NASA Space Apps 2026. Challenges: *Flame in Freefall* (main) and *Build a Junior Astronaut Mission Trainer*.

- **Know:** browse NASA microgravity combustion experiments, papers, extracted test conditions and findings.
- **Decide:** design a habitat and see what has been tested at its oxygen level and where the gaps are.
- **Train:** a game for kids 7–12 running a Moon base, driven by the same findings.

## Layout and who owns what

| Path | What | Owner |
|---|---|---|
| `web/` | Dashboard: Rust (Axum + Askama templates), Tailwind. Reads Supabase; calls the backend for AI | Kavin |
| `crates/`, `migrations/`, root `Cargo.*` | `fire-intel` backend: ingestion, extraction, AI answers. Source of truth is `MonsterLXW/Fireset` | Gus |
| `db/` | Supabase setup SQL | Kavin |

Change files outside your own area only after telling the owner.

## How to work together without breaking each other

1. `git pull` before you start and again before you push.
2. One branch per task, named `yourname/short-topic`. Keep changes small.
3. Before pushing, the part you touched must build:
   - dashboard: `cd web && cargo build`
   - backend: `cargo build` in the repo root
4. Merge to `main` only when it builds. If `main` moved, rebase or merge it into your branch first; never force-push `main`.
5. Say what you changed in the commit message, in plain words.

## Rules that are easy to break

- **No secrets in the repo.** No tokens, database passwords or connection strings; `.env` files stay local.
  The only key allowed in code is the Supabase *publishable* key in `web/src/store.rs`.
- **No invented data.** Every number or claim shown on the site comes from the database or the NASA brief,
  with its source. If something is missing, show "—" or say so; do not fill it in.
- **Database schema changes** go through the backend's `migrations/` (Gus), then `db/` if access rules change.
  The public key must stay read-only.
- **`web/static/app.css` is generated.** After changing templates or `web/styles/input.css`, run
  `tailwindcss -c tailwind.config.js -i styles/input.css -o static/app.css --minify` in `web/`.
- **Text on the site** is plain and specific: no marketing slogans, no decorative labels.
- Anything that writes to the database from the dashboard must respect `READ_ONLY`.

See `README.md` for how to run each part and where each page gets its data.
