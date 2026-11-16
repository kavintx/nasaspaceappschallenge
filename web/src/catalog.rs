//! The experiment catalog, built from the research database: every experiment
//! with its linked papers and the test conditions and findings extracted from
//! them. Built once and cached for a few minutes.

use crate::backend::{f, https, s};
use crate::store::Store;
use crate::units::{self, Span};
use serde_json::Value;
use std::collections::HashMap;
use std::time::Instant;

#[derive(Clone)]
pub struct PaperRef {
    pub href: String,
    pub title: String,
    pub year: String,
    pub kind: String,
    pub status: String,
}

pub struct Finding {
    pub text: String,
    /// prevention | detection | suppression | other | ""
    pub category: String,
    pub paper_href: String,
    pub source: String,
    pub confidence: Option<f64>,
    pub verified: bool,
}

#[derive(Default)]
pub struct Exp {
    /// URL slug, e.g. "saffire-i"
    pub id: String,
    /// Exactly as the backend stores it, e.g. "SAFFIRE I"
    pub acronym: String,
    pub name: String,
    pub description: Option<String>,
    /// True when name and description come from the NASA brief, not the database.
    pub from_brief: bool,
    pub category: &'static str,
    pub fuel_phase: &'static str,
    pub hardware: &'static str,
    pub fuels: Vec<String>,
    pub o2_values: Vec<f64>,
    pub o2_raw: Vec<String>,
    pub pressures: Vec<String>,
    pub airflows: Vec<String>,
    pub sensors: Vec<String>,
    pub objectives: Vec<String>,
    pub conditions: usize,
    pub findings: Vec<Finding>,
    pub papers: Vec<PaperRef>,
}

impl Exp {
    pub fn o2_max(&self) -> Option<f64> {
        self.o2_values.iter().cloned().reduce(f64::max)
    }
    pub fn o2_min(&self) -> Option<f64> {
        self.o2_values.iter().cloned().reduce(f64::min)
    }
}

/// What a test recorded about the flame, from the `outcome` column of `conditions`.
/// Anything missing or unrecognised is `Unknown`: the site never guesses a verdict.
#[derive(Clone, Copy, PartialEq)]
pub enum Outcome {
    Burned,
    WentOut,
    NoIgnition,
    Unknown,
}

impl Outcome {
    pub fn parse(raw: Option<String>) -> Self {
        match raw.unwrap_or_default().trim().to_lowercase().as_str() {
            "ignited" | "sustained_flame" => Outcome::Burned,
            "self_extinguished" => Outcome::WentOut,
            "no_ignition" => Outcome::NoIgnition,
            _ => Outcome::Unknown,
        }
    }
}

/// One extracted test condition that names a fuel.
pub struct Test {
    /// Every stated oxygen level in percent (both ends of a range), for the per-habitat verdicts.
    pub o2: Vec<f64>,
    pub o2_raw: Option<String>,
    /// Oxygen in percent and pressure in kPa as stated values or ranges. Empty when the paper
    /// didn't state one or it couldn't be read: see units.rs.
    pub o2_spans: Vec<Span>,
    pub kpa_spans: Vec<Span>,
    pub pressure: Option<String>,
    /// Where in the paper the outcome (or, failing that, the conditions) was read from.
    pub source_page: Option<String>,
    pub source_quote: Option<String>,
    pub airflow: Option<String>,
    pub outcome: Outcome,
    pub outcome_note: Option<String>,
    pub paper_href: String,
    pub paper_title: String,
    /// The paper on NTRS, when the database has a plain https link for it.
    pub paper_url: Option<String>,
    /// Experiment acronyms, empty when the test is not tied to one.
    pub experiments: Vec<String>,
}

impl Test {
    pub fn o2_max(&self) -> Option<f64> {
        self.o2.iter().cloned().reduce(f64::max)
    }

    /// True when the test can be placed on the oxygen-pressure map: both are stated, and at
    /// most one of them is a list. Two lists can't be paired without guessing which went with which.
    pub fn plottable(&self) -> bool {
        !self.o2_spans.is_empty() && !self.kpa_spans.is_empty() && (self.o2_spans.len() == 1 || self.kpa_spans.len() == 1)
    }

    /// True when the row has oxygen or pressure text that states no readable value.
    pub fn unread(&self) -> bool {
        (self.o2_raw.is_some() && self.o2_spans.is_empty()) || (self.pressure.is_some() && self.kpa_spans.is_empty())
    }
}

