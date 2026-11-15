# Flame in Freefall

NASA Space Apps Challenge 2026 — *Flame in Freefall: SI-Powered Fire Safety Insights from Microgravity Combustion Data*.

Photograph an everyday item and see how its materials did in NASA's own microgravity fire tests — what burned, what self-extinguished, what the crew would breathe — at Moon-base (30% O₂) and ISS (21% O₂) conditions. Browse, compare, ask questions with citations, and manage the research pipeline.

## Quick start — one command

You need **Docker Desktop** (or any working `docker`), **Python 3.12 or newer**, and **git**. Nothing else — no Rust installation, no cargo.

```sh
git clone https://github.com/kavintx/nasaspaceappschallenge
cd nasaspaceappschallenge
py main.py
```

The first run asks **one** thing: which model identifies your photos (an Anthropic/OpenAI key, or Ollama). Everything else is automatic — it clones the research backend next to this repo if you don't have it, sets it up silently, generates the shared site token into both `.env` files, then builds and starts **both** containers. The research database and model server are the project's own setup, filled in by the team — a first-time runner is never asked and never needs to know about them. Keys are typed hidden and written only to local `.env` files — never printed, never committed. Later runs ask nothing and start in seconds.

Open **http://localhost:3101**. The first build compiles everything inside a `rust:1-bookworm` container and takes a few minutes; the results are kept in named Docker volumes, so every later start is nearly instant.

Scanning runs on your own model key and Supabase — no backend needed for it. If you skip or decline the backend, browsing and Burn Scan still work (the dashboard reads the research database from Supabase with a read-only publishable key; material synonyms fall back to exact name matches); accounts, AI answers and the Pipeline page need the backend running.

### main.py flags

| Flag | What it does |
|---|---|
| *(none)* | First run: wizard, then build and run dashboard **+ research backend** (backend cloned to `../fireset` if missing) |
| `--configure` | Re-run the scanner setup questions and exit |
| `--port N` | Dashboard port |
| `--backend-port N` | Backend port (default 8080) |
| `--no-build` | Skip cargo builds, start the existing binaries |
| `--stop` | Stop the containers this script started |
| `--logs` | Follow the dashboard's log |

The containers it manages are `flame-web` and `flame-backend`; everything it compiles lives in the `kav-target`, `fireset-target` and `fire-intel-cargo-registry` volumes.

## How it fits together

```
NASA NTRS ──> backend (Fireset repo) ──writes──> Supabase database <──reads── dashboard (web/)
                    ^                                                               │
                    └────── AI answers, accounts, pipeline (+ material synonyms) ────┘
```

