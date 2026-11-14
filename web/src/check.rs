//! The habitat check: the three-question form ("/"), its result ("/check") and
//! the mission replay game ("/mission"). All three read the same query string:
//! `h` = habitat key, `m` = material (repeated), `p` = protection (repeated).

use crate::ai::{self, AiResult};
use crate::air;
use crate::catalog::{Catalog, Finding, Material, Outcome, Test};
use crate::views::{context_for, load_catalog, num, render, Habitat, EARTH_O2, HABITATS};
use crate::AppState;
use askama::Template;
use axum::{
    extract::{RawQuery, State},
    response::Response,
};
use serde_json::json;
use std::cmp::Ordering;
use std::sync::Arc;

type AppRef = State<Arc<AppState>>;

/// At most this many material chips on the form; the rest are counted in a note.
const MAX_CHIPS: usize = 24;

const PROTECTIONS: [(&str, &str); 3] =
    [("airflow", "Airflow control"), ("detect", "Smoke detection"), ("suppress", "Fire suppression")];

// ---------- icons ----------

pub fn icon(name: &str) -> String {
    let body = match name {
        "wind" => r#"<path d="M17.7 7.7a2.5 2.5 0 1 1 1.8 4.3H2"/><path d="M9.6 4.6A2 2 0 1 1 11 8H2"/><path d="M12.6 19.4A2 2 0 1 0 14 16H2"/>"#,
        "bell" => r#"<path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9"/><path d="M10.3 21a1.94 1.94 0 0 0 3.4 0"/>"#,
        "shield" => r#"<path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10"/>"#,
        "spark" => r#"<polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2"/>"#,
        "flame" => r#"<path d="M8.5 14.5A2.5 2.5 0 0 0 11 12c0-1.38-.5-2-1-3-1.072-2.143-.224-4.054 2-6 .5 2.5 2 4.9 4 6.5 2 1.6 3 3.5 3 5.5a7 7 0 1 1-14 0c0-1.153.433-2.294 1-3a2.5 2.5 0 0 0 2.5 2.5z"/>"#,
        "cloud" => r#"<path d="M17.5 19H9a7 7 0 1 1 6.71-9h1.79a4.5 4.5 0 1 1 0 9Z"/>"#,
        "gauge" => r#"<path d="m12 14 4-4"/><path d="M3.34 19a10 10 0 1 1 17.32 0"/>"#,
        _ => "",
    };
    format!(
        r#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{body}</svg>"#
    )
}

// ---------- the query string ----------

struct Picks {
    hab: &'static Habitat,
    /// Names exactly as the catalog has them. Anything not in the catalog is dropped,
    /// so a material with no test can never be checked.
    materials: Vec<String>,
    /// airflow, detect, suppress
    prot: [bool; 3],
    analyze: bool,
}

fn pairs(raw: Option<String>) -> Vec<(String, String)> {
    raw.unwrap_or_default()
        .split('&')
        .filter_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            let dec = |x: &str| urlencoding::decode(&x.replace('+', " ")).map(|c| c.into_owned()).ok();
            Some((dec(k)?, dec(v)?))
        })
        .collect()
}

fn picks(raw: Option<String>, cat: Option<&Catalog>) -> Picks {
    let q = pairs(raw);
    fn values<'a>(q: &'a [(String, String)], key: &'a str) -> impl Iterator<Item = &'a str> {
        q.iter().filter(move |(k, _)| k == key).map(|(_, v)| v.as_str())
    }
    let get = |key: &'static str| values(&q, key);
    let hab = get("h").find_map(|k| HABITATS.iter().find(|h| h.key == k)).unwrap_or(&HABITATS[0]);
    let mut materials: Vec<String> = vec![];
    if let Some(cat) = cat {
        for want in get("m") {
            if let Some(m) = cat.materials.iter().find(|m| m.name.eq_ignore_ascii_case(want.trim())) {
                if !materials.contains(&m.name) {
                    materials.push(m.name.clone());
                }
            }
        }
    }
    let has = |key: &'static str| get("p").any(|v| v == key);
    Picks { hab, materials, prot: [has("airflow"), has("detect"), has("suppress")], analyze: get("analyze").next().is_some() }
}

/// The query string for a habitat with the same materials and protections.
fn query(p: &Picks, hab_key: &str) -> String {
    let mut out = vec![format!("h={hab_key}")];
    out.extend(p.materials.iter().map(|m| format!("m={}", urlencoding::encode(m))));
    out.extend(PROTECTIONS.iter().zip(p.prot).filter(|(_, on)| *on).map(|((key, _), _)| format!("p={key}")));
    out.join("&")
}

