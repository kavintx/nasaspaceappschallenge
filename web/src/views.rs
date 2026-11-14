//! Page handlers and the view models each template renders.

use crate::ai::{self, AiResult};
use crate::backend::{f, https, s as js, Backend};
use crate::catalog::{self, paper_ref, Catalog, Exp, Finding, PaperRef, NA};
use crate::AppState;
use askama::Template;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

type AppRef = State<Arc<AppState>>;

/// How long the experiment catalog is reused before it is rebuilt.
const CATALOG_TTL: Duration = Duration::from_secs(300);

pub(crate) fn render<T: Template>(t: T) -> Response {
    match t.render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("template error: {e}")).into_response(),
    }
}

pub(crate) fn message_page(active: &'static str, title: &str, message: String) -> Response {
    render(ErrorPage { active, page_title: title.into(), context_line: String::new(), message })
}

const NOT_CONNECTED: &str = "The AI backend isn't connected, so this page is unavailable. Habitat Risk, Explorer, Compare and Literature work without it. To turn it on, set API_BASE_URL and API_AUTH_TOKEN in web/.env and restart the app.";

pub(crate) fn backend(app: &AppState) -> Result<&Backend, Response> {
    app.backend
        .as_ref()
        .ok_or_else(|| message_page("", "AI backend not connected", NOT_CONNECTED.into()))
}

/// The cached catalog, rebuilt when stale or when `refresh` is set.
pub(crate) async fn load_catalog(app: &AppState, refresh: bool) -> Result<Arc<Catalog>, String> {
    let mut slot = app.catalog.lock().await;
    if let Some(c) = slot.as_ref() {
        if !refresh && c.built.elapsed() < CATALOG_TTL {
            return Ok(c.clone());
        }
    }
    let fresh = Arc::new(catalog::build(&app.store).await?);
    *slot = Some(fresh.clone());
    Ok(fresh)
}

async fn catalog_or_page(app: &AppState, refresh: bool) -> Result<Arc<Catalog>, Response> {
    load_catalog(app, refresh).await.map_err(|e| {
        eprintln!("catalog error: {e}");
        message_page("", "Data unavailable", e)
    })
}

// ---------- shared formatting ----------

pub struct Cat {
    pub label: String,
    pub class: &'static str,
}

pub fn category(raw: &str) -> Cat {
    match raw {
        "prevention" => Cat { label: "Prevention".into(), class: "text-amber" },
        "detection" => Cat { label: "Detection".into(), class: "text-cyan" },
        "suppression" => Cat { label: "Suppression".into(), class: "text-sky" },
        _ => Cat { label: "Other".into(), class: "text-muted" },
    }
}

pub(crate) fn num(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 {
        format!("{}", v as i64)
    } else {
        format!("{v:.1}")
    }
}

