"""Extract microgravity test conditions from the NTRS papers listed in Supabase.

For each paper: download the PDF from NTRS, read the text page by page, ask Claude
for the test conditions it reports, then keep only rows whose quote is found
word for word on the cited page (and whose oxygen figure appears on that page).

Usage: python extract.py <workdir> <path to web/.env>   [ntrs_id ...]
Writes <workdir>/out/<ntrs>.json per paper and <workdir>/rows.json overall.
"""
import concurrent.futures as cf
import json
import os
import re
import sys
import unicodedata
import urllib.request

import anthropic
import fitz

WORK, ENV = sys.argv[1], sys.argv[2]
ONLY = set(sys.argv[3:])
SUPA = "https://hdawtwegmvuczzshrgmr.supabase.co/rest/v1"
SUPA_KEY = "sb_publishable_9BFkivzzJw701pXLfe8ttA_MNP-Pc6I"  # public read-only key from web/src/store.rs
MODEL = "claude-opus-5-5"
SECTION_CHARS = 140_000
MAX_ROWS_PER_SECTION = 30

for line in open(ENV, encoding="utf-8"):
    if line.startswith("ANTHROPIC_API_KEY="):
        os.environ["ANTHROPIC_API_KEY"] = line.split("=", 1)[1].strip()
client = anthropic.Anthropic(timeout=600.0)

os.makedirs(f"{WORK}/pdf", exist_ok=True)
os.makedirs(f"{WORK}/out", exist_ok=True)


def rest(path):
    req = urllib.request.Request(f"{SUPA}/{path}", headers={"apikey": SUPA_KEY})
    return json.load(urllib.request.urlopen(req, timeout=30))


def norm(text):
    """Lower-case words and numbers separated by single spaces (for finding figures)."""
    text = unicodedata.normalize("NFKC", text).lower().replace("­", "")
    text = re.sub(r"-\s*\n\s*", "", text)  # words split across lines
    return re.sub(r"[^a-z0-9%.]+", " ", text).strip()


def squash(text):
    """Letters and digits only. PDF text breaks spacing around subscripts and line ends,
    so quotes are compared with every space and punctuation mark removed."""
    text = unicodedata.normalize("NFKC", text).lower()
    return re.sub(r"[^a-z0-9]+", "", text)


OUTCOMES = ["ignited", "sustained_flame", "self_extinguished", "no_ignition"]
NULLABLE_STR = {"anyOf": [{"type": "string"}, {"type": "null"}]}
SCHEMA = {
    "type": "object",
    "additionalProperties": False,
    "required": ["conditions"],
    "properties": {
        "conditions": {
            "type": "array",
            "items": {
                "type": "object",
                "additionalProperties": False,
                "required": ["fuel_type", "fuel_geometry", "oxygen_concentration", "pressure", "airflow", "sensors", "objective",
                             "experiment", "outcome", "outcome_note", "source_page", "source_quote", "provenance_kind", "confidence"],
                "properties": {
                    "fuel_type": {"type": "string"},
                    "fuel_geometry": NULLABLE_STR,
                    "oxygen_concentration": NULLABLE_STR,
                    "pressure": NULLABLE_STR,
                    "airflow": NULLABLE_STR,
                    "sensors": {"type": "array", "items": {"type": "string"}},
                    "objective": NULLABLE_STR,
                    "experiment": NULLABLE_STR,
                    "outcome": {"anyOf": [{"type": "string", "enum": OUTCOMES}, {"type": "null"}]},
                    "outcome_note": NULLABLE_STR,
                    "source_page": {"type": "integer"},
                    "source_quote": {"type": "string"},
                    "provenance_kind": {"type": "string", "enum": ["reported", "extracted", "inferred"]},
                    "confidence": {"type": "number"},
                },
            },
        }
    },
}

