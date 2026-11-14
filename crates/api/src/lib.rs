use ai_engine::AiClient;
use async_stream::stream;
use axum::{
    extract::{DefaultBodyLimit, Path, Query, Request, State},
    http::{header, HeaderValue, Method, StatusCode},
    middleware::{self, Next},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::{get, patch, post},
    Json, Router,
};
use db::{
    repo_records::{ConditionPatch, FindingPatch},
    Db,
};
use futures_util::StreamExt;
use pipeline::PipelineService;
use rag::{HabitatScenario, RagEngine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use shared::{types::ChatTurn, AppConfig, Error};
use std::convert::Infallible;
use subtle::ConstantTimeEq;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;
use tracing::error;
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    pub config: AppConfig,
    pub db: Db,
    pub ai: AiClient,
    pub pipeline: PipelineService,
    pub rag: RagEngine,
}

pub struct ApiError(pub Error);

impl From<Error> for ApiError {
    fn from(e: Error) -> Self {
        Self(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.0.status_code())
            .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let client_message = if status.is_server_error() {
            error!(status = status.as_u16(), error = %self.0, "internal server error");
            "Internal server error".to_string()
        } else {
            self.0.to_string()
        };
        let body = json!({
            "error": client_message,
            "status": status.as_u16()
        });
        (status, Json(body)).into_response()
    }
}

pub type ApiResult<T> = std::result::Result<T, ApiError>;

async fn require_bearer_auth(
    State(st): State<AppState>,
    req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let expected = st.config.api_auth_token.as_bytes();
    let provided = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|t| t.trim().as_bytes());

    match provided {
        Some(token) if !expected.is_empty() && token.len() == expected.len() && token.ct_eq(expected).into() => {
            Ok(next.run(req).await)
        }
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}

pub fn build_router(state: AppState) -> Router {
    let origins: Vec<HeaderValue> = state
        .config
        .allowed_origins
        .iter()
        .filter_map(|o| o.parse::<HeaderValue>().ok())
        .collect();

    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods([Method::GET, Method::POST, Method::PATCH])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE]);

    let protected = Router::new()
        // Provider diagnostics
        .route("/api/v1/ai/health", get(ai_health))
        .route("/api/v1/ai/models", get(ai_models))
        .route("/api/v1/ai/test", post(ai_test))
        // Scientific extraction
        .route("/api/v1/ai/extract/{paper_id}", post(ai_extract_paper))
        .route("/api/v1/ai/extract/batch", post(ai_extract_batch))
        // Experiment summaries
        .route("/api/v1/ai/summarize/{experiment_id}", post(ai_summarize_experiment))
        // Research assistant (JSON + SSE streaming)
        .route("/api/v1/ai/chat", post(ai_chat))
        // Experiment comparison
        .route("/api/v1/ai/compare", post(ai_compare))
        // Habitat analysis
        .route("/api/v1/ai/habitat/analyze", post(ai_habitat_analyze))
        // Research question answering
        .route("/api/v1/ai/query", post(ai_query))
        // Human verification/correction & pipeline routes
        .route("/api/v1/conditions/{condition_id}", patch(patch_condition))
        .route("/api/v1/findings/{finding_id}", patch(patch_finding))
        .route("/api/v1/pipeline/ingest", post(trigger_ingest))
        .route("/api/v1/pipeline/sheet/import", post(import_sheet))
        .route("/api/v1/pipeline/nasa/scrape", post(scrape_nasa_experiments))
        .route("/api/v1/pipeline/status", get(pipeline_status))
        .route("/api/v1/experiments", get(list_experiments))
        .route("/api/v1/papers", get(list_papers))
        .route("/api/v1/papers/{paper_id}/extractions", get(get_paper_extractions))
        .layer(middleware::from_fn_with_state(state.clone(), require_bearer_auth));

    Router::new()
        .merge(protected)
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

// ---------- Provider diagnostics ----------

async fn ai_health(State(st): State<AppState>) -> impl IntoResponse {
    let health = st.ai.health().await;
    let code = if health.ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (code, Json(health))
}

async fn ai_models(State(st): State<AppState>) -> ApiResult<Json<Value>> {
    let models = st.ai.list_models().await?;
    Ok(Json(json!({
        "provider": st.config.ai_provider,
        "configured_model": st.config.vllm_model,
        "models": models
    })))
}