fn capitalize(text: &str) -> String {
    let mut c = text.chars();
    match c.next() {
        Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// "a / b / c +2 more"
fn some_of(list: &[String], max: usize) -> String {
    if list.is_empty() {
        return NA.into();
    }
    let mut out = list.iter().take(max).cloned().collect::<Vec<_>>().join(" / ");
    if list.len() > max {
        out.push_str(&format!(" +{} more", list.len() - max));
    }
    out
}

/// One value as-is, several as a count.
fn one_or_count(list: &[String], noun: &str) -> String {
    match list.len() {
        0 => NA.into(),
        1 => list[0].clone(),
        n => format!("{n} {noun}"),
    }
}

fn o2_text(e: &Exp) -> String {
    match (e.o2_min(), e.o2_max()) {
        (Some(a), Some(b)) if (a - b).abs() < 1e-9 => format!("{}%", num(a)),
        (Some(a), Some(b)) => format!("{}–{}%", num(a), num(b)),
        _ => some_of(&e.o2_raw, 2),
    }
}

fn fuel_phase(e: &Exp) -> String {
    if e.fuel_phase.is_empty() { NA.into() } else { capitalize(e.fuel_phase) }
}

fn hardware(e: &Exp) -> String {
    if e.hardware.is_empty() { NA.into() } else { e.hardware.to_string() }
}

// ---------- habitats ----------

pub struct Habitat {
    pub key: &'static str,
    pub name: &'static str,
    pub o2_pct: Option<f64>,
    pub psi: Option<f64>,
    pub note: &'static str,
}

const EARTH_O2: f64 = 21.0;
const EARTH_PSI: f64 = 14.7;

// Atmospheres come from the challenge brief. Mars has no published target there,
// so it is left unspecified rather than invented.
pub const HABITATS: [Habitat; 4] = [
    Habitat { key: "iss", name: "ISS", o2_pct: Some(21.0), psi: Some(14.7), note: "Earth-like cabin air" },
    Habitat { key: "moon_base", name: "Moon Base", o2_pct: Some(30.0), psi: Some(10.2), note: "Planned · challenge brief" },
    Habitat { key: "rover", name: "Pressurized Rover", o2_pct: Some(30.0), psi: Some(10.2), note: "Expected to match Moon Base" },
    Habitat { key: "mars", name: "Mars", o2_pct: None, psi: None, note: "Atmosphere not yet specified" },
];

pub struct HabitatCard {
    pub key: &'static str,
    pub name: &'static str,
    pub o2: String,
    pub psi: String,
    pub note: &'static str,
    pub selected: bool,
}

pub struct EvidenceRow {
    pub rank: String,
    pub id: String,
    pub acronym: String,
    pub cat: Cat,
    pub line: String,
    pub badge: String,
    pub badge_class: &'static str,
}

pub struct Insight {
    pub kind: &'static str,
    pub class: &'static str,
    pub title: &'static str,
    pub body: &'static str,
    pub links: Vec<(&'static str, &'static str)>,
}

#[derive(Template)]
#[template(path = "habitat.html")]
pub struct HabitatPage {
    pub active: &'static str,
    pub page_title: String,
    pub context_line: String,
    pub cards: Vec<HabitatCard>,
    pub sel_name: &'static str,
    pub sel_key: &'static str,
    pub big_figure: String,
    pub big_caption: String,
    pub verdict: String,
    pub explanation: String,
    pub pp_habitat: String,
    pub pp_earth: String,
    pub earth_pos: f64,
    pub hab_pos: Option<f64>,
    pub evidence: Vec<EvidenceRow>,
    pub evidence_note: Option<String>,
    pub total: usize,
    pub insights: Vec<Insight>,
    pub backend_on: bool,
    pub can_analyze: bool,
    pub f_materials: String,
    pub f_vent: String,
    pub ai: Option<AiResult>,
}

#[derive(Deserialize)]
pub struct HabitatQuery {
    h: Option<String>,
    analyze: Option<String>,
    materials: Option<String>,
    vent: Option<String>,
}

pub fn context_for(key: &str) -> String {
    let h = HABITATS.iter().find(|h| h.key == key).unwrap_or(&HABITATS[1]);
    match (h.o2_pct, h.psi) {
        (Some(o), Some(p)) => format!("{} · {}% O₂ · {} psi", h.name, num(o), num(p)),
        _ => format!("{} · atmosphere TBD", h.name),
    }
}

pub async fn habitat(State(app): AppRef, Query(q): Query<HabitatQuery>) -> Response {
    let key = q.h.clone().unwrap_or_else(|| "moon_base".into());
    let sel = HABITATS.iter().find(|h| h.key == key).unwrap_or(&HABITATS[1]);

    // Evidence: which experiments tested oxygen levels as high as this habitat's.
    let (evidence, evidence_note, total) = match load_catalog(&app, false).await {
        Err(e) => (vec![], Some(e), 0),
        Ok(cat) => {
            let mut ranked: Vec<&Exp> = cat.exps.iter().collect();
            let covers = |e: &Exp| matches!((e.o2_max(), sel.o2_pct), (Some(m), Some(h)) if m >= h);
            ranked.sort_by(|a, b| {
                covers(b)
                    .cmp(&covers(a))
                    .then(b.o2_max().partial_cmp(&a.o2_max()).unwrap_or(std::cmp::Ordering::Equal))
                    .then(b.findings.len().cmp(&a.findings.len()))
            });
            let rows = ranked
                .iter()
                .take(6)
                .enumerate()
                .map(|(i, e)| {
                    let tested = if e.o2_values.is_empty() {
                        "No oxygen levels extracted yet".to_string()
                    } else {
                        format!("Tested at {} O₂", o2_text(e))
                    };
                    let (badge, badge_class) = match (e.o2_max(), sel.o2_pct) {
                        (Some(m), Some(h)) if m >= h => (format!("Reaches {}% O₂", num(h)), "text-cyan"),
                        (Some(_), Some(h)) => (format!("Below {}% O₂", num(h)), "text-dim"),
                        _ => (String::new(), "text-dim"),
                    };
                    EvidenceRow {
                        rank: format!("{:02}", i + 1),
                        id: e.id.clone(),
                        acronym: e.acronym.clone(),
                        cat: category(e.category),
                        line: format!("{tested} · {} papers · {} findings", e.papers.len(), e.findings.len()),
                        badge,
                        badge_class,
                    }
                })
                .collect();
            (rows, None, cat.exps.len())
        }
    };

    let scale_max = 40.0;
    let (big_figure, verdict, explanation, pp_habitat, hab_pos) = match (sel.o2_pct, sel.psi) {
        (Some(o), Some(p)) => {
            let delta = (o / EARTH_O2 - 1.0) * 100.0;
            let big = if delta.abs() < 0.5 { "±0%".to_string() } else { format!("+{}%", delta.round() as i64) };
            let (verdict, expl) = if delta.abs() < 0.5 {
                (
                    "Earth-normal oxygen.".to_string(),
                    "Same oxygen fraction as Earth air. Microgravity still changes how flames spread, because hot gases don't rise.".to_string(),
                )
            } else {
                (
                    "More oxygen. More flammable.".to_string(),
                    format!(
                        "{}% oxygen at {} psi keeps breathing normal while cutting spacesuit pre-breathe time, but flammability depends heavily on oxygen concentration. Materials cleared for Earth air need re-testing.",
                        num(o),
                        num(p)
                    ),
                )
            };
            (big, verdict, expl, format!("{:.2} psi", o / 100.0 * p), Some((o / scale_max * 100.0).min(100.0)))
        }
        _ => (
            NA.to_string(),
            "Atmosphere not yet specified.".to_string(),
            "No Mars habitat atmosphere is given in the challenge data. Microgravity and Earth-gravity results can bound what happens at partial gravity.".to_string(),
            NA.to_string(),
            None,
        ),
    };

    let cards = HABITATS
        .iter()
        .map(|h| HabitatCard {
            key: h.key,
            name: h.name,
            o2: h.o2_pct.map(|o| format!("{}% O₂", num(o))).unwrap_or_else(|| "O₂ TBD".into()),
            psi: h.psi.map(|p| format!("{} psi", num(p))).unwrap_or_else(|| NA.into()),
            note: h.note,
            selected: h.key == sel.key,
        })
        .collect();

    let insights = vec![
        Insight {
            kind: "Prevention",
            class: "text-amber",
            title: "Pick materials for the habitat's air.",
            body: "Flammability rises with oxygen concentration, and in microgravity airflow matters too. BASS, BASS-II and Saffire burned fabrics and plastics to map how solids ignite and spread.",
            links: vec![("BASS-II", "bass-ii"), ("SAFFIRE III", "saffire-iii")],
        },
        Insight {
            kind: "Detection",
            class: "text-cyan",
            title: "Smoke behaves differently in orbit.",
            body: "SAME and SAME-R measured smoke particles in microgravity to inform spacecraft smoke detectors.",
            links: vec![("SAME", "same"), ("SAME-R", "same-r")],
        },
        Insight {
            kind: "Suppression",
            class: "text-sky",
            title: "Diluting the air puts fires out.",
            body: "FLEX burned fuel droplets while adding carbon dioxide or helium to the chamber, both of which suppress fire.",
            links: vec![("FLEX", "flex"), ("FLEX-2", "flex-2")],
        },
    ];

    let f_materials = q.materials.clone().unwrap_or_else(|| "Nomex, PMMA, polyimide wire insulation".into());
    let f_vent = q.vent.clone().unwrap_or_else(|| "10".into());
    let ai = match (&app.backend, sel.o2_pct, sel.psi, q.analyze.is_some()) {
        (Some(b), Some(o), Some(p), true) => {
            let materials: Vec<&str> = f_materials.split(',').map(str::trim).filter(|m| !m.is_empty()).collect();
            if materials.is_empty() {
                Some(ai::from_reply(Err("List at least one material to assess.".into())))
            } else {
                let mut scenario = json!({
                    "materials": materials,
                    "habitat_name": sel.name,
                    "oxygen_percent": o,
                    "pressure_kpa": (p * 6.89476 * 10.0).round() / 10.0,
                });
                if let Ok(v) = f_vent.trim().parse::<f64>() {
                    scenario["airflow_cm_s"] = json!(v);
                }
                Some(ai::from_reply(b.habitat(scenario).await))
            }
        }
        _ => None,
    };

    render(HabitatPage {
        active: "habitat",
        page_title: "Habitat Risk".into(),
        context_line: context_for(sel.key),
        cards,
        sel_name: sel.name,
        sel_key: sel.key,
        big_figure,
        big_caption: "Oxygen concentration vs. Earth air".into(),
        verdict,
        explanation,
        pp_habitat,
        pp_earth: format!("{:.2} psi", EARTH_O2 / 100.0 * EARTH_PSI),
        earth_pos: EARTH_O2 / scale_max * 100.0,
        hab_pos,
        evidence,
        evidence_note,
        total,
        insights,
        backend_on: app.backend.is_some(),
        can_analyze: sel.o2_pct.is_some(),
        f_materials,
        f_vent,
        ai,
    })
}

// ---------- explorer ----------

pub struct ExplorerRow {
    pub id: String,
    pub acronym: String,
    pub cat: Cat,
    pub fuel: String,
    pub o2: String,
    pub airflow: String,
    pub sensors: String,
    pub papers: String,
}

#[derive(Template)]
#[template(path = "explorer.html")]
pub struct ExplorerPage {
    pub active: &'static str,
    pub page_title: String,
    pub context_line: String,
    pub rows: Vec<ExplorerRow>,
    pub total: usize,
    pub f_category: String,
    pub f_fuel: String,
    pub f_hardware: String,
    pub f_q: String,
    pub hardware_options: Vec<String>,
    pub status_line: String,
}

#[derive(Deserialize, Default)]
pub struct ExplorerQuery {
    category: Option<String>,
    fuel: Option<String>,
    hardware: Option<String>,
    q: Option<String>,
    refresh: Option<String>,
}

pub async fn explorer(State(app): AppRef, Query(q): Query<ExplorerQuery>) -> Response {
    let cat = match catalog_or_page(&app, q.refresh.is_some()).await {
        Ok(c) => c,
        Err(page) => return page,
    };
    let mut hardware_options: Vec<String> = cat.exps.iter().map(|e| e.hardware.to_string()).filter(|h| !h.is_empty()).collect();
    hardware_options.sort();
    hardware_options.dedup();

    let fc = q.category.unwrap_or_default();
    let ff = q.fuel.unwrap_or_default();
    let fh = q.hardware.unwrap_or_default();
    let fq = q.q.unwrap_or_default();
    let ql = fq.trim().to_lowercase();

    let rows = cat
        .exps
        .iter()
        .filter(|e| fc.is_empty() || e.category == fc)
        .filter(|e| ff.is_empty() || e.fuel_phase == ff)
        .filter(|e| fh.is_empty() || e.hardware == fh)
        .filter(|e| {
            ql.is_empty()
                || format!(
                    "{} {} {} {} {}",
                    e.acronym,
                    e.name,
                    e.description.clone().unwrap_or_default(),
                    e.fuels.join(" "),
                    e.sensors.join(" ")
                )
                .to_lowercase()
                .contains(&ql)
        })
        .map(|e| ExplorerRow {
            id: e.id.clone(),
            acronym: e.acronym.clone(),
            cat: category(e.category),
            fuel: fuel_phase(e),
            o2: o2_text(e),
            airflow: one_or_count(&e.airflows, "settings"),
            sensors: some_of(&e.sensors, 2),
            papers: format!("{:02}", e.papers.len()),
        })
        .collect();

    let mins = cat.built.elapsed().as_secs() / 60;
    let mut status_line = format!(
        "Rolled up from {} papers · updated {}",
        cat.paper_count,
        if mins == 0 { "just now".to_string() } else { format!("{mins} min ago") }
    );
    if cat.failed > 0 {
        status_line.push_str(&format!(" · {} papers could not be read", cat.failed));
    }

    render(ExplorerPage {
        active: "explorer",
        page_title: "Explorer".into(),
        context_line: context_for("moon_base"),
        rows,
        total: cat.exps.len(),
        f_category: fc,
        f_fuel: ff,
        f_hardware: fh,
        f_q: fq,
        hardware_options,
        status_line,
    })
}

// ---------- experiment detail ----------

pub struct Tile {
    pub label: &'static str,
    pub value: String,
    pub caption: String,
}

pub struct FindingView {
    pub text: String,
    pub cat: Option<Cat>,
    pub paper_href: String,
    pub source: String,
    pub confidence: Option<String>,
    pub verified: bool,
}

fn finding_view(x: &Finding) -> FindingView {
    FindingView {
        text: x.text.clone(),
        cat: (!x.category.is_empty()).then(|| category(&x.category)),
        paper_href: x.paper_href.clone(),
        source: x.source.clone(),
        confidence: x.confidence.map(|c| format!("{}%", (c * 100.0).round() as i64)),
        verified: x.verified,
    }
}

#[derive(Template)]
#[template(path = "detail.html")]
pub struct DetailPage {
    pub active: &'static str,
    pub page_title: String,
    pub context_line: String,
    pub id: String,
    pub acronym: String,
    pub name: String,
    pub cat: Cat,
    pub hardware: String,
    pub tiles: Vec<Tile>,
    pub brief: Option<String>,
    pub brief_source: &'static str,
    pub objectives: Vec<String>,
    pub findings: Vec<FindingView>,
    pub findings_total: usize,
    pub sensor_list: Vec<String>,
    pub papers: Vec<PaperRef>,
    pub ai_on: bool,
    pub ai: Option<AiResult>,
}

#[derive(Deserialize, Default)]
pub struct DetailQuery {
    summarize: Option<String>,
}

pub async fn detail(State(app): AppRef, Path(id): Path<String>, Query(q): Query<DetailQuery>) -> Response {
    let cat = match catalog_or_page(&app, false).await {
        Ok(c) => c,
        Err(page) => return page,
    };
    let Some(e) = cat.exps.iter().find(|e| e.id == id) else {
        return (
            StatusCode::NOT_FOUND,
            message_page("explorer", "Not found", format!("No experiment with id \"{id}\".")),
        )
            .into_response();
    };
    let ai = match (&app.backend, q.summarize.is_some()) {
        (Some(b), true) => Some(ai::from_reply(b.summarize(&e.acronym).await)),
        _ => None,
    };
    let tiles = vec![
        Tile { label: "Oxygen tested", value: o2_text(e), caption: format!("Across {} extracted test conditions", e.conditions) },
        Tile { label: "Pressure", value: one_or_count(&e.pressures, "settings"), caption: if e.pressures.len() > 1 { some_of(&e.pressures, 3) } else { "Chamber pressure".into() } },
        Tile { label: "Airflow", value: one_or_count(&e.airflows, "settings"), caption: if e.airflows.len() > 1 { some_of(&e.airflows, 3) } else { "Imposed flow".into() } },
        Tile { label: "Fuel", value: fuel_phase(e), caption: if e.fuels.is_empty() { "No fuels extracted yet".into() } else { some_of(&e.fuels, 4) } },
    ];
    let hw = hardware(e);
    render(DetailPage {
        active: "explorer",
        page_title: e.acronym.clone(),
        context_line: format!("{} · {}", category(e.category).label, hw),
        id: e.id.clone(),
        acronym: e.acronym.clone(),
        name: e.name.clone(),
        cat: category(e.category),
        hardware: hw,
        tiles,
        brief: e.description.clone(),
        brief_source: if e.from_brief { "From the NASA challenge brief" } else { "From the research database" },
        objectives: e.objectives.iter().take(4).cloned().collect(),
        findings: e.findings.iter().take(12).map(finding_view).collect(),
        findings_total: e.findings.len(),
        sensor_list: e.sensors.clone(),
        papers: e.papers.clone(),
        ai_on: app.backend.is_some(),
        ai,
    })
}

// ---------- compare ----------

pub struct CompareCol {
    pub slot: &'static str,
    pub id: String,
    pub acronym: String,
    pub meta: String,
    pub remove_href: String,
}

pub struct CompareRow {
    pub label: &'static str,
    pub cells: Vec<String>,
    pub differs: bool,
}

#[derive(Template)]
#[template(path = "compare.html")]
pub struct ComparePage {
    pub active: &'static str,
    pub page_title: String,
    pub context_line: String,
    pub cols: Vec<CompareCol>,
    pub rows: Vec<CompareRow>,
    pub options: Vec<(String, String)>,
    pub selected: Vec<String>,
    pub can_add: bool,
    pub next_slot: &'static str,
    pub f_focus: String,
    pub ai_on: bool,
    pub ai: Option<AiResult>,
}

#[derive(Deserialize, Default)]
pub struct CompareQuery {
    a: Option<String>,
    b: Option<String>,
    c: Option<String>,
    ai: Option<String>,
    focus: Option<String>,
}

pub async fn compare(State(app): AppRef, Query(q): Query<CompareQuery>) -> Response {
    let cat = match catalog_or_page(&app, false).await {
        Ok(c) => c,
        Err(page) => return page,
    };
    let want_ai = q.ai.is_some();
    let f_focus = q.focus.clone().unwrap_or_default();
    let untouched = q.a.is_none() && q.b.is_none() && q.c.is_none();
    let picks: Vec<String> = if untouched {
        vec!["bass-ii".into(), "saffire-iii".into(), "flex-2".into()]
    } else {
        vec![q.a.unwrap_or_default(), q.b.unwrap_or_default(), q.c.unwrap_or_default()]
    };
    // Compact the picks so chosen experiments always fill slots a, b, c in order.
    const SLOTS: [&str; 3] = ["a", "b", "c"];
    let mut chosen: Vec<&Exp> = vec![];
    for id in &picks {
        if let Some(e) = cat.exps.iter().find(|e| &e.id == id) {
            if !chosen.iter().any(|c| c.id == e.id) {
                chosen.push(e);
            }
        }
    }
    let selected: Vec<String> = chosen.iter().map(|e| e.id.clone()).collect();

    let cols = chosen
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let params: Vec<String> = chosen
                .iter()
                .filter(|o| o.id != e.id)
                .enumerate()
                .map(|(j, o)| format!("{}={}", SLOTS[j], o.id))
                .collect();
            CompareCol {
                slot: SLOTS[i],
                id: e.id.clone(),
                acronym: e.acronym.clone(),
                meta: format!("{} · {}", category(e.category).label, hardware(e)),
                remove_href: if params.is_empty() { "/compare?a=".into() } else { format!("/compare?{}", params.join("&")) },
            }
        })
        .collect();

    let row = |label: &'static str, cell: &dyn Fn(&Exp) -> String| {
        let cells: Vec<String> = chosen.iter().map(|e| cell(e)).collect();
        let differs = cells.len() > 1 && cells.iter().any(|c| c != &cells[0]);
        CompareRow { label, cells, differs }
    };
    let rows = vec![
        row("Full name", &|e| e.name.clone()),
        row("Objective", &|e| e.objectives.first().cloned().or_else(|| e.description.clone()).unwrap_or_else(|| NA.into())),
        row("Fuel", &|e| if e.fuels.is_empty() { fuel_phase(e) } else { format!("{} · {}", fuel_phase(e), some_of(&e.fuels, 3)) }),
        row("O₂ tested", &|e| o2_text(e)),
        row("Pressure", &|e| some_of(&e.pressures, 3)),
        row("Airflow", &|e| some_of(&e.airflows, 3)),
        row("Sensors", &|e| some_of(&e.sensors, 4)),
        row("Hardware", &|e| hardware(e)),
        row("Top finding", &|e| e.findings.first().map(|x| x.text.clone()).unwrap_or_else(|| NA.into())),
        row("Evidence", &|e| format!("{} papers · {} findings", e.papers.len(), e.findings.len())),
    ];

    let ai = match (&app.backend, want_ai) {
        (Some(b), true) => Some(if chosen.len() < 2 {
            ai::from_reply(Err("Pick at least two experiments to compare.".into()))
        } else {
            let names: Vec<String> = chosen.iter().map(|e| e.acronym.clone()).collect();
            ai::from_reply(b.compare(&names, &f_focus).await)
        }),
        _ => None,
    };
    render(ComparePage {
        active: "compare",
        page_title: "Compare".into(),
        context_line: context_for("moon_base"),
        cols,
        rows,
        options: cat.exps.iter().map(|e| (e.id.clone(), e.acronym.clone())).collect(),
        can_add: chosen.len() < 3,
        next_slot: SLOTS[chosen.len().min(2)],
        selected,
        f_focus,
        ai_on: app.backend.is_some(),
        ai,
    })
}

