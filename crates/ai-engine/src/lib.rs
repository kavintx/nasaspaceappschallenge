use async_stream::try_stream;
use bytes::Bytes;
use db::Db;
use futures_util::{Stream, StreamExt};
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use shared::{types::ChatTurn, AppConfig, Error, Result};
use std::pin::Pin;
use std::time::{Duration, Instant};
use tracing::{debug, warn};

#[derive(Clone)]
pub struct AiClient {
    http: reqwest::Client,
    config: AppConfig,
    db: Option<Db>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    #[serde(default)]
    pub owned_by: Option<String>,
    #[serde(default)]
    pub root: Option<String>,
    #[serde(default)]
    pub max_model_len: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MessagesResponse {
    pub id: String,
    pub model: String,
    pub text: String,
    pub stop_reason: Option<String>,
    pub input_tokens: i32,
    pub output_tokens: i32,
    pub latency_ms: i32,
}

#[derive(Debug, Clone, Serialize)]
pub struct HealthStatus {
    pub ok: bool,
    pub provider: String,
    pub base_url: String,
    pub configured_model: String,
    pub model_verified: bool,
    pub available_models: Vec<String>,
    pub embeddings_supported: bool,
    pub latency_ms: i32,
    pub error: Option<String>,
}

pub type TextStream = Pin<Box<dyn Stream<Item = Result<String>> + Send>>;

impl AiClient {
    pub fn new(config: AppConfig, db: Option<Db>) -> Result<Self> {
        config.assert_supported();
        let http = reqwest::Client::builder()
            .timeout(config.vllm_timeout)
            .connect_timeout(Duration::from_secs(15))
            .build()?;
        Ok(Self { http, config, db })
    }

    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    fn anthropic_headers(&self) -> Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(
            "anthropic-version",
            HeaderValue::from_static("2023-06-01"),
        );
        if !self.config.vllm_api_key.trim().is_empty() {
            let key = HeaderValue::from_str(self.config.vllm_api_key.trim())
                .map_err(|e| Error::Config(format!("invalid VLLM_API_KEY header: {e}")))?;
            headers.insert("x-api-key", key);
        }
        Ok(headers)
    }

    fn base_url(&self) -> &str {
        self.config.vllm_base_url.trim_end_matches('/')
    }

    /// Query GET {VLLM_BASE_URL}/v1/models.
    pub async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let url = format!("{}/v1/models", self.base_url());
        let started = Instant::now();
        let resp = self
            .http
            .get(&url)
            .headers(self.anthropic_headers()?)
            .send()
            .await
            .map_err(|e| Error::AiUnavailable(format!("vLLM /v1/models unreachable: {e}")))?;

        let status = resp.status().as_u16();
        let latency_ms = started.elapsed().as_millis() as i32;
        let body = resp.text().await.unwrap_or_default();

        if !(200..300).contains(&status) {
            self.audit("models", None, Some(status as i32), false, None, None, latency_ms, Some(&body))
                .await;
            return Err(Error::AiProvider { status, body });
        }

        let parsed: Value = serde_json::from_str(&body)?;
        let models: Vec<ModelInfo> = parsed
            .get("data")
            .cloned()
            .map(serde_json::from_value)
            .transpose()?
            .unwrap_or_default();

