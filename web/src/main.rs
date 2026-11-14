//! Flame in Freefall — fire safety insights from NASA microgravity combustion data.
//! A Rust (Axum) web server that renders the dashboard and reads from Supabase.

mod data;
mod views;

use axum::{routing::get, Router};
use std::sync::Arc;
use tower_http::services::ServeDir;

pub struct AppState {
    pub data: data::Source,
    /// Optional AI endpoint for "Ask the Data" (Gus's /ask service).
    pub ask_api_url: Option<String>,
    pub http: reqwest::Client,
}

#[tokio::main]
async fn main() {
    let state = Arc::new(AppState {
        data: data::Source::from_env(),
        ask_api_url: std::env::var("ASK_API_URL").ok().filter(|s| !s.is_empty()),
        http: reqwest::Client::new(),
    });

    let static_dir = std::env::var("STATIC_DIR").unwrap_or_else(|_| "static".into());
    let app = Router::new()
        .route("/", get(views::habitat))
        .route("/explorer", get(views::explorer))
        .route("/experiments/{id}", get(views::detail))
        .route("/compare", get(views::compare))
        .route("/ask", get(views::ask))
        .route("/healthz", get(|| async { "ok" }))
        .nest_service("/static", ServeDir::new(static_dir))
        .fallback(views::not_found)
        .with_state(state);

    let port = std::env::var("PORT").unwrap_or_else(|_| "3000".into());
    let addr = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await.expect("bind failed");
    println!("Flame in Freefall running on http://localhost:{port}");
    axum::serve(listener, app).await.expect("server error");
}
