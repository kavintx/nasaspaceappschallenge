"""Second pass over rows.json: keep an outcome only when the paper states it.

For every row with an outcome, the cited page and its neighbours are sent back to the
model with the claimed outcome. The outcome survives only if the model confirms it and
gives a quote that is found word for word in that text. Otherwise it becomes "unknown".

Usage: python verify.py <workdir> <path to web/.env>
Reads <workdir>/rows.json and <workdir>/pdf/*.pdf; writes <workdir>/conditions.json.
"""
import concurrent.futures as cf
import json
import os
import re
import sys
import unicodedata

import anthropic
import fitz

WORK, ENV = sys.argv[1], sys.argv[2]
MODEL = "claude-opus-5-5"
for line in open(ENV, encoding="utf-8"):
    if line.startswith("ANTHROPIC_API_KEY="):
        os.environ["ANTHROPIC_API_KEY"] = line.split("=", 1)[1].strip()
client = anthropic.Anthropic(timeout=300.0)


def squash(text):
    return re.sub(r"[^a-z0-9]+", "", unicodedata.normalize("NFKC", text).lower())


SCHEMA = {
    "type": "object",
    "additionalProperties": False,
    "required": ["supported", "outcome_quote", "outcome_note"],
    "properties": {
        "supported": {"type": "boolean"},
        "outcome_quote": {"anyOf": [{"type": "string"}, {"type": "null"}]},
        "outcome_note": {"anyOf": [{"type": "string"}, {"type": "null"}]},
    },
}
MEANING = {
    "ignited": "the material ignited (nothing more is claimed)",
    "sustained_flame": "the flame kept burning or spread along the material",
    "self_extinguished": "the flame went out on its own, quenched, or blew off",
    "no_ignition": "the material did not ignite or did not burn",
}
PROMPT = """A database row claims the following about a test reported in a NASA paper. Check it against the page text below.

Fuel: {fuel}
Oxygen: {o2}
Pressure: {pressure}
Airflow: {airflow}
Claimed outcome: {outcome} — meaning {meaning}

Set supported to true only if the text below explicitly states that outcome for this fuel under these conditions (a statement, a table entry or a figure caption). If the text describes a different fuel, a different condition, a simulation, a normal-gravity test, or only implies the outcome, set supported to false.

When supported, outcome_quote must be 6 to 40 consecutive words copied EXACTLY from the text that state the outcome, and outcome_note a plain summary of at most 15 words. Otherwise both are null.

{text}"""

pdfs = {}


def pages_of(ntrs):
    if ntrs not in pdfs:
        doc = fitz.open(f"{WORK}/pdf/{ntrs}.pdf")
        pdfs[ntrs] = {i + 1: p.get_text() for i, p in enumerate(doc)}
    return pdfs[ntrs]


def verify(row):
    pages = pages_of(row["ntrs_id"])
    nums = [n for n in (row["source_page"] - 1, row["source_page"], row["source_page"] + 1) if n in pages]
    text = "\n".join(f"=== PAGE {n} ===\n{pages[n]}" for n in nums)
    with client.beta.messages.stream(
        model=MODEL,
        max_tokens=8000,
        betas=["server-side-fallback-2026-07-01"],
        fallbacks="default",
        output_config={"effort": "medium", "format": {"type": "json_schema", "schema": SCHEMA}},
        messages=[{"role": "user", "content": PROMPT.format(
            fuel=row["fuel_type"], o2=row["oxygen_concentration"] or "not stated", pressure=row["pressure"] or "not stated",
            airflow=row["airflow"] or "not stated", outcome=row["outcome"], meaning=MEANING[row["outcome"]], text=text)}],
    ) as stream:
        msg = stream.get_final_message()
    if msg.stop_reason in ("refusal", "max_tokens"):
        return None, None, None, msg.usage
    reply = json.loads(next(b.text for b in msg.content if b.type == "text"))
    quote = reply.get("outcome_quote") or ""
    if not reply["supported"] or len(quote.split()) < 5:
        return None, None, None, msg.usage
    q = squash(quote)
    page = next((n for n in [row["source_page"]] + nums if q in squash(pages[n])), None)
    if page is None:
        return None, None, None, msg.usage
    return page, quote, reply.get("outcome_note"), msg.usage


def main():
    rows = json.load(open(f"{WORK}/rows.json", encoding="utf-8"))
    rows = [r for r in rows if r["provenance_kind"] != "inferred" and r["confidence"] >= 0.5]
    for ntrs in {r["ntrs_id"] for r in rows}:
        pages_of(ntrs)  # read PDFs up front, single-threaded
    todo = [r for r in rows if r["outcome"]]
    tin = tout = kept = 0

    def work(r):
        try:
            return r, verify(r)
        except Exception as e:
            print("verify failed:", type(e).__name__, str(e)[:120], flush=True)
            return r, (None, None, None, None)

    with cf.ThreadPoolExecutor(max_workers=6) as pool:
        for r, (page, quote, note, usage) in pool.map(work, todo):
            if usage:
                tin += usage.input_tokens
                tout += usage.output_tokens
            if page:
                kept += 1
                r["outcome_source_page"], r["outcome_source_quote"] = page, quote
                r["outcome_note"] = note or r["outcome_note"]
            else:
                r["outcome"] = None
    out = []
    for r in rows:
        confirmed = bool(r["outcome"])
        out.append({
            "paper_id": r["paper_id"], "ntrs_id": r["ntrs_id"], "experiment_id": r["experiment_id"],
            "fuel_type": r["fuel_type"], "fuel_geometry": r["fuel_geometry"], "oxygen_concentration": r["oxygen_concentration"],
            "pressure": r["pressure"], "airflow": r["airflow"], "sensors": r["sensors"], "objective": r["objective"],
            "source_page": r["source_page"], "source_quote": r["source_quote"], "provenance_kind": r["provenance_kind"],
            "confidence": r["confidence"], "outcome": r["outcome"] if confirmed else "unknown",
            "outcome_note": r["outcome_note"] if confirmed else None,
            "outcome_source_page": r.get("outcome_source_page") if confirmed else None,
            "outcome_source_quote": r.get("outcome_source_quote") if confirmed else None,
        })
    json.dump(out, open(f"{WORK}/conditions.json", "w", encoding="utf-8"), ensure_ascii=False, indent=1)
    print(f"rows {len(out)}, outcomes claimed {len(todo)}, confirmed {kept}, tokens in {tin} out {tout}, approx ${tin / 1e6 * 4 + tout / 1e6 * 20:.2f}")


main()
