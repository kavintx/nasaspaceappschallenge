//! Renders the backend's `GroundedAnswer` (Markdown answer + cited papers).

use crate::backend::{f, flag, https, s};
use serde_json::Value;

pub struct Line {
    pub text: String,
    /// 0 = paragraph, 1 = bullet, 2 = heading
    pub kind: u8,
}

pub struct Citation {
    pub title: String,
    pub meta: String,
    pub quote: String,
    pub href: Option<String>,
    pub ntrs_url: Option<String>,
}

#[derive(Default)]
pub struct AiResult {
    pub error: Option<String>,
    pub insufficient: bool,
    pub lines: Vec<Line>,
    pub citations: Vec<Citation>,
    pub footer: String,
}

pub fn from_reply(reply: Result<Value, String>) -> AiResult {
    let v = match reply {
        Ok(v) => v,
        Err(e) => return AiResult { error: Some(e), ..Default::default() },
    };
    let lines = text_lines(&s(&v, "answer").unwrap_or_default());
    let citations: Vec<Citation> = v
        .get("citations")
        .and_then(|c| c.as_array())
        .map(|a| a.iter().map(citation).collect())
        .unwrap_or_default();
    let mut footer = vec![];
    if let Some(n) = f(&v, "evidence_count") {
        footer.push(format!("{} evidence passages", n as i64));
    }
    if let Some(m) = s(&v, "model") {
        footer.push(format!("Model: {m}"));
    }
    AiResult {
        error: lines.is_empty().then(|| "The AI service returned an empty answer.".to_string()),
        insufficient: flag(&v, "insufficient_evidence"),
        lines,
        citations,
        footer: footer.join(" · "),
    }
}

fn citation(c: &Value) -> Citation {
    let ntrs = s(c, "ntrs_id");
    let mut meta = vec![];
    if let Some(n) = &ntrs {
        meta.push(format!("NTRS {n}"));
    }
    if let Some(p) = f(c, "page") {
        meta.push(format!("p. {}", p as i64));
    }
    Citation {
        title: s(c, "title").unwrap_or_else(|| "Untitled NASA paper".into()),
        meta: meta.join(" · "),
        quote: s(c, "quote").unwrap_or_default(),
        href: ntrs.or_else(|| s(c, "paper_id")).map(|i| format!("/papers/{}", urlencoding::encode(&i))),
        ntrs_url: https(c, "ntrs_url"),
    }
}

fn clean(text: &str) -> String {
    text.replace("**", "").replace("__", "").replace('`', "").trim().to_string()
}

/// Light Markdown handling: headings, bullets and paragraphs.
fn text_lines(text: &str) -> Vec<Line> {
    // Some models prepend their reasoning; keep only what follows it.
    let text = text.rsplit("</think>").next().unwrap_or(text);
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.chars().all(|c| matches!(c, '-' | '|' | ':' | ' ' | '=' | '*')))
        .map(|l| {
            if l.starts_with('#') {
                return Line { text: clean(l.trim_start_matches('#')), kind: 2 };
            }
            match l.strip_prefix("- ").or_else(|| l.strip_prefix("* ")).or_else(|| l.strip_prefix("• ")) {
                Some(rest) => Line { text: clean(rest), kind: 1 },
                None => Line { text: clean(l), kind: 0 },
            }
        })
        .filter(|l| !l.text.is_empty())
        .collect()
}
