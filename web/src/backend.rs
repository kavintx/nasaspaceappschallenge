//! Client for the fire-intel research backend (`/api/v1/*`): papers, extracted
//! conditions and findings, and the AI endpoints. Every call sends the bearer
//! token; the token stays on this server and never reaches the browser.

use crate::config::Config;
use serde_json::{json, Value};
use std::time::Duration;

pub struct Backend {
    base: String,
    token: String,
    http: reqwest::Client,
}

pub type ApiResult<T> = Result<T, String>;

const READ_TIMEOUT: Duration = Duration::from_secs(20);
const AI_TIMEOUT: Duration = Duration::from_secs(180);

impl Backend {
    /// Enabled only when `API_AUTH_TOKEN` is set.
    pub fn from_cfg(cfg: &Config) -> Option<Self> {
        let token = cfg.get("API_AUTH_TOKEN")?;
        let base = cfg
            .get("API_BASE_URL")
            .unwrap_or_else(|| "http://127.0.0.1:8080".into())
            .trim_end_matches('/')
            .to_string();
        println!("research backend: {base}");
        Some(Backend { base, token, http: reqwest::Client::new() })
    }

    async fn finish(&self, what: &str, res: Result<reqwest::Response, reqwest::Error>) -> ApiResult<Value> {
        let resp = res.map_err(|e| {
            if e.is_timeout() {
                format!("The research backend took too long to answer ({what}).")
            } else {
                format!("Couldn't reach the research backend at {} ({what}).", self.base)
            }
        })?;
        let status = resp.status();
        if status.as_u16() == 401 {
            return Err("The research backend rejected the access token. Check API_AUTH_TOKEN.".into());
        }
        let body: Value = resp.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            let detail = body.get("error").and_then(|e| e.as_str()).unwrap_or("no details");
            return Err(format!("The research backend returned {} for {what}: {detail}.", status.as_u16()));
        }
        Ok(body)
    }

    async fn get(&self, path: &str, query: &[(&str, String)]) -> ApiResult<Value> {
        let res = self
            .http
            .get(format!("{}/api/v1{path}", self.base))
            .bearer_auth(&self.token)
            .query(query)
            .timeout(READ_TIMEOUT)
            .send()
            .await;
        self.finish(path, res).await
    }

    async fn post(&self, path: &str, body: Value) -> ApiResult<Value> {
        let res = self
            .http
            .post(format!("{}/api/v1{path}", self.base))
            .bearer_auth(&self.token)
            .json(&body)
            .timeout(AI_TIMEOUT)
            .send()
            .await;
        self.finish(path, res).await
    }

    fn list(v: Value) -> Vec<Value> {
        match v {
            Value::Array(a) => a,
            Value::Object(mut o) => ["items", "papers", "experiments", "data"]
                .iter()
                .find_map(|k| match o.remove(*k) {
                    Some(Value::Array(a)) => Some(a),
                    _ => None,
                })
                .unwrap_or_default(),
            _ => vec![],
        }
    }

    pub async fn experiments(&self) -> ApiResult<Vec<Value>> {
        Ok(Self::list(self.get("/experiments", &[]).await?))
    }

    pub async fn papers(&self, status: Option<&str>, limit: usize) -> ApiResult<Vec<Value>> {
        let mut q = vec![("limit", limit.to_string()), ("offset", "0".to_string())];
        if let Some(s) = status.filter(|s| !s.is_empty()) {
            q.push(("status", s.to_string()));
        }
        Ok(Self::list(self.get("/papers", &q).await?))
    }

    pub async fn extractions(&self, paper_id: &str) -> ApiResult<Value> {
        self.get(&format!("/papers/{}/extractions", urlencoding::encode(paper_id)), &[]).await
    }

    pub async fn pipeline_status(&self) -> ApiResult<Value> {
        self.get("/pipeline/status", &[]).await
    }

    pub async fn query(&self, question: &str) -> ApiResult<Value> {
        self.post("/ai/query", json!({ "question": question, "top_k": 8 })).await
    }

    pub async fn summarize(&self, experiment_id: &str) -> ApiResult<Value> {
        self.post(&format!("/ai/summarize/{}", urlencoding::encode(experiment_id)), json!({})).await
    }

    pub async fn compare(&self, experiment_ids: &[String], focus: &str) -> ApiResult<Value> {
        let mut body = json!({ "experiment_ids": experiment_ids });
        if !focus.trim().is_empty() {
            body["focus"] = json!(focus.trim());
        }
        self.post("/ai/compare", body).await
    }

    pub async fn habitat(&self, scenario: Value) -> ApiResult<Value> {
        self.post("/ai/habitat/analyze", scenario).await
    }
}

// ---------- lenient JSON helpers ----------

pub fn s(v: &Value, key: &str) -> Option<String> {
    match v.get(key)? {
        Value::String(x) if !x.trim().is_empty() => Some(x.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

pub fn f(v: &Value, key: &str) -> Option<f64> {
    match v.get(key)? {
        Value::Number(n) => n.as_f64(),
        Value::String(x) => x.trim().parse().ok(),
        _ => None,
    }
}

pub fn flag(v: &Value, key: &str) -> bool {
    v.get(key).and_then(|b| b.as_bool()).unwrap_or(false)
}

/// Only plain https links are ever rendered as links.
pub fn https(v: &Value, key: &str) -> Option<String> {
    s(v, key).filter(|u| u.starts_with("https://"))
}

fn norm(x: &str) -> String {
    x.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_uppercase()
}

/// Find the backend's id for one of our catalog experiments, by acronym.
pub fn match_experiment(list: &[Value], acronym: &str) -> Option<String> {
    let want = norm(acronym);
    let key = |e: &Value| norm(&s(e, "acronym").or_else(|| s(e, "name")).unwrap_or_default());
    list.iter()
        .find(|e| key(e) == want)
        // The backend indexes ACME as one program; our catalog splits it by experiment.
        .or_else(|| list.iter().find(|e| want.starts_with("ACME") && key(e) == "ACME"))
        .and_then(|e| s(e, "id"))
}