// ---------- ask the data (streaming chat) ----------

#[derive(Template)]
#[template(path = "ask.html")]
pub struct AskPage {
    pub active: &'static str,
    pub page_title: String,
    pub context_line: String,
    pub question: String,
    pub experiments: Vec<String>,
    pub suggestions: Vec<&'static str>,
}

#[derive(Deserialize, Default)]
pub struct AskQuery {
    q: Option<String>,
}

pub async fn ask(State(app): AppRef, Query(q): Query<AskQuery>) -> Response {
    if let Err(page) = backend(&app) {
        return page;
    }
    let experiments = app
        .store
        .experiments()
        .await
        .map(|list| list.iter().filter_map(|e| js(e, "acronym")).collect())
        .unwrap_or_default();
    render(AskPage {
        active: "ask",
        page_title: "Ask the Data".into(),
        context_line: "Answers cite NASA NTRS papers".into(),
        question: q.q.unwrap_or_default().trim().to_string(),
        experiments,
        suggestions: vec![
            "How does low cabin pressure with elevated oxygen affect flame spread?",
            "How does airflow change flame spread in microgravity?",
            "Which experiments studied fire suppression?",
        ],
    })
}

// ---------- literature ----------

#[derive(Template)]
#[template(path = "papers.html")]
pub struct PapersPage {
    pub active: &'static str,
    pub page_title: String,
    pub context_line: String,
    pub error: Option<String>,
    pub rows: Vec<PaperRef>,
    pub loaded: usize,
    pub counts: Vec<(String, String)>,
    pub f_status: String,
    pub f_q: String,
}

