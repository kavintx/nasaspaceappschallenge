//! Flame in Freefall — fire safety insights from NASA microgravity combustion data.
//! A Rust (Axum) web server. It reads the research database in Supabase and
//! calls the fire-intel backend for AI answers and pipeline actions.

mod admin;
mod ai;
mod air;
mod backend;
mod catalog;
mod check;
mod config;
mod store;
mod views;

use axum::{
    routing::{get, post},
    Router,
};
use std::sync::Arc;
use tower_http::services::ServeDir;

pub struct AppState {
    /// The research database (Supabase), read-only.
    pub store: store::Store,
    /// The fire-intel backend (AI, pipeline, editing). None until API_AUTH_TOKEN is set.
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
        store: store::Store::from_cfg(&cfg),
        backend: backend::Backend::from_cfg(&cfg),
        catalog: tokio::sync::Mutex::new(None),
        read_only: cfg.get("READ_ONLY").is_some_and(|v| v.eq_ignore_ascii_case("true") || v == "1"),
    });
    if state.backend.is_none() {
        println!("AI backend: not connected. Browsing works; AI answers and the Pipeline page need API_AUTH_TOKEN in web/.env.");
    }

    let static_dir = cfg.get("STATIC_DIR").unwrap_or_else(|| "static".into());
    let app = Router::new()
        .route("/", get(check::home))
        .route("/check", get(check::check))
        .route("/mission", get(check::mission))
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

    // Use PORT if set; otherwise start at 3100 and step past ports that are taken.
    let fixed = cfg.get("PORT").and_then(|p| p.trim().parse::<u16>().ok());
    let candidates: Vec<u16> = match fixed {
        Some(p) => vec![p],
        None => (3100..3110).collect(),
    };
    let mut bound = None;
    for port in candidates {
        if let Ok(l) = tokio::net::TcpListener::bind(("0.0.0.0", port)).await {
            bound = Some((l, port));
            break;
        }
        println!("port {port} is already in use");
    }
    let Some((listener, port)) = bound else {
        eprintln!("Could not find a free port. Close the other program or set PORT in web/.env.");
        std::process::exit(1);
    };
    println!("Flame in Freefall running on http://localhost:{port}");
    axum::serve(listener, app).await.expect("server error");
}
