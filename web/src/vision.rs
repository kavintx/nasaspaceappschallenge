//! Photo identification for Burn Scan: one prompt, one JSON shape, two providers.
//!
//!   VISION_PROVIDER = "backend" | "ollama" | "anthropic" | "openai"   (default: ollama)
//!   VISION_FALLBACK = "anthropic" | "openai" | "none"     (default: none)
//!
//! - backend: the research backend's POST /api/v1/ai/scan (fire-intel). Uses the same
//!   API_BASE_URL + API_AUTH_TOKEN pair the rest of the site already calls, and the
//!   backend's own vision model. The reply already matches the /ai/scan shape below.
//! - ollama: a local vision model. OLLAMA_BASE_URL (default http://127.0.0.1:11434),
//!   OLLAMA_VISION_MODEL (default llama3.2-vision).
//! - anthropic: the Anthropic Messages API with a base64 image block. Key from
//!   ANTHROPIC_API_KEY, model from ANTHROPIC_MODEL (default claude-haiku-5-5). A key that
//!   isn't tied to one workspace also needs ANTHROPIC_WORKSPACE_ID.
//! - openai: the OpenAI chat-completions API with an image data URL (any endpoint that
//!   speaks the same protocol works — set OPENAI_BASE_URL). Key from OPENAI_API_KEY,
//!   model from OPENAI_VISION_MODEL (default gpt-4o-mini).
//!
//! If the first provider errors, takes longer than 20 s, or answers with JSON that
//! doesn't match the schema, the fallback is tried once when one is set. The reply's
//! "model" field names the provider that actually answered.
//!
//! The key and the image are never logged, and the image is never written to disk.
//! This lives in the site only until the backend ships its own /ai/scan.

use crate::config::Config;
use base64::Engine;
use serde_json::{json, Value};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(20);
/// The backend queues a bigger model than the local one; give it room.
const SCAN_TIMEOUT: Duration = Duration::from_secs(90);
const ANTHROPIC_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2026-10-21";

#[derive(Clone, Copy, PartialEq)]
enum Provider {
    Ollama,
    Anthropic,
    OpenAI,
    Backend,
}

pub enum VisionError {
    /// No provider could be reached or is set up: shown as "Scanner not connected".
    NotConnected,
    /// A provider answered badly or refused. Safe to show; never contains the key.
    Failed(String),
}

pub struct Vision {
    primary: Provider,
    fallback: Option<Provider>,
    http: reqwest::Client,
    ollama_url: String,
    ollama_model: String,
    anthropic_key: Option<String>,
    anthropic_model: String,
    /// Sent as `anthropic-workspace-id` when set. Needed by keys not scoped to a workspace.
    anthropic_workspace: Option<String>,
    openai_key: Option<String>,
    openai_model: String,
    /// Chat-completions root; any OpenAI-compatible server works.
    openai_url: String,
    /// Provider::Backend target: the same base + token the rest of the site uses.
    api_base: Option<String>,
    api_token: Option<String>,
}

/// What one provider attempt came to.
enum Attempt {
    Ok(Value),
    /// Missing key, or nothing listening at the Ollama address.
    NotConfigured,
    Failed(String),
}