PROMPT = """You are extracting structured test conditions from a NASA technical paper for a research database on fire in spacecraft.

The paper text follows, one page at a time, each page introduced by a line "=== PAGE n ===".
Experiments this paper is linked to in the database: {experiments}

Extract the test conditions this paper REPORTS RESULTS FOR. Be thorough: read tables, figure captions and the results text. Give one row per distinct combination of fuel, oxygen level, pressure and airflow that the paper reports. Where the paper gives a range instead of listing each test (for example "oxygen was varied from 17% to 21%"), give one row with the range. Where different oxygen levels had different outcomes, give a separate row for each.

Include a row only when ALL of these hold:
- It is a physical combustion test that was actually run in microgravity or in space: the International Space Station, a Cygnus vehicle (Saffire), the Space Shuttle or Mir, a drop tower, or a parabolic-flight aircraft.
- The paper itself reports the test (not a plan for a future test, not a numerical simulation, not a normal-gravity ground test, not a summary of somebody else's earlier paper).
- The page states the fuel or material that was burned.

Fields:
- fuel_type: the material or fuel as the paper names it, short (for example "PMMA", "cotton-fiberglass fabric (SIBAL)", "Nomex", "n-heptane"). No test numbers.
- fuel_geometry: shape and size if stated (for example "rod, 6.4 mm diameter", "flat sheet, 0.3 mm thick"), else null.
- oxygen_concentration: oxygen by volume as a percentage string, for example "21%" or "17-21%". Convert a mole fraction such as 0.21 to "21%". null if the page does not state it.
- pressure: as stated, with units (for example "1 atm", "101 kPa", "8.2 psia"), else null.
- airflow: imposed flow speed and direction as stated (for example "20 cm/s concurrent", "quiescent"), else null.
- sensors: instruments named for this test, [] if none.
- objective: one short sentence starting with where the test ran (for example "ISS, Microgravity Science Glovebox: ..."), else null.
- experiment: one of the linked experiment acronyms above if this test belongs to it, else null.
- outcome: only when the paper states what the flame did in this test: "ignited" (it lit; nothing more said), "sustained_flame" (it kept burning or the flame spread), "self_extinguished" (it lit, then went out on its own or blew off), "no_ignition" (it did not light). Otherwise null. Never infer an outcome.
- outcome_note: a few words from the paper on the outcome (for example "flame spread the full length of the sample"), else null.
- source_page: the PAGE number where the quote is.
- source_quote: 8 to 40 consecutive words copied EXACTLY from that page that state the condition (ideally the fuel and the oxygen level). Do not paraphrase, do not join text from different places, do not fix typos.
- provenance_kind: "reported" when the page states every field you filled in directly; "extracted" when you combined statements from this paper; "inferred" when any filled field is your inference.
- confidence: 0 to 1.

Limits: at most {max_rows} rows. If there are more, keep solid materials first, then the widest spread of oxygen levels. If nothing qualifies, return an empty list. Do not invent tests.

PAPER: {title}

{text}"""


def ask(title, experiments, text):
    with client.beta.messages.stream(
        model=MODEL,
        max_tokens=32000,
        betas=["server-side-fallback-2026-07-01"],
        fallbacks="default",
        output_config={"effort": "high", "format": {"type": "json_schema", "schema": SCHEMA}},
        messages=[{"role": "user", "content": PROMPT.format(experiments=", ".join(experiments) or "none", max_rows=MAX_ROWS_PER_SECTION, title=title, text=text)}],
    ) as stream:
        msg = stream.get_final_message()
    if msg.stop_reason in ("refusal", "max_tokens"):
        raise RuntimeError(f"stop_reason {msg.stop_reason}")
    body = next(b.text for b in msg.content if b.type == "text")
    return json.loads(body)["conditions"], msg.usage.input_tokens, msg.usage.output_tokens


def check(row, squashed, normed):
    """Returns (row or None, reason). Fixes source_page when the quote is on a neighbouring page."""
    if len(norm(row["source_quote"]).split()) < 6:
        return None, "quote too short"
    quote = squash(row["source_quote"])
    page = row["source_page"]
    found = next((p for p in [page, page - 1, page + 1] if p in squashed and quote in squashed[p]), None)
    if found is None:
        found = next((p for p in squashed if quote in squashed[p]), None)
    if found is None:
        return None, "quote not found in the paper"
    row["source_page"] = found
    o2 = row.get("oxygen_concentration")
    if o2:
        for number in re.findall(r"\d+(?:\.\d+)?", o2):
            n = float(number)
            forms = {number, f"{n:g}", f"{n / 100:g}", f"{n / 100:.2f}", f"{n / 100:.3f}".rstrip("0")}
            if not any(re.search(rf"(?<![\d.]){re.escape(f)}(?!\d)", normed[found]) for f in forms if f):
                return None, f"oxygen figure {number} not on page {found}"
    if row["outcome"] not in OUTCOMES:
        row["outcome"] = None
    if not row["fuel_type"].strip():
        return None, "no fuel"
    return row, "ok"