        self.audit("models", None, Some(status as i32), true, None, None, latency_ms, None)
            .await;
        Ok(models)
    }

    /// Verify connectivity, model presence, and embeddings route support.
    pub async fn health(&self) -> HealthStatus {
        let started = Instant::now();
        match self.list_models().await {
            Ok(models) => {
                let ids: Vec<String> = models.into_iter().map(|m| m.id).collect();
                let verified = ids.iter().any(|id| id == &self.config.vllm_model);
                let embed_ok = self.probe_embeddings().await;
                HealthStatus {
                    ok: verified,
                    provider: self.config.ai_provider.clone(),
                    base_url: self.base_url().to_string(),
                    configured_model: self.config.vllm_model.clone(),
                    model_verified: verified,
                    available_models: ids,
                    embeddings_supported: embed_ok,
                    latency_ms: started.elapsed().as_millis() as i32,
                    error: if verified {
                        None
                    } else {
                        Some(format!(
                            "configured model '{}' not found on vLLM server",
                            self.config.vllm_model
                        ))
                    },
                }
            }
            Err(e) => HealthStatus {
                ok: false,
                provider: self.config.ai_provider.clone(),
                base_url: self.base_url().to_string(),
                configured_model: self.config.vllm_model.clone(),
                model_verified: false,
                available_models: vec![],
                embeddings_supported: false,
                latency_ms: started.elapsed().as_millis() as i32,
                error: Some(e.to_string()),
            },
        }
    }

    /// Send a non-streaming Anthropic Messages API request to POST {VLLM_BASE_URL}/v1/messages.
    pub async fn complete(
        &self,
        system: Option<&str>,
        messages: &[ChatTurn],
        max_tokens: u32,
        temperature: f32,
    ) -> Result<MessagesResponse> {
        let url = format!("{}/v1/messages", self.base_url());
        let msg_json: Vec<Value> = messages
            .iter()
            .map(|m| json!({ "role": m.role, "content": m.content }))
            .collect();

        let mut payload = json!({
            "model": self.config.vllm_model,
            "max_tokens": max_tokens,
            "temperature": temperature,
            "messages": msg_json,
            "stream": false
        });
        if let Some(sys) = system {
            if !sys.trim().is_empty() {
                payload["system"] = Value::String(sys.to_string());
            }
        }

        let mut last_err: Option<Error> = None;
        let max_attempts = self.config.vllm_max_retries.max(1);

        for attempt in 1..=max_attempts {
            let started = Instant::now();
            let resp = self
                .http
                .post(&url)
                .headers(self.anthropic_headers()?)
                .json(&payload)
                .send()
                .await;

            match resp {
                Ok(r) => {
                    let status = r.status().as_u16();
                    let latency_ms = started.elapsed().as_millis() as i32;
                    let body = r.text().await.unwrap_or_default();

                    if (200..300).contains(&status) {
                        let parsed = parse_anthropic_message(&body, &self.config.vllm_model, latency_ms)?;
                        self.audit(
                            "messages",
                            Some(&self.config.vllm_model),
                            Some(status as i32),
                            true,
                            Some(parsed.input_tokens),
                            Some(parsed.output_tokens),
                            latency_ms,
                            None,
                        )
                        .await;
                        return Ok(parsed);
                    }

                    let err = Error::AiProvider {
                        status,
                        body: body.clone(),
                    };
                    self.audit(
                        "messages",
                        Some(&self.config.vllm_model),
                        Some(status as i32),
                        false,
                        None,
                        None,
                        latency_ms,
                        Some(&body),
                    )
                    .await;

                    if status < 500 && status != 429 {
                        return Err(err);
                    }
                    last_err = Some(err);
                }
                Err(e) => {
                    let latency_ms = started.elapsed().as_millis() as i32;
                    let msg = e.to_string();
                    self.audit(
                        "messages",
                        Some(&self.config.vllm_model),
                        None,
                        false,
                        None,
                        None,
                        latency_ms,
                        Some(&msg),
                    )
                    .await;
                    last_err = Some(Error::AiUnavailable(msg));
                }
            }

            if attempt < max_attempts {
                let backoff = Duration::from_millis(500 * (1 << (attempt - 1)));
                warn!(attempt, ?backoff, "retrying vLLM /v1/messages call");
                tokio::time::sleep(backoff).await;
            }
        }

        Err(last_err.unwrap_or_else(|| Error::AiUnavailable("vLLM request failed".into())))
    }

    /// Stream text deltas from Anthropic Messages SSE (`POST /v1/messages` with `stream: true`).
    pub async fn stream_messages(
        &self,
        system: Option<&str>,
        messages: &[ChatTurn],
        max_tokens: u32,
        temperature: f32,
    ) -> Result<TextStream> {
        let url = format!("{}/v1/messages", self.base_url());
        let msg_json: Vec<Value> = messages
            .iter()
            .map(|m| json!({ "role": m.role, "content": m.content }))
            .collect();

        let mut payload = json!({
            "model": self.config.vllm_model,
            "max_tokens": max_tokens,
            "temperature": temperature,
            "messages": msg_json,
            "stream": true
        });
        if let Some(sys) = system {
            if !sys.trim().is_empty() {
                payload["system"] = Value::String(sys.to_string());
            }
        }

        let resp = self
            .http
            .post(&url)
            .headers(self.anthropic_headers()?)
            .json(&payload)
            .send()
            .await
            .map_err(|e| Error::AiUnavailable(format!("vLLM stream connection failed: {e}")))?;

        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::AiProvider { status, body });
        }

        let byte_stream = resp.bytes_stream();
        let stream = try_stream! {
            let mut buf = String::new();
            futures_util::pin_mut!(byte_stream);
            while let Some(chunk_res) = byte_stream.next().await {
                let bytes: Bytes = chunk_res.map_err(Error::Http)?;
                buf.push_str(&String::from_utf8_lossy(&bytes));
                while let Some(pos) = buf.find('\n') {
                    let line = buf[..pos].trim_end_matches('\r').to_string();
                    buf.drain(..=pos);
                    if let Some(data) = line.strip_prefix("data:") {
                        let data = data.trim();
                        if data.is_empty() || data == "[DONE]" {
                            continue;
                        }
                        if let Ok(ev) = serde_json::from_str::<Value>(data) {
                            if let Some(delta_text) = extract_sse_text_delta(&ev) {
                                if !delta_text.is_empty() {
                                    yield delta_text;
                                }
                            }
                        }
                    }
                }
            }
        };

        Ok(Box::pin(stream))
    }

    /// Attempt to generate embeddings via {VLLM_BASE_URL}/v1/embeddings.
    /// Returns Ok(None) if the server does not serve an embedding model.
    pub async fn embed(&self, texts: &[String]) -> Result<Option<Vec<Vec<f32>>>> {
        if texts.is_empty() {
            return Ok(Some(vec![]));
        }
        let url = format!("{}/v1/embeddings", self.base_url());
        let payload = json!({
            "model": self.config.vllm_embed_model,
            "input": texts
        });
        let started = Instant::now();
        let resp = match self
            .http
            .post(&url)
            .headers(self.anthropic_headers()?)
            .json(&payload)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                debug!(error = %e, "embeddings endpoint unreachable");
                return Ok(None);
            }
        };

        let status = resp.status().as_u16();
        let latency_ms = started.elapsed().as_millis() as i32;
        let body = resp.text().await.unwrap_or_default();

        if !(200..300).contains(&status) {
            self.audit(
                "embeddings",
                Some(&self.config.vllm_embed_model),
                Some(status as i32),
                false,
                None,
                None,
                latency_ms,
                Some(&body),
            )
            .await;
            return Ok(None);
        }

        let parsed: Value = serde_json::from_str(&body)?;
        let data = match parsed.get("data").and_then(|d| d.as_array()) {
            Some(arr) => arr,
            None => return Ok(None),
        };

        let mut out = Vec::with_capacity(data.len());
        for item in data {
            let vec_vals = item
                .get("embedding")
                .and_then(|e| e.as_array())
                .ok_or_else(|| Error::Pipeline("missing embedding array in response".into()))?;
            let mut v = Vec::with_capacity(vec_vals.len());
            for num in vec_vals {
                v.push(num.as_f64().unwrap_or(0.0) as f32);
            }
            if v.len() != self.config.vllm_embed_dim {
                return Err(Error::Config(format!(
                    "embedding dimension mismatch: configured VLLM_EMBED_DIM={}, server returned {}",
                    self.config.vllm_embed_dim,
                    v.len()
                )));
            }
            out.push(v);
        }

        self.audit(
            "embeddings",
            Some(&self.config.vllm_embed_model),
            Some(status as i32),
            true,
            None,
            None,
            latency_ms,
            None,
        )
        .await;
        Ok(Some(out))
    }

    async fn probe_embeddings(&self) -> bool {
        matches!(self.embed(&["probe".to_string()]).await, Ok(Some(_)))
    }

    #[allow(clippy::too_many_arguments)]
    async fn audit(
        &self,
        route: &str,
        model: Option<&str>,
        http_status: Option<i32>,
        ok: bool,
        in_tokens: Option<i32>,
        out_tokens: Option<i32>,
        latency_ms: i32,
        error: Option<&str>,
    ) {
        if let Some(db) = &self.db {
            let _ = db
                .log_ai_call(
                    route,
                    model,
                    http_status,
                    ok,
                    in_tokens,
                    out_tokens,
                    latency_ms,
                    error,
                )
                .await;
        }
    }
}