#[derive(Debug, Deserialize, Default)]
struct TestRequest {
    prompt: Option<String>,
}

async fn ai_test(
    State(st): State<AppState>,
    body: Option<Json<TestRequest>>,
) -> ApiResult<Json<Value>> {
    let prompt = body
        .and_then(|b| b.0.prompt)
        .unwrap_or_else(|| "Respond with 'VLLM_ANTHROPIC_OK' and confirm microgravity combustion readiness.".into());

    let resp = st
        .ai
        .complete(
            Some("You are verifying connectivity to the vLLM Anthropic Messages API."),
            &[ChatTurn {
                role: "user".to_string(),
                content: prompt,
            }],
            128,
            0.0,
        )
        .await?;

    Ok(Json(json!({
        "ok": true,
        "provider": st.config.ai_provider,
        "model": resp.model,
        "response": resp.text,
        "input_tokens": resp.input_tokens,
        "output_tokens": resp.output_tokens,
        "latency_ms": resp.latency_ms
    })))
}

// ---------- Scientific extraction ----------

async fn ai_extract_paper(
    State(st): State<AppState>,
    Path(paper_id): Path<String>,
) -> ApiResult<Json<Value>> {
    let paper = resolve_paper(&st.db, &paper_id).await?;
    let result = st.pipeline.extract_and_store_claims(paper.id).await?;
    let conditions = st.db.conditions_for_paper(paper.id).await?;
    let findings = st.db.findings_for_paper(paper.id).await?;

    Ok(Json(json!({
        "paper_id": paper.id,
        "ntrs_id": paper.ntrs_id,
        "title": paper.title,
        "extraction": result,
        "stored_conditions": conditions,
        "stored_findings": findings
    })))
}

#[derive(Debug, Deserialize, Default)]
struct BatchExtractRequest {
    paper_ids: Option<Vec<String>>,
    limit: Option<i64>,
}

#[derive(Debug, Serialize)]
struct BatchItemResult {
    paper_id: Uuid,
    ntrs_id: String,
    ok: bool,
    error: Option<String>,
}

async fn ai_extract_batch(
    State(st): State<AppState>,
    body: Option<Json<BatchExtractRequest>>,
) -> ApiResult<Json<Value>> {
    let req = body.map(|b| b.0).unwrap_or_default();
    let limit = req.limit.unwrap_or(5).clamp(1, 50);

    let papers = if let Some(ids) = req.paper_ids {
        let mut list = Vec::new();
        for id_str in ids {
            list.push(resolve_paper(&st.db, &id_str).await?);
        }
        list
    } else {
        let mut list = st.db.papers_needing_claims(limit).await?;
        if list.is_empty() {
            list = st.db.list_papers(None, limit).await?;
        }
        list
    };

    let mut results = Vec::with_capacity(papers.len());
    for p in papers {
        match st.pipeline.extract_and_store_claims(p.id).await {
            Ok(_) => {
                let _ = st.db.update_paper_status(p.id, "complete").await;
                results.push(BatchItemResult {
                    paper_id: p.id,
                    ntrs_id: p.ntrs_id,
                    ok: true,
                    error: None,
                });
            }
            Err(e) => results.push(BatchItemResult {
                paper_id: p.id,
                ntrs_id: p.ntrs_id,
                ok: false,
                error: Some(e.to_string()),
            }),
        }
    }

    let ok_count = results.iter().filter(|r| r.ok).count();
    Ok(Json(json!({
        "total": results.len(),
        "succeeded": ok_count,
        "results": results
    })))
}

// ---------- Experiment summaries ----------

async fn ai_summarize_experiment(
    State(st): State<AppState>,
    Path(experiment_id): Path<String>,
) -> ApiResult<Json<Value>> {
    let summary = st.rag.summarize_experiment(&experiment_id).await?;
    Ok(Json(serde_json::to_value(summary).map_err(|e| Error::Pipeline(e.to_string()))?))
}

// ---------- Research assistant (JSON + SSE streaming) ----------

#[derive(Debug, Deserialize)]
struct ChatRequest {
    message: String,
    conversation_id: Option<Uuid>,
    experiment: Option<String>,
    #[serde(default)]
    stream: bool,
}

