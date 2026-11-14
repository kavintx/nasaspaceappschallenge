use ai_engine::AiClient;
use api::{build_router, AppState};
use db::Db;
use pipeline::PipelineService;
use rag::RagEngine;
use shared::AppConfig;
use tracing::info;
use tracing_subscriber::{fmt, EnvFilter};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    fmt().with_env_filter(filter).init();

    let config = AppConfig::from_env();
    config.assert_supported();

    let db = Db::connect(&config.database_url).await?;

    let ai = AiClient::new(config.clone(), Some(db.clone()))?;
    let pipeline = PipelineService::new(config.clone(), db.clone(), ai.clone())?;
    let rag = RagEngine::new(config.clone(), db.clone(), ai.clone());

    let bind_addr = config.bind_addr.clone();
    let state = AppState {
        config,
        db,
        ai,
        pipeline,
        rag,
    };

    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    info!(addr = %bind_addr, "fire-intel API server listening");
    axum::serve(listener, app).await?;
    Ok(())
}