fn air_line(h: &Habitat) -> String {
    format!("{}% O₂ · {} psi", num(h.o2_pct), num(h.psi))
}

// ---------- home: the three questions ----------

pub struct HabOption {
    pub key: &'static str,
    pub name: &'static str,
    pub air: String,
    pub img: &'static str,
    pub selected: bool,
}

pub struct ProtOption {
    pub key: &'static str,
    pub label: &'static str,
    pub icon: String,
}

#[derive(Template)]
#[template(path = "home.html")]
pub struct HomePage {
    pub active: &'static str,
    pub page_title: String,
    pub context_line: String,
    pub habitats: Vec<HabOption>,
    pub materials: Vec<String>,
    pub more: usize,
    pub data_error: Option<String>,
    pub protections: Vec<ProtOption>,
}

fn prot_icon(key: &str) -> String {
    icon(match key {
        "airflow" => "wind",
        "detect" => "bell",
        _ => "shield",
    })
}

pub async fn home(State(app): AppRef) -> Response {
    let (materials, more, data_error) = match load_catalog(&app, false).await {
        Ok(cat) => (
            cat.materials.iter().take(MAX_CHIPS).map(|m| m.name.clone()).collect(),
            cat.materials.len().saturating_sub(MAX_CHIPS),
            None,
        ),
        Err(e) => (vec![], 0, Some(e)),
    };
    render(HomePage {
        active: "check",
        page_title: "Check a habitat".into(),
        context_line: "Built on NASA microgravity fire experiments".into(),
        habitats: HABITATS
            .iter()
            .enumerate()
            .map(|(i, h)| HabOption { key: h.key, name: h.name, air: air_line(h), img: h.thumb, selected: i == 0 })
            .collect(),
        materials,
        more,
        data_error,
        protections: PROTECTIONS.iter().map(|(key, label)| ProtOption { key, label, icon: prot_icon(key) }).collect(),
    })
}

// ---------- result ----------

pub struct Evidence {
    pub text: String,
    pub href: String,
    pub paper: String,
}

pub struct MaterialRow {
    pub name: String,
    /// The plain verdict, from a recorded outcome when there is one.
    pub verdict: String,
    pub verdict_class: &'static str,
    /// "Tested at N% O₂ or higher" / "Tested up to M% O₂"; empty when the verdict already says it.
    pub status: String,
    pub status_class: &'static str,
    pub evidence: Vec<Evidence>,
}

pub struct Variant {
    pub key: &'static str,
    pub name: &'static str,
    pub selected: bool,
    pub href: String,
    pub mission_href: String,
    pub context: String,
    pub figure: String,
    pub verdict: &'static str,
    pub compare_line: String,
    /// 0 at Earth's 21% oxygen, 1 at the Moon Base's 30%: drives the flame drawing.
    pub heat: String,
    pub summary: String,
    pub rows: Vec<MaterialRow>,
}

pub struct FindingLine {
    pub text: String,
    pub href: String,
    pub source: String,
}

pub struct Protection {
    pub label: &'static str,
    pub class: &'static str,
    pub icon: String,
    pub title: &'static str,
    pub picked: bool,
    pub brief: &'static str,
    pub findings: Vec<FindingLine>,
    pub none_note: &'static str,
}

pub struct GasSource {
    pub material: String,
    pub table_name: &'static str,
    pub gases: String,
}

pub struct GasRow {
    pub formula: &'static str,
    pub name: &'static str,
    pub limit_1h: &'static str,
    pub limit_24h: &'static str,
    pub page: &'static str,
    pub from_yours: bool,
}

#[derive(Template)]
#[template(path = "check.html")]
pub struct CheckPage {
    pub active: &'static str,
    pub page_title: String,
    pub context_line: String,
    pub variants: Vec<Variant>,
    pub picked_any: bool,
    pub data_error: Option<String>,
    pub protections: Vec<Protection>,
    pub gas_sources: Vec<GasSource>,
    pub gas_uncovered: Vec<String>,
    pub gas_rows: Vec<GasRow>,
    pub analyzer_url: &'static str,
    pub limits_url: &'static str,
    pub table_page: &'static str,
    pub backend_on: bool,
    pub sel_key: &'static str,
    pub f_materials: Vec<String>,
    pub f_protections: Vec<&'static str>,
    pub ai: Option<AiResult>,
}

