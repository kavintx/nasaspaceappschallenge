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

/// Why a call to one of the newer endpoints (photo scan, material lookup) failed.
pub enum Fail {
    /// The backend answered 404 or 405: this endpoint is not deployed yet.
    NotLive,
    Other(String),
}

const READ_TIMEOUT: Duration = Duration::from_secs(15);
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
        println!("AI backend: {base}");
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
        self.post("/ai/query", json!({ "question": question })).await
    }

    /// `experiment` is a UUID or an acronym such as "SAFFIRE I".
    pub async fn summarize(&self, experiment_id: &str) -> ApiResult<Value> {
        self.post(&format!("/ai/summarize/{}", urlencoding::encode(experiment_id)), json!({})).await
    }

    pub async fn compare(&self, experiments: &[String], focus: &str) -> ApiResult<Value> {
        let mut body = json!({ "experiments": experiments });
        if !focus.trim().is_empty() {
            body["focus"] = json!(focus.trim());
        }
        self.post("/ai/compare", body).await
    }

    pub async fn habitat(&self, scenario: Value) -> ApiResult<Value> {
        self.post("/ai/habitat/analyze", scenario).await
    }

    // ----- photo scan and material lookup -----

    /// Like `finish`, but tells "this endpoint doesn't exist yet" apart from other errors.
    async fn finish_new(&self, what: &str, res: Result<reqwest::Response, reqwest::Error>) -> Result<Value, Fail> {
        match &res {
            Ok(resp) if matches!(resp.status().as_u16(), 404 | 405) => Err(Fail::NotLive),
            _ => self.finish(what, res).await.map_err(Fail::Other),
        }
    }

    /// Sends one photo to the backend's vision model. `mode` is "item" or "label".
    /// The bytes are held only for this request; nothing is written to disk.
    pub async fn scan(&self, image: Vec<u8>, mime: &str, mode: &str) -> Result<Value, Fail> {
        let part = reqwest::multipart::Part::bytes(image)
            .file_name("photo.jpg")
            .mime_str(mime)
            .map_err(|_| Fail::Other("That file type isn't an image.".into()))?;
        let form = reqwest::multipart::Form::new().part("image", part).text("mode", mode.to_string());
        let res = self
            .http
            .post(format!("{}/api/v1/ai/scan", self.base))
            .bearer_auth(&self.token)
            .multipart(form)
            .timeout(AI_TIMEOUT)
            .send()
            .await;
        self.finish_new("/ai/scan", res).await
    }

    /// Asks the backend which tested fuels each material name corresponds to.
    pub async fn match_materials(&self, names: &[String]) -> Result<Value, Fail> {
        let res = self
            .http
            .post(format!("{}/api/v1/materials/match", self.base))
            .bearer_auth(&self.token)
            .json(&json!({ "materials": names }))
            .timeout(READ_TIMEOUT)
            .send()
            .await;
        self.finish_new("/materials/match", res).await
    }

    // ----- diagnostics and pipeline control -----

    /// The health route answers 503 with the same JSON body when the model is down,
    /// so the body is returned whatever the status.
    pub async fn health(&self) -> ApiResult<Value> {
        let resp = self
            .http
            .get(format!("{}/api/v1/ai/health", self.base))
            .bearer_auth(&self.token)
            .timeout(READ_TIMEOUT)
            .send()
            .await
            .map_err(|_| format!("Couldn't reach the research backend at {}.", self.base))?;
        if resp.status().as_u16() == 401 {
            return Err("The research backend rejected the access token. Check API_AUTH_TOKEN.".into());
        }
        resp.json().await.map_err(|_| "The research backend sent an unreadable health reply.".to_string())
    }

    pub async fn test_model(&self, prompt: &str) -> ApiResult<Value> {
        let body = if prompt.trim().is_empty() { json!({}) } else { json!({ "prompt": prompt.trim() }) };
        self.post("/ai/test", body).await
    }

    pub async fn ingest(&self, program: &str, limit_per_program: Option<u32>, process_jobs: Option<u32>) -> ApiResult<Value> {
        let mut body = json!({});
        if !program.trim().is_empty() {
            body["program"] = json!(program.trim());
        }
        if let Some(n) = limit_per_program {
            body["limit_per_program"] = json!(n);
        }
        if let Some(n) = process_jobs {
            body["process_jobs"] = json!(n);
        }
        self.post("/pipeline/ingest", body).await
    }

    pub async fn sheet_import(&self, url: &str) -> ApiResult<Value> {
        self.post("/pipeline/sheet/import", json!({ "url": url })).await
    }

    pub async fn extract(&self, paper_id: &str) -> ApiResult<Value> {
        self.post(&format!("/ai/extract/{}", urlencoding::encode(paper_id)), json!({})).await
    }

    pub async fn extract_batch(&self, limit: u32) -> ApiResult<Value> {
        self.post("/ai/extract/batch", json!({ "limit": limit })).await
    }

    /// Edit or verify one extracted row. `kind` is "conditions" or "findings".
    pub async fn patch_row(&self, kind: &str, id: &str, body: Value) -> ApiResult<Value> {
        let path = format!("/{kind}/{}", urlencoding::encode(id));
        let res = self
            .http
            .patch(format!("{}/api/v1{path}", self.base))
            .bearer_auth(&self.token)
            .json(&body)
            .timeout(READ_TIMEOUT)
            .send()
            .await;
        self.finish(&path, res).await
    }

    /// Opens the streaming chat. The caller relays the event stream to the browser.
    pub async fn chat_stream(&self, body: Value) -> ApiResult<reqwest::Response> {
        let resp = self
            .http
            .post(format!("{}/api/v1/ai/chat", self.base))
            .bearer_auth(&self.token)
            .json(&body)
            .timeout(AI_TIMEOUT)
            .send()
            .await
            .map_err(|_| format!("Couldn't reach the research backend at {}.", self.base))?;
        match resp.status().as_u16() {
            200..=299 => Ok(resp),
            401 => Err("The research backend rejected the access token. Check API_AUTH_TOKEN.".into()),
            code => Err(format!("The research backend returned {code} for the chat.")),
        }
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