- **Supabase** holds the research database: experiments, papers, extracted test conditions and findings.
- **`web/`** — this dashboard (Rust: Axum + Askama templates, Tailwind). It reads the database straight from Supabase with the publishable key, and relays anything that needs writing or AI to the backend. The browser never sees the backend token: the dashboard holds it server-side and swaps it for an HttpOnly session cookie of its own.
- **The [Fireset backend](https://github.com/MonsterLXW/Fireset)** ingests NTRS papers, extracts conditions with a self-hosted model, answers questions with citations, runs the Burn Scan vision endpoint and owns the user accounts. The `crates/` folder here is a snapshot of it (`f4622a8`); the Fireset repo is the source of truth.
- **`db/supabase_setup.sql`** — the backend's tables, with public read access set read-only.

### Where each page gets its data

| Page | Source |
|---|---|
| Explorer, experiment pages, Compare table, Habitat evidence, Literature, paper pages | Supabase (rolled up per experiment, cached 5 minutes; "Refresh" rebuilds) |
| Burn Scan (photo → materials → test results) | photo → your own model key (Anthropic/OpenAI/Ollama, via `/scan/identify`); verdicts from Supabase; material-name synonyms via backend `POST /materials/match` when it runs (exact matches only without it) |
| Log in / sign up / account | backend `/api/v1/auth/*`, relayed with an HttpOnly cookie |
| Ask the Data (streaming chat) | backend `POST /ai/chat`, through `/ask/stream` |
| Habitat Risk → AI risk assessment | backend `POST /ai/habitat/analyze` |
| Compare → AI comparison | backend `POST /ai/compare` |
| Experiment → AI synthesis | backend `POST /ai/summarize/{acronym}` |
| Pipeline | backend `GET /ai/health`, `GET /pipeline/status`, `POST /ai/test`, ingest/import/extract |
| Paper page actions | backend `POST /ai/extract/{id}`, `PATCH /conditions/{id}`, `PATCH /findings/{id}` |

Category, fuel phase, hardware, and the names and one-line descriptions of the 13 known experiments come from a small table in `web/src/catalog.rs`, taken from the NASA challenge brief. Rejected conditions and findings are left out of the roll-ups.

## Configuration

The dashboard reads `web/.env` (`main.py` creates one from `web/.env.example` on first run). Nothing secret is ever committed.

| Setting | Meaning |
|---|---|
| `PORT` | Dashboard port (default 3101; it steps past busy ports if unset) |
| `API_BASE_URL` | Where the dashboard reaches the backend. From inside its Docker container that is `http://host.docker.internal:8080`; from a plain `cargo run`, `http://127.0.0.1:8080` |
| `API_AUTH_TOKEN` | The backend's site token. Server-side only, never sent to a browser |
| `VISION_PROVIDER` | `anthropic`, `openai` or `ollama` — always a key on this machine, never the research backend. **Leave it unset** and the scanner picks itself by whichever key is present (anthropic → openai → ollama) — no ollama install needed unless you want it |
| `VISION_FALLBACK` | `anthropic`, `openai` or `none`: tried once if the provider fails |
| `OLLAMA_BASE_URL`, `OLLAMA_VISION_MODEL` | Local vision model for scanning |
| `ANTHROPIC_API_KEY`, `ANTHROPIC_MODEL` | Only for the anthropic provider |
| `READ_ONLY=true` | **Set this on any public deployment**: turns off ingest, extraction and editing. Browsing, AI answers and chat still work |
| `STATIC_DIR`, `SUPABASE_URL`, `SUPABASE_KEY` | Rarely needed; sane defaults built in |

The backend has its own `.env` (in the Fireset checkout): `DATABASE_URL` (use the Supabase **Session pooler** string — the backend uses prepared statements), `API_AUTH_TOKEN` (24+ chars), `BIND_ADDR` (default `0.0.0.0:8080`), `VLLM_BASE_URL` and model ids. It applies its SQL migrations itself on startup.

## Accounts

With the backend connected, **`/signup`** and **`/login`** are separate pages and **`/account`** shows the profile. Passwords are argon2id-hashed; sessions are random tokens stored only as SHA-256 hashes, 30-day expiry, per-email rate limiting on sign-up and log-in. The browser only ever gets an HttpOnly cookie — the site token stays on the dashboard server.

## Photos and the camera

- **Laptop/desktop:** "Take a photo" opens a live in-page camera. Browsers only grant webcam access on `localhost` or HTTPS — over a plain LAN address it falls back to the file picker.
- **Phone:** the buttons open the phone's own camera app (via the file input's `capture` attribute). Open `http://<your-computer's-LAN-IP>:3101` from the same Wi-Fi; on Windows, allow the port through the firewall if it doesn't load.
- Photos are resized in the browser, sent for identification, and **never stored or logged**.

## Developing without Docker

If your machine allows cargo (Docker exists precisely because some don't — Windows Smart App Control blocks unsigned build binaries):

```sh
cd web && cargo run        # http://localhost:3100 or next free port
```

After editing templates or `web/styles/input.css`, regenerate the stylesheet:

```sh
tailwindcss -c tailwind.config.js -i styles/input.css -o static/app.css --minify
```

`static/app.css` is generated — never hand-edit it.

## What the public key can do

The publishable key in `web/src/store.rs` can only read `experiments`, `experiment_papers`, `conditions`, `findings`, and `papers` without their full text. It cannot write anything, and cannot read chat history, job logs, users or sessions. The backend connects as the database owner.

## Deploy

`web/Dockerfile` builds a container that runs anywhere Docker does (Render, Fly.io, Railway). Set `READ_ONLY=true` and keep `API_AUTH_TOKEN` in the platform's secret store.

## Troubleshooting

| Symptom | Fix |
|---|---|
| `error: Docker is not running` | Start Docker Desktop, wait for the whale, re-run |
| Port busy | `python main.py --port 3102` |
| Scan says "Scanner not connected" | Run `python main.py --configure` and pick a scanner with a key (anthropic/openai/ollama), then restart |
| Camera never opens | Not on `localhost`/HTTPS — the file-picker fallback (and phone camera) still works |
| Phone can't reach the site | Windows Firewall: allow inbound TCP on the port for "Private networks" |
| Weird state after a crash | `python main.py --stop` then run again |
| First build very slow | Normal — crates download once into the registry volume |

## Working in this repo

See [AGENTS.md](AGENTS.md): one branch per task (`yourname/short-topic`), PRs into `main`, no force-pushes, and **no secrets in the repo, ever**.
