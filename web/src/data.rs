//! Data access: reads from Supabase (PostgREST) in production, or from a local
//! JSON snapshot when `DATA_FIXTURE` is set (offline development).

use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct CountRow {
    pub count: i64,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Experiment {
    pub id: String,
    pub name: String,
    pub acronym: String,
    pub category: Option<String>,
    pub fuel_type: Option<String>,
    pub fuel_details: Option<String>,
    pub hardware: Option<String>,
    pub platform: Option<String>,
    pub gravity: Option<String>,
    pub oxygen_min_pct: Option<f64>,
    pub oxygen_max_pct: Option<f64>,
    pub pressure_min_kpa: Option<f64>,
    pub pressure_max_kpa: Option<f64>,
    pub airflow_min_cms: Option<f64>,
    pub airflow_max_cms: Option<f64>,
    pub sensors: Option<Vec<String>>,
    pub objective: Option<String>,
    pub key_findings: Option<String>,
    pub ai_summary: Option<String>,
    pub earth_application: Option<String>,
    pub psi_url: Option<String>,
    /// Embedded `papers(count)` from PostgREST.
    #[serde(default)]
    pub papers: Option<Vec<CountRow>>,
}

impl Experiment {
    pub fn paper_count(&self) -> i64 {
        self.papers
            .as_ref()
            .and_then(|v| v.first())
            .map(|c| c.count)
            .unwrap_or(0)
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Paper {
    pub id: i64,
    pub experiment_id: Option<String>,
    pub ntrs_id: Option<String>,
    pub title: String,
    pub authors: Option<String>,
    pub year: Option<i32>,
    #[serde(rename = "abstract")]
    pub abstract_text: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HabitatScore {
    pub experiment_id: String,
    pub habitat: Option<String>,
    pub score: f64,
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct Fixture {
    experiments: Vec<Experiment>,
    #[serde(default)]
    papers: Vec<Paper>,
    #[serde(default)]
    habitat_scores: Vec<HabitatScore>,
}

pub enum Source {
    Supabase {
        url: String,
        key: String,
        http: reqwest::Client,
    },
    Fixture(Fixture),
}

pub type DataResult<T> = Result<T, String>;

const PAPER_COLUMNS: &str = "id,experiment_id,ntrs_id,title,authors,year,abstract,url";

impl Source {
    pub fn from_env() -> Self {
        if let Ok(path) = std::env::var("DATA_FIXTURE") {
            let raw = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("cannot read DATA_FIXTURE {path}: {e}"));
            let mut fx: Fixture = serde_json::from_str(&raw).expect("invalid fixture JSON");
            // Mirror PostgREST's embedded paper count.
            let mut counts: HashMap<String, i64> = HashMap::new();
            for p in &fx.papers {
                if let Some(id) = &p.experiment_id {
                    *counts.entry(id.clone()).or_default() += 1;
                }
            }
            for e in &mut fx.experiments {
                e.papers = Some(vec![CountRow {
                    count: *counts.get(&e.id).unwrap_or(&0),
                }]);
            }
            println!("data source: fixture {path}");
            return Source::Fixture(fx);
        }
        let url = std::env::var("SUPABASE_URL")
            .unwrap_or_else(|_| "https://hdawtwegmvuczzshrgmr.supabase.co".into());
        // Publishable (read-only) key: safe to ship. Never put the service_role key here.
        let key = std::env::var("SUPABASE_KEY")
            .unwrap_or_else(|_| "sb_publishable_9BFkivzzJw701pXLfe8ttA_MNP-Pc6I".into());
        println!("data source: supabase {url}");
        Source::Supabase {
            url,
            key,
            http: reqwest::Client::new(),
        }
    }

    async fn get<T: for<'de> Deserialize<'de>>(
        &self,
        table: &str,
        query: &[(&str, String)],
    ) -> DataResult<T> {
        let Source::Supabase { url, key, http } = self else {
            unreachable!()
        };
        let resp = http
            .get(format!("{url}/rest/v1/{table}"))
            .header("apikey", key)
            .header("Authorization", format!("Bearer {key}"))
            .query(query)
            .send()
            .await
            .map_err(|e| format!("Supabase request failed: {e}"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Supabase {table} returned {status}: {body}"));
        }
        resp.json::<T>()
            .await
            .map_err(|e| format!("Supabase {table} parse error: {e}"))
    }

    pub async fn experiments(&self) -> DataResult<Vec<Experiment>> {
        let mut list = match self {
            Source::Fixture(fx) => fx.experiments.clone(),
            Source::Supabase { .. } => {
                self.get(
                    "experiments",
                    &[("select", "*,papers(count)".into())],
                )
                .await?
            }
        };
        list.sort_by(|a, b| a.acronym.to_lowercase().cmp(&b.acronym.to_lowercase()));
        Ok(list)
    }

    pub async fn experiment(&self, id: &str) -> DataResult<Option<Experiment>> {
        Ok(self.experiments().await?.into_iter().find(|e| e.id == id))
    }

    pub async fn papers_for(&self, experiment_id: &str) -> DataResult<Vec<Paper>> {
        match self {
            Source::Fixture(fx) => Ok(fx
                .papers
                .iter()
                .filter(|p| p.experiment_id.as_deref() == Some(experiment_id))
                .cloned()
                .collect()),
            Source::Supabase { .. } => {
                self.get(
                    "papers",
                    &[
                        ("select", PAPER_COLUMNS.into()),
                        ("experiment_id", format!("eq.{experiment_id}")),
                        ("order", "year.desc.nullslast".into()),
                    ],
                )
                .await
            }
        }
    }

    pub async fn habitat_scores(&self, habitat: &str) -> DataResult<Vec<HabitatScore>> {
        let mut rows: Vec<HabitatScore> = match self {
            Source::Fixture(fx) => fx
                .habitat_scores
                .iter()
                .filter(|s| s.habitat.as_deref() == Some(habitat))
                .cloned()
                .collect(),
            Source::Supabase { .. } => {
                self.get(
                    "habitat_scores",
                    &[
                        ("select", "experiment_id,habitat,score,reason".into()),
                        ("habitat", format!("eq.{habitat}")),
                    ],
                )
                .await?
            }
        };
        rows.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        Ok(rows)
    }

    /// Keyword search over paper titles and abstracts (fallback when the AI
    /// endpoint is not configured).
    pub async fn search_papers(&self, terms: &[String]) -> DataResult<Vec<Paper>> {
        if terms.is_empty() {
            return Ok(vec![]);
        }
        match self {
            Source::Fixture(fx) => Ok(fx
                .papers
                .iter()
                .filter(|p| {
                    let hay = format!(
                        "{} {}",
                        p.title.to_lowercase(),
                        p.abstract_text.clone().unwrap_or_default().to_lowercase()
                    );
                    terms.iter().any(|t| hay.contains(t.as_str()))
                })
                .take(6)
                .cloned()
                .collect()),
            Source::Supabase { .. } => {
                let ors: Vec<String> = terms
                    .iter()
                    .flat_map(|t| [format!("title.ilike.*{t}*"), format!("abstract.ilike.*{t}*")])
                    .collect();
                self.get(
                    "papers",
                    &[
                        ("select", PAPER_COLUMNS.into()),
                        ("or", format!("({})", ors.join(","))),
                        ("limit", "6".into()),
                    ],
                )
                .await
            }
        }
    }
}
