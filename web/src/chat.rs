//! Ask the Data: a streaming research chat that runs on this machine's own AI
//! key (the same provider setup that reads photos) and answers only from the
//! research data read straight out of Supabase. No research backend involved.
//!
//! The browser contract is unchanged: POST /ask/stream answers with SSE events
//! `{"type":"delta","text"}`, `{"type":"meta","conversation_id","citations"}`,
//! `{"type":"error","error"}` and `{"type":"done","conversation_id"}`.

use crate::admin::same_origin;
use crate::backend::s as js;
use crate::store::Store;
use crate::vision::{ChatCfg, ChatOpenError, ChatPiece};
use crate::AppState;
use axum::{
    body::{Body, Bytes},
    extract::{Json, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

type AppRef = State<Arc<AppState>>;

#[derive(Deserialize)]
pub struct ChatRequest {
    message: String,
    conversation_id: Option<String>,
    experiment: Option<String>,
}

const MAX_MESSAGE_CHARS: usize = 4000;
/// How many past turns travel with the next question.
const HISTORY_MESSAGES: usize = 10;
/// Rough character budget for the research data pasted into the prompt.
const DIGEST_BUDGET: usize = 42_000;

static CONV_SEQ: AtomicU64 = AtomicU64::new(0);

fn new_conversation_id() -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{:x}-{:x}", nanos, CONV_SEQ.fetch_add(1, Ordering::Relaxed))
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.trim().to_string()
    } else {
        let cut: String = text.trim().chars().take(max).collect();
        format!("{cut}…")
    }
}

pub async fn chat_stream(State(app): AppRef, headers: HeaderMap, Json(req): Json<ChatRequest>) -> Response {
    if !same_origin(&headers) {
        return (StatusCode::FORBIDDEN, "Cross-site request refused.").into_response();
    }
    let message = req.message.trim().to_string();
    if message.is_empty() || message.chars().count() > MAX_MESSAGE_CHARS {
        return (StatusCode::BAD_REQUEST, "Write a question of up to 4,000 characters.").into_response();
    }

    let known: Vec<String> = app.chat.read().await.keys().cloned().collect();
    let conv_id = req
        .conversation_id
        .filter(|c| !c.is_empty() && known.iter().any(|k| k == c))
        .unwrap_or_else(new_conversation_id);
    let history = app.chat.read().await.get(&conv_id).cloned().unwrap_or_default();

    // The research digest comes from Supabase; building it up front means the
    // store error (if any) is a plain failed request and the stream only ever
    // carries the answer.
    let scope = req.experiment.filter(|e| !e.is_empty());
    let (digest, paper_titles) = match build_digest(&app.store, scope.as_deref()).await {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_GATEWAY, e).into_response(),
    };

    let cfg = { app.vision.read().await.chat_cfg() };
    let question = message.clone();
    let state = Arc::clone(&app);
    let (tx, rx) = mpsc::channel::<Result<Bytes, std::io::Error>>(16);
    tokio::spawn(async move {
        run_chat(tx, cfg, digest, paper_titles, history, question, conv_id, state).await;
    });

    Response::builder()
        .header(axum::http::header::CONTENT_TYPE, "text/event-stream")
        .header(axum::http::header::CACHE_CONTROL, "no-cache")
        .body(Body::from_stream(SseStream(rx)))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// Bridges the mpsc channel to the Stream shape axum's body wants.
struct SseStream(mpsc::Receiver<Result<Bytes, std::io::Error>>);