#[derive(Deserialize, Default)]
pub struct PapersQuery {
    status: Option<String>,
    q: Option<String>,
}

pub async fn papers(State(app): AppRef, Query(q): Query<PapersQuery>) -> Response {
    let f_status = q.status.unwrap_or_default();
    let f_q = q.q.unwrap_or_default();
    let mut page = PapersPage {
        active: "papers",
        page_title: "Literature".into(),
        context_line: "NASA NTRS papers · AI-extracted".into(),
        error: None,
        rows: vec![],
        loaded: 0,
        counts: vec![],
        f_status: f_status.clone(),
        f_q: f_q.clone(),
    };
    match app.store.papers(None).await {
        Err(e) => page.error = Some(e),
        Ok(all) => {
            // Status counts across every paper, most common first.
            let mut counts: Vec<(String, usize)> = vec![];
            for p in &all {
                let st = js(p, "status").unwrap_or_else(|| "unknown".into());
                match counts.iter_mut().find(|(k, _)| *k == st) {
                    Some(c) => c.1 += 1,
                    None => counts.push((st, 1)),
                }
            }
            counts.sort_by(|a, b| b.1.cmp(&a.1));
            page.counts = counts.into_iter().map(|(k, n)| (k, n.to_string())).collect();

            let ql = f_q.trim().to_lowercase();
            let shown: Vec<&Value> = all
                .iter()
                .filter(|p| f_status.is_empty() || js(p, "status").as_deref() == Some(f_status.as_str()))
                .collect();
            page.loaded = shown.len();
            page.rows = shown
                .into_iter()
                .filter(|p| {
                    ql.is_empty()
                        || format!("{} {}", js(p, "title").unwrap_or_default(), js(p, "abstract").unwrap_or_default())
                            .to_lowercase()
                            .contains(&ql)
                })
                .map(paper_ref)
                .collect();
        }
    }
    render(page)
}