fn outcome_rank(o: Outcome) -> u8 {
    match o {
        Outcome::Burned => 0,
        Outcome::WentOut => 1,
        Outcome::NoIgnition => 2,
        Outcome::Unknown => 3,
    }
}

/// The oxygen level of a test closest to the habitat's, and how far away it is.
fn nearest(t: &Test, h: f64) -> Option<(f64, f64)> {
    t.o2.iter().map(|v| (*v, (v - h).abs())).min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal))
}

fn outcome_words(o: Outcome) -> Option<&'static str> {
    match o {
        Outcome::Burned => Some("burned"),
        Outcome::WentOut => Some("went out on its own"),
        Outcome::NoIgnition => Some("did not ignite"),
        Outcome::Unknown => None,
    }
}

fn material_row(m: &Material, h: f64) -> MaterialRow {
    // Tests nearest the habitat's oxygen level first; at the same distance, a test that burned first.
    let mut tests: Vec<&Test> = m.tests.iter().collect();
    tests.sort_by(|a, b| {
        let da = nearest(a, h).map(|x| x.1).unwrap_or(f64::INFINITY);
        let db = nearest(b, h).map(|x| x.1).unwrap_or(f64::INFINITY);
        da.partial_cmp(&db).unwrap_or(Ordering::Equal).then(outcome_rank(a.outcome).cmp(&outcome_rank(b.outcome)))
    });

    let max = m.tests.iter().filter_map(Test::o2_max).reduce(f64::max);
    let (status, status_class) = match max {
        Some(x) if x >= h => (format!("Tested at {}% O₂ or higher", num(h)), "text-fg"),
        Some(x) => (format!("Tested up to {}% O₂", num(x)), "text-amber"),
        None => ("Tested. Oxygen level not recorded.".to_string(), "text-muted"),
    };

    // A verdict only when a test recorded one. Otherwise the status is all we can say.
    let lead = tests.iter().find(|t| t.outcome != Outcome::Unknown);
    let (verdict, verdict_class, status) = match lead {
        Some(t) => {
            let at = nearest(t, h).map(|(v, _)| format!(" at {}% O₂", num(v))).unwrap_or_default();
            let (text, class) = match t.outcome {
                Outcome::Burned => (format!("Burned{at} in a NASA test"), "text-burn"),
                Outcome::WentOut => (format!("Went out on its own{at}"), "text-safe"),
                _ => (format!("Did not ignite{at}"), "text-safe"),
            };
            (text, class, status)
        }
        None => (status, status_class, String::new()),
    };

    let evidence = tests
        .iter()
        .take(3)
        .map(|t| {
            let mut parts = vec![];
            if !t.experiments.is_empty() {
                parts.push(t.experiments.join(" / "));
            }
            parts.push(t.o2_raw.clone().map(|o| format!("{o} oxygen")).unwrap_or_else(|| "oxygen not recorded".into()));
            parts.extend(t.pressure.clone());
            parts.extend(t.airflow.clone().map(|a| format!("airflow {a}")));
            parts.extend(outcome_words(t.outcome).map(str::to_string));
            parts.extend(t.outcome_note.clone());
            Evidence { text: parts.join(" · "), href: t.paper_href.clone(), paper: t.paper_title.clone() }
        })
        .collect();

    MaterialRow { name: m.name.clone(), verdict, verdict_class, status, status_class, evidence }
}

fn summary(chosen: &[&Material], h: f64) -> String {
    let max = |m: &Material| m.tests.iter().filter_map(Test::o2_max).reduce(f64::max);
    let at = chosen.iter().filter(|m| max(m).is_some_and(|x| x >= h)).count();
    let lower = chosen.iter().filter(|m| max(m).is_some_and(|x| x < h)).count();
    let unknown = chosen.len() - at - lower;
    let mut out = vec![format!("{at} of {} tested at this oxygen level.", chosen.len())];
    if lower > 0 {
        out.push(format!("{lower} tested lower."));
    }
    if unknown > 0 {
        out.push(format!("{unknown} with no oxygen level recorded."));
    }
    out.join(" ")
}

fn top_findings(cat: &Catalog, category: &str, n: usize) -> Vec<FindingLine> {
    let mut all: Vec<&Finding> = cat.exps.iter().flat_map(|e| e.findings.iter()).filter(|f| f.category == category).collect();
    all.sort_by(|a, b| b.verified.cmp(&a.verified).then(b.confidence.partial_cmp(&a.confidence).unwrap_or(Ordering::Equal)));
    let mut out: Vec<FindingLine> = vec![];
    for f in all {
        if out.len() < n && !out.iter().any(|x| x.text == f.text) {
            out.push(FindingLine { text: f.text.clone(), href: f.paper_href.clone(), source: f.source.clone() });
        }
    }
    out
}

