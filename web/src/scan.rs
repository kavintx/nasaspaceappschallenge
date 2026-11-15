//! Burn Scan: photograph an item, confirm what it is made of, and see how those
//! materials did in NASA's fire tests at Moon Base and ISS oxygen levels.
//!
//!   POST /scan/identify  ->  vision.rs (Ollama or Anthropic)       (photo -> parts and materials)
//!   POST /scan/result    ->  backend POST /api/v1/materials/match (material name -> tested fuels)
//! Keys and the backend token stay on this server and never reach the browser.
//! Photos are passed straight through and never written to disk. Verdicts come
//! only from the `conditions` table; gases and limits only from `air.rs`.
//! The request and response shapes this file expects are in web/SCAN_API.md.

use crate::admin::same_origin;
use crate::air;
use crate::backend::{f, flag, s, Fail};
use crate::catalog::{Catalog, Outcome, Test};
use crate::views::{load_catalog, num, render, HABITATS};
use crate::vision::VisionError;
use crate::AppState;
use askama::Template;
use axum::{
    body::Bytes,
    extract::{Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::Arc;

type AppRef = State<Arc<AppState>>;

/// Photos over 2 MiB stop here.
/// The browser resizes to a 1280 px long edge first, which is far below this.
pub const MAX_IMAGE_BYTES: usize = 2 * 1024 * 1024;
const MAX_MATERIALS: usize = 12;
const MAX_NAME_CHARS: usize = 60;
/// A test counts as "at" a habitat's oxygen level when it is within this many percentage points.
const O2_TOLERANCE: f64 = 1.0;
/// Result cards compare these two habitats, in this order.
const SHOWN: [&str; 2] = ["moon_base", "iss"];

/// The same message on the public website, where there is no box to paste a key into.
const NOT_CONNECTED_HOSTED: &str = "The photo scanner isn’t available right now. You can still type the materials in.";

const NOT_CONNECTED: &str = "Scanner not connected. Paste your Anthropic or OpenAI key in the box above, or run Ollama — you can still type the materials in.";

// ---------- pages ----------

pub struct Stat {
    pub value: String,
    pub label: &'static str,
    pub href: &'static str,
}

/// The home page: what the site is, and live counts from the research database.
#[derive(Template)]
#[template(path = "overview.html")]
pub struct OverviewPage {
    pub active: &'static str,
    pub page_title: String,
    pub stats: Vec<Stat>,
    /// The most-tested fuels: name, number of test conditions, and a search link.
    pub materials: Vec<(String, usize, String)>,
    pub data_error: Option<String>,
}

pub async fn overview(State(app): AppRef) -> Response {
    let (stats, materials, data_error) = match load_catalog(&app, false).await {
        Ok(cat) => {
            let tests: usize = cat.materials.iter().map(|m| m.tests.len()).sum();
            (
                vec![
                    Stat { value: cat.exps.len().to_string(), label: "NASA experiments", href: "/explorer" },
                    Stat { value: cat.paper_count.to_string(), label: "papers from NTRS", href: "/papers" },
                    Stat { value: tests.to_string(), label: "test conditions that name a fuel", href: "/explorer" },
                    Stat { value: cat.materials.len().to_string(), label: "fuels and materials named", href: "/scan" },
                ],
                cat.materials.iter().take(12).map(|m| (m.name.clone(), m.tests.len(), format!("/explorer?q={}", urlencoding::encode(&m.name)))).collect(),
                None,
            )
        }
        Err(e) => (vec![], vec![], Some(e)),
    };
    render(OverviewPage {
        active: "overview",
        page_title: "Overview".into(),
        stats,
        materials,
        data_error,
    })
}

/// The scan screen.
#[derive(Template)]
#[template(path = "scan.html")]
pub struct ScanPage {
    pub scanner_on: bool,
    /// True when no Anthropic/OpenAI key is stored — the page shows its
    /// one-box unlock, which arms the scanner without a restart.
    pub needs_key: bool,
}

pub async fn scan_page(State(app): AppRef) -> Response {
    let v = app.vision.read().await;
    let (mut scanner_on, mut needs_key) = (v.may_work(), !v.has_own_key());
    drop(v);
    // The public website never asks visitors for a key (that box is for people running
    // their own copy), and it has no local Ollama: the scanner is on only with a key set.
    if crate::hosted() {
        scanner_on = !needs_key;
        needs_key = false;
    }
    render(ScanPage { scanner_on, needs_key })
}

#[derive(Template)]
#[template(path = "items.html")]
pub struct ItemsPage {
    pub active: &'static str,
    pub page_title: String,
    pub context_line: String,
}

pub async fn items() -> Response {
    render(ItemsPage { active: "items", page_title: "My items".into(), context_line: "Saved on this device only".into() })
}

// ---------- identify: photo -> parts and materials ----------

#[derive(Deserialize)]
pub struct IdentifyQuery {
    mode: Option<String>,
}

fn refused(reason: &str, message: &str) -> Response {
    Json(json!({ "ok": false, "reason": reason, "message": message })).into_response()
}

pub async fn identify(State(app): AppRef, headers: HeaderMap, Query(q): Query<IdentifyQuery>, body: Bytes) -> Response {
    if !same_origin(&headers) {
        return (StatusCode::FORBIDDEN, "Cross-site request refused.").into_response();
    }
    let mime = headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("");
    if !mime.starts_with("image/") || body.is_empty() {
        return refused("bad_image", "That wasn't a photo. Try again.");
    }
    if body.len() > MAX_IMAGE_BYTES {
        return refused("bad_image", "That photo is too large. Try again.");
    }
    let mode = if q.mode.as_deref() == Some("label") { "label" } else { "item" };
    // `body` is dropped when this function returns: the photo is not kept.
    let outcome = { app.vision.read().await.scan(&body, mime, mode).await };
    match outcome {
        Ok(reply) => Json(read_scan(&reply)).into_response(),
        Err(VisionError::NotConnected) => refused("not_connected", if crate::hosted() { NOT_CONNECTED_HOSTED } else { NOT_CONNECTED }),
        Err(VisionError::Failed(message)) => refused("error", &message),
    }
}

/// A confidence as 0..1, whether the backend sent 0.8 or 80.
fn confidence(v: &Value) -> Option<f64> {
    f(v, "confidence").map(|c| if c > 1.0 { c / 100.0 } else { c }).filter(|c| (0.0..=1.0).contains(c))
}

fn share(v: &Value) -> Option<f64> {
    f(v, "share").or_else(|| f(v, "percent")).filter(|p| *p > 0.0 && *p <= 100.0)
}

fn read_material(m: &Value) -> Option<Value> {
    let name = match m {
        Value::String(x) => Some(x.trim().to_string()).filter(|x| !x.is_empty()),
        _ => s(m, "material").or_else(|| s(m, "name")),
    }?;
    Some(json!({ "material": name, "share": share(m), "confidence": confidence(m) }))
}

/// The backend's scan reply, reduced to what the page shows. Anything the reply
/// leaves out stays empty; nothing is filled in here.
fn read_scan(v: &Value) -> Value {
    let mut parts = vec![];
    for p in v.get("parts").and_then(|x| x.as_array()).into_iter().flatten() {
        let materials: Vec<Value> = match p.get("materials").and_then(|x| x.as_array()) {
            Some(list) => list.iter().filter_map(read_material).collect(),
            // A flat part: { "name": "zipper", "material": "metal" }
            None => s(p, "material")
                .map(|name| json!({ "material": name, "share": share(p), "confidence": confidence(p) }))
                .into_iter()
                .collect(),
        };
        if !materials.is_empty() {
            parts.push(json!({ "name": s(p, "name").or_else(|| s(p, "part")), "materials": materials }));
        }
    }
    json!({
        "ok": true,
        "item": s(v, "item").or_else(|| s(v, "item_name")),
        "confidence": confidence(v),
        "unclear": flag(v, "unclear") || parts.is_empty(),
        "message": s(v, "message"),
        "model": s(v, "model"),
        "summary": s(v, "summary"),
        "parts": parts,
    })
}

// ---------- result: materials -> NASA test verdicts ----------

#[derive(Deserialize)]
pub struct ResultRequest {
    materials: Vec<String>,
}

/// What the backend's material lookup said about one name.
#[derive(Default)]
struct Match {
    /// Fuel names as the `conditions` table has them. Real equivalences only.
    fuels: Vec<String>,
    /// A different material that was tested, with the backend's note on how it differs.
    closest: Option<(String, Option<String>)>,
}

fn strings(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Array(a)) => a.iter().filter_map(|x| x.as_str()).map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect(),
        Some(Value::String(x)) if !x.trim().is_empty() => vec![x.trim().to_string()],
        _ => vec![],
    }
}

