//! Pipeline control, extraction, human verification and the chat relay.
//! Everything here that changes the database is refused when READ_ONLY is set.

use crate::backend::{flag, s as js, Backend};
use crate::views::{backend, message_page, render};
use crate::AppState;
use askama::Template;
use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
    Form,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

type AppRef = State<Arc<AppState>>;

pub const PROGRAMS: [&str; 13] = [
    "BASS", "BASS-II", "SAFFIRE I", "SAFFIRE II", "SAFFIRE III", "SAME", "SAME-R", "FLEX", "FLEX-2", "CFI", "ACME",
    "SPICE", "SLICE",
];

/// Refuse form posts that come from another website.
///
/// Works behind a proxy (a Cloudflare tunnel, for instance): the browser's Origin is
/// checked against every name the server is reached under — the direct Host header or
/// the proxy's X-Forwarded-Host — so a tunnel rewriting Host doesn't break its own
/// visitors. A page on some other site still can't forge either value.
pub(crate) fn same_origin(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN).and_then(|o| o.to_str().ok()) else {
        return true; // plain same-site form posts from browsers all carry Origin; anything without it is a non-browser client
    };
    let want = trim_host(origin);
    let matches = |name: &str| headers.get(name).and_then(|h| h.to_str().ok()).is_some_and(|v| trim_host(v) == want);
    matches("host") || matches("x-forwarded-host")
}

/// "https://abc.trycloudflare.com" or "localhost:3101:443"-style values down to the host[:port].
fn trim_host(value: &str) -> &str {
    let rest = value.split("://").nth(1).unwrap_or(value);
    let rest = rest.trim_end_matches('/');
    if let Some(stripped) = rest.strip_suffix(":443") {
        return stripped;
    }
    rest
}

/// Checks shared by every action that writes: same site, not read-only, backend connected.
fn writable<'a>(app: &'a AppState, headers: &HeaderMap) -> Result<&'a Backend, Response> {
    if !same_origin(headers) {
        return Err((StatusCode::FORBIDDEN, "Cross-site request refused.").into_response());
    }
    if app.read_only {
        return Err((
            StatusCode::FORBIDDEN,
            message_page("", "Read-only", "This copy of the app is read-only, so pipeline and editing actions are turned off.".into()),
        )
            .into_response());
    }
    backend(app)
}

async fn forget_catalog(app: &AppState) {
    *app.catalog.lock().await = None;
}

// ---------- pipeline page ----------

pub struct ActionResult {
    pub title: String,
    pub error: Option<String>,
    pub pairs: Vec<(String, String)>,
    pub lines: Vec<String>,
    pub text: Option<String>,
}

#[derive(Template)]
#[template(path = "pipeline.html")]
pub struct PipelinePage {
    pub active: &'static str,
    pub page_title: String,
    pub context_line: String,
    pub read_only: bool,
    pub health_ok: bool,
    pub health_line: String,
    pub experiments_count: String,
    pub paper_counts: Vec<(String, String)>,
    pub job_counts: Vec<(String, String)>,
    pub programs: Vec<&'static str>,
    pub result: Option<ActionResult>,
}

