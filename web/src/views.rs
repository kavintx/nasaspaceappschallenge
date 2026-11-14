//! Page handlers and the view models each template renders.

use crate::ai::{self, AiResult};
use crate::backend::{self, f, flag, https, s as js};
use crate::data::{Experiment, Paper};
use crate::AppState;
use askama::Template;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

type AppRef = State<Arc<AppState>>;

fn render<T: Template>(t: T) -> Response {
    match t.render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("template error: {e}")).into_response(),
    }
}

fn data_error(msg: String) -> Response {
    eprintln!("data error: {msg}");
    render(ErrorPage {
        active: "",
        page_title: "Data unavailable".into(),
        context_line: String::new(),
        message: msg,
    })
}

// ---------- shared formatting ----------

pub struct Cat {
    pub label: String,
    pub class: &'static str,
}

pub fn category(raw: Option<&str>) -> Cat {
    match raw.unwrap_or("other") {
        "prevention" => Cat { label: "Prevention".into(), class: "text-amber" },
        "detection" => Cat { label: "Detection".into(), class: "text-cyan" },
        "suppression" => Cat { label: "Suppression".into(), class: "text-sky" },
        _ => Cat { label: "Other".into(), class: "text-muted" },
    }
}

fn num(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 {
        format!("{}", v as i64)
    } else {
        format!("{v:.1}")
    }
}

fn range(min: Option<f64>, max: Option<f64>, unit: &str) -> Option<String> {
    match (min, max) {
        (Some(a), Some(b)) if (a - b).abs() < 1e-9 => Some(format!("{}{unit}", num(a))),
        (Some(a), Some(b)) => Some(format!("{}–{}{unit}", num(a), num(b))),
        (None, Some(b)) => Some(format!("≤{}{unit}", num(b))),
        (Some(a), None) => Some(format!("≥{}{unit}", num(a))),
        (None, None) => None,
    }
}

const NA: &str = "—";

