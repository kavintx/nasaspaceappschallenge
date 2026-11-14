-- Flame in Freefall — database schema (Supabase / Postgres)
-- Owner: Kavin. Gus's pipeline writes into these tables; the frontend reads them.

create extension if not exists vector;

-- One row per PSI investigation
create table experiments (
  id            text primary key,              -- short slug, e.g. 'bass-ii'
  name          text not null,                 -- 'Burning and Suppression of Solids II'
  acronym       text not null,                 -- 'BASS-II'
  category      text check (category in ('prevention','detection','suppression','other')),
  fuel_type     text check (fuel_type in ('solid','liquid','gas','mixed','none')),
  fuel_details  text,                          -- materials, sample shape/size
  hardware      text,                          -- 'MSG flow duct', 'CIR / MDCA', 'CIR / ACME'
  platform      text default 'ISS',
  gravity       text default 'microgravity',
  oxygen_min_pct numeric,                      -- O2 range tested
  oxygen_max_pct numeric,
  pressure_min_kpa numeric,
  pressure_max_kpa numeric,
  airflow_min_cms numeric,                     -- cm/s
  airflow_max_cms numeric,
  sensors       text[],                        -- ['thermocouples','video','radiometer']
  objective     text,
  key_findings  text,
  ai_summary    text,                          -- filled by Gus's /summarize job
  earth_application text,
  psi_url       text,
  updated_at    timestamptz default now()
);

-- NTRS papers linked to an experiment (filled by Gus's NTRS script + online teammate's sheet)
create table papers (
  id            bigint generated always as identity primary key,
  experiment_id text references experiments(id) on delete cascade,
  ntrs_id       text unique,
  title         text not null,
  authors       text,
  year          int,
  abstract      text,
  url           text,
  embedding     vector(1536)                   -- for /ask search
);

-- Cached habitat relevance scores (filled by Gus's /rank job)
create table habitat_scores (
  experiment_id text references experiments(id) on delete cascade,
  habitat       text check (habitat in ('iss','moon_base','rover','mars')),
  score         numeric check (score between 0 and 100),
  reason        text,
  primary key (experiment_id, habitat)
);

create index on papers (experiment_id);

-- Public read-only access for the dashboard; writes only via service key (Gus's pipeline)
alter table experiments    enable row level security;
alter table papers         enable row level security;
alter table habitat_scores enable row level security;
create policy "public read" on experiments    for select using (true);
create policy "public read" on papers         for select using (true);
create policy "public read" on habitat_scores for select using (true);

-- Seed: the investigations named in the challenge prompt
insert into experiments (id, acronym, name, category, fuel_type, hardware) values
 ('bass',      'BASS',      'Burning and Suppression of Solids',               'prevention',  'mixed',  'MSG flow duct'),
 ('bass-ii',   'BASS-II',   'Burning and Suppression of Solids II',            'prevention',  'mixed',  'MSG flow duct'),
 ('saffire-i', 'Saffire-I', 'Spacecraft Fire Safety Demonstration I',          'prevention',  'solid',  'Cygnus vehicle'),
 ('saffire-ii','Saffire-II','Spacecraft Fire Safety Demonstration II',         'prevention',  'solid',  'Cygnus vehicle'),
 ('saffire-iii','Saffire-III','Spacecraft Fire Safety Demonstration III',      'prevention',  'solid',  'Cygnus vehicle'),
 ('same',      'SAME',      'Smoke and Aerosol Measurement Experiment',        'detection',   'none',   'MSG'),
 ('same-r',    'SAME-R',    'Smoke and Aerosol Measurement Experiment Reflight','detection',  'none',   'MSG'),
 ('flex',      'FLEX',      'Flame Extinguishment Experiment',                 'suppression', 'liquid', 'CIR / MDCA'),
 ('flex-2',    'FLEX-2',    'Flame Extinguishment Experiment 2',               'suppression', 'liquid', 'CIR / MDCA'),
 ('cfi',       'CFI',       'Cool Flames Investigation',                       'other',       'liquid', 'CIR / MDCA'),
 ('acme-bre',  'ACME BRE',  'ACME Burning Rate Emulator',                      'prevention',  'gas',    'CIR / ACME'),
 ('acme-cfi-g','ACME CFI-G','ACME Cool Flames Investigation with Gases',       'other',       'gas',    'CIR / ACME'),
 ('acme-efield','ACME E-FIELD','ACME Electric-Field Effects on Laminar Diffusion Flames','other','gas','CIR / ACME'),
 ('spice',     'SPICE',     'Smoke Point In Co-flow Experiment',               'other',       'gas',    'MSG flow duct'),
 ('slice',     'SLICE',     'Structure & Liftoff In Combustion Experiment',    'other',       'gas',    'MSG flow duct');