/// A fuel named in the extracted test conditions, with every test of it.
pub struct Material {
    pub name: String,
    pub tests: Vec<Test>,
}

pub struct Catalog {
    pub exps: Vec<Exp>,
    /// Fuels from the extracted test conditions, most tested first.
    pub materials: Vec<Material>,
    pub paper_count: usize,
    /// Papers whose extractions could not be read.
    pub failed: usize,
    pub built: Instant,
}

pub const NA: &str = "—";

pub fn paper_ref(p: &Value) -> PaperRef {
    let id = s(p, "ntrs_id").or_else(|| s(p, "id")).unwrap_or_default();
    let cap = |x: String| {
        let mut c = x.chars();
        c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
    };
    PaperRef {
        href: format!("/papers/{}", urlencoding::encode(&id)),
        title: s(p, "title").unwrap_or_else(|| "Untitled".into()),
        year: s(p, "publication_date").map(|d| d.chars().take(4).collect()).unwrap_or_else(|| NA.into()),
        kind: s(p, "pub_type").unwrap_or_else(|| NA.into()),
        status: s(p, "status").map(cap).unwrap_or_else(|| NA.into()),
    }
}

pub struct Meta {
    pub category: &'static str,
    pub fuel_phase: &'static str,
    pub hardware: &'static str,
    /// Official name and a one-line description, when the NASA brief gives them.
    pub name: Option<&'static str>,
    pub about: Option<&'static str>,
}

/// Facts about each experiment taken from the NASA challenge brief. Category,
/// fuel phase and hardware are not in the database, and the database's seeded
/// names and descriptions do not match NASA's, so the brief's are shown instead.
fn meta(acronym: &str) -> Meta {
    let key: String = acronym.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_uppercase();
    let m = |category, fuel_phase, hardware, name, about| Meta { category, fuel_phase, hardware, name: Some(name), about: Some(about) };
    const SOLIDS: &str = "Studies the flammability of solid materials such as fabrics and plastics, which depends on oxygen concentration and, in microgravity, on airflow.";
    const DROPLETS: &str = "Burned droplets of liquid fuel in the Multi-user Droplet Combustion Apparatus inside the Combustion Integrated Rack.";
    match key.as_str() {
        "BASS" => m("prevention", "solid", "MSG flow duct", "Burning and Suppression of Solids", SOLIDS),
        "BASSII" => m("prevention", "solid", "MSG flow duct", "Burning and Suppression of Solids II", SOLIDS),
        "SAFFIREI" => m("prevention", "solid", "Cygnus spacecraft", "Spacecraft Fire Safety Demonstration I", SOLIDS),
        "SAFFIREII" => m("prevention", "solid", "Cygnus spacecraft", "Spacecraft Fire Safety Demonstration II", SOLIDS),
        "SAFFIREIII" => m("prevention", "solid", "Cygnus spacecraft", "Spacecraft Fire Safety Demonstration III", SOLIDS),
        "SAME" => m("detection", "solid", "MSG", "Smoke and Aerosol Measurement Experiment", "Tested fire detection in microgravity by measuring smoke."),
        "SAMER" => m("detection", "solid", "MSG", "Smoke and Aerosol Measurement Experiment Reflight", "Tested fire detection in microgravity by measuring smoke."),
        "FLEX" => m("suppression", "liquid", "CIR / MDCA", "Flame Extinguishment Experiment", "Studied fire suppression by diluting the chamber atmosphere with carbon dioxide and helium while burning droplets of liquid fuel."),
        "FLEX2" => m("suppression", "liquid", "CIR / MDCA", "Flame Extinguishment Experiment 2", DROPLETS),
        "CFI" => m("other", "liquid", "CIR / MDCA", "Cool Flames Investigation", DROPLETS),
        "ACME" => m("other", "gas", "CIR", "Advanced Combustion via Microgravity Experiments", "Six experiments on flames of gaseous fuels, all run on the same modular hardware in the Combustion Integrated Rack. They include the Burning Rate Emulator (materials flammability), Cool Flames Investigation with Gases (up to 85% oxygen) and E-FIELD Flames."),
        "SPICE" => m("other", "gas", "MSG flow duct", "Smoke Point In Co-flow Experiment", "Run in the Microgravity Science Glovebox using the fan-driven flow duct that was built for it."),
        "SLICE" => m("other", "gas", "MSG flow duct", "Structure & Liftoff In Combustion Experiment", "Run in the Microgravity Science Glovebox using the flow duct built for SPICE."),
        _ => Meta { category: "other", fuel_phase: "", hardware: "", name: None, about: None },
    }
}