fn parse_anthropic_message(
    body: &str,
    default_model: &str,
    latency_ms: i32,
) -> Result<MessagesResponse> {
    let v: Value = serde_json::from_str(body)?;
    let id = v
        .get("id")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let model = v
        .get("model")
        .and_then(|x| x.as_str())
        .unwrap_or(default_model)
        .to_string();
    let stop_reason = v
        .get("stop_reason")
        .and_then(|x| x.as_str())
        .map(String::from);

    let mut text_parts = Vec::new();
    if let Some(blocks) = v.get("content").and_then(|c| c.as_array()) {
        for b in blocks {
            if b.get("type").and_then(|t| t.as_str()) == Some("text") {
                if let Some(t) = b.get("text").and_then(|t| t.as_str()) {
                    text_parts.push(t.to_string());
                }
            }
        }
    }

    let usage = v.get("usage");
    let input_tokens = usage
        .and_then(|u| u.get("input_tokens"))
        .and_then(|n| n.as_i64())
        .unwrap_or(0) as i32;
    let output_tokens = usage
        .and_then(|u| u.get("output_tokens"))
        .and_then(|n| n.as_i64())
        .unwrap_or(0) as i32;

    Ok(MessagesResponse {
        id,
        model,
        text: text_parts.join("").trim().to_string(),
        stop_reason,
        input_tokens,
        output_tokens,
        latency_ms,
    })
}

fn extract_sse_text_delta(ev: &Value) -> Option<String> {
    if ev.get("type").and_then(|t| t.as_str()) == Some("content_block_delta") {
        let delta = ev.get("delta")?;
        if delta.get("type").and_then(|t| t.as_str()) == Some("text_delta") {
            return delta.get("text").and_then(|t| t.as_str()).map(String::from);
        }
    }
    None
}

pub fn extract_json_block(raw: &str) -> &str {
    let trimmed = raw.trim();
    if let Some(start) = trimmed.find("```json") {
        let after = &trimmed[start + 7..];
        if let Some(end) = after.find("```") {
            return after[..end].trim();
        }
    }
    if let Some(start) = trimmed.find("```") {
        let after = &trimmed[start + 3..];
        if let Some(end) = after.find("```") {
            return after[..end].trim();
        }
    }
    if let (Some(l), Some(r)) = (trimmed.find('{'), trimmed.rfind('}')) {
        if r >= l {
            return &trimmed[l..=r];
        }
    }
    trimmed
}
