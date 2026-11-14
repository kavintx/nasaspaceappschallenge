//! Turns an AI endpoint's JSON reply into simple headed text blocks, whatever
//! its exact shape, plus any cited papers.

use crate::backend::s;
use serde_json::Value;

pub struct Line {
    pub text: String,
    pub bullet: bool,
}

pub struct Block {
    pub heading: Option<String>,
    pub lines: Vec<Line>,
}

pub struct Citation {
    pub title: String,
    pub meta: String,
    pub snippet: String,
    pub href: Option<String>,
}

#[derive(Default)]
pub struct AiResult {
    pub error: Option<String>,
    pub blocks: Vec<Block>,
    pub citations: Vec<Citation>,
}

const SKIP: &[&str] = &[
    "citations", "ok", "provider", "model", "input_tokens", "output_tokens", "latency_ms", "id",
    "experiment_id", "experiment_ids", "created_at", "status",
];

pub fn from_reply(reply: Result<Value, String>) -> AiResult {
    match reply {
        Err(e) => AiResult { error: Some(e), ..Default::default() },
        Ok(v) => {
            let mut blocks = vec![];
            walk(None, &v, &mut blocks);
            blocks.retain(|b| b.heading.is_some() || !b.lines.is_empty());
            let error = blocks.is_empty().then(|| "The AI service returned an empty answer.".to_string());
            AiResult { error, blocks, citations: citations(&v) }
        }
    }
}

fn citations(v: &Value) -> Vec<Citation> {
    v.get("citations")
        .and_then(|c| c.as_array())
        .map(|a| {
            a.iter()
                .map(|c| {
                    let id = s(c, "ntrs_id").or_else(|| s(c, "paper_id"));
                    Citation {
                        title: s(c, "title").unwrap_or_else(|| "Untitled NASA paper".into()),
                        meta: s(c, "ntrs_id").map(|n| format!("NTRS {n}")).unwrap_or_default(),
                        snippet: s(c, "snippet").unwrap_or_default(),
                        href: id.map(|i| format!("/papers/{}", urlencoding::encode(&i))),
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn humanize(key: &str) -> String {
    let t = key.replace('_', " ");
    let mut c = t.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => t,
    }
}

fn clean(text: &str) -> String {
    text.replace("**", "").replace('`', "").trim().to_string()
}

fn text_lines(text: &str) -> Vec<Line> {
    // Some models prepend their reasoning; keep only what follows it.
    let text = text.rsplit("</think>").next().unwrap_or(text);
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| {
            let l = l.trim_start_matches('#').trim();
            match l.strip_prefix("- ").or_else(|| l.strip_prefix("* ")).or_else(|| l.strip_prefix("• ")) {
                Some(rest) => Line { text: clean(rest), bullet: true },
                None => Line { text: clean(l), bullet: false },
            }
        })
        .filter(|l| !l.text.is_empty())
        .collect()
}

fn scalar(v: &Value) -> Option<String> {
    match v {
        Value::String(x) => Some(clean(x)),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(if *b { "Yes".into() } else { "No".into() }),
        _ => None,
    }
}

fn walk(heading: Option<String>, v: &Value, out: &mut Vec<Block>) {
    match v {
        Value::Null => {}
        Value::String(text) => out.push(Block { heading, lines: text_lines(text) }),
        Value::Number(_) | Value::Bool(_) => out.push(Block {
            heading,
            lines: vec![Line { text: scalar(v).unwrap_or_default(), bullet: false }],
        }),
        Value::Array(items) => {
            let lines = items
                .iter()
                .filter_map(|item| match item {
                    Value::Object(o) => Some(
                        o.iter()
                            .filter(|(k, _)| !SKIP.contains(&k.as_str()))
                            .filter_map(|(k, val)| scalar(val).map(|x| format!("{}: {x}", humanize(k))))
                            .collect::<Vec<_>>()
                            .join(" · "),
                    ),
                    other => scalar(other),
                })
                .filter(|t| !t.is_empty())
                .map(|text| Line { text, bullet: true })
                .collect();
            out.push(Block { heading, lines });
        }
        Value::Object(map) => {
            if heading.is_some() {
                out.push(Block { heading, lines: vec![] });
            }
            for (k, val) in map {
                if !SKIP.contains(&k.as_str()) {
                    walk(Some(humanize(k)), val, out);
                }
            }
        }
    }
}
