//! Read-only access to the research database in Supabase, through its REST API
//! with the publishable key. The database only lets that key read experiments,
//! papers (without full text), conditions and findings.

use crate::config::Config;
use serde_json::{json, Value};
use std::time::Duration;

pub struct Store {
    url: String,
    key: String,
    http: reqwest::Client,
}

pub type StoreResult<T> = Result<T, String>;

const PAGE: usize = 1000;
const MAX_PAGES: usize = 20;
const PAPER_COLUMNS: &str =
    "id,ntrs_id,title,abstract,authors,publication_date,doi,ntrs_url,pdf_url,pub_type,pages_total,status,updated_at";

impl Store {
    pub fn from_cfg(cfg: &Config) -> Self {
        let url = cfg
            .get("SUPABASE_URL")
            .unwrap_or_else(|| "https://hdawtwegmvuczzshrgmr.supabase.co".into())
            .trim_end_matches('/')
            .to_string();
        // Publishable (read-only) key: safe to ship. Never put a secret or service_role key here.
        let key = cfg
            .get("SUPABASE_KEY")
            .unwrap_or_else(|| "sb_publishable_9BFkivzzJw701pXLfe8ttA_MNP-Pc6I".into());
        println!("research database: {url}");
        Store { url, key, http: reqwest::Client::new() }
    }

    async fn page(&self, table: &str, query: &[(&str, String)], offset: usize) -> StoreResult<Vec<Value>> {
        let resp = self
            .http
            .get(format!("{}/rest/v1/{table}", self.url))
            .header("apikey", &self.key)
            .query(query)
            .query(&[("limit", PAGE.to_string()), ("offset", offset.to_string())])
            .timeout(Duration::from_secs(20))
            .send()
            .await
            .map_err(|_| format!("Couldn't reach the research database at {}.", self.url))?;
        let status = resp.status();
        let body: Value = resp.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            let detail = body.get("message").and_then(|m| m.as_str()).unwrap_or("no details");
            return Err(format!("The research database returned {} for {table}: {detail}.", status.as_u16()));
        }
        match body {
            Value::Array(rows) => Ok(rows),
            _ => Err(format!("The research database sent an unexpected reply for {table}.")),
        }
    }

    /// Every matching row, fetched a page at a time.
    async fn rows(&self, table: &str, query: &[(&str, String)]) -> StoreResult<Vec<Value>> {
        let mut all = vec![];
        for n in 0..MAX_PAGES {
            let batch = self.page(table, query, n * PAGE).await?;
            let last = batch.len() < PAGE;
            all.extend(batch);
            if last {
                break;
            }
        }
        Ok(all)
    }

    pub async fn experiments(&self) -> StoreResult<Vec<Value>> {
        self.rows(
            "experiments",
            &[("select", "id,acronym,name,description,source".into()), ("order", "acronym.asc".into())],
        )
        .await
    }

    pub async fn papers(&self, status: Option<&str>) -> StoreResult<Vec<Value>> {
        let mut q = vec![("select", PAPER_COLUMNS.to_string()), ("order", "publication_date.desc.nullslast,id.asc".to_string())];
        if let Some(st) = status.filter(|x| !x.is_empty()) {
            q.push(("status", format!("eq.{st}")));
        }
        self.rows("papers", &q).await
    }

    pub async fn links(&self) -> StoreResult<Vec<Value>> {
        self.rows("experiment_papers", &[("select", "experiment_id,paper_id".into()), ("order", "paper_id.asc,experiment_id.asc".into())]).await
    }

    pub async fn conditions(&self) -> StoreResult<Vec<Value>> {
        self.rows("conditions", &[("select", "*".into()), ("order", "id.asc".into())]).await
    }

    pub async fn findings(&self) -> StoreResult<Vec<Value>> {
        self.rows("findings", &[("select", "*".into()), ("order", "id.asc".into())]).await
    }

    /// One paper with its experiments, conditions and findings. `key` is the
    /// paper's id or its NTRS id. Ok(None) when there is no such paper.
    pub async fn paper_bundle(&self, key: &str) -> StoreResult<Option<Value>> {
        let is_uuid = key.len() == 36 && key.chars().all(|c| c.is_ascii_hexdigit() || c == '-');
        let column = if is_uuid { "id" } else { "ntrs_id" };
        let found = self
            .page("papers", &[("select", PAPER_COLUMNS.to_string()), (column, format!("eq.{key}"))], 0)
            .await?;
        let Some(paper) = found.into_iter().next() else { return Ok(None) };
        let id = paper.get("id").and_then(|v| v.as_str()).unwrap_or_default().to_string();
        let by_paper = [("select", "*".to_string()), ("paper_id", format!("eq.{id}")), ("order", "created_at.asc".to_string())];
        let link_query = [("select", "experiment_id".to_string()), ("paper_id", format!("eq.{id}"))];
        let (conditions, findings, links, experiments) = tokio::join!(
            self.rows("conditions", &by_paper),
            self.rows("findings", &by_paper),
            self.rows("experiment_papers", &link_query),
            self.experiments(),
        );
        let linked: Vec<String> = links?
            .iter()
            .filter_map(|l| l.get("experiment_id").and_then(|v| v.as_str()).map(str::to_string))
            .collect();
        let experiments: Vec<Value> = experiments?
            .into_iter()
            .filter(|e| e.get("id").and_then(|v| v.as_str()).is_some_and(|i| linked.iter().any(|l| l == i)))
            .collect();
        Ok(Some(json!({ "paper": paper, "experiments": experiments, "conditions": conditions?, "findings": findings? })))
    }
}
