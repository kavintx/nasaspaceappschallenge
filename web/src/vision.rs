//! Photo identification for Burn Scan: one prompt, one JSON shape, three providers.
//! The scanner always runs on the machine's own keys — the research backend is a
//! database and answer service, never the thing that reads a photo.
//!
//!   VISION_PROVIDER = "ollama" | "anthropic" | "openai"   (default: auto-pick from the keys present)
//!   VISION_FALLBACK = "anthropic" | "openai" | "none"     (default: none)
//!
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

use crate::config::Config;
use base64::Engine;
use serde_json::{json, Value};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(20);
const ANTHROPIC_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2026-10-21";

#[derive(Clone, Copy, PartialEq)]
enum Provider {
    Ollama,
    Anthropic,
    OpenAI,
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
        // Auto-pick so nobody has to run ollama just to scan: whichever key is
        // present becomes the scanner. Set VISION_PROVIDER to override.
        let auto = || match (cfg.get_file_first("ANTHROPIC_API_KEY"), cfg.get_file_first("OPENAI_API_KEY")) {
            (Some(_), _) => Provider::Anthropic,
            (None, Some(_)) => Provider::OpenAI,
            _ => Provider::Ollama,
        };
        let primary = match named("VISION_PROVIDER").as_deref() {
            Some("anthropic") => Provider::Anthropic,
            Some("openai") => Provider::OpenAI,
            Some("ollama") => Provider::Ollama,
            _ => auto(),
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
            anthropic_key: cfg.get_file_first("ANTHROPIC_API_KEY").map(|k| k.trim().trim_matches('"').to_string()).filter(|k| !k.is_empty()),
            anthropic_model: cfg.get_file_first("ANTHROPIC_MODEL").unwrap_or_else(|| "claude-haiku-5-5".into()),
            anthropic_workspace: cfg.get_file_first("ANTHROPIC_WORKSPACE_ID"),
            openai_key: cfg.get_file_first("OPENAI_API_KEY"),
            openai_model: cfg.get_file_first("OPENAI_VISION_MODEL").unwrap_or_else(|| "gpt-4o-mini".into()),
            openai_url: cfg.get_file_first("OPENAI_BASE_URL").unwrap_or_else(|| "https://api.openai.com/v1".into()).trim_end_matches('/').to_string(),
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
        vision
    }

    fn label(&self, p: Provider) -> String {
        match p {
            Provider::Ollama => format!("ollama:{}", self.ollama_model),
            Provider::Anthropic => format!("anthropic:{}", self.anthropic_model),
            Provider::OpenAI => format!("openai:{}", self.openai_model),
        }
    }

    /// True when a key on this machine can drive the scanner — the Scan page
    /// shows its unlock box when this is false.
    pub fn has_own_key(&self) -> bool {
        self.anthropic_key.is_some() || self.openai_key.is_some()
    }

    /// The live provider plan for the research chat: primary, then fallback.
    pub fn chat_cfg(&self) -> ChatCfg {
        let mut providers = vec![self.chat_provider(self.primary)];
        if let Some(f) = self.fallback.filter(|f| *f != self.primary) {
            providers.push(self.chat_provider(f));
        }
        ChatCfg { http: self.http.clone(), providers }
    }

    fn chat_provider(&self, p: Provider) -> ChatProvider {
        match p {
            Provider::Ollama => ChatProvider {
                kind: ChatKind::Ollama,
                url: self.ollama_url.clone(),
                model: self.ollama_model.clone(),
                key: None,
                workspace: None,
            },
            Provider::Anthropic => ChatProvider {
                kind: ChatKind::Anthropic,
                url: String::new(),
                model: self.anthropic_model.clone(),
                key: self.anthropic_key.clone(),
                workspace: self.anthropic_workspace.clone(),
            },
            Provider::OpenAI => ChatProvider {
                kind: ChatKind::OpenAI,
                url: self.openai_url.clone(),
                model: self.openai_model.clone(),
                key: self.openai_key.clone(),
                workspace: None,
            },
        }
    }

