//! The one-box unlock on Burn Scan: paste your model key, get scanning.
//!
//! POST /setup saves the key to web/.env and swaps the live scanner — no
//! restart, no terminal, no settings page. The key is never echoed back and
//! never logged; the page only ever shows that one is saved.

use crate::admin::same_origin;
use crate::config::Config;
use crate::vision::Vision;
use crate::AppState;
use axum::extract::{Form, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;
use std::sync::Arc;

/// Relative to where the server runs — the same file config.rs reads.
const ENV_PATH: &str = ".env";

#[derive(Debug, Default, Deserialize)]
pub struct UnlockForm {
    anthropic_key: Option<String>,
    openai_key: Option<String>,
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

pub async fn unlock(State(app): State<Arc<AppState>>, headers: HeaderMap, form: Form<UnlockForm>) -> Response {
    if !same_origin(&headers) {
        return (StatusCode::FORBIDDEN, "Cross-site request refused.").into_response();
    }
    // Keys are only ever pasted into a copy on your own computer, never the public website.
    if crate::hosted() {
        return Redirect::to("/scan").into_response();
    }
    // Anthropic wins when both are pasted; it is the one the page recommends.
    let Some((key_env, provider_env, key)) = [
        form.anthropic_key.as_deref().map(str::trim).filter(|k| k.len() >= 20).map(|k| ("ANTHROPIC_API_KEY", "anthropic", k)),
        form.openai_key.as_deref().map(str::trim).filter(|k| k.len() >= 20).map(|k| ("OPENAI_API_KEY", "openai", k)),
    ]
    .into_iter()
    .flatten()
    .next() else {
        // Nothing typed — no harm done, back you go.
        return Redirect::to("/scan").into_response();
    };

    let mut lines: Vec<String> = std::fs::read_to_string(ENV_PATH)
        .map(|raw| raw.lines().map(str::to_string).collect())
        .unwrap_or_default();
    upsert(&mut lines, key_env, key);
    upsert(&mut lines, "VISION_PROVIDER", provider_env);
    if let Err(e) = std::fs::write(ENV_PATH, lines.join("\n") + "\n") {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("Couldn’t save: {e}")).into_response();
    }

    // Re-arm the scanner from the file we just wrote; it reads .env file-first.
    let fresh = Vision::from_cfg(&Config::load());
    {
        let mut v = app.vision.write().await;
        *v = fresh;
    }
    Redirect::to("/scan").into_response()
}