impl futures_util::Stream for SseStream {
    type Item = Result<Bytes, std::io::Error>;
    fn poll_next(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Option<Self::Item>> {
        self.0.poll_recv(cx)
    }
}

async fn ev(tx: &mpsc::Sender<Result<Bytes, std::io::Error>>, event: Value) -> bool {
    tx.send(Ok(Bytes::from(format!("data: {}\n\n", event)))).await.is_ok()
}

async fn run_chat(
    tx: mpsc::Sender<Result<Bytes, std::io::Error>>,
    cfg: ChatCfg,
    digest: String,
    paper_titles: Vec<(String, String)>,
    history: Vec<(String, String)>,
    question: String,
    conv_id: String,
    state: Arc<AppState>,
) {
    let system = format!(
        "You are the research assistant inside \"Flame in Freefall\", a NASA Space Apps site about how \
         everyday materials would burn in a spacecraft, built on NASA's microgravity combustion experiments \
         (ISS Saffire, FLEX, FLEX-2, SPICE, SLICE and others) and the papers indexed from NTRS.\n\n\
         Rules:\n\
         - Answer ONLY from the RESEARCH DATA below. Never invent experiment names, numbers, findings or paper ids.\n\
         - When you state a fact, cite the paper it came from as (NTRS xxxxxxx) using an id from the PAPERS list. \
         If the data doesn't back a statement, don't make that statement.\n\
         - If the data doesn't cover the question, say so plainly in a sentence or two and point at what it does cover.\n\
         - Plain text: short paragraphs and \"-\" bullets. No headings, no code blocks. Keep it under about 350 words.\n\n\
         RESEARCH DATA\n{digest}"
    );

    let mut messages: Vec<(String, String)> = history.iter().rev().take(HISTORY_MESSAGES).cloned().collect::<Vec<_>>();
    messages.reverse();
    messages.push(("user".to_string(), question.clone()));

    match cfg.open(&system, &messages).await {
        Err(why) => {
            let msg = match why {
                ChatOpenError::NotConnected => "No AI key is set on this machine yet. Paste an Anthropic or OpenAI key on the Scan page to switch the AI on.".to_string(),
                ChatOpenError::Failed(m) => m,
            };
            ev(&tx, json!({ "type": "error", "error": msg })).await;
            ev(&tx, json!({ "type": "done", "conversation_id": conv_id })).await;
            return;
        }
        Ok((resp, kind)) => {
            let mut stream = resp.bytes_stream();
            let mut buf = String::new();
            let mut text = String::new();
            let mut err: Option<String> = None;
            let mut finished = false;
            while let Some(chunk) = stream.next().await {
                match chunk {
                    Err(_) => {
                        err = Some("The AI service connection dropped mid-answer. Try again.".to_string());
                        break;
                    }
                    Ok(bytes) => {
                        buf.push_str(&String::from_utf8_lossy(&bytes));
                        while let Some(pos) = buf.find('\n') {
                            let line: String = buf[..pos].trim().to_string();
                            buf.drain(..=pos);
                            match kind.piece(&line) {
                                ChatPiece::Delta(t) => {
                                    text.push_str(&t);
                                    if !ev(&tx, json!({ "type": "delta", "text": t })).await {
                                        return;
                                    }
                                }
                                ChatPiece::Done => {
                                    finished = true;
                                    break;
                                }
                                ChatPiece::Err(m) => {
                                    err = Some(m);
                                    finished = true;
                                    break;
                                }
                                ChatPiece::Skip => {}
                            }
                        }
                        // Only an unfinished last line survives here; if a provider
                        // ever sends a pathological run without newlines, cap it.
                        if buf.len() > 128 * 1024 {
                            buf.clear();
                        }
                    }
                }
                if finished {
                    break;
                }
            }
            if text.trim().is_empty() {
                ev(&tx, json!({ "type": "error", "error": err.unwrap_or_else(|| "The AI service returned an empty answer.".to_string()) }))
                    .await;
                ev(&tx, json!({ "type": "done", "conversation_id": conv_id })).await;
                return;
            }
            if let Some(m) = err {
                // partial answer streamed already; still report the problem.
                ev(&tx, json!({ "type": "error", "error": m })).await;
            }
            let citations = citations_for(&text, &paper_titles);
            ev(&tx, json!({ "type": "meta", "conversation_id": conv_id, "citations": citations })).await;

            let mut all = history;
            all.push(("user".to_string(), clip(&question, MAX_MESSAGE_CHARS)));
            all.push(("assistant".to_string(), clip(&text, 8000)));
            let keep = all.len().saturating_sub(HISTORY_MESSAGES);
            let trimmed = all.split_off(keep);
            {
                let mut conversations = state.chat.write().await;
                if conversations.len() > 200 {
                    conversations.clear();
                }
                conversations.insert(conv_id.clone(), trimmed);
            }
            ev(&tx, json!({ "type": "done", "conversation_id": conv_id })).await;
        }
    }
}

/// Papers whose NTRS id the answer actually cites, in order of first mention.
fn citations_for(answer: &str, titles: &[(String, String)]) -> Vec<Value> {
    let mut found: Vec<String> = vec![];
    let bytes = answer.as_bytes();
    let mut i = 0;
    while let Some(p) = find_case_insensitive(answer, i, b"ntrs") {
        let mut j = p + 4;
        while j < bytes.len() && (bytes[j] as char).is_ascii_whitespace() {
            j += 1;
        }
        let start = j;
        while j < bytes.len() && (bytes[j] as char).is_ascii_digit() {
            j += 1;
        }
        if j - start >= 6 {
            let id = &answer[start..j];
            if !found.iter().any(|f| f == id) {
                found.push(id.to_string());
            }
        }
        i = j.max(p + 4);
    }
    found
        .iter()
        .filter_map(|id| titles.iter().find(|(ntrs, _)| ntrs == id).map(|(ntrs, title)| json!({ "ntrs_id": ntrs, "title": title })))
        .collect()
}

fn find_case_insensitive(haystack: &str, from: usize, needle: &[u8]) -> Option<usize> {
    let h = haystack.as_bytes();
    (from..h.len().saturating_sub(needle.len() - 1)).find(|&k| h[k..k + needle.len()].eq_ignore_ascii_case(needle))
}

/// Flatten the Supabase rows into a compact, citable brief for the model.
async fn build_digest(store: &Store, scope: Option<&str>) -> Result<(String, Vec<(String, String)>), String> {
    let (exps, papers, conditions, findings) = tokio::join!(store.experiments(), store.papers(None), store.conditions(), store.findings());
    let (exps, papers, conditions, findings) = (exps?, papers?, conditions?, findings?);

    let acronyms: HashMap<String, String> = exps
        .iter()
        .filter_map(|e| Some((js(e, "id")?, js(e, "acronym").or_else(|| js(e, "name"))?)))
        .collect();
    let scope_id = scope.and_then(|sc| {
        exps.iter()
            .find(|e| js(e, "acronym").or_else(|| js(e, "name")).is_some_and(|a| a.eq_ignore_ascii_case(sc)))
            .and_then(|e| js(e, "id"))
    });

    let mut out = String::from("EXPERIMENTS (NASA microgravity combustion program):\n");
    for e in &exps {
        let acr = js(e, "acronym").or_else(|| js(e, "name")).unwrap_or_default();
        let name = js(e, "name").unwrap_or_default();
        let desc = js(e, "description").map(|d| clip(&d, 220)).unwrap_or_default();
        out.push_str(&format!("- {acr}: {name}. {desc}\n"));
    }

    let in_scope = |row: &Value| match &scope_id {
        None => true,
        Some(id) => js(row, "experiment_id").is_some_and(|e| &e == id),
    };

    out.push_str("\nTESTED CONDITIONS (setups the papers describe):\n");
    let mut used = out.chars().count();
    let mut shown = 0;
    for c in conditions.iter().filter(|r| in_scope(r)) {
        if shown >= 500 || used >= DIGEST_BUDGET / 2 {
            out.push_str("- (more rows left out for space)\n");
            break;
        }
        let exp = js(c, "experiment_id").and_then(|id| acronyms.get(&id).cloned()).unwrap_or_else(|| "unlinked".into());
        let mut bits = vec![format!("[{exp}]")];
        for (col, label) in [("fuel_type", "fuel"), ("fuel_geometry", "shape"), ("oxygen_concentration", "o2"), ("pressure", "pressure"), ("airflow", "airflow")] {
            if let Some(v) = js(c, col) {
                bits.push(format!("{label}={}", clip(&v, 60)));
            }
        }
        if let Some(o) = js(c, "objective") {
            bits.push(format!("objective: {}", clip(&o, 160)));
        }
        if let Some(p) = js(c, "source_page") {
            bits.push(format!("(source p.{p})"));
        }
        let line = format!("- {}\n", bits.join(" "));
        used += line.chars().count();
        out.push_str(&line);
        shown += 1;
    }

    out.push_str("\nFINDINGS (extracted from the papers):\n");
    let mut shown = 0;
    for f in findings.iter().filter(|r| in_scope(r)) {
        if shown >= 700 || used >= DIGEST_BUDGET {
            out.push_str("- (more rows left out for space)\n");
            break;
        }
        let exp = js(f, "experiment_id").and_then(|id| acronyms.get(&id).cloned()).unwrap_or_else(|| "unlinked".into());
        let Some(text) = js(f, "text") else { continue };
        let page = js(f, "source_page").map(|p| format!(" (source p.{p})")).unwrap_or_default();
        let line = format!("- [{exp}] {}{page}\n", clip(&text, 280));
        used += line.chars().count();
        out.push_str(&line);
        shown += 1;
    }

    let mut titles: Vec<(String, String)> = vec![];
    out.push_str("\nPAPERS (cite by id as shown):\n");
    for p in &papers {
        let Some(ntrs) = js(p, "ntrs_id") else { continue };
        let title = js(p, "title").unwrap_or_else(|| "Untitled NASA paper".into());
        titles.push((ntrs.clone(), title.clone()));
        if titles.len() <= 300 && used < DIGEST_BUDGET + 8_000 {
            let line = format!("- NTRS {ntrs}: {}\n", clip(&title, 160));
            used += line.chars().count();
            out.push_str(&line);
        }
    }
    Ok((out, titles))
}
