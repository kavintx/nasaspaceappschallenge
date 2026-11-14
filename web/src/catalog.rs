//! The experiment catalog, built from the research backend: every experiment
//! with its linked papers and the test conditions and findings extracted from
//! them. Built once and cached, because it takes one request per paper.

use crate::backend::{f, s, Backend};
use futures::stream::{self, StreamExt};
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

pub struct Catalog {
    pub exps: Vec<Exp>,
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

/// Category, fuel phase and hardware are not in the database. These come from
/// the NASA challenge brief.
fn meta(acronym: &str) -> (&'static str, &'static str, &'static str) {
    let key: String = acronym.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_uppercase();
    match key.as_str() {
        "BASS" | "BASSII" => ("prevention", "solid", "MSG flow duct"),
        "SAFFIREI" | "SAFFIREII" | "SAFFIREIII" => ("prevention", "solid", "Cygnus spacecraft"),
        "SAME" | "SAMER" => ("detection", "solid", "MSG"),
        "FLEX" | "FLEX2" => ("suppression", "liquid", "CIR / MDCA"),
        "CFI" => ("other", "liquid", "CIR / MDCA"),
        "ACME" => ("other", "gas", "CIR"),
        "SPICE" | "SLICE" => ("other", "gas", "MSG flow duct"),
        _ => ("other", "", ""),
    }
}

/// Numbers in an oxygen string such as "21%" or "18-21% O2". Only strings that
/// state a percentage are used, and digits inside names like "O2" are ignored.
pub fn pct_numbers(text: &str) -> Vec<f64> {
    if !text.contains('%') {
        return vec![];
    }
    let chars: Vec<char> = text.chars().collect();
    let mut out = vec![];
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            let after_letter = start > 0 && chars[start - 1].is_alphabetic();
            if !after_letter {
                let tok: String = chars[start..i].iter().collect();
                if let Ok(n) = tok.trim_end_matches('.').parse::<f64>() {
                    if n > 0.0 && n <= 100.0 {
                        out.push(n);
                    }
                }
            }
        } else {
            i += 1;
        }
    }
    out
}

fn add_distinct(list: &mut Vec<String>, value: Option<String>) {
    if let Some(v) = value {
        if !list.iter().any(|x| x.eq_ignore_ascii_case(&v)) {
            list.push(v);
        }
    }
}

pub async fn build(b: &Backend) -> Result<Catalog, String> {
    let raw = b.experiments().await?;
    let papers = b.papers(None, 200).await?;
    let ids: Vec<String> = papers.iter().filter_map(|p| s(p, "id").or_else(|| s(p, "ntrs_id"))).collect();
    let paper_count = ids.len();

    // One request per paper, at most 4 at a time.
    let results: Vec<Result<Value, String>> = stream::iter(ids)
        .map(|id| async move { b.extractions(&id).await })
        .buffer_unordered(4)
        .collect()
        .await;

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
            let (category, fuel_phase, hardware) = meta(&acronym);
            let mut e = acc.remove(&uuid).unwrap_or_default();
            e.findings.sort_by(|a, b| {
                b.verified
                    .cmp(&a.verified)
                    .then(b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal))
            });
            Some(Exp {
                id: acronym.to_lowercase().replace(' ', "-"),
                name: s(r, "name").unwrap_or_else(|| acronym.clone()),
                description: s(r, "description"),
                acronym,
                category,
                fuel_phase,
                hardware,
                ..e
            })
        })
        .collect();
    exps.sort_by(|a, b| a.acronym.cmp(&b.acronym));
    Ok(Catalog { exps, paper_count, failed, built: Instant::now() })
}