fn o2(e: &Experiment) -> String {
    range(e.oxygen_min_pct, e.oxygen_max_pct, "%").unwrap_or_else(|| NA.into())
}
fn airflow(e: &Experiment) -> String {
    range(e.airflow_min_cms, e.airflow_max_cms, " cm/s").unwrap_or_else(|| NA.into())
}
fn pressure(e: &Experiment) -> String {
    range(e.pressure_min_kpa, e.pressure_max_kpa, " kPa").unwrap_or_else(|| NA.into())
}
fn fuel_label(e: &Experiment) -> String {
    match e.fuel_type.as_deref() {
        Some("none") => "No fuel".into(),
        Some(f) => capitalize(f),
        None => NA.into(),
    }
}
fn sensors(e: &Experiment) -> String {
    e.sensors
        .as_ref()
        .filter(|s| !s.is_empty())
        .map(|s| s.join(" / "))
        .unwrap_or_else(|| NA.into())
}
fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}
fn lines(s: Option<&str>) -> Vec<String> {
    s.unwrap_or("")
        .lines()
        .map(|l| l.trim().trim_start_matches(['-', '•', '*']).trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
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
    pub score: Option<String>,
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
    pub ranked: bool,
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
    let key = q.h.unwrap_or_else(|| "moon_base".into());
    let sel = HABITATS.iter().find(|h| h.key == key).unwrap_or(&HABITATS[1]);

    let experiments = match app.data.experiments().await {
        Ok(e) => e,
        Err(e) => return data_error(e),
    };
    let scores = match app.data.habitat_scores(sel.key).await {
        Ok(s) => s,
        Err(e) => return data_error(e),
    };
    let by_id: HashMap<&str, &Experiment> = experiments.iter().map(|e| (e.id.as_str(), e)).collect();

    let ranked = !scores.is_empty();
    let evidence: Vec<EvidenceRow> = if ranked {
        scores
            .iter()
            .filter_map(|s| by_id.get(s.experiment_id.as_str()).map(|e| (s, *e)))
            .take(6)
            .enumerate()
            .map(|(i, (s, e))| EvidenceRow {
                rank: format!("{:02}", i + 1),
                id: e.id.clone(),
                acronym: e.acronym.clone(),
                cat: category(e.category.as_deref()),
                line: s.reason.clone().or_else(|| e.objective.clone()).unwrap_or_else(|| e.name.clone()),
                score: Some(format!("{}", s.score.round() as i64)),
            })
            .collect()
    } else {
        experiments
            .iter()
            .take(6)
            .enumerate()
            .map(|(i, e)| EvidenceRow {
                rank: format!("{:02}", i + 1),
                id: e.id.clone(),
                acronym: e.acronym.clone(),
                cat: category(e.category.as_deref()),
                line: e.objective.clone().unwrap_or_else(|| e.name.clone()),
                score: None,
            })
            .collect()
    };

    let scale_max = 40.0;
    let (big_figure, big_caption, verdict, explanation, pp_habitat, hab_pos) = match (sel.o2_pct, sel.psi) {
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
            (
                big,
                "Oxygen concentration vs. Earth air".to_string(),
                verdict,
                expl,
                format!("{:.2} psi", o / 100.0 * p),
                Some((o / scale_max * 100.0).min(100.0)),
            )
        }
        _ => (
            NA.to_string(),
            "Oxygen concentration vs. Earth air".to_string(),
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
            links: vec![("BASS-II", "bass-ii"), ("Saffire-III", "saffire-iii")],
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
            let gravity = match sel.key {
                "iss" => "microgravity",
                "mars" => "Mars gravity (3/8 g)",
                _ => "lunar gravity (1/6 g)",
            };
            let scenario = json!({
                "habitat_name": sel.name,
                "oxygen_percent": o,
                "pressure_kpa": (p * 6.89476 * 10.0).round() / 10.0,
                "ventilation_velocity_cm_s": f_vent.trim().parse::<f64>().unwrap_or(10.0),
                "gravity_level": gravity,
                "materials": materials,
                "notes": "Atmosphere from the NASA Space Apps challenge brief",
            });
            Some(ai::from_reply(b.habitat(scenario).await))
        }
        _ => None,
    };

    render(HabitatPage {
        backend_on: app.backend.is_some(),
        can_analyze: sel.o2_pct.is_some(),
        f_materials,
        f_vent,
        ai,
        active: "habitat",
        page_title: "Habitat Risk".into(),
        context_line: context_for(sel.key),
        cards,
        sel_name: sel.name,
        sel_key: sel.key,
        big_figure,
        big_caption,
        verdict,
        explanation,
        pp_habitat,
        pp_earth: format!("{:.2} psi", EARTH_O2 / 100.0 * EARTH_PSI),
        earth_pos: EARTH_O2 / scale_max * 100.0,
        hab_pos,
        evidence,
        ranked,
        total: experiments.len(),
        insights,
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
}

#[derive(Deserialize, Default)]
pub struct ExplorerQuery {
    category: Option<String>,
    fuel: Option<String>,
    hardware: Option<String>,
    q: Option<String>,
}

pub async fn explorer(State(app): AppRef, Query(f): Query<ExplorerQuery>) -> Response {
    let experiments = match app.data.experiments().await {
        Ok(e) => e,
        Err(e) => return data_error(e),
    };
    let total = experiments.len();
    let mut hardware_options: Vec<String> = experiments.iter().filter_map(|e| e.hardware.clone()).collect();
    hardware_options.sort();
    hardware_options.dedup();

    let fc = f.category.clone().unwrap_or_default();
    let ff = f.fuel.clone().unwrap_or_default();
    let fh = f.hardware.clone().unwrap_or_default();
    let fq = f.q.clone().unwrap_or_default();
    let ql = fq.trim().to_lowercase();

    let rows = experiments
        .iter()
        .filter(|e| fc.is_empty() || e.category.as_deref() == Some(fc.as_str()))
        .filter(|e| ff.is_empty() || e.fuel_type.as_deref() == Some(ff.as_str()))
        .filter(|e| fh.is_empty() || e.hardware.as_deref() == Some(fh.as_str()))
        .filter(|e| {
            ql.is_empty()
                || [
                    Some(e.acronym.as_str()),
                    Some(e.name.as_str()),
                    e.objective.as_deref(),
                    e.fuel_details.as_deref(),
                    e.key_findings.as_deref(),
                ]
                .iter()
                .flatten()
                .any(|s| s.to_lowercase().contains(&ql))
        })
        .map(|e| ExplorerRow {
            id: e.id.clone(),
            acronym: e.acronym.clone(),
            cat: category(e.category.as_deref()),
            fuel: fuel_label(e),
            o2: o2(e),
            airflow: airflow(e),
            sensors: sensors(e),
            papers: format!("{:02}", e.paper_count()),
        })
        .collect();

    render(ExplorerPage {
        active: "explorer",
        page_title: "Explorer".into(),
        context_line: context_for("moon_base"),
        rows,
        total,
        f_category: fc,
        f_fuel: ff,
        f_hardware: fh,
        f_q: fq,
        hardware_options,
    })
}

// ---------- experiment detail ----------

pub struct Tile {
    pub label: &'static str,
    pub value: String,
    pub caption: String,
}

#[derive(Template)]
#[template(path = "detail.html")]
pub struct DetailPage {
    pub active: &'static str,
    pub page_title: String,
    pub context_line: String,
    pub e: Experiment,
    pub cat: Cat,
    pub meta: String,
    pub tiles: Vec<Tile>,
    pub brief: Option<String>,
    pub findings: Vec<String>,
    pub sensor_list: Vec<String>,
    pub papers: Vec<Paper>,
    pub related: Vec<PaperRow>,
    pub backend_on: bool,
    pub ai: Option<AiResult>,
}

#[derive(Deserialize, Default)]
pub struct DetailQuery {
    summarize: Option<String>,
}

pub async fn detail(State(app): AppRef, Path(id): Path<String>, Query(q): Query<DetailQuery>) -> Response {
    let e = match app.data.experiment(&id).await {
        Ok(Some(e)) => e,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                render(ErrorPage {
                    active: "explorer",
                    page_title: "Not found".into(),
                    context_line: String::new(),
                    message: format!("No experiment with id \"{id}\"."),
                }),
            )
                .into_response()
        }
        Err(err) => return data_error(err),
    };
    let papers = match app.data.papers_for(&id).await {
        Ok(p) => p,
        Err(err) => return data_error(err),
    };
    let cat = category(e.category.as_deref());
    let meta = [
        Some(cat.label.clone()),
        e.hardware.clone(),
        e.platform.clone(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ");
    let tiles = vec![
        Tile { label: "Oxygen range", value: o2(&e), caption: "O₂ concentration tested".into() },
        Tile { label: "Pressure", value: pressure(&e), caption: "Chamber pressure".into() },
        Tile { label: "Airflow speed", value: airflow(&e), caption: "Imposed flow".into() },
        Tile {
            label: "Fuel samples",
            value: fuel_label(&e),
            caption: e.fuel_details.clone().unwrap_or_else(|| "Sample details pending".into()),
        },
    ];
    let brief = e.ai_summary.clone().or_else(|| e.objective.clone());
    let findings = lines(e.key_findings.as_deref());
    let sensor_list = e.sensors.clone().unwrap_or_default();

    // From the research backend: papers that mention this experiment, and an AI summary on request.
    let mut related = vec![];
    let mut ai = None;
    if let Some(b) = &app.backend {
        if let Ok(all) = b.papers(None, 200).await {
            let name = e.name.to_lowercase();
            related = all
                .iter()
                .filter(|p| {
                    let text = format!("{} {}", js(p, "title").unwrap_or_default(), js(p, "abstract").unwrap_or_default());
                    text.contains(&e.acronym) || text.to_lowercase().contains(&name)
                })
                .take(8)
                .map(paper_row)
                .collect();
        }
        if q.summarize.is_some() {
            ai = Some(match b.experiments().await {
                Err(err) => ai::from_reply(Err(err)),
                Ok(list) => match backend::match_experiment(&list, &e.acronym) {
                    Some(uuid) => ai::from_reply(b.summarize(&uuid).await),
                    None => ai::from_reply(Err(format!("{} hasn't been indexed by the research backend yet.", e.acronym))),
                },
            });
        }
    }
    render(DetailPage {
        related,
        backend_on: app.backend.is_some(),
        ai,
        active: "explorer",
        page_title: e.acronym.clone(),
        context_line: meta.clone(),
        cat,
        meta,
        tiles,
        brief,
        findings,
        sensor_list,
        papers,
        e,
    })
}

// ---------- compare ----------

pub struct CompareCol {
    pub slot: &'static str,
    pub id: String,
    pub acronym: String,
    pub meta: String,
    pub cat_class: &'static str,
    pub remove_href: String,
}

pub struct CompareRow {
    pub label: &'static str,
    pub cells: Vec<String>,
    pub differs: bool,
    pub links: Vec<Option<String>>,
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
    pub backend_on: bool,
    pub f_focus: String,
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
    let experiments = match app.data.experiments().await {
        Ok(e) => e,
        Err(e) => return data_error(e),
    };
    let want_ai = q.ai.is_some();
    let f_focus = q.focus.clone().unwrap_or_default();
    let untouched = q.a.is_none() && q.b.is_none() && q.c.is_none();
    let picks: [(&'static str, String); 3] = if untouched {
        [("a", "bass-ii".into()), ("b", "saffire-iii".into()), ("c", "flex-2".into())]
    } else {
        [
            ("a", q.a.unwrap_or_default()),
            ("b", q.b.unwrap_or_default()),
            ("c", q.c.unwrap_or_default()),
        ]
    };
    // Compact the picks so chosen experiments always fill slots a, b, c in order.
    const SLOTS: [&str; 3] = ["a", "b", "c"];
    let chosen: Vec<(&'static str, &Experiment)> = picks
        .iter()
        .filter_map(|(_, id)| experiments.iter().find(|e| &e.id == id))
        .enumerate()
        .map(|(i, e)| (SLOTS[i], e))
        .collect();
    let selected: Vec<String> = chosen.iter().map(|(_, e)| e.id.clone()).collect();

    let cols = chosen
        .iter()
        .map(|(slot, e)| {
            let params: Vec<String> = chosen
                .iter()
                .filter(|(s, _)| s != slot)
                .enumerate()
                .map(|(i, (_, o))| format!("{}={}", SLOTS[i], o.id))
                .collect();
            let cat = category(e.category.as_deref());
            CompareCol {
                slot,
                id: e.id.clone(),
                acronym: e.acronym.clone(),
                meta: [Some(cat.label.clone()), e.platform.clone(), e.hardware.clone()]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" · "),
                cat_class: cat.class,
                remove_href: if params.is_empty() { "/compare?a=".into() } else { format!("/compare?{}", params.join("&")) },
            }
        })
        .collect();

    let row = |label: &'static str, f: &dyn Fn(&Experiment) -> String| {
        let cells: Vec<String> = chosen.iter().map(|(_, e)| f(e)).collect();
        let differs = cells.len() > 1 && cells.iter().any(|c| c != &cells[0]);
        CompareRow { label, links: vec![None; cells.len()], cells, differs }
    };
    let mut rows = vec![
        row("Objective", &|e| e.objective.clone().unwrap_or_else(|| e.name.clone())),
        row("Fuel", &|e| match &e.fuel_details {
            Some(d) => format!("{} · {d}", fuel_label(e)),
            None => fuel_label(e),
        }),
        row("O₂ conditions", &|e| o2(e)),
        row("Pressure", &|e| pressure(e)),
        row("Airflow", &|e| airflow(e)),
        row("Sensors", &|e| sensors(e)),
        row("Hardware", &|e| e.hardware.clone().unwrap_or_else(|| NA.into())),
        row("Key findings", &|e| {
            let l = lines(e.key_findings.as_deref());
            if l.is_empty() { "Pending research pipeline".into() } else { l.join(" ") }
        }),
    ];
    rows.push(CompareRow {
        label: "Source",
        cells: chosen
            .iter()
            .map(|(_, e)| if e.psi_url.is_some() { "NASA PSI ↗".into() } else { "Open experiment →".into() })
            .collect(),
        links: chosen
            .iter()
            .map(|(_, e)| Some(e.psi_url.clone().unwrap_or_else(|| format!("/experiments/{}", e.id))))
            .collect(),
        differs: false,
    });

    let options = experiments.iter().map(|e| (e.id.clone(), e.acronym.clone())).collect();
    let can_add = chosen.len() < 3;
    let ai = match (&app.backend, want_ai) {
        (Some(b), true) => Some(match b.experiments().await {
            Err(err) => ai::from_reply(Err(err)),
            Ok(list) => {
                let mut ids = vec![];
                let mut missing = vec![];
                for (_, e) in &chosen {
                    match backend::match_experiment(&list, &e.acronym) {
                        Some(id) => {
                            if !ids.contains(&id) {
                                ids.push(id)
                            }
                        }
                        None => missing.push(e.acronym.clone()),
                    }
                }
                if ids.len() < 2 {
                    ai::from_reply(Err(if missing.is_empty() {
                        "Pick at least two different experiments to compare.".to_string()
                    } else {
                        format!("Not indexed by the research backend yet: {}. The AI needs at least two indexed experiments.", missing.join(", "))
                    }))
                } else {
                    let mut out = ai::from_reply(b.compare(&ids, &f_focus).await);
                    if !missing.is_empty() && out.error.is_none() {
                        out.error = Some(format!("Left out (not indexed yet): {}.", missing.join(", ")));
                    }
                    out
                }
            }
        }),
        _ => None,
    };
    render(ComparePage {
        backend_on: app.backend.is_some(),
        f_focus,
        ai,
        active: "compare",
        page_title: "Compare".into(),
        context_line: context_for("moon_base"),
        cols,
        rows,
        options,
        next_slot: SLOTS[chosen.len().min(2)],
        selected,
        can_add,
    })
}

// ---------- ask the data ----------

#[derive(Template)]
#[template(path = "ask.html")]
pub struct AskPage {
    pub active: &'static str,
    pub page_title: String,
    pub context_line: String,
    pub question: String,
    pub backend_on: bool,
    pub ai: Option<AiResult>,
    pub matched: Vec<(String, String, String)>,
    pub suggestions: Vec<&'static str>,
}

#[derive(Deserialize, Default)]
pub struct AskQuery {
    q: Option<String>,
}

const STOPWORDS: &[&str] = &[
    "what", "which", "where", "when", "does", "with", "that", "this", "from", "most", "about",
    "have", "there", "their", "into", "they", "will", "would", "should", "could", "levels", "level",
];

fn keywords(q: &str) -> Vec<String> {
    let mut out: Vec<String> = q
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 3 && !STOPWORDS.contains(w))
        .map(|w| w.to_string())
        .collect();
    out.dedup();
    out.truncate(5);
    out
}

pub async fn ask(State(app): AppRef, Query(q): Query<AskQuery>) -> Response {
    let question = q.q.unwrap_or_default().trim().to_string();
    let mut page = AskPage {
        active: "ask",
        page_title: "Ask the Data".into(),
        context_line: context_for("moon_base"),
        question: question.clone(),
        backend_on: app.backend.is_some(),
        ai: None,
        matched: vec![],
        suggestions: vec![
            "How does low cabin pressure with elevated oxygen affect flame spread?",
            "How does airflow change flame spread in microgravity?",
            "Which experiments studied fire suppression?",
        ],
    };
    if question.is_empty() {
        return render(page);
    }
    if let Some(b) = &app.backend {
        let mut out = ai::from_reply(b.query(&question).await);
        // The question is already the page headline.
        out.blocks.retain(|blk| blk.heading.as_deref() != Some("Question"));
        if out.blocks.len() == 1 {
            out.blocks[0].heading = None;
        }
        page.ai = Some(out);
        return render(page);
    }
    // Not connected: show catalog experiments whose text matches the question.
    let terms = keywords(&question);
    if let Ok(exps) = app.data.experiments().await {
        page.matched = exps
            .iter()
            .filter(|e| {
                let hay = format!(
                    "{} {} {} {} {}",
                    e.acronym,
                    e.name,
                    e.objective.clone().unwrap_or_default(),
                    e.key_findings.clone().unwrap_or_default(),
                    e.category.clone().unwrap_or_default()
                )
                .to_lowercase();
                terms.iter().any(|t| hay.contains(t.as_str()))
            })
            .take(6)
            .map(|e| (e.id.clone(), e.acronym.clone(), e.name.clone()))
            .collect();
    }
    render(page)
}

// ---------- literature (research backend) ----------

pub struct PaperRow {
    pub href: String,
    pub title: String,
    pub year: String,
    pub kind: String,
    pub status: String,
}

fn paper_row(p: &Value) -> PaperRow {
    let id = js(p, "ntrs_id").or_else(|| js(p, "id")).unwrap_or_default();
    PaperRow {
        href: format!("/papers/{}", urlencoding::encode(&id)),
        title: js(p, "title").unwrap_or_else(|| "Untitled".into()),
        year: js(p, "publication_date").map(|d| d.chars().take(4).collect()).unwrap_or_else(|| NA.into()),
        kind: js(p, "pub_type").unwrap_or_else(|| NA.into()),
        status: js(p, "status").map(|x| capitalize(&x)).unwrap_or_else(|| NA.into()),
    }
}

#[derive(Template)]
#[template(path = "papers.html")]
pub struct PapersPage {
    pub active: &'static str,
    pub page_title: String,
    pub context_line: String,
    pub backend_on: bool,
    pub error: Option<String>,
    pub rows: Vec<PaperRow>,
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
        backend_on: app.backend.is_some(),
        error: None,
        rows: vec![],
        loaded: 0,
        counts: vec![],
        f_status: f_status.clone(),
        f_q: f_q.clone(),
    };
    let Some(b) = &app.backend else { return render(page) };
    match b.papers(Some(&f_status), 200).await {
        Err(e) => page.error = Some(e),
        Ok(all) => {
            page.loaded = all.len();
            let ql = f_q.trim().to_lowercase();
            page.rows = all
                .iter()
                .filter(|p| {
                    ql.is_empty()
                        || format!("{} {}", js(p, "title").unwrap_or_default(), js(p, "abstract").unwrap_or_default())
                            .to_lowercase()
                            .contains(&ql)
                })
                .map(paper_row)
                .collect();
        }
    }
    if let Ok(st) = b.pipeline_status().await {
        if let Some(list) = st.get("papers_by_status").and_then(|x| x.as_array()) {
            page.counts = list
                .iter()
                .filter_map(|x| Some((js(x, "status")?, js(x, "count")?)))
                .collect();
        }
    }
    render(page)
}