async fn ai_chat(
    State(st): State<AppState>,
    Json(req): Json<ChatRequest>,
) -> ApiResult<Response> {
    if req.message.trim().is_empty() {
        return Err(Error::BadRequest("message cannot be empty".into()).into());
    }

    if req.stream {
        let (conv_id, citations, mut token_stream) = st
            .rag
            .chat_stream(req.conversation_id, &req.message, req.experiment.as_deref())
            .await?;
        let db = st.db.clone();

        let sse_stream = stream! {
            let meta = json!({
                "type": "meta",
                "conversation_id": conv_id,
                "citations": citations,
                "model": st.config.vllm_model
            });
            yield Ok::<Event, Infallible>(Event::default().event("meta").data(meta.to_string()));

            let mut full_reply = String::new();
            while let Some(item) = token_stream.next().await {
                match item {
                    Ok(delta) => {
                        full_reply.push_str(&delta);
                        let payload = json!({ "type": "delta", "text": delta });
                        yield Ok(Event::default().event("delta").data(payload.to_string()));
                    }
                    Err(e) => {
                        let err_json = json!({ "type": "error", "error": e.to_string() });
                        yield Ok(Event::default().event("error").data(err_json.to_string()));
                        break;
                    }
                }
            }

            let _ = db.append_message(conv_id, "assistant", &full_reply, &citations).await;
            yield Ok(Event::default().event("done").data(json!({ "type": "done", "conversation_id": conv_id }).to_string()));
        };

        return Ok(Sse::new(sse_stream)
            .keep_alive(KeepAlive::default())
            .into_response());
    }

    let ans = st
        .rag
        .chat(req.conversation_id, &req.message, req.experiment.as_deref())
        .await?;
    Ok(Json(ans).into_response())
}

// ---------- Experiment comparison ----------

#[derive(Debug, Deserialize)]
struct CompareRequest {
    experiments: Vec<String>,
    focus: Option<String>,
}

async fn ai_compare(
    State(st): State<AppState>,
    Json(req): Json<CompareRequest>,
) -> ApiResult<Json<Value>> {
    let ans = st
        .rag
        .compare_experiments(&req.experiments, req.focus.as_deref())
        .await?;
    Ok(Json(serde_json::to_value(ans).map_err(|e| Error::Pipeline(e.to_string()))?))
}

// ---------- Habitat analysis ----------

async fn ai_habitat_analyze(
    State(st): State<AppState>,
    Json(scenario): Json<HabitatScenario>,
) -> ApiResult<Json<Value>> {
    let ans = st.rag.analyze_habitat(&scenario).await?;
    Ok(Json(serde_json::to_value(ans).map_err(|e| Error::Pipeline(e.to_string()))?))
}

// ---------- Research question answering ----------

#[derive(Debug, Deserialize)]
struct ResearchQueryRequest {
    question: String,
    experiment: Option<String>,
    top_k: Option<usize>,
}

async fn ai_query(
    State(st): State<AppState>,
    Json(req): Json<ResearchQueryRequest>,
) -> ApiResult<Json<Value>> {
    if req.question.trim().is_empty() {
        return Err(Error::BadRequest("question cannot be empty".into()).into());
    }
    let ans = st
        .rag
        .query_research(
            &req.question,
            req.experiment.as_deref(),
            req.top_k.unwrap_or(10).clamp(1, 30),
        )
        .await?;
    Ok(Json(serde_json::to_value(ans).map_err(|e| Error::Pipeline(e.to_string()))?))
}

// ---------- Supporting data, verification/correction & pipeline routes ----------

async fn list_experiments(State(st): State<AppState>) -> ApiResult<Json<Value>> {
    let exps = st.db.list_experiments().await?;
    Ok(Json(json!({ "experiments": exps })))
}

#[derive(Debug, Deserialize, Default)]
struct PapersQuery {
    status: Option<String>,
    limit: Option<i64>,
}

async fn list_papers(
    State(st): State<AppState>,
    Query(q): Query<PapersQuery>,
) -> ApiResult<Json<Value>> {
    let papers = st
        .db
        .list_papers(q.status.as_deref(), q.limit.unwrap_or(50).clamp(1, 200))
        .await?;
    Ok(Json(json!({ "papers": papers })))
}