impl Vision {
    pub fn from_cfg(cfg: &Config) -> Self {
        let named = |key: &str| cfg.get_file_first(key).map(|v| v.trim().to_lowercase());
        let primary = match named("VISION_PROVIDER").as_deref() {
            Some("anthropic") => Provider::Anthropic,
            Some("openai") => Provider::OpenAI,
            Some("backend") => Provider::Backend,
            _ => Provider::Ollama,
        };
        let fallback = match named("VISION_FALLBACK").as_deref() {
            Some("anthropic") if primary != Provider::Anthropic => Some(Provider::Anthropic),
            Some("openai") if primary != Provider::OpenAI => Some(Provider::OpenAI),
            _ => None,
        };
        let vision = Vision {
            primary,
            fallback,
            http: reqwest::Client::new(),
            ollama_url: cfg.get_file_first("OLLAMA_BASE_URL").unwrap_or_else(|| "http://127.0.0.1:11434".into()).trim_end_matches('/').to_string(),
            ollama_model: cfg.get_file_first("OLLAMA_VISION_MODEL").unwrap_or_else(|| "llama3.2-vision".into()),
            anthropic_key: cfg.get_file_first("ANTHROPIC_API_KEY"),
            anthropic_model: cfg.get_file_first("ANTHROPIC_MODEL").unwrap_or_else(|| "claude-haiku-5-5".into()),
            anthropic_workspace: cfg.get_file_first("ANTHROPIC_WORKSPACE_ID"),
            openai_key: cfg.get_file_first("OPENAI_API_KEY"),
            openai_model: cfg.get_file_first("OPENAI_VISION_MODEL").unwrap_or_else(|| "gpt-4o-mini".into()),
            openai_url: cfg.get_file_first("OPENAI_BASE_URL").unwrap_or_else(|| "https://api.openai.com/v1".into()).trim_end_matches('/').to_string(),
            // Same rule as backend.rs: the backend is enabled by the token.
            api_token: cfg.get_file_first("API_AUTH_TOKEN"),
            api_base: cfg
                .get("API_AUTH_TOKEN")
                .map(|_| cfg.get_file_first("API_BASE_URL").unwrap_or_else(|| "http://127.0.0.1:8080".into()).trim_end_matches('/').to_string()),
        };
        // Names only. The key itself is never printed.
        println!(
            "photo scanner: {}{}",
            vision.label(primary),
            fallback.map(|f| format!(", fallback {}", vision.label(f))).unwrap_or_default()
        );
        if (primary == Provider::Anthropic || fallback == Some(Provider::Anthropic)) && vision.anthropic_key.is_none() {
            println!("photo scanner: anthropic is not configured (ANTHROPIC_API_KEY is not set in web/.env).");
        }
        if (primary == Provider::OpenAI || fallback == Some(Provider::OpenAI)) && vision.openai_key.is_none() {
            println!("photo scanner: openai is not configured (OPENAI_API_KEY is not set in web/.env).");
        }
        if (primary == Provider::Backend || fallback == Some(Provider::Backend)) && vision.api_token.is_none() {
            println!("photo scanner: research backend is not configured (API_AUTH_TOKEN is not set in web/.env).");
        }
        vision
    }

    fn label(&self, p: Provider) -> String {
        match p {
            Provider::Ollama => format!("ollama:{}", self.ollama_model),
            Provider::Anthropic => format!("anthropic:{}", self.anthropic_model),
            Provider::OpenAI => format!("openai:{}", self.openai_model),
            Provider::Backend => "research backend".to_string(),
        }
    }

    /// One line for the settings page: which providers are armed. Names only.
    pub fn status(&self) -> String {
        match self.fallback {
            Some(f) => format!("{} (fallback {})", self.label(self.primary), self.label(f)),
            None => self.label(self.primary).clone(),
        }
    }

    /// The provider names, for the settings page's summary after a save.
    pub fn provider_name(&self) -> String {
        self.label(self.primary)
    }

    /// False only when the page can already tell no provider will answer.
    pub fn may_work(&self) -> bool {
        let usable = |p: Provider| match p {
            Provider::Ollama => true,
            Provider::Anthropic => self.anthropic_key.is_some(),
            Provider::OpenAI => self.openai_key.is_some(),
            Provider::Backend => self.api_token.is_some(),
        };
        usable(self.primary) || self.fallback.is_some_and(usable)
    }

    /// Identify one photo. `mode` is "item" or "label". Returns the schema's JSON
    /// with "model" set to the provider that answered.
    pub async fn scan(&self, image: &[u8], mime: &str, mode: &str) -> Result<Value, VisionError> {
        let data = base64::engine::general_purpose::STANDARD.encode(image);
        let first = self.attempt(self.primary, &data, mime, mode).await;
        let (provider, outcome) = match (first, self.fallback) {
            (Attempt::Ok(v), _) => (self.primary, Attempt::Ok(v)),
            (failed, Some(fallback)) => {
                if let Attempt::Failed(why) = &failed {
                    eprintln!("photo scanner: {} failed ({why}); trying {}", self.label(self.primary), self.label(fallback));
                }
                match self.attempt(fallback, &data, mime, mode).await {
                    // When the fallback isn't set up either, the first provider's problem is the one to report.
                    Attempt::NotConfigured => (self.primary, failed),
                    other => (fallback, other),
                }
            }
            (failed, None) => (self.primary, failed),
        };
        match outcome {
            Attempt::Ok(mut v) => {
                // The backend already names the model that answered ("vllm:…"); keep it.
                if provider != Provider::Backend || v.get("model").and_then(Value::as_str).is_none() {
                    v["model"] = json!(self.label(provider));
                }
                Ok(v)
            }
            Attempt::NotConfigured => Err(VisionError::NotConnected),
            Attempt::Failed(why) => Err(VisionError::Failed(why)),
        }
    }