fn headline(h: &Habitat) -> (String, &'static str) {
    let delta = (h.o2_pct / EARTH_O2 - 1.0) * 100.0;
    if delta.abs() < 0.5 {
        ("±0%".into(), "Same oxygen as Earth air.")
    } else if delta > 0.0 {
        (format!("+{}%", delta.round() as i64), "More oxygen than Earth air. Materials burn more easily.")
    } else {
        (format!("−{}%", delta.abs().round() as i64), "Less oxygen than Earth air.")
    }
}

pub async fn check(State(app): AppRef, RawQuery(raw): RawQuery) -> Response {
    let (cat, data_error) = match load_catalog(&app, false).await {
        Ok(c) => (Some(c), None),
        Err(e) => (None, Some(e)),
    };
    let p = picks(raw, cat.as_deref());
    let chosen: Vec<&Material> = match &cat {
        Some(c) => p.materials.iter().filter_map(|n| c.materials.iter().find(|m| &m.name == n)).collect(),
        None => vec![],
    };

    // Every habitat is rendered so the switcher can change the page without a reload.
    let variants = HABITATS
        .iter()
        .map(|h| {
            let (figure, verdict) = headline(h);
            Variant {
                key: h.key,
                name: h.name,
                selected: h.key == p.hab.key,
                href: format!("/check?{}", query(&p, h.key)),
                mission_href: format!("/mission?{}", query(&p, h.key)),
                context: context_for(h.key),
                figure,
                verdict,
                compare_line: format!("{}% oxygen here · {}% on Earth", num(h.o2_pct), num(EARTH_O2)),
                heat: format!("{:.2}", ((h.o2_pct - EARTH_O2) / 9.0).clamp(0.0, 1.0)),
                summary: summary(&chosen, h.o2_pct),
                rows: chosen.iter().map(|m| material_row(m, h.o2_pct)).collect(),
            }
        })
        .collect();

    let findings = |category: &str| cat.as_deref().map(|c| top_findings(c, category, 2)).unwrap_or_default();
    let protections = vec![
        Protection {
            label: "Prevent",
            class: "text-amber",
            icon: icon("wind"),
            title: if p.prot[0] { "Airflow control planned" } else { "No airflow control selected" },
            picked: p.prot[0],
            brief: "Flammability depends on oxygen, and in microgravity on airflow too.",
            findings: findings("prevention"),
            none_note: "No prevention findings loaded yet.",
        },
        Protection {
            label: "Detect",
            class: "text-cyan",
            icon: icon("bell"),
            title: if p.prot[1] { "Smoke detection planned" } else { "No smoke detection selected" },
            picked: p.prot[1],
            brief: "NASA tested smoke detection in microgravity in SAME and SAME-R.",
            findings: findings("detection"),
            none_note: "No detection findings loaded yet.",
        },
        Protection {
            label: "Put out",
            class: "text-sky",
            icon: icon("shield"),
            title: if p.prot[2] { "Fire suppression planned" } else { "No fire suppression selected" },
            picked: p.prot[2],
            brief: "Diluting the air with carbon dioxide or helium suppresses fire (FLEX).",
            findings: findings("suppression"),
            none_note: "No suppression findings loaded yet.",
        },
    ];

    // Air safety: which gases NASA's table lists for the chosen materials.
    let mut gas_sources = vec![];
    let mut gas_uncovered = vec![];
    let mut emitted: Vec<&str> = vec![];
    for name in &p.materials {
        match air::source_for(name) {
            Some(s) => {
                emitted.extend(s.gases.iter().copied());
                gas_sources.push(GasSource { material: name.clone(), table_name: s.material, gases: s.gases.join(", ") });
            }
            None => gas_uncovered.push(name.clone()),
        }
    }
    let gas_rows = air::GASES
        .iter()
        .filter(|g| g.limit_1h.is_some() || emitted.contains(&g.formula))
        .map(|g| GasRow {
            formula: g.formula,
            name: g.name,
            limit_1h: g.limit_1h.unwrap_or("No limit in this document"),
            limit_24h: g.limit_24h.unwrap_or("No limit in this document"),
            page: g.limit_page,
            from_yours: emitted.contains(&g.formula),
        })
        .collect();

    // The AI check runs only when asked for (the button adds analyze=1).
    let ai = match (&app.backend, p.analyze) {
        (Some(_), true) if p.materials.is_empty() => Some(ai::from_reply(Err("Pick at least one material first.".into()))),
        (Some(b), true) => Some(ai::from_reply(
            b.habitat(json!({
                "materials": p.materials,
                "habitat_name": p.hab.name,
                "oxygen_percent": p.hab.o2_pct,
                "pressure_kpa": (p.hab.psi * 6.89476 * 10.0).round() / 10.0,
            }))
            .await,
        )),
        _ => None,
    };

    render(CheckPage {
        active: "check",
        page_title: "Fire safety check".into(),
        context_line: context_for(p.hab.key),
        variants,
        picked_any: !p.materials.is_empty(),
        data_error,
        protections,
        gas_sources,
        gas_uncovered,
        gas_rows,
        analyzer_url: air::ANALYZER_URL,
        limits_url: air::LIMITS_URL,
        table_page: air::TABLE_PAGE,
        backend_on: app.backend.is_some(),
        sel_key: p.hab.key,
        f_materials: p.materials.clone(),
        f_protections: PROTECTIONS.iter().zip(p.prot).filter(|(_, on)| *on).map(|((key, _), _)| *key).collect(),
        ai,
    })
}