def paper(p, acronyms):
    ntrs = p["ntrs_id"]
    out_path = f"{WORK}/out/{ntrs}.json"
    if os.path.exists(out_path):
        return json.load(open(out_path, encoding="utf-8"))
    pdf_path = f"{WORK}/pdf/{ntrs}.pdf"
    if not os.path.exists(pdf_path):
        req = urllib.request.Request(p["pdf_url"], headers={"User-Agent": "research-script/1.0"})
        with urllib.request.urlopen(req, timeout=120) as r, open(pdf_path, "wb") as f:
            f.write(r.read())
    doc = fitz.open(pdf_path)
    pages = {i + 1: page.get_text() for i, page in enumerate(doc)}
    normed = {n: norm(t) for n, t in pages.items()}
    squashed = {n: squash(t) for n, t in pages.items()}
    result = {"ntrs_id": ntrs, "paper_id": p["id"], "pages": len(pages), "chars": sum(map(len, pages.values())),
              "rows": [], "dropped": [], "in_tokens": 0, "out_tokens": 0, "error": None}
    if result["chars"] < 800:
        result["error"] = "no readable text in the PDF"
    else:
        sections, current, size = [], [], 0
        for n, t in pages.items():
            if size + len(t) > SECTION_CHARS and current:
                sections.append(current)
                current, size = [], 0
            current.append(n)
            size += len(t)
        sections.append(current)
        try:
            for nums in sections:
                text = "\n".join(f"=== PAGE {n} ===\n{pages[n]}" for n in nums)
                rows, tin, tout = ask(p["title"], acronyms, text)
                result["in_tokens"] += tin
                result["out_tokens"] += tout
                for row in rows:
                    kept, why = check(row, squashed, normed)
                    if kept:
                        result["rows"].append(kept)
                    else:
                        result["dropped"].append({"why": why, "fuel": row.get("fuel_type"), "quote": row.get("source_quote", "")[:120]})
        except Exception as e:  # keep going; the paper is reported as failed
            result["error"] = f"{type(e).__name__}: {str(e)[:200]}"
            result["rows"] = []
    if not result["error"]:
        json.dump(result, open(out_path, "w", encoding="utf-8"), ensure_ascii=False, indent=1)
    return result


def main():
    papers = rest("papers?select=id,ntrs_id,title,pdf_url&pdf_url=not.is.null&order=ntrs_id.asc")
    exps = {e["id"]: e["acronym"] for e in rest("experiments?select=id,acronym")}
    links = {}
    for l in rest("experiment_papers?select=experiment_id,paper_id"):
        links.setdefault(l["paper_id"], []).append(l["experiment_id"])
    if ONLY:
        papers = [p for p in papers if p["ntrs_id"] in ONLY]
    acr_to_id = {a: i for i, a in exps.items()}
    results = []
    with cf.ThreadPoolExecutor(max_workers=4) as pool:
        jobs = {pool.submit(paper, p, [exps[e] for e in links.get(p["id"], [])]): p for p in papers}
        for job in cf.as_completed(jobs):
            p = jobs[job]
            try:
                r = job.result()
            except Exception as e:
                r = {"ntrs_id": p["ntrs_id"], "paper_id": p["id"], "rows": [], "dropped": [], "in_tokens": 0, "out_tokens": 0,
                     "pages": 0, "chars": 0, "error": f"{type(e).__name__}: {str(e)[:200]}"}
            results.append(r)
            print(f'{r["ntrs_id"]}: {r["pages"]} pages, kept {len(r["rows"])}, dropped {len(r["dropped"])}'
                  + (f', ERROR {r["error"]}' if r["error"] else ""), flush=True)
    rows, seen = [], set()
    for r in sorted(results, key=lambda r: r["ntrs_id"]):
        linked = {exps[e] for e in links.get(r["paper_id"], [])}
        for row in r["rows"]:
            key = (r["paper_id"], row["fuel_type"].lower(), row["oxygen_concentration"], row["pressure"], row["airflow"], row["outcome"])
            if key in seen:
                continue
            seen.add(key)
            exp = row["experiment"] if row["experiment"] in linked else None
            rows.append({**row, "paper_id": r["paper_id"], "ntrs_id": r["ntrs_id"], "experiment_id": acr_to_id.get(exp)})
    json.dump(rows, open(f"{WORK}/rows.json", "w", encoding="utf-8"), ensure_ascii=False, indent=1)
    tin, tout = sum(r["in_tokens"] for r in results), sum(r["out_tokens"] for r in results)
    print(f"\npapers {len(results)}, failed {sum(1 for r in results if r['error'])}, rows kept {len(rows)}, "
          f"dropped {sum(len(r['dropped']) for r in results)}, tokens in {tin} out {tout}, "
          f"approx ${tin / 1e6 * 4 + tout / 1e6 * 20:.2f}")


main()