fn read_matches(v: &Value) -> HashMap<String, Match> {
    let mut out = HashMap::new();
    for m in v.get("matches").and_then(|x| x.as_array()).into_iter().flatten() {
        let Some(query) = s(m, "query").or_else(|| s(m, "material")) else { continue };
        let closest = m.get("closest").filter(|c| c.is_object()).and_then(|c| {
            let fuel = s(c, "fuel").or_else(|| s(c, "name"))?;
            Some((fuel, s(c, "note")))
        });
        out.insert(query.to_lowercase(), Match { fuels: strings(m.get("fuels")), closest });
    }
    out
}

fn tests_for<'a>(cat: Option<&'a Catalog>, fuels: &[String]) -> Vec<&'a Test> {
    let Some(cat) = cat else { return vec![] };
    cat.materials
        .iter()
        .filter(|m| fuels.iter().any(|f| f.eq_ignore_ascii_case(&m.name)))
        .flat_map(|m| m.tests.iter())
        .collect()
}

fn severity(o: Outcome) -> u8 {
    match o {
        Outcome::Burned => 3,
        Outcome::WentOut => 2,
        Outcome::NoIgnition => 1,
        Outcome::Unknown => 0,
    }
}

/// The oxygen value of a test that falls at this habitat's level, if any.
fn at_level(t: &Test, o2: f64) -> Option<f64> {
    t.o2.iter().copied().filter(|v| (v - o2).abs() <= O2_TOLERANCE).min_by(|a, b| (a - o2).abs().partial_cmp(&(b - o2).abs()).unwrap_or(Ordering::Equal))
}

