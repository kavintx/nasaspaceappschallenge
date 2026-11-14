# Burn Scan: what the site expects from the backend

Proposed shapes, written before the endpoints existed. The site is built to these;
if the backend ships something different, change `web/src/scan.rs` (`read_scan`, `read_matches`).

**Photo identification currently runs in the site itself** (`web/src/vision.rs`), with Ollama or
Anthropic chosen by `VISION_PROVIDER` / `VISION_FALLBACK` (see `web/.env.example`). It returns the
`/ai/scan` shape below plus `"model"`, the provider that answered. When the backend ships `/ai/scan`,
point `scan::identify` at it and delete `vision.rs`.

Material lookup goes to `{API_BASE_URL}/api/v1/materials/match` with `Authorization: Bearer {API_AUTH_TOKEN}`,
like every other backend call. A 404 or 405 is shown as "Material lookup not connected".

## POST /api/v1/ai/scan

Request: `multipart/form-data`
- `image`: one JPEG, long edge at most 1280 px, under 2 MiB
- `mode`: `item` (default) or `label` (read the fibre/material text on a label)

Response:

```json
{
  "item": "Hoodie",
  "confidence": 0.82,
  "unclear": false,
  "message": null,
  "summary": "A grey hooded sweatshirt that looks like a cotton blend, with a metal zipper.",
  "model": "anthropic:claude-haiku-5-5",
  "parts": [
    { "name": "body", "materials": [
      { "material": "cotton", "share": 80, "confidence": 0.8 },
      { "material": "polyester", "share": 20, "confidence": 0.6 }
    ] },
    { "name": "drawstring", "materials": [ { "material": "nylon", "share": null, "confidence": 0.5 } ] }
  ]
}
```

- `confidence` is 0–1 (0–100 is also accepted). Below 0.6 the site flags the guess and asks for another photo.
- `unclear: true`, or no parts, means the photo could not be read. `message` may say why.
- `share` is a rough percentage of that part, or null.
- `summary` is one or two sentences on what the photo shows. It describes the photo only and says nothing about fire or tests.

## POST /api/v1/materials/match

Request: `{ "materials": ["cotton", "polyester"] }`

Response:

```json
{
  "matches": [
    { "query": "cotton", "fuels": ["Cotton", "cotton fabric"], "closest": null },
    { "query": "polyester", "fuels": [], "closest": { "fuel": "Nomex", "note": "a different synthetic" } }
  ]
}
```

- `fuels`: values of `conditions.fuel_type` that are the same material as `query`. Real equivalences only.
- `closest`: a different tested material, used only when `fuels` is empty. The site shows it in its own
  block, labelled as a different material, and never copies its verdict onto the scanned material.

## What the site reads from Supabase

`conditions.fuel_type`, `oxygen_concentration`, `pressure`, `airflow`, and, when present,
`outcome` (`ignited`, `sustained_flame`, `self_extinguished`, `no_ignition`) and `outcome_note`.
A missing or unrecognised `outcome` is shown as "Tested, outcome not recorded".