    /// False only when the page can already tell no provider will answer.
    pub fn may_work(&self) -> bool {
        let usable = |p: Provider| match p {
            Provider::Ollama => true,
            Provider::Anthropic => self.anthropic_key.is_some(),
            Provider::OpenAI => self.openai_key.is_some(),
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
                v["model"] = json!(self.label(provider));
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
        };
        match raw {
            Ok(text) => match parse(&text) {
                Some(v) => Attempt::Ok(v),
                None => Attempt::Failed(format!("{} answered in a form the scanner can't read.", self.label(provider))),
            },
            Err(attempt) => attempt,
        }
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
                401 => "Anthropic rejected the API key. Check ANTHROPIC_API_KEY in web/.env.".to_string(),
                // Allowed key, refused request: say what Anthropic said, so it can be fixed.
                403 => format!("Anthropic refused the request (403): {detail}"),
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

// ---------- text chat (Ask the Data) ----------
//
// The same providers and keys that read photos also drive the research chat.
// This snapshot hands an owned, cheap-to-move plan to the chat streaming task.

#[derive(Clone, Copy, PartialEq)]
pub enum ChatKind {
    Anthropic,
    OpenAI,
    Ollama,
}

/// One parsed line from a provider's stream.
pub enum ChatPiece {
    Delta(String),
    Done,
    Err(String),
    Skip,
}

impl ChatKind {
    /// Turn one raw line of the provider's stream into a chat event.
    pub fn piece(self, line: &str) -> ChatPiece {
        match self {
            ChatKind::Ollama => {
                let Ok(v) = serde_json::from_str::<Value>(line) else { return ChatPiece::Skip };
                if v.get("done").and_then(Value::as_bool).unwrap_or(false) {
                    return ChatPiece::Done;
                }
                match v.pointer("/message/content").and_then(Value::as_str) {
                    Some(t) if !t.is_empty() => ChatPiece::Delta(t.to_string()),
                    _ => ChatPiece::Skip,
                }
            }
            ChatKind::Anthropic | ChatKind::OpenAI => {
                let Some(data) = line.trim().strip_prefix("data:").map(str::trim) else { return ChatPiece::Skip };
                if data.is_empty() {
                    return ChatPiece::Skip;
                }
                if data == "[DONE]" {
                    return ChatPiece::Done;
                }
                let Ok(v) = serde_json::from_str::<Value>(data) else { return ChatPiece::Skip };
                match self {
                    ChatKind::Anthropic => match v.get("type").and_then(Value::as_str) {
                        Some("content_block_delta") => v
                            .pointer("/delta/text")
                            .and_then(Value::as_str)
                            .filter(|t| !t.is_empty())
                            .map(|t| ChatPiece::Delta(t.to_string()))
                            .unwrap_or(ChatPiece::Skip),
                        Some("message_stop") => ChatPiece::Done,
                        Some("error") => ChatPiece::Err(
                            v.pointer("/error/message").and_then(Value::as_str).unwrap_or("the AI service reported an error").to_string(),
                        ),
                        _ => ChatPiece::Skip,
                    },
                    _ => v
                        .pointer("/choices/0/delta/content")
                        .and_then(Value::as_str)
                        .filter(|t| !t.is_empty())
                        .map(|t| ChatPiece::Delta(t.to_string()))
                        .unwrap_or(ChatPiece::Skip),
                }
            }
        }
    }
}

pub enum ChatOpenError {
    /// No provider on this machine can answer — the Scan page's box is the fix.
    NotConnected,
    /// A provider was reached but refused or answered badly. Safe to show.
    Failed(String),
}

struct ChatProvider {
    kind: ChatKind,
    url: String,
    model: String,
    key: Option<String>,
    workspace: Option<String>,
}

/// Owned, clone-cheap chat plan taken from the live Vision config.
pub struct ChatCfg {
    http: reqwest::Client,
    providers: Vec<ChatProvider>,
}

impl ChatCfg {
    /// Opens a streaming chat completion, trying the primary provider then the fallback.
    pub async fn open(&self, system: &str, messages: &[(String, String)]) -> Result<(reqwest::Response, ChatKind), ChatOpenError> {
        let msgs: Vec<Value> = messages.iter().map(|(r, c)| json!({ "role": r, "content": c })).collect();
        let mut last = ChatOpenError::NotConnected;
        for p in &self.providers {
            let body = match p.kind {
                ChatKind::Anthropic => json!({
                    "model": p.model, "max_tokens": 2000, "stream": true, "system": system, "messages": msgs,
                }),
                kind => {
                    let mut all = vec![json!({ "role": "system", "content": system })];
                    all.extend(msgs.iter().cloned());
                    match kind {
                        ChatKind::OpenAI => json!({ "model": p.model, "max_tokens": 2000, "stream": true, "messages": all }),
                        _ => json!({ "model": p.model, "stream": true, "messages": all }),
                    }
                }
            };
            let request = match p.kind {
                ChatKind::Anthropic => match &p.key {
                    Some(k) => {
                        let mut r = self.http.post(ANTHROPIC_URL).header("x-api-key", k).header("anthropic-version", ANTHROPIC_VERSION);
                        if let Some(w) = &p.workspace {
                            r = r.header("anthropic-workspace-id", w);
                        }
                        r
                    }
                    None => {
                        last = ChatOpenError::Failed("This machine has no Anthropic key set.".into());
                        continue;
                    }
                },
                ChatKind::OpenAI => match &p.key {
                    Some(k) => self.http.post(format!("{}/chat/completions", p.url)).bearer_auth(k),
                    None => {
                        last = ChatOpenError::Failed("This machine has no OpenAI key set.".into());
                        continue;
                    }
                },
                ChatKind::Ollama => self.http.post(format!("{}/api/chat", p.url)),
            };
            let opened = tokio::time::timeout(Duration::from_secs(15), request.json(&body).send()).await;
            let resp = match opened {
                Err(_) => {
                    last = ChatOpenError::Failed(format!("{} did not answer in time.", self.name(p.kind)));
                    continue;
                }
                Ok(Err(e)) => {
                    last = if e.is_connect() {
                        ChatOpenError::Failed(match p.kind {
                            ChatKind::Ollama => "Nothing is listening for Ollama on this machine.".to_string(),
                            _ => format!("Couldn't reach {}.", self.name(p.kind)),
                        })
                    } else {
                        ChatOpenError::Failed(format!("Couldn't reach {}.", self.name(p.kind)))
                    };
                    continue;
                }
                Ok(Ok(r)) => r,
            };
            let status = resp.status().as_u16();
            if (200..300).contains(&status) {
                return Ok((resp, p.kind));
            }
            let reply: Value = resp.json().await.unwrap_or(Value::Null);
            let detail = reply
                .pointer("/error/message")
                .and_then(Value::as_str)
                .or_else(|| reply.get("error").and_then(Value::as_str))
                .unwrap_or("no details");
            last = ChatOpenError::Failed(match (p.kind, status) {
                (ChatKind::Anthropic, 401) | (ChatKind::Anthropic, 403) | (ChatKind::OpenAI, 401) => {
                    format!("{} rejected the API key. Check the key in web/.env.", self.name(p.kind))
                }
                (_, 404) if p.kind == ChatKind::Ollama => format!(
                    "The model \"{}\" isn't installed in Ollama. Run: ollama pull {}",
                    p.model, p.model
                ),
                (_, 404) => format!("{} doesn't know the model \"{}\".", self.name(p.kind), p.model),
                (_, 429) => format!("{} is rate-limiting this key. Try again in a moment.", self.name(p.kind)),
                _ => format!("{} returned {status}: {detail}", self.name(p.kind)),
            });
        }
        Err(last)
    }

    fn name(&self, kind: ChatKind) -> &'static str {
        match kind {
            ChatKind::Anthropic => "Anthropic",
            ChatKind::OpenAI => "OpenAI",
            ChatKind::Ollama => "Ollama",
        }
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