/// One verdict per habitat, from recorded outcomes only. A test at another oxygen
/// level never produces a verdict here; it is reported as "not tested at this level".
fn verdicts(tests: &[&Test]) -> Vec<Value> {
    SHOWN
        .iter()
        .filter_map(|key| HABITATS.iter().find(|h| h.key == *key))
        .filter_map(|h| Some((h, h.o2_pct?, h.psi?)))
        .map(|(h, o2, psi)| {
            let here: Vec<(&Test, f64)> = tests.iter().filter_map(|t| at_level(t, o2).map(|v| (*t, v))).collect();
            let (code, label, detail) = if tests.is_empty() {
                ("none", "No NASA test".to_string(), String::new())
            } else if here.is_empty() {
                let mut seen: Vec<f64> = tests.iter().flat_map(|t| t.o2.iter().copied()).collect();
                seen.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
                seen.dedup();
                let detail = if seen.is_empty() {
                    "NASA's tests of it don't record an oxygen level.".to_string()
                } else {
                    format!("NASA tested it at {}% O₂.", seen.iter().map(|v| num(*v)).collect::<Vec<_>>().join(", "))
                };
                ("not_at_level", format!("Not tested at {}% O₂", num(o2)), detail)
            } else {
                let (worst, value) = here.iter().copied().max_by_key(|(t, _)| severity(t.outcome)).unwrap_or(here[0]);
                let (code, label) = match worst.outcome {
                    Outcome::Burned => ("burned", "Burned"),
                    Outcome::WentOut => ("went_out", "Went out on its own"),
                    Outcome::NoIgnition => ("no_ignition", "Did not ignite"),
                    Outcome::Unknown => ("unrecorded", "Tested, outcome not recorded"),
                };
                let n = here.len();
                (code, label.to_string(), format!("{n} test{} in NASA papers at {}% O₂", if n == 1 { "" } else { "s" }, num(value)))
            };
            json!({
                "key": h.key,
                "name": h.name,
                "air": format!("{}% O₂ · {} psi", num(o2), num(psi)),
                "verdict": code,
                "label": label,
                "detail": detail,
            })
        })
        .collect()
}

/// Up to six tests as plain lines, each with a link to its paper.
fn evidence(tests: &[&Test]) -> Vec<Value> {
    let mut sorted: Vec<&Test> = tests.to_vec();
    sorted.sort_by_key(|t| std::cmp::Reverse(severity(t.outcome)));
    sorted
        .iter()
        .take(6)
        .map(|t| {
            let mut parts = vec![];
            if !t.experiments.is_empty() {
                parts.push(t.experiments.join(" / "));
            }
            // The database's oxygen text often already says "O2" or "air"; don't repeat it.
            parts.push(match &t.o2_raw {
                Some(o) if o.to_lowercase().contains("o2") || o.to_lowercase().contains("oxygen") => o.clone(),
                Some(o) if o.trim().eq_ignore_ascii_case("air") => "in air".to_string(),
                Some(o) => format!("{o} oxygen"),
                None => "oxygen not recorded".into(),
            });
            parts.extend(t.pressure.clone());
            parts.extend(t.airflow.clone().map(|a| format!("airflow {a}")));
            parts.push(
                match t.outcome {
                    Outcome::Burned => "burned",
                    Outcome::WentOut => "went out on its own",
                    Outcome::NoIgnition => "did not ignite",
                    Outcome::Unknown => "outcome not recorded",
                }
                .to_string(),
            );
            parts.extend(t.outcome_note.clone());
            json!({
                "text": parts.join(" · "),
                "href": t.paper_url.clone().unwrap_or_else(|| t.paper_href.clone()),
                "paper": t.paper_title,
            })
        })
        .collect()
}