// ---------- mission replay ----------

#[derive(Template)]
#[template(path = "mission.html")]
pub struct MissionPage {
    pub active: &'static str,
    pub page_title: String,
    pub context_line: String,
    pub hab_name: &'static str,
    pub back_href: String,
    pub steps: Vec<(&'static str, &'static str, String)>,
    /// Everything the game needs, as JSON for static/mission.js.
    pub data: String,
}

pub async fn mission(State(app): AppRef, RawQuery(raw): RawQuery) -> Response {
    let cat = load_catalog(&app, false).await.ok();
    let p = picks(raw, cat.as_deref());
    let h = p.hab;

    // The first chosen material plays the part of "what catches fire".
    let material = p.materials.first().and_then(|n| cat.as_deref()?.materials.iter().find(|m| &m.name == n));
    // Only a verdict that a test recorded is passed on (the status is empty when there is none).
    let recorded = material.map(|m| material_row(m, h.o2_pct)).filter(|r| !r.status.is_empty()).map(|r| {
        json!({ "text": r.verdict, "burned": r.verdict_class == "text-burn",
                "href": r.evidence.first().map(|e| e.href.clone()), "paper": r.evidence.first().map(|e| e.paper.clone()) })
    });
    let fact = |category: &str| {
        cat.as_deref().and_then(|c| top_findings(c, category, 1).into_iter().next()).map(|f| json!({ "text": f.text, "href": f.href, "source": f.source }))
    };
    let gases = material.and_then(|m| air::source_for(&m.name)).map(|s| {
        json!({
            "tableName": s.material,
            "list": s.gases.iter().filter_map(|g| air::gas(g)).map(|g| json!({ "formula": g.formula, "name": g.name })).collect::<Vec<_>>(),
        })
    });
    let data = json!({
        "habitat": { "name": h.name, "o2": h.o2_pct, "earthO2": EARTH_O2, "figure": headline(h).0 },
        "material": material.map(|m| m.name.clone()),
        "recorded": recorded,
        "protections": { "airflow": p.prot[0], "detect": p.prot[1], "suppress": p.prot[2] },
        "facts": { "prevention": fact("prevention"), "detection": fact("detection"), "suppression": fact("suppression") },
        "gases": gases,
        "sources": { "analyzer": air::ANALYZER_URL, "limits": air::LIMITS_URL, "tablePage": air::TABLE_PAGE },
        "checkHref": format!("/check?{}", query(&p, h.key)),
    });

    render(MissionPage {
        active: "check",
        page_title: "Mission replay".into(),
        context_line: context_for(h.key),
        hab_name: h.name,
        back_href: format!("/check?{}", query(&p, h.key)),
        steps: [("Spark", "spark"), ("Flame", "flame"), ("Smoke", "bell"), ("Airflow", "wind"), ("Put out", "shield"), ("Is the air safe?", "gauge")]
            .iter()
            .enumerate()
            .map(|(i, (title, ic))| (*title, ["01", "02", "03", "04", "05", "06"][i], icon(ic)))
            .collect(),
        // "<" is escaped so nothing in the data can close the script tag.
        data: data.to_string().replace('<', "\\u003c"),
    })
}