    async fn attempt(&self, provider: Provider, data: &str, mime: &str, mode: &str) -> Attempt {
        let raw = match provider {
            Provider::Ollama => self.ask_ollama(data, mode).await,
            Provider::Anthropic => self.ask_anthropic(data, mime, mode).await,
            Provider::OpenAI => self.ask_openai(data, mime, mode).await,
            Provider::Backend => self.ask_backend(data, mime, mode).await,
        };
        match raw {
            Ok(text) => match parse(&text) {
                Some(v) => Attempt::Ok(v),
                None => Attempt::Failed(format!("{} answered in a form the scanner can't read.", self.label(provider))),
            },
            Err(attempt) => attempt,
        }
    }

    /// The research backend's /api/v1/ai/scan, called server-to-server: raw photo
    /// bytes with an image content type, bearer token like every other backend call.
    /// Its reply is already schema JSON, so it only passes through `parse`'s checks.
    async fn ask_backend(&self, data: &str, mime: &str, mode: &str) -> Result<String, Attempt> {
        let base = self.api_base.as_deref().ok_or(Attempt::NotConfigured)?;
        let token = self.api_token.as_deref().ok_or(Attempt::NotConfigured)?;
        let image = base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|_| Attempt::Failed("The scanner could not read the photo.".into()))?;
        let resp = self
            .http
            .post(format!("{base}/api/v1/ai/scan?mode={mode}"))
            .bearer_auth(token)
            .header(reqwest::header::CONTENT_TYPE, mime)
            .body(image)
            .timeout(SCAN_TIMEOUT)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    Attempt::Failed(format!(
                        "The research backend took longer than {} seconds to read the photo.",
                        SCAN_TIMEOUT.as_secs()
                    ))
                } else {
                    Attempt::Failed(format!("Couldn't reach the research backend at {base} for photo scanning."))
                }
            })?;
        let status = resp.status();
        let reply: Value = resp.json().await.unwrap_or(Value::Null);
        if status.as_u16() == 401 {
            return Err(Attempt::Failed("The research backend rejected the access token. Check API_AUTH_TOKEN.".into()));
        }
        if !status.is_success() {
            let why = reply
                .get("error")
                .or_else(|| reply.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("no details");
            return Err(Attempt::Failed(format!(
                "The research backend refused the photo ({}): {why}.",
                status.as_u16()
            )));
        }
        Ok(reply.to_string())
    }

    async fn ask_ollama(&self, data: &str, mode: &str) -> Result<String, Attempt> {
        let body = json!({
            "model": self.ollama_model,
            "stream": false,
            "format": schema(),
            "options": { "temperature": 0 },
            "messages": [{ "role": "user", "content": prompt(mode), "images": [data] }],
        });
        let resp = self
            .http
            .post(format!("{}/api/chat", self.ollama_url))
            .json(&body)
            .timeout(TIMEOUT)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    Attempt::Failed(format!("The local vision model took longer than {} seconds.", TIMEOUT.as_secs()))
                } else {
                    Attempt::NotConfigured
                }
            })?;
        let status = resp.status();
        let reply: Value = resp.json().await.unwrap_or(Value::Null);
        if status.as_u16() == 404 {
            return Err(Attempt::Failed(format!(
                "The local vision model \"{}\" isn't installed in Ollama. Run: ollama pull {}",
                self.ollama_model, self.ollama_model
            )));
        }
        if !status.is_success() {
            let detail = reply.get("error").and_then(|e| e.as_str()).unwrap_or("no details");
            return Err(Attempt::Failed(format!("Ollama returned {}: {detail}", status.as_u16())));
        }
        reply
            .pointer("/message/content")
            .and_then(|c| c.as_str())
            .map(str::to_string)
            .ok_or_else(|| Attempt::Failed("Ollama sent an empty reply.".into()))
    }

    /// OpenAI chat completions with the photo as an image data URL, and the answer
    /// forced into our schema via structured outputs. The same protocol covers many
    /// other servers — OPENAI_BASE_URL points at whichever one is configured.
    async fn ask_openai(&self, data: &str, mime: &str, mode: &str) -> Result<String, Attempt> {
        let key = self.openai_key.as_deref().ok_or(Attempt::NotConfigured)?;
        let body = json!({
            "model": self.openai_model,
            "max_tokens": 4000,
            "response_format": { "type": "json_schema", "json_schema": { "name": "burn_scan", "schema": schema() } },
            "messages": [{ "role": "user", "content": [
                { "type": "text", "text": prompt(mode) },
                { "type": "image_url", "image_url": { "url": format!("data:{mime};base64,{data}") } },
            ] }],
        });
        let resp = self
            .http
            .post(format!("{}/chat/completions", self.openai_url))
            .bearer_auth(key)
            .json(&body)
            .timeout(TIMEOUT)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    Attempt::Failed(format!("OpenAI took longer than {} seconds.", TIMEOUT.as_secs()))
                } else {
                    Attempt::Failed(format!("Couldn't reach the OpenAI endpoint at {}.", self.openai_url))
                }
            })?;
        let status = resp.status();
        let reply: Value = resp.json().await.unwrap_or(Value::Null);
        if status.as_u16() == 401 {
            return Err(Attempt::Failed("OpenAI rejected the API key. Check OPENAI_API_KEY in web/.env.".into()));
        }
        if !status.is_success() {
            let why = reply
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("no details");
            return Err(Attempt::Failed(format!("OpenAI returned {}: {why}.", status.as_u16())));
        }
        reply
            .pointer("/choices/0/message/content")
            .and_then(|c| c.as_str())
            .map(str::to_string)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Attempt::Failed("OpenAI sent an empty reply.".into()))
    }

    async fn ask_anthropic(&self, data: &str, mime: &str, mode: &str) -> Result<String, Attempt> {
        let Some(key) = &self.anthropic_key else { return Err(Attempt::NotConfigured) };
        // Image first, then the text, as the vision guide recommends.
        let body = json!({
            "model": self.anthropic_model,
            "max_tokens": 4000,
            "output_config": {
                "effort": "low",
                "format": { "type": "json_schema", "schema": schema() },
            },
            "messages": [{
                "role": "user",
                "content": [
                    { "type": "image", "source": { "type": "base64", "media_type": mime, "data": data } },
                    { "type": "text", "text": prompt(mode) },
                ],
            }],
        });
        let mut request = self.http.post(ANTHROPIC_URL).header("x-api-key", key).header("anthropic-version", ANTHROPIC_VERSION);
        if let Some(workspace) = &self.anthropic_workspace {
            request = request.header("anthropic-workspace-id", workspace);
        }
        let resp = request
            .json(&body)
            .timeout(TIMEOUT)
            .send()
            .await
            .map_err(|e| {
                Attempt::Failed(if e.is_timeout() {
                    format!("Anthropic took longer than {} seconds.", TIMEOUT.as_secs())
                } else {
                    "Couldn't reach the Anthropic API.".to_string()
                })
            })?;
        let status = resp.status().as_u16();
        let reply: Value = resp.json().await.unwrap_or(Value::Null);
        if !(200..300).contains(&status) {
            // The API's own error text describes the request; it never echoes the key.
            let detail = reply.pointer("/error/message").and_then(|m| m.as_str()).unwrap_or("no details");
            return Err(Attempt::Failed(match status {
                401 | 403 => "Anthropic rejected the API key. Check ANTHROPIC_API_KEY in web/.env.".to_string(),
                404 => format!("Anthropic doesn't know the model \"{}\". Check ANTHROPIC_MODEL.", self.anthropic_model),
                400 if detail.contains("anthropic-workspace-id") => {
                    "This Anthropic key isn't tied to a workspace. Set ANTHROPIC_WORKSPACE_ID in web/.env.".to_string()
                }
                429 => "Anthropic is rate-limiting this key. Try again in a moment.".to_string(),
                _ => format!("Anthropic returned {status}: {detail}"),
            }));
        }
        match reply.get("stop_reason").and_then(|r| r.as_str()) {
            Some("refusal") => return Err(Attempt::Failed("Anthropic declined to describe this photo.".into())),
            Some("max_tokens") => return Err(Attempt::Failed("Anthropic's answer was cut off.".into())),
            _ => {}
        }
        reply
            .get("content")
            .and_then(|c| c.as_array())
            .and_then(|blocks| blocks.iter().find(|b| b.get("type").and_then(|t| t.as_str()) == Some("text")))
            .and_then(|b| b.get("text"))
            .and_then(|t| t.as_str())
            .map(str::to_string)
            .ok_or_else(|| Attempt::Failed("Anthropic sent an empty reply.".into()))
    }
}