/// The gases NASA's Table I lists for this material, with the crew limits for each.
fn gases(names: &[&str]) -> Value {
    let Some(src) = names.iter().find_map(|n| air::source_for(n)) else { return Value::Null };
    let list: Vec<Value> = src
        .gases
        .iter()
        .filter_map(|g| air::gas(g))
        .map(|g| json!({ "formula": g.formula, "name": g.name, "limit_1h": g.limit_1h, "limit_24h": g.limit_24h, "page": g.limit_page }))
        .collect();
    json!({ "table_name": src.material, "list": list })
}

pub async fn result(State(app): AppRef, headers: HeaderMap, Json(req): Json<ResultRequest>) -> Response {
    if !same_origin(&headers) {
        return (StatusCode::FORBIDDEN, "Cross-site request refused.").into_response();
    }
    let mut names: Vec<String> = vec![];
    for raw in &req.materials {
        let name: String = raw.trim().chars().take(MAX_NAME_CHARS).collect();
        if !name.is_empty() && !names.iter().any(|n| n.eq_ignore_ascii_case(&name)) && names.len() < MAX_MATERIALS {
            names.push(name);
        }
    }

    // NASA test data. An unreachable or empty database is reported, never papered over.
    let (cat, data, data_message) = match load_catalog(&app, false).await {
        Ok(c) if c.materials.is_empty() => {
            (Some(c), "empty", Some("NASA's test table has no rows yet, so there are no verdicts to show.".to_string()))
        }
        Ok(c) => (Some(c), "ok", None),
        Err(e) => (None, "error", Some(e)),
    };

    // Material lookup is the backend's job. Without it, only a name that is
    // exactly a tested fuel's name is found; no synonyms are guessed here.
    let exact_only = "Material lookup not connected. Only exact name matches are shown.";
    let (lookup, lookup_message, matches) = match &app.backend {
        None => ("not_connected", Some(exact_only.to_string()), None),
        Some(_) if names.is_empty() => ("ok", None, Some(HashMap::new())),
        Some(b) => match b.match_materials(&names).await {
            Ok(v) => ("ok", None, Some(read_matches(&v))),
            Err(Fail::NotLive) => ("not_connected", Some(exact_only.to_string()), None),
            Err(Fail::Other(m)) => ("error", Some(format!("{m} Only exact name matches are shown.")), None),
        },
    };

    let materials: Vec<Value> = names
        .iter()
        .map(|name| {
            let found = match &matches {
                Some(map) => map.get(&name.to_lowercase()).map(|m| (m.fuels.clone(), m.closest.clone())).unwrap_or_default(),
                None => (vec![name.clone()], None),
            };
            let (fuels, closest) = found;
            let tests = tests_for(cat.as_deref(), &fuels);
            // Only fuels that really have tests are named on the card.
            let fuels: Vec<&String> = fuels
                .iter()
                .filter(|f| cat.as_deref().is_some_and(|c| c.materials.iter().any(|m| m.name.eq_ignore_ascii_case(f))))
                .collect();
            // "Closest tested" is a different material: it keeps its own verdicts in its own block.
            let closest = closest.filter(|_| tests.is_empty()).and_then(|(fuel, note)| {
                let its = tests_for(cat.as_deref(), std::slice::from_ref(&fuel));
                (!its.is_empty()).then(|| json!({ "fuel": fuel, "note": note, "habitats": verdicts(&its), "evidence": evidence(&its) }))
            });
            let mut gas_names: Vec<&str> = vec![name.as_str()];
            gas_names.extend(fuels.iter().map(|f| f.as_str()));
            json!({
                "query": name,
                "tested": !tests.is_empty(),
                "fuels": fuels,
                "habitats": verdicts(&tests),
                "evidence": evidence(&tests),
                "closest": closest,
                "gases": gases(&gas_names),
            })
        })
        .collect();

    Json(json!({
        "data": data,
        "data_message": data_message,
        "lookup": lookup,
        "lookup_message": lookup_message,
        "materials": materials,
        "sources": {
            "analyzer": air::ANALYZER_URL,
            "limits": air::LIMITS_URL,
            "table_page": air::TABLE_PAGE,
        },
    }))
    .into_response()
}