pub struct ConditionRow {
    pub fuel: String,
    pub oxidizer: String,
    pub o2: String,
    pub pressure: String,
    pub flow: String,
    pub gravity: String,
    pub geometry: String,
    pub notes: Option<String>,
    pub verified: bool,
}

pub struct FindingRow {
    pub claim: String,
    pub category: String,
    pub metric: Option<String>,
    pub confidence: Option<String>,
    pub quote: Option<String>,
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
    pub conditions: Vec<ConditionRow>,
    pub findings: Vec<FindingRow>,
}

pub async fn paper(State(app): AppRef, Path(id): Path<String>) -> Response {
    let Some(b) = &app.backend else {
        return data_error("The research backend isn't connected, so papers can't be shown yet.".into());
    };
    let data = match b.extractions(&id).await {
        Ok(d) => d,
        Err(e) => return data_error(e),
    };
    let p = data.get("paper").cloned().unwrap_or(Value::Null);
    let authors = match p.get("authors") {
        Some(Value::Array(a)) => a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(", "),
        Some(Value::String(x)) => x.clone(),
        _ => String::new(),
    };
    let or_na = |x: Option<String>| x.unwrap_or_else(|| NA.into());
    let rows = |key: &str| data.get(key).and_then(|x| x.as_array()).cloned().unwrap_or_default();
    let conditions = rows("conditions")
        .iter()
        .map(|c| ConditionRow {
            fuel: or_na(js(c, "fuel_material")),
            oxidizer: or_na(js(c, "oxidizer")),
            o2: or_na(f(c, "oxygen_percent").map(|x| format!("{}%", num(x)))),
            pressure: or_na(f(c, "pressure_kpa").map(|x| format!("{} kPa", num(x)))),
            flow: or_na(f(c, "flow_velocity_cm_s").map(|x| format!("{} cm/s", num(x)))),
            gravity: or_na(js(c, "gravity_level")),
            geometry: or_na(js(c, "sample_geometry")),
            notes: js(c, "notes"),
            verified: flag(c, "verified"),
        })
        .collect();
    let findings = rows("findings")
        .iter()
        .map(|x| FindingRow {
            claim: or_na(js(x, "claim")),
            category: js(x, "category").map(|c| capitalize(&c.replace('_', " "))).unwrap_or_default(),
            metric: match (js(x, "metric_name"), js(x, "metric_value")) {
                (Some(n), Some(v)) => Some(format!("{}: {v}", n.replace('_', " "))),
                (None, Some(v)) => Some(v),
                _ => None,
            },
            confidence: f(x, "confidence").map(|c| format!("{}%", (c * 100.0).round() as i64)),
            quote: js(x, "source_quote"),
            verified: flag(x, "verified"),
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
        conditions,
        findings,
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
    (
        StatusCode::NOT_FOUND,
        render(ErrorPage {
            active: "",
            page_title: "Not found".into(),
            context_line: String::new(),
            message: "That page doesn't exist.".into(),
        }),
    )
        .into_response()
}