/// Every oxygen level in percent that this text states, both ends of a range included.
/// Read by units.rs, the one parser for these columns.
pub fn pct_numbers(text: &str) -> Vec<f64> {
    ends(&units::oxygen_pct(text))
}

fn ends(spans: &[Span]) -> Vec<f64> {
    spans.iter().flat_map(|s| if s.is_range() { vec![s.min, s.max] } else { vec![s.min] }).collect()
}

/// The numeric columns when the row has them, otherwise the text column read by units.rs.
fn stated(row: &Value, min: &str, max: &str, text: Option<&str>, read: fn(&str) -> Vec<Span>) -> Vec<Span> {
    match units::column(f(row, min), f(row, max)) {
        Some(span) => vec![span],
        None => text.map(read).unwrap_or_default(),
    }
}

fn add_distinct(list: &mut Vec<String>, value: Option<String>) {
    if let Some(v) = value {
        if !list.iter().any(|x| x.eq_ignore_ascii_case(&v)) {
            list.push(v);
        }
    }
}

pub async fn build(store: &Store) -> Result<Catalog, String> {
    let (raw, papers, links, conditions, findings) =
        tokio::join!(store.experiments(), store.papers(None), store.links(), store.conditions(), store.findings());
    let (raw, papers, links, conditions, findings) = (raw?, papers?, links?, conditions?, findings?);
    let paper_count = papers.len();

    // Group everything by paper, then roll each paper up into its experiments.
    let group = |rows: Vec<Value>, key: &str| {
        let mut map: HashMap<String, Vec<Value>> = HashMap::new();
        for row in rows {
            if let Some(id) = s(&row, key) {
                map.entry(id).or_default().push(row);
            }
        }
        map
    };
    let mut links = group(links, "paper_id");
    let mut conditions = group(conditions, "paper_id");
    let mut findings = group(findings, "paper_id");
    let results: Vec<Result<Value, String>> = papers
        .into_iter()
        .map(|paper| {
            let id = s(&paper, "id").unwrap_or_default();
            let experiments: Vec<Value> = links
                .remove(&id)
                .unwrap_or_default()
                .iter()
                .filter_map(|l| s(l, "experiment_id"))
                .map(|e| serde_json::json!({ "id": e }))
                .collect();
            Ok(serde_json::json!({
                "paper": paper,
                "experiments": experiments,
                "conditions": conditions.remove(&id).unwrap_or_default(),
                "findings": findings.remove(&id).unwrap_or_default(),
            }))
        })
        .collect();

    let acronyms: HashMap<String, String> =
        raw.iter().filter_map(|r| Some((s(r, "id")?, s(r, "acronym").or_else(|| s(r, "name"))?))).collect();
    let mut materials: Vec<Material> = vec![];
    let mut acc: HashMap<String, Exp> = HashMap::new();
    let mut failed = 0;
    for res in results {
        let Ok(x) = res else {
            failed += 1;
            continue;
        };
        let paper = x.get("paper").cloned().unwrap_or(Value::Null);
        let pref = paper_ref(&paper);
        let ntrs = s(&paper, "ntrs_id");
        let rows = |key: &str| x.get(key).and_then(|v| v.as_array()).cloned().unwrap_or_default();
        let linked: Vec<String> = rows("experiments").iter().filter_map(|e| s(e, "id")).collect();
        for id in &linked {
            acc.entry(id.clone()).or_default().papers.push(pref.clone());
        }
        // A row belongs to its own experiment, or to the paper's experiment when there is only one.
        let owners = |row: &Value| match s(row, "experiment_id") {
            Some(id) => vec![id],
            None if linked.len() == 1 => linked.clone(),
            None => vec![],
        };
        let rejected = |row: &Value| s(row, "verification_status").as_deref() == Some("rejected");

        for c in rows("conditions").iter().filter(|c| !rejected(c)) {
            if let Some(fuel) = s(c, "fuel_type") {
                let o2_raw = s(c, "oxygen_concentration");
                let pressure = s(c, "pressure");
                let o2_spans: Vec<Span> = stated(c, "oxygen_pct_min", "oxygen_pct_max", o2_raw.as_deref(), units::oxygen_pct)
                    .into_iter()
                    .filter(|s| s.min > 0.0 && s.max <= 100.0)
                    .collect();
                let test = Test {
                    o2: ends(&o2_spans),
                    o2_raw,
                    kpa_spans: stated(c, "pressure_kpa_min", "pressure_kpa_max", pressure.as_deref(), units::pressure_kpa),
                    o2_spans,
                    pressure,
                    source_page: s(c, "outcome_source_page").or_else(|| s(c, "source_page")),
                    source_quote: s(c, "outcome_source_quote").or_else(|| s(c, "source_quote")),
                    airflow: s(c, "airflow"),
                    // These two columns may not exist yet; a missing column reads as unknown.
                    outcome: Outcome::parse(s(c, "outcome")),
                    outcome_note: s(c, "outcome_note"),
                    paper_href: pref.href.clone(),
                    paper_title: pref.title.clone(),
                    paper_url: https(&paper, "ntrs_url"),
                    // Only the experiment the row itself names. A paper's single linked experiment is
                    // not assumed: proceedings volumes report many unrelated tests.
                    experiments: s(c, "experiment_id").and_then(|id| acronyms.get(&id).cloned()).into_iter().collect(),
                };
                match materials.iter_mut().find(|m| m.name.eq_ignore_ascii_case(&fuel)) {
                    Some(m) => m.tests.push(test),
                    None => materials.push(Material { name: fuel, tests: vec![test] }),
                }
            }
            for id in owners(c) {
                let e = acc.entry(id).or_default();
                e.conditions += 1;
                add_distinct(&mut e.fuels, s(c, "fuel_type"));
                add_distinct(&mut e.pressures, s(c, "pressure"));
                add_distinct(&mut e.airflows, s(c, "airflow"));
                add_distinct(&mut e.objectives, s(c, "objective"));
                if let Some(o) = s(c, "oxygen_concentration") {
                    e.o2_values.extend(pct_numbers(&o));
                    add_distinct(&mut e.o2_raw, Some(o));
                }
                if let Some(list) = c.get("sensors").and_then(|v| v.as_array()) {
                    for item in list {
                        add_distinct(&mut e.sensors, item.as_str().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()));
                    }
                }
            }
        }
        for fd in rows("findings").iter().filter(|x| !rejected(x)) {
            let Some(text) = s(fd, "text") else { continue };
            let mut source = vec![];
            if let Some(n) = &ntrs {
                source.push(format!("NTRS {n}"));
            }
            if let Some(p) = f(fd, "source_page") {
                source.push(format!("p. {}", p as i64));
            }
            for id in owners(fd) {
                acc.entry(id).or_default().findings.push(Finding {
                    text: text.clone(),
                    category: s(fd, "category").unwrap_or_default().to_lowercase(),
                    paper_href: pref.href.clone(),
                    source: source.join(" · "),
                    confidence: f(fd, "confidence"),
                    verified: matches!(s(fd, "verification_status").as_deref(), Some("verified") | Some("corrected")),
                });
            }
        }
    }

    let mut exps: Vec<Exp> = raw
        .iter()
        .filter_map(|r| {
            let uuid = s(r, "id")?;
            let acronym = s(r, "acronym").or_else(|| s(r, "name"))?;
            let m = meta(&acronym);
            let mut e = acc.remove(&uuid).unwrap_or_default();
            e.findings.sort_by(|a, b| {
                b.verified
                    .cmp(&a.verified)
                    .then(b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal))
            });
            Some(Exp {
                id: acronym.to_lowercase().replace(' ', "-"),
                name: m.name.map(str::to_string).or_else(|| s(r, "name")).unwrap_or_else(|| acronym.clone()),
                description: m.about.map(str::to_string).or_else(|| s(r, "description")),
                from_brief: m.about.is_some(),
                acronym,
                category: m.category,
                fuel_phase: m.fuel_phase,
                hardware: m.hardware,
                ..e
            })
        })
        .collect();
    exps.sort_by(|a, b| a.acronym.cmp(&b.acronym));
    let tests = || materials.iter().flat_map(|m| m.tests.iter());
    println!(
        "flammability map: {} tests of {} fuel names; {} can be placed by oxygen and pressure; {} have oxygen or pressure text that states no readable value; {} fuel names have 3 or more placeable tests",
        tests().count(),
        materials.len(),
        tests().filter(|t| t.plottable()).count(),
        tests().filter(|t| t.unread()).count(),
        materials.iter().filter(|m| m.tests.iter().filter(|t| t.plottable()).count() >= 3).count(),
    );
    materials.sort_by(|a, b| b.tests.len().cmp(&a.tests.len()).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(Catalog { exps, materials, paper_count, failed, built: Instant::now() })
}