/// The same instructions for both providers.
fn prompt(mode: &str) -> String {
    let task = if mode == "label" {
        "The photo should show a care label, fibre-content tag or packaging material mark. Read the printed \
         material content and report only what the label states, with the stated percentages as shares. \
         Use one part named \"label\". If no material text is readable, set unclear to true."
    } else {
        "The photo should show one everyday item. Name the item, list its visible parts, and give the most \
         likely material or materials of each part with a rough percentage share of that part (null if you \
         can't estimate one)."
    };
    format!(
        "You identify what a photographed item is made of, for a tool that compares materials with NASA \
         spacecraft fire tests.\n\n{task}\n\n\
         Rules:\n\
         - Use plain, common material names: cotton, polyester, nylon, wool, leather, paper, cardboard, wood, \
         glass, steel, aluminium, copper, rubber, silicone, PVC, polyethylene, polypropylene, PET, acrylic, \
         polycarbonate, ABS, polyurethane foam. Use the specific name when you can tell and \"plastic\" or \
         \"metal\" when you can't.\n\
         - Give one material per part: your single best reading. List two or more only when the part really is \
         a blend or mix (for example a cotton and polyester fabric, or a label that lists several fibres). \
         Never add a generic name such as \"plastic\" next to a specific guess for the same part.\n\
         - share is the rough percentage of that part made of that material. Shares within one part add up \
         to 100 or less. A part with one material has share 100.\n\
         - confidence is a number from 0 to 1 for how sure you are from this photo alone. A photo rarely proves \
         a material: plastics and synthetic fabrics that look alike get 0.5 or lower. Do not guess high.\n\
         - The top-level confidence is how sure you are about what the item is.\n\
         - If the photo is too blurry, too dark or too far away to tell what anything is made of, set unclear \
         to true, say why in message in one short sentence, and return an empty parts list. A close-up that \
         shows only part of an item is fine: name what you can see and report its material.\n\
         - summary is one or two plain sentences, at most 40 words, saying what the photo shows and what it \
         appears to be made of, with your uncertainty in plain words (\"looks like\", \"probably\"). Describe \
         only what is visible. Say nothing about fire, burning, safety or NASA: you have no test data. \
         null when unclear is true.\n\
         - Do not describe or identify any person in the photo.\n\n\
         Reply with one JSON object and nothing else, in exactly this shape:\n\
         {{\"item\": string or null, \"confidence\": number, \"unclear\": boolean, \"message\": string or null, \
         \"summary\": string or null, \
         \"parts\": [{{\"name\": string or null, \"materials\": [{{\"material\": string, \"share\": number or null, \
         \"confidence\": number}}]}}]}}"
    )
}