fn humanize(key: &str) -> String {
    let t = key.replace('_', " ");
    let mut c = t.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

fn scalar(v: &Value) -> Option<String> {
    match v {
        Value::String(x) if !x.is_empty() => Some(x.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(if *b { "Yes".into() } else { "No".into() }),
        _ => None,
    }
}

/// Shows any JSON reply as label/value pairs plus list lines.
fn result_from(title: &str, reply: Result<Value, String>) -> ActionResult {
    let mut out = ActionResult { title: title.into(), error: None, pairs: vec![], lines: vec![], text: None };
    match reply {
        Err(e) => out.error = Some(e),
        Ok(Value::Object(map)) => {
            for (k, v) in &map {
                match v {
                    Value::Array(items) => {
                        for item in items {
                            let line = match item {
                                Value::Object(o) => o
                                    .iter()
                                    .filter_map(|(ik, iv)| scalar(iv).map(|x| format!("{}: {x}", humanize(ik))))
                                    .collect::<Vec<_>>()
                                    .join(" · "),
                                other => scalar(other).unwrap_or_default(),
                            };
                            if !line.is_empty() {
                                out.lines.push(format!("{}: {line}", humanize(k)));
                            }
                        }
                    }
                    other => {
                        if let Some(x) = scalar(other) {
                            out.pairs.push((humanize(k), x));
                        }
                    }
                }
            }
        }
        Ok(other) => out.text = Some(other.to_string()),
    }
    out
}

async fn pipeline_page(app: &AppState, result: Option<ActionResult>) -> Response {
    let b = match backend(app) {
        Ok(b) => b,
        Err(page) => return page,
    };
    let (health, status) = tokio::join!(b.health(), b.pipeline_status());
    let (health_ok, health_line) = match health {
        // The reply also carries the model server's address; that is never shown.
        Ok(h) => (
            flag(&h, "ok"),
            [
                js(&h, "configured_model"),
                js(&h, "latency_ms").map(|ms| format!("{ms} ms")),
                Some(if flag(&h, "model_verified") { "model verified".to_string() } else { "model not verified".to_string() }),
                Some(if flag(&h, "embeddings_supported") { "embeddings on".to_string() } else { "embeddings off".to_string() }),
                js(&h, "error"),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · "),
        ),
        Err(e) => (false, e),
    };
    let status = status.unwrap_or(Value::Null);
    // Count lists arrive as arrays whose last item is the count.
    let counts = |key: &str| -> Vec<(String, String)> {
        status
            .get(key)
            .and_then(|x| x.as_array())
            .map(|list| {
                list.iter()
                    .filter_map(|row| {
                        let a = row.as_array()?;
                        let (n, labels) = a.split_last()?;
                        Some((labels.iter().filter_map(|l| l.as_str()).collect::<Vec<_>>().join(" · "), n.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    render(PipelinePage {
        active: "pipeline",
        page_title: "Pipeline".into(),
        context_line: "Ingest, extract and check the research database".into(),
        read_only: app.read_only,
        health_ok,
        health_line,
        experiments_count: js(&status, "experiments_count").unwrap_or_else(|| "—".into()),
        paper_counts: counts("papers_by_status"),
        job_counts: counts("jobs_by_kind_status"),
        programs: PROGRAMS.to_vec(),
        result,
    })
}

pub async fn pipeline(State(app): AppRef) -> Response {
    pipeline_page(&app, None).await
}

#[derive(Deserialize)]
pub struct TestForm {
    prompt: Option<String>,
}

pub async fn test_model(State(app): AppRef, headers: HeaderMap, Form(form): Form<TestForm>) -> Response {
    if !same_origin(&headers) {
        return (StatusCode::FORBIDDEN, "Cross-site request refused.").into_response();
    }
    let b = match backend(&app) {
        Ok(b) => b,
        Err(page) => return page,
    };
    let reply = b.test_model(&form.prompt.unwrap_or_default()).await;
    let mut result = result_from("Model test", reply.clone());
    if let Ok(v) = &reply {
        result.text = js(v, "response");
        result.pairs.retain(|(k, _)| k != "Response");
    }
    pipeline_page(&app, Some(result)).await
}

#[derive(Deserialize)]
pub struct IngestForm {
    program: Option<String>,
    limit_per_program: Option<String>,
    process_jobs: Option<String>,
}

pub async fn ingest(State(app): AppRef, headers: HeaderMap, Form(form): Form<IngestForm>) -> Response {
    let b = match writable(&app, &headers) {
        Ok(b) => b,
        Err(r) => return r,
    };
    let program = form.program.unwrap_or_default();
    let n = |x: Option<String>| x.and_then(|v| v.trim().parse::<u32>().ok()).map(|v| v.min(50));
    let reply = if !program.is_empty() && !PROGRAMS.contains(&program.as_str()) {
        Err(format!("Unknown program \"{program}\"."))
    } else {
        b.ingest(&program, n(form.limit_per_program), n(form.process_jobs)).await
    };
    forget_catalog(&app).await;
    let title = if program.is_empty() { "NTRS ingest · all programs".to_string() } else { format!("NTRS ingest · {program}") };
    pipeline_page(&app, Some(result_from(&title, reply))).await
}

/// The backend only accepts https links on these hosts; check before sending.
fn sheet_url_ok(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else { return false };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("").to_lowercase();
    if host.contains('@') || host.contains(':') {
        return false;
    }
    host == "docs.google.com"
        || host == "sheets.googleapis.com"
        || host.ends_with(".googleusercontent.com")
        || host.ends_with(".nasa.gov")
}

#[derive(Deserialize)]
pub struct SheetForm {
    url: String,
}

pub async fn sheet_import(State(app): AppRef, headers: HeaderMap, Form(form): Form<SheetForm>) -> Response {
    let b = match writable(&app, &headers) {
        Ok(b) => b,
        Err(r) => return r,
    };
    let url = form.url.trim();
    let reply = if sheet_url_ok(url) {
        b.sheet_import(url).await
    } else {
        Err("That link isn't accepted. Use an https:// link on docs.google.com, sheets.googleapis.com, googleusercontent.com or nasa.gov.".into())
    };
    forget_catalog(&app).await;
    pipeline_page(&app, Some(result_from("Google Sheets import", reply))).await
}

#[derive(Deserialize)]
pub struct BatchForm {
    limit: Option<String>,
}

pub async fn extract_batch(State(app): AppRef, headers: HeaderMap, Form(form): Form<BatchForm>) -> Response {
    let b = match writable(&app, &headers) {
        Ok(b) => b,
        Err(r) => return r,
    };
    let limit = form.limit.and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(5).clamp(1, 25);
    let reply = b.extract_batch(limit).await;
    forget_catalog(&app).await;
    let mut result = result_from("Batch AI extraction", reply.clone());
    if let Ok(v) = &reply {
        result.lines = v
            .get("results")
            .and_then(|r| r.as_array())
            .map(|rows| {
                rows.iter()
                    .map(|r| {
                        let id = js(r, "ntrs_id").or_else(|| js(r, "paper_id")).unwrap_or_default();
                        match (flag(r, "ok"), js(r, "error")) {
                            (true, _) => format!("NTRS {id}: extracted"),
                            (false, Some(e)) => format!("NTRS {id}: failed, {e}"),
                            (false, None) => format!("NTRS {id}: failed"),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
    }
    pipeline_page(&app, Some(result)).await
}

// ---------- paper actions ----------

fn back_to_paper(paper: &str, key: &str, text: &str) -> Response {
    Redirect::to(&format!("/papers/{}?{key}={}", urlencoding::encode(paper), urlencoding::encode(text))).into_response()
}

pub async fn extract_paper(State(app): AppRef, headers: HeaderMap, Path(id): Path<String>) -> Response {
    let b = match writable(&app, &headers) {
        Ok(b) => b,
        Err(r) => return r,
    };
    let reply = b.extract(&id).await;
    forget_catalog(&app).await;
    match reply {
        Ok(v) => {
            let count = |k: &str| v.get(k).and_then(|x| x.as_array()).map(|a| a.len()).unwrap_or(0);
            back_to_paper(
                &id,
                "msg",
                &format!("AI extraction finished: {} conditions and {} findings stored.", count("stored_conditions"), count("stored_findings")),
            )
        }
        Err(e) => back_to_paper(&id, "err", &e),
    }
}

const STATUSES: [&str; 4] = ["pending", "verified", "corrected", "rejected"];
const CATEGORIES: [&str; 4] = ["prevention", "detection", "suppression", "other"];

#[derive(Deserialize)]
pub struct ConditionForm {
    paper: String,
    fuel_type: Option<String>,
    fuel_geometry: Option<String>,
    oxygen_concentration: Option<String>,
    pressure: Option<String>,
    airflow: Option<String>,
    sensors: Option<String>,
    objective: Option<String>,
    source_page: Option<String>,
    verification_status: Option<String>,
    correction_note: Option<String>,
}

/// Adds a text field to the update only when it was filled in.
fn put(body: &mut Value, key: &str, value: &Option<String>) {
    if let Some(v) = value.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        body[key] = json!(v);
    }
}

fn put_common(body: &mut Value, page: &Option<String>, status: &Option<String>, note: &Option<String>) -> Result<(), String> {
    if let Some(p) = page.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        body["source_page"] = json!(p.parse::<u32>().map_err(|_| "Source page must be a number.".to_string())?);
    }
    if let Some(st) = status.as_deref().filter(|st| !st.is_empty()) {
        if !STATUSES.contains(&st) {
            return Err(format!("Unknown status \"{st}\"."));
        }
        body["verification_status"] = json!(st);
    }
    put(body, "correction_note", note);
    Ok(())
}

async fn save_row(app: &AppState, b: &Backend, kind: &str, id: &str, paper: &str, body: Result<Value, String>) -> Response {
    let body = match body {
        Ok(v) if v.as_object().is_some_and(|o| !o.is_empty()) => v,
        Ok(_) => return back_to_paper(paper, "err", "Nothing to save."),
        Err(e) => return back_to_paper(paper, "err", &e),
    };
    match b.patch_row(kind, id, body).await {
        Ok(_) => {
            forget_catalog(app).await;
            back_to_paper(paper, "msg", "Saved.")
        }
        Err(e) => back_to_paper(paper, "err", &e),
    }
}

pub async fn edit_condition(State(app): AppRef, headers: HeaderMap, Path(id): Path<String>, Form(form): Form<ConditionForm>) -> Response {
    let b = match writable(&app, &headers) {
        Ok(b) => b,
        Err(r) => return r,
    };
    let mut body = json!({});
    put(&mut body, "fuel_type", &form.fuel_type);
    put(&mut body, "fuel_geometry", &form.fuel_geometry);
    put(&mut body, "oxygen_concentration", &form.oxygen_concentration);
    put(&mut body, "pressure", &form.pressure);
    put(&mut body, "airflow", &form.airflow);
    put(&mut body, "objective", &form.objective);
    if let Some(list) = form.sensors.as_deref().filter(|x| !x.trim().is_empty()) {
        body["sensors"] = json!(list.split(',').map(str::trim).filter(|x| !x.is_empty()).collect::<Vec<_>>());
    }
    let checked = put_common(&mut body, &form.source_page, &form.verification_status, &form.correction_note);
    save_row(&app, b, "conditions", &id, &form.paper, checked.map(|_| body)).await
}

#[derive(Deserialize)]
pub struct FindingForm {
    paper: String,
    text: Option<String>,
    category: Option<String>,
    source_page: Option<String>,
    verification_status: Option<String>,
    correction_note: Option<String>,
}

pub async fn edit_finding(State(app): AppRef, headers: HeaderMap, Path(id): Path<String>, Form(form): Form<FindingForm>) -> Response {
    let b = match writable(&app, &headers) {
        Ok(b) => b,
        Err(r) => return r,
    };
    let mut body = json!({});
    put(&mut body, "text", &form.text);
    let mut checked = put_common(&mut body, &form.source_page, &form.verification_status, &form.correction_note);
    if let Some(c) = form.category.as_deref().filter(|c| !c.is_empty()) {
        if CATEGORIES.contains(&c) {
            body["category"] = json!(c);
        } else {
            checked = Err(format!("Unknown category \"{c}\"."));
        }
    }
    save_row(&app, b, "findings", &id, &form.paper, checked.map(|_| body)).await
}

// (The chat used to relay the backend's /ai/chat stream. Ask the Data now runs
//  on this machine's own key straight from the Supabase data — see chat.rs.)

