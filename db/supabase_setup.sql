-- Sets up the fire-intel schema in Supabase and opens read-only public access for the dashboard.
-- Run once in the Supabase SQL editor. It removes the early dashboard scaffold tables first.

set search_path = public, extensions;

drop function if exists public.match_papers(extensions.vector, int);
drop table if exists public.habitat_scores;
drop table if exists public.papers;
drop table if exists public.experiments;

create extension if not exists vector with schema extensions;
create extension if not exists pg_trgm with schema extensions;

-- New tables are not reachable through the public API unless granted below.
alter default privileges for role postgres in schema public revoke all on tables from anon, authenticated;
alter default privileges for role postgres in schema public revoke all on sequences from anon, authenticated;
alter default privileges for role postgres in schema public revoke all on functions from anon, authenticated;

-- ===== migrations/0001_init.sql, unchanged =====
-- FLAME research platform (fire-intel): initial schema

-- ---------- papers (NTRS publications) ----------
CREATE TABLE papers (
    id                uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    ntrs_id           text NOT NULL UNIQUE,          -- NTRS citation id (numeric submission id or uuid)
    title             text NOT NULL DEFAULT '',
    abstract          text NOT NULL DEFAULT '',
    authors           jsonb NOT NULL DEFAULT '[]',   -- ["Family, Given", ...]
    publication_date  text,                          -- ISO date or partial, as reported by NTRS
    doi               text,
    ntrs_url          text NOT NULL DEFAULT '',
    pdf_url           text,
    pdf_path          text,                          -- local storage path once downloaded
    pub_type          text NOT NULL DEFAULT '',
    fulltext          text NOT NULL DEFAULT '',      -- normalized extracted text
    pages_total       int,
    discovery_query   text NOT NULL DEFAULT '',
    status            text NOT NULL DEFAULT 'discovered'
        CHECK (status IN ('discovered','metadata','downloaded','extracted','chunked',
                          'matched','extracted_claims','complete','failed')),
    error             text,
    retry_count       int NOT NULL DEFAULT 0,
    first_seen_at     timestamptz NOT NULL DEFAULT now(),
    updated_at        timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX papers_status_idx ON papers(status);

-- ---------- experiments ----------
CREATE TABLE experiments (
    id                 uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    acronym            text NOT NULL UNIQUE,
    name               text NOT NULL DEFAULT '',
    aliases            jsonb NOT NULL DEFAULT '[]',
    description        text NOT NULL DEFAULT '',
    source             text NOT NULL DEFAULT 'seed',  -- seed | sheet | discovered
    verification_status text NOT NULL DEFAULT 'seed'
        CHECK (verification_status IN ('seed','reviewed','verified')),
    created_at         timestamptz NOT NULL DEFAULT now(),
    updated_at         timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE experiment_papers (
    experiment_id  uuid NOT NULL REFERENCES experiments(id) ON DELETE CASCADE,
    paper_id       uuid NOT NULL REFERENCES papers(id) ON DELETE CASCADE,
    match_method   text NOT NULL DEFAULT 'acronym' CHECK (match_method IN ('acronym','sheet','ai','manual')),
    confidence     real NOT NULL DEFAULT 1.0,
    created_at     timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (experiment_id, paper_id)
);

-- ---------- chunks (searchable, page-tracked, embedded) ----------
CREATE TABLE paper_chunks (
    id                uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    paper_id          uuid NOT NULL REFERENCES papers(id) ON DELETE CASCADE,
    ordinal           int NOT NULL,
    page_start        int,                    -- 1-based page of PDF the chunk starts on
    page_end          int,
    char_start        int NOT NULL,
    char_end          int NOT NULL,
    text              text NOT NULL,
    text_tsv          tsvector GENERATED ALWAYS AS (to_tsvector('english', text)) STORED,
    embedding         vector(1024),           -- dim must match VLLM_EMBED_DIM; NULL until served
    embedding_model   text,
    embedding_status  text NOT NULL DEFAULT 'pending'
        CHECK (embedding_status IN ('pending','ready','unavailable','failed')),
    UNIQUE (paper_id, ordinal)
);
CREATE INDEX chunks_tsv_idx ON paper_chunks USING gin (text_tsv);
CREATE INDEX chunks_embedding_idx ON paper_chunks USING hnsw (embedding vector_cosine_ops);

-- ---------- experimental conditions (per paper, optionally per experiment) ----------
CREATE TABLE conditions (
    id                  uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    paper_id            uuid NOT NULL REFERENCES papers(id) ON DELETE CASCADE,
    experiment_id       uuid REFERENCES experiments(id) ON DELETE SET NULL,
    fuel_type           text,
    fuel_geometry       text,
    oxygen_concentration text,
    pressure            text,
    airflow             text,
    sensors             jsonb,                     -- ["camera", ...]
    objective           text,
    source_page         int,                       -- 1-based, NULL when unknown
    source_quote        text,
    provenance_kind     text NOT NULL DEFAULT 'extracted'
        CHECK (provenance_kind IN ('reported','extracted','inferred','unknown')),
    confidence          real NOT NULL DEFAULT 0.0, -- 0..1 extraction confidence
    verification_status text NOT NULL DEFAULT 'pending'
        CHECK (verification_status IN ('pending','verified','corrected','rejected')),
    correction_note     text,
    created_at          timestamptz NOT NULL DEFAULT now(),
    updated_at          timestamptz NOT NULL DEFAULT now()
);

-- ---------- findings (claims, linked to source page, classified) ----------
CREATE TABLE findings (
    id                  uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    paper_id            uuid NOT NULL REFERENCES papers(id) ON DELETE CASCADE,
    experiment_id       uuid REFERENCES experiments(id) ON DELETE SET NULL,
    text                text NOT NULL,
    category            text CHECK (category IN ('prevention','detection','suppression','other') OR category IS NULL),
    source_page         int,
    source_quote        text,
    provenance_kind     text NOT NULL DEFAULT 'extracted'
        CHECK (provenance_kind IN ('reported','extracted','inferred','unknown')),
    confidence          real NOT NULL DEFAULT 0.0,
    verification_status text NOT NULL DEFAULT 'pending'
        CHECK (verification_status IN ('pending','verified','corrected','rejected')),
    correction_note     text,
    created_at          timestamptz NOT NULL DEFAULT now(),
    updated_at          timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX findings_category_idx ON findings(category);

-- ---------- pipeline jobs (retries, incremental, status tracking) ----------
CREATE TABLE pipeline_jobs (
    id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    kind          text NOT NULL,                -- discover|metadata|download|extract_text|chunk|match|extract_claims|embed
    paper_id      uuid REFERENCES papers(id) ON DELETE CASCADE,
    payload       jsonb NOT NULL DEFAULT '{}',
    status        text NOT NULL DEFAULT 'queued'
        CHECK (status IN ('queued','running','done','failed','skipped')),
    attempts      int NOT NULL DEFAULT 0,
    max_attempts  int NOT NULL DEFAULT 3,
    error         text,
    available_at  timestamptz NOT NULL DEFAULT now(),
    started_at    timestamptz,
    finished_at   timestamptz,
    created_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX pipeline_jobs_due_idx ON pipeline_jobs(status, available_at) WHERE status IN ('queued','failed');

-- ---------- Google Sheet import bookkeeping ----------
CREATE TABLE sheet_imports (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    sheet_url   text NOT NULL,
    total_rows  int NOT NULL DEFAULT 0,
    ok_rows     int NOT NULL DEFAULT 0,
    errors      jsonb NOT NULL DEFAULT '[]',
    created_at  timestamptz NOT NULL DEFAULT now()
);

-- ---------- conversations (research assistant history) ----------
CREATE TABLE conversations (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    title       text NOT NULL DEFAULT '',
    created_at  timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE messages (
    id               uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    conversation_id  uuid NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    role             text NOT NULL CHECK (role IN ('user','assistant')),
    content          text NOT NULL,
    citations        jsonb NOT NULL DEFAULT '[]',
    created_at       timestamptz NOT NULL DEFAULT now()
);

-- ---------- autonomous AI workflows ----------
CREATE TABLE workflow_runs (
    id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    goal          text NOT NULL,
    status        text NOT NULL DEFAULT 'running'
        CHECK (status IN ('running','done','failed','cancelled')),
    max_steps     int NOT NULL DEFAULT 24,
    result_summary text,
    error          text,
    created_at     timestamptz NOT NULL DEFAULT now(),
    finished_at    timestamptz
);
CREATE TABLE workflow_steps (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id      uuid NOT NULL REFERENCES workflow_runs(id) ON DELETE CASCADE,
    seq         int NOT NULL,
    kind        text NOT NULL CHECK (kind IN ('thought','tool_call','tool_result','final')),
    tool_name   text,
    input       jsonb,
    output      jsonb,
    ok          boolean,
    error       text,
    created_at  timestamptz NOT NULL DEFAULT now()
);

-- ---------- AI call audit ----------
CREATE TABLE ai_call_log (
    id          bigserial PRIMARY KEY,
    route       text NOT NULL,                  -- messages|embeddings|models
    model       text,
    http_status int,
    ok          boolean NOT NULL,
    in_tokens   int,
    out_tokens  int,
    latency_ms  int,
    error       text,
    created_at  timestamptz NOT NULL DEFAULT now()
);

-- ---------- seed experiments (names, aliases and descriptions per NASA's brief; keep in sync with fix_experiment_names.sql) ----------
INSERT INTO experiments (acronym, name, aliases, description, source) VALUES
 ('BASS',        'Burning and Suppression of Solids',                        '["BASS-I"]', 'ISS study of the flammability of solid materials such as fabrics and plastics, run in the Microgravity Science Glovebox (MSG) using the fan-driven flow duct built for SPICE.', 'seed'),
 ('BASS-II',     'Burning and Suppression of Solids II',                     '["BASS II","BASS-2"]', 'Second ISS study of the flammability of solid materials such as fabrics and plastics, run in the Microgravity Science Glovebox (MSG) flow duct.', 'seed'),
 ('SAFFIRE I',   'Spacecraft Fire Safety Demonstration I',                   '["SAFFIRE-I","Saffire I","Saffire-I"]', 'Spacecraft Fire Safety Demonstration: flammability of solid materials in microgravity.', 'seed'),
 ('SAFFIRE II',  'Spacecraft Fire Safety Demonstration II',                  '["SAFFIRE-II","Saffire II","Saffire-II"]', 'Spacecraft Fire Safety Demonstration: flammability of solid materials in microgravity.', 'seed'),
 ('SAFFIRE III', 'Spacecraft Fire Safety Demonstration III',                 '["SAFFIRE-III","Saffire III","Saffire-III"]', 'Spacecraft Fire Safety Demonstration: flammability of solid materials in microgravity.', 'seed'),
 ('SAME',        'Smoke and Aerosol Measurement Experiment',                 '[]', 'Tested fire detection in microgravity by measuring smoke.', 'seed'),
 ('SAME-R',      'Smoke and Aerosol Measurement Experiment Reflight',        '["SAME Reflight"]', 'Reflight of SAME: tested fire detection in microgravity by measuring smoke.', 'seed'),
 ('FLEX',        'Flame Extinguishment Experiment',                          '[]', 'Studied fire suppression by diluting the chamber atmosphere with carbon dioxide and helium while burning droplets of liquid fuel in the Multi-user Droplet Combustion Apparatus (MDCA) in the Combustion Integrated Rack (CIR).', 'seed'),
 ('FLEX-2',      'Flame Extinguishment Experiment 2',                        '["FLEX II","FLEX 2"]', 'Burning droplets of liquid fuel in the Multi-user Droplet Combustion Apparatus (MDCA) in the Combustion Integrated Rack (CIR).', 'seed'),
 ('CFI',         'Cool Flames Investigation',                                '["Cool Flames"]', 'Burning droplets of liquid fuel in the Multi-user Droplet Combustion Apparatus (MDCA) in the Combustion Integrated Rack (CIR).', 'seed'),
 ('ACME',        'Advanced Combustion via Microgravity Experiments',         '["BRE","Burning Rate Emulator","CFI-G","Cool Flames Investigation with Gases","E-FIELD Flames"]', 'Six experiments on flames of gaseous fuels, all run with the same modular hardware in the Combustion Integrated Rack (CIR). Includes the Burning Rate Emulator (materials flammability in still atmospheres), Cool Flames Investigation with Gases (up to 85% oxygen) and E-FIELD Flames.', 'seed'),
 ('SPICE',       'Smoke Point In Co-flow Experiment',                        '[]', 'Run in the Microgravity Science Glovebox (MSG) using the modular fan-driven flow duct that was built for it.', 'seed'),
 ('SLICE',       'Structure & Liftoff In Combustion Experiment',             '["Structure and Liftoff In Combustion Experiment"]', 'Run in the Microgravity Science Glovebox (MSG) using the flow duct built for SPICE.', 'seed');

-- Tell the backend's migration runner (sqlx) that migration 1 is already applied.
create table if not exists public._sqlx_migrations (
    version bigint primary key,
    description text not null,
    installed_on timestamptz not null default now(),
    success boolean not null,
    checksum bytea not null,
    execution_time bigint not null
);
insert into public._sqlx_migrations (version, description, success, checksum, execution_time)
values (1, 'init', true, decode('10e9c6970315cad688d2c4f85be68ea3b59a792ffac5933a751acef22cb013aa049e64a15b482ab1e84f5b170ee65113','hex'), 0)
on conflict (version) do nothing;

-- ===== access rules =====
-- The backend connects as the database owner and is unaffected by these.
-- The public (publishable-key) API may only read the research tables.
do $$
declare t text;
begin
  foreach t in array array['papers','experiments','experiment_papers','paper_chunks','conditions','findings',
                           'pipeline_jobs','sheet_imports','conversations','messages','workflow_runs',
                           'workflow_steps','ai_call_log','_sqlx_migrations'] loop
    execute format('alter table public.%I enable row level security', t);
    execute format('revoke all on public.%I from anon, authenticated', t);
  end loop;
end $$;
revoke all on sequence public.ai_call_log_id_seq from anon, authenticated;

grant select on public.experiments, public.experiment_papers, public.conditions, public.findings to anon, authenticated;
-- Papers: everything except the full text, the server file path and internal bookkeeping.
grant select (id, ntrs_id, title, abstract, authors, publication_date, doi, ntrs_url, pdf_url, pub_type, pages_total, status, updated_at)
  on public.papers to anon, authenticated;

create policy "public read" on public.experiments       for select to anon, authenticated using (true);
create policy "public read" on public.experiment_papers for select to anon, authenticated using (true);
create policy "public read" on public.conditions        for select to anon, authenticated using (true);
create policy "public read" on public.findings          for select to anon, authenticated using (true);
create policy "public read" on public.papers            for select to anon, authenticated using (true);

select acronym, name from public.experiments order by acronym;