pub struct ConditionRow {
    pub id: String,
    pub e_fuel: String,
    pub e_geometry: String,
    pub e_o2: String,
    pub e_pressure: String,
    pub e_airflow: String,
    pub e_sensors: String,
    pub e_objective: String,
    pub e_page: String,
    pub e_status: String,
    pub e_note: String,
    pub fuel: String,
    pub geometry: Option<String>,
    pub o2: String,
    pub pressure: String,
    pub airflow: String,
    pub sensors: String,
    pub source: String,
    pub confidence: String,
    pub status: String,
    pub verified: bool,
}

pub struct PaperFinding {
    pub id: String,
    pub e_category: String,
    pub e_page: String,
    pub e_status: String,
    pub e_note: String,
    pub text: String,
    pub cat: Option<Cat>,
    pub meta: String,
    pub quote: Option<String>,
    pub status: String,
    pub verified: bool,
}

#[derive(Template)]
#[template(path = "paper.html")]
pub struct PaperPage {
    pub active: &'static str,
    pub page_title: String,
    pub context_line: String,
    pub title: String,
    pub authors: String,
    pub abstract_text: Option<String>,
    pub ntrs_url: Option<String>,
    pub pdf_url: Option<String>,
    pub experiments: Vec<(String, String)>,
    pub conditions: Vec<ConditionRow>,
    pub findings: Vec<PaperFinding>,
    pub paper_key: String,
    pub statuses: [&'static str; 4],
    pub categories: [&'static str; 4],
    pub can_edit: bool,
    pub msg: Option<String>,
    pub err: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct PaperQuery {
    msg: Option<String>,
    err: Option<String>,
}

pub async fn paper(State(app): AppRef, Path(id): Path<String>, Query(pq): Query<PaperQuery>) -> Response {
    let data = match app.store.paper_bundle(&id).await {
        Ok(Some(d)) => d,
        Ok(None) => {
            return (StatusCode::NOT_FOUND, message_page("papers", "Paper not found", format!("No paper with id \"{id}\"."))).into_response()
        }
        Err(e) => return message_page("papers", "Paper unavailable", e),
    };
    let p = data.get("paper").cloned().unwrap_or(Value::Null);
    let authors = match p.get("authors") {
        Some(Value::Array(a)) => a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(", "),
        Some(Value::String(x)) => x.clone(),
        _ => String::new(),
    };
    let or_na = |x: Option<String>| x.unwrap_or_else(|| NA.into());
    let rows = |key: &str| data.get(key).and_then(|x| x.as_array()).cloned().unwrap_or_default();
    let pct = |x: &Value| f(x, "confidence").map(|c| format!("{}%", (c * 100.0).round() as i64));
    let page_no = |x: &Value| f(x, "source_page").map(|n| format!("p. {}", n as i64));
    let status = |x: &Value| js(x, "verification_status").map(|v| capitalize(&v)).unwrap_or_else(|| "Pending".into());
    let verified = |x: &Value| matches!(js(x, "verification_status").as_deref(), Some("verified") | Some("corrected"));

    let experiments = rows("experiments")
        .iter()
        .filter_map(|e| js(e, "acronym"))
        .map(|a| (a.to_lowercase().replace(' ', "-"), a))
        .collect();
    let conditions = rows("conditions")
        .iter()
        .map(|c| ConditionRow {
            id: js(c, "id").unwrap_or_default(),
            e_fuel: js(c, "fuel_type").unwrap_or_default(),
            e_geometry: js(c, "fuel_geometry").unwrap_or_default(),
            e_o2: js(c, "oxygen_concentration").unwrap_or_default(),
            e_pressure: js(c, "pressure").unwrap_or_default(),
            e_airflow: js(c, "airflow").unwrap_or_default(),
            e_sensors: c
                .get("sensors")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(", "))
                .unwrap_or_default(),
            e_objective: js(c, "objective").unwrap_or_default(),
            e_page: js(c, "source_page").unwrap_or_default(),
            e_status: js(c, "verification_status").unwrap_or_else(|| "pending".into()),
            e_note: js(c, "correction_note").unwrap_or_default(),
            fuel: or_na(js(c, "fuel_type")),
            geometry: js(c, "fuel_geometry"),
            o2: or_na(js(c, "oxygen_concentration")),
            pressure: or_na(js(c, "pressure")),
            airflow: or_na(js(c, "airflow")),
            sensors: c
                .get("sensors")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(", "))
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| NA.into()),
            source: or_na(page_no(c)),
            confidence: or_na(pct(c)),
            status: status(c),
            verified: verified(c),
        })
        .collect();
    let findings = rows("findings")
        .iter()
        .map(|x| PaperFinding {
            id: js(x, "id").unwrap_or_default(),
            e_category: js(x, "category").unwrap_or_default().to_lowercase(),
            e_page: js(x, "source_page").unwrap_or_default(),
            e_status: js(x, "verification_status").unwrap_or_else(|| "pending".into()),
            e_note: js(x, "correction_note").unwrap_or_default(),
            text: or_na(js(x, "text")),
            cat: js(x, "category").map(|c| category(&c.to_lowercase())),
            meta: [page_no(x), pct(x).map(|c| format!("AI confidence {c}")), js(x, "provenance_kind").map(|k| capitalize(&k))]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · "),
            quote: js(x, "source_quote"),
            status: status(x),
            verified: verified(x),
        })
        .collect();
    let year = js(&p, "publication_date").map(|d| d.chars().take(4).collect::<String>());
    render(PaperPage {
        active: "papers",
        page_title: "Paper".into(),
        context_line: [js(&p, "ntrs_id").map(|n| format!("NTRS {n}")), year, js(&p, "pub_type")]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · "),
        title: js(&p, "title").unwrap_or_else(|| "Untitled NASA paper".into()),
        authors,
        abstract_text: js(&p, "abstract"),
        ntrs_url: https(&p, "ntrs_url"),
        pdf_url: https(&p, "pdf_url"),
        experiments,
        conditions,
        findings,
        paper_key: id,
        statuses: ["pending", "verified", "corrected", "rejected"],
        categories: ["prevention", "detection", "suppression", "other"],
        can_edit: app.backend.is_some() && !app.read_only,
        msg: pq.msg.filter(|m| !m.is_empty()),
        err: pq.err.filter(|m| !m.is_empty()),
    })
}

// ---------- errors ----------

#[derive(Template)]
#[template(path = "error.html")]
pub struct ErrorPage {
    pub active: &'static str,
    pub page_title: String,
    pub context_line: String,
    pub message: String,
}

pub async fn not_found() -> Response {
    (StatusCode::NOT_FOUND, message_page("", "Not found", "That page doesn't exist.".into())).into_response()
}
