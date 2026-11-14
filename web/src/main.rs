//! Flame in Freefall — fire safety insights from NASA microgravity combustion data.
//! A Rust (Axum) web server that renders the dashboard from the fire-intel research backend.

mod admin;
mod ai;
mod backend;
mod catalog;
mod config;
mod views;

use axum::{
    routing::{get, post},
    Router,
};
use std::sync::Arc;
use tower_http::services::ServeDir;

pub struct AppState {
    /// The fire-intel research backend (papers, findings, AI). None until API_AUTH_TOKEN is set.
    pub backend: Option<backend::Backend>,
    /// Cached experiment catalog and when it was built.
    pub catalog: tokio::sync::Mutex<Option<Arc<catalog::Catalog>>>,
    /// When true (READ_ONLY=true), nothing that changes the database is allowed.
    pub read_only: bool,
}

#[tokio::main]
async fn main() {
    // Pure-Rust-friendly TLS (ring): builds on Windows without CMake or NASM.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let cfg = config::Config::load();
    let state = Arc::new(AppState {
        backend: backend::Backend::from_cfg(&cfg),
        catalog: tokio::sync::Mutex::new(None),
        read_only: cfg.get("READ_ONLY").is_some_and(|v| v.eq_ignore_ascii_case("true") || v == "1"),
    });
    if state.backend.is_none() {
        println!("research backend: not connected (set API_AUTH_TOKEN in web/.env)");
    }

    let static_dir = cfg.get("STATIC_DIR").unwrap_or_else(|| "static".into());
    let app = Router::new()
        .route("/", get(views::habitat))
        .route("/explorer", get(views::explorer))
        .route("/experiments/{id}", get(views::detail))
        .route("/compare", get(views::compare))
        .route("/ask", get(views::ask))
        .route("/papers", get(views::papers))
        .route("/papers/{id}", get(views::paper))
        .route("/ask/stream", post(admin::chat_stream))
        .route("/pipeline", get(admin::pipeline))
        .route("/pipeline/test", post(admin::test_model))
        .route("/pipeline/ingest", post(admin::ingest))
        .route("/pipeline/sheet", post(admin::sheet_import))
        .route("/pipeline/extract-batch", post(admin::extract_batch))
        .route("/papers/{id}/extract", post(admin::extract_paper))
        .route("/conditions/{id}", post(admin::edit_condition))
        .route("/findings/{id}", post(admin::edit_finding))
        .route("/healthz", get(|| async { "ok" }))
        .nest_service("/static", ServeDir::new(static_dir))
        .fallback(views::not_found)
        .with_state(state);

    let port = cfg.get("PORT").unwrap_or_else(|| "3100".into());
    let addr = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await.expect("bind failed");
    println!("Flame in Freefall running on http://localhost:{port}");
    axum::serve(listener, app).await.expect("server error");
}