/// The /ai/scan reply shape, as JSON Schema. Sent to both providers.
fn schema() -> Value {
    let nullable = |kind: &str| json!({ "anyOf": [{ "type": kind }, { "type": "null" }] });
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["item", "confidence", "unclear", "message", "summary", "parts"],
        "properties": {
            "item": nullable("string"),
            "confidence": { "type": "number", "description": "0 to 1" },
            "unclear": { "type": "boolean" },
            "message": nullable("string"),
            "summary": nullable("string"),
            "parts": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["name", "materials"],
                    "properties": {
                        "name": nullable("string"),
                        "materials": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "additionalProperties": false,
                                "required": ["material", "share", "confidence"],
                                "properties": {
                                    "material": { "type": "string" },
                                    "share": { "anyOf": [{ "type": "number", "description": "percent of this part, 0 to 100" }, { "type": "null" }] },
                                    "confidence": { "type": "number", "description": "0 to 1" },
                                },
                            },
                        },
                    },
                },
            },
        },
    })
}

/// The provider's text as schema-shaped JSON, or None when it doesn't match.
fn parse(text: &str) -> Option<Value> {
    // Some local models wrap JSON in a code fence despite the instructions.
    let trimmed = text.trim().trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim();
    let v: Value = serde_json::from_str(trimmed).ok()?;
    let unit = |x: &Value| x.as_f64().is_some_and(|n| (0.0..=1.0).contains(&n));
    let text_or_null = |x: Option<&Value>| x.is_some_and(|x| x.is_string() || x.is_null());
    let ok = v.is_object()
        && text_or_null(v.get("item"))
        && text_or_null(v.get("message"))
        && text_or_null(v.get("summary"))
        && v.get("unclear").is_some_and(|u| u.is_boolean())
        && v.get("confidence").is_some_and(unit)
        && v.get("parts")?.as_array()?.iter().all(|p| {
            text_or_null(p.get("name"))
                && p.get("materials").and_then(|m| m.as_array()).is_some_and(|list| {
                    list.iter().all(|m| {
                        m.get("material").and_then(|n| n.as_str()).is_some_and(|n| !n.trim().is_empty())
                            && m.get("confidence").is_some_and(unit)
                            && m.get("share").is_some_and(|s| s.is_null() || s.as_f64().is_some_and(|n| (0.0..=100.0).contains(&n)))
                    })
                })
        });
    ok.then_some(v)
}