async fn get_paper_extractions(
    State(st): State<AppState>,
    Path(paper_id): Path<String>,
) -> ApiResult<Json<Value>> {
    let paper = resolve_paper(&st.db, &paper_id).await?;
    let experiments = st.db.experiments_for_paper(paper.id).await?;
    let conditions = st.db.conditions_for_paper(paper.id).await?;
    let findings = st.db.findings_for_paper(paper.id).await?;
    Ok(Json(json!({
        "paper": paper,
        "experiments": experiments,
        "conditions": conditions,
        "findings": findings
    })))
}

async fn patch_condition(
    State(st): State<AppState>,
    Path(condition_id): Path<Uuid>,
    Json(patch): Json<ConditionPatch>,
) -> ApiResult<Json<Value>> {
    let updated = st
        .db
        .update_condition(condition_id, &patch)
        .await?
        .ok_or_else(|| Error::NotFound(format!("condition {condition_id}")))?;
    Ok(Json(json!({ "condition": updated })))
}

async fn patch_finding(
    State(st): State<AppState>,
    Path(finding_id): Path<Uuid>,
    Json(patch): Json<FindingPatch>,
) -> ApiResult<Json<Value>> {
    let updated = st
        .db
        .update_finding(finding_id, &patch)
        .await?
        .ok_or_else(|| Error::NotFound(format!("finding {finding_id}")))?;
    Ok(Json(json!({ "finding": updated })))
}

#[derive(Debug, Deserialize, Default)]
struct IngestRequest {
    program: Option<String>,
    limit_per_program: Option<usize>,
    process_jobs: Option<usize>,
}

async fn trigger_ingest(
    State(st): State<AppState>,
    body: Option<Json<IngestRequest>>,
) -> ApiResult<Json<Value>> {
    let req = body.map(|b| b.0).unwrap_or_default();
    let mut summary = st
        .pipeline
        .discover(req.program.as_deref(), req.limit_per_program.unwrap_or(5))
        .await?;
    let max_jobs = req.process_jobs.unwrap_or(0);
    if max_jobs > 0 {
        summary.processed = st.pipeline.run_worker_until_empty(max_jobs).await?;
    }
    Ok(Json(serde_json::to_value(summary).map_err(|e| Error::Pipeline(e.to_string()))?))
}

#[derive(Debug, Deserialize)]
struct SheetImportRequest {
    url: String,
}

async fn import_sheet(
    State(st): State<AppState>,
    Json(req): Json<SheetImportRequest>,
) -> ApiResult<Json<Value>> {
    let ntrs = ntrs::NtrsClient::new(&st.config)?;
    let rec = ntrs.import_google_sheet_url(&st.db, &req.url).await?;
    Ok(Json(serde_json::to_value(rec).map_err(|e| Error::Pipeline(e.to_string()))?))
}

#[derive(Debug, Deserialize, Default)]
struct NasaScrapeRequest {
    urls: Option<Vec<(String, String)>>,
}

async fn scrape_nasa_experiments(
    State(st): State<AppState>,
    body: Option<Json<NasaScrapeRequest>>,
) -> ApiResult<Json<Value>> {
    let req = body.map(|b| b.0).unwrap_or_default();
    let ntrs = ntrs::NtrsClient::new(&st.config)?;
    let experiments = ntrs.sync_nasa_experiment_metadata(&st.db, req.urls).await?;
    Ok(Json(json!({ "experiments": experiments })))
}

async fn pipeline_status(State(st): State<AppState>) -> ApiResult<Json<Value>> {
    let papers = st.db.paper_status_counts().await?;
    let jobs = st.db.job_status_counts().await?;
    let experiments = st.db.list_experiments().await?.len();
    Ok(Json(json!({
        "experiments_count": experiments,
        "papers_by_status": papers,
        "jobs_by_kind_status": jobs
    })))
}

async fn resolve_paper(db: &Db, id_or_ntrs: &str) -> Result<db::repo_papers::Paper, Error> {
    if let Ok(uuid) = Uuid::parse_str(id_or_ntrs) {
        db.get_paper(uuid)
            .await?
            .ok_or_else(|| Error::NotFound(format!("paper {uuid}")))
    } else {
        db.get_paper_by_ntrs_id(id_or_ntrs)
            .await?
            .ok_or_else(|| Error::NotFound(format!("paper ntrs_id={id_or_ntrs}")))
    }
}
