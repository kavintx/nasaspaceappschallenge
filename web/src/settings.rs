//! /settings — choose the photo scanner and store its keys, in the browser.
//!
//! Everything is saved to web/.env (the same file `main.py` writes). Saved
//! secret values are never sent back to the browser — the page only shows
//! whether one is stored. Scanner changes take effect immediately; the
//! research backend address/token is re-read when the site restarts.

use crate::config::Config;
use crate::vision::Vision;
use crate::AppState;
use askama::Template;
use axum::extract::{Form, State};
use axum::response::{Html, IntoResponse, Response};
use serde::Deserialize;
use std::sync::Arc;

/// Relative to where the server runs — the same path config.rs reads.
const ENV_PATH: &str = ".env";

#[derive(Template)]
#[template(path = "settings.html")]
struct SettingsPage {
    active: &'static str,
    page_title: String,
    context_line: String,
    provider: String,
    fallback: String,
    ollama_url: String,
    ollama_model: String,
    anthropic_model: String,
    anthropic_key_set: bool,
    openai_model: String,
    openai_base: String,
    openai_key_set: bool,
    api_base: String,
    api_token_set: bool,
    status: String,
    connected: bool,
    note: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct SettingsForm {
    provider: Option<String>,
    fallback: Option<String>,
    ollama_url: Option<String>,
    ollama_model: Option<String>,
    anthropic_key: Option<String>,
    anthropic_model: Option<String>,
    openai_key: Option<String>,
    openai_model: Option<String>,
    openai_base: Option<String>,
    api_base: Option<String>,
    api_token: Option<String>,
}

/// Set KEY=value on the first matching line, or append it.
fn upsert(lines: &mut Vec<String>, key: &str, value: &str) {
    for line in lines.iter_mut() {
        let t = line.trim_start();
        if t.starts_with('#') {
            continue;
        }
        if line.split_once('=').is_some_and(|(k, _)| k.trim() == key) {
            *line = format!("{key}={value}");
            return;
        }
    }
    lines.push(format!("{key}={value}"));
}

fn read_env_lines() -> Vec<String> {
    std::fs::read_to_string(ENV_PATH)
        .map(|raw| raw.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

/// (name, is-set) pairs for the non-secret inputs, so the form shows truth.
fn build_page(cfg: &Config, status: String, connected: bool, note: String) -> SettingsPage {
    let get = |k: &str, d: &str| cfg.get(k).unwrap_or_else(|| d.into());
    SettingsPage {
        active: "settings",
        page_title: "Settings".into(),
        context_line: "Scanner provider and keys".into(),
        provider: cfg.get("VISION_PROVIDER").unwrap_or_else(|| "ollama".into()),
        fallback: cfg.get("VISION_FALLBACK").unwrap_or_else(|| "none".into()),
        ollama_url: get("OLLAMA_BASE_URL", "http://127.0.0.1:11434"),
        ollama_model: get("OLLAMA_VISION_MODEL", "llama3.2-vision"),
        anthropic_model: get("ANTHROPIC_MODEL", "claude-haiku-5-5"),
        anthropic_key_set: cfg.get_file_first("ANTHROPIC_API_KEY").is_some(),
        openai_model: get("OPENAI_VISION_MODEL", "gpt-4o-mini"),
        openai_base: get("OPENAI_BASE_URL", "https://api.openai.com/v1"),
        openai_key_set: cfg.get_file_first("OPENAI_API_KEY").is_some(),
        api_base: get("API_BASE_URL", "http://127.0.0.1:8080"),
        api_token_set: cfg.get_file_first("API_AUTH_TOKEN").is_some(),
        status,
        connected,
        note,
    }
}

fn render(page: SettingsPage) -> Response {
    match page.render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => {
            eprintln!("settings template error: {e}");
            axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

pub async fn page(State(app): State<Arc<AppState>>) -> Response {
    let cfg = Config::load();
    let (status, connected) = {
        let v = app.vision.read().await;
        (v.status(), v.may_work())
    };
    render(build_page(&cfg, status, connected, String::new()))
}

pub async fn save(State(app): State<Arc<AppState>>, form: Form<SettingsForm>) -> Response {
    let f = form.0;
    let mut lines = read_env_lines();

    if let Some(p) = f.provider.filter(|p| matches!(p.as_str(), "ollama" | "anthropic" | "openai" | "backend")) {
        upsert(&mut lines, "VISION_PROVIDER", &p);
        let fallback = match f.fallback.as_deref() {
            Some("anthropic") if p != "anthropic" => "anthropic",
            Some("openai") if p != "openai" => "openai",
            _ => "none",
        };
        upsert(&mut lines, "VISION_FALLBACK", fallback);
    }
    // Plain settings: blank means "leave as is".
    for (key, value) in [
        ("OLLAMA_BASE_URL", f.ollama_url),
        ("OLLAMA_VISION_MODEL", f.ollama_model),
        ("ANTHROPIC_MODEL", f.anthropic_model),
        ("OPENAI_VISION_MODEL", f.openai_model),
        ("OPENAI_BASE_URL", f.openai_base),
        ("API_BASE_URL", f.api_base),
    ] {
        if let Some(v) = value.map(|v| v.trim().to_string()).filter(|v| !v.is_empty()) {
            upsert(&mut lines, key, &v);
        }
    }
    // Secrets: only written when actually typed, never blanked from the form.
    for (key, value) in [
        ("ANTHROPIC_API_KEY", f.anthropic_key),
        ("OPENAI_API_KEY", f.openai_key),
        ("API_AUTH_TOKEN", f.api_token),
    ] {
        if let Some(v) = value.map(|v| v.trim().to_string()).filter(|v| !v.is_empty()) {
            upsert(&mut lines, key, &v);
        }
    }

    if let Err(e) = std::fs::write(ENV_PATH, lines.join("\n") + "\n") {
        let cfg = Config::load();
        let (status, connected) = {
            let v = app.vision.read().await;
            (v.status(), v.may_work())
        };
        return render(build_page(&cfg, status, connected, format!("Couldn’t save: {e}")));
    }

    // Re-arm the scanner from the file we just wrote. It reads .env file-first,
    // so values saved here win over the ones the container was launched with.
    let fresh = Vision::from_cfg(&Config::load());
    let note = format!("Saved. Scanner now: {}.", fresh.status());
    {
        let mut v = app.vision.write().await;
        *v = fresh;
    }
    let connected = app.vision.read().await.may_work();
    let cfg = Config::load();
    let status = app.vision.read().await.status();
    render(build_page(&cfg, status, connected, note))
}
