use ai_engine::AiClient;
use api::{build_router, AppState};
use clap::{Parser, Subcommand};
use db::Db;
use ntrs::NtrsClient;
use pipeline::PipelineService;
use rag::RagEngine;
use shared::{types::ChatTurn, AppConfig};
use tracing::info;
use tracing_subscriber::{fmt, EnvFilter};
use uuid::Uuid;

#[derive(Parser)]
#[command(
    name = "fire-intel",
    about = "NASA Microgravity Combustion Research & Spacecraft Fire Safety Intelligence CLI"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run PostgreSQL migrations and seed the 13 NASA microgravity combustion programs
    InitDb,
    /// Verify Tailscale vLLM Anthropic Messages endpoint connectivity and model availability
    AiHealth,
    /// Send a smoke-test prompt to the vLLM /v1/messages endpoint
    AiTest {
        #[arg(long, default_value = "Reply with VLLM_ANTHROPIC_OK and list 3 microgravity combustion parameters.")]
        prompt: String,
    },
    /// Discover NASA NTRS publications and optionally run full PDF + AI extraction
    Ingest {
        #[arg(long)]
        program: Option<String>,
        #[arg(long, default_value_t = 3)]
        limit: usize,
        #[arg(long, default_value_t = 0)]
        process_jobs: usize,
    },
    /// Process queued background ingestion/extraction jobs
    Worker {
        #[arg(long, default_value_t = 25)]
        max_jobs: usize,
    },
    /// Run structured AI extraction on a specific paper UUID or NTRS ID
    Extract {
        paper_id: String,
    },
    /// Import a Google Sheet (published URL or local CSV path) of experiments and NTRS links
    ImportSheet {
        url_or_path: String,
    },
    /// Generate a grounded scientific summary for an experiment acronym or UUID
    Summarize {
        experiment: String,
    },
    /// Ask a grounded research question across ingested NASA NTRS papers
    Query {
        question: String,
        #[arg(long)]
        experiment: Option<String>,
        #[arg(long, default_value_t = 8)]
        top_k: usize,
    },
    /// Start the Axum HTTP API server
    Serve,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    fmt().with_env_filter(filter).init();

    let cli = Cli::parse();
    let config = AppConfig::from_env();
    config.assert_supported();

    let db = Db::connect(&config.database_url).await?;
    let ai = AiClient::new(config.clone(), Some(db.clone()))?;
    let pipeline = PipelineService::new(config.clone(), db.clone(), ai.clone())?;
    let rag = RagEngine::new(config.clone(), db.clone(), ai.clone());

    match cli.command {
        Commands::InitDb => {
            let exps = db.list_experiments().await?;
            println!("Database migrated and seeded with {} NASA combustion programs.", exps.len());
        }
        Commands::AiHealth => {
            let health = ai.health().await;
            println!("{}", serde_json::to_string_pretty(&health)?);
            if !health.ok {
                std::process::exit(1);
            }
        }
        Commands::AiTest { prompt } => {
            let resp = ai
                .complete(
                    Some("You are a NASA microgravity combustion research assistant."),
                    &[ChatTurn {
                        role: "user".to_string(),
                        content: prompt,
                    }],
                    256,
                    0.0,
                )
                .await?;
            println!("{}", serde_json::to_string_pretty(&resp)?);
        }
        Commands::Ingest {
            program,
            limit,
            process_jobs,
        } => {
            let mut summary = pipeline.discover(program.as_deref(), limit).await?;
            if process_jobs > 0 {
                summary.processed = pipeline.run_worker_until_empty(process_jobs).await?;
            }
            println!("{}", serde_json::to_string_pretty(&summary)?);
        }
        Commands::Worker { max_jobs } => {
            let ran = pipeline.run_worker_until_empty(max_jobs).await?;
            println!("Processed {ran} queued pipeline jobs.");
        }
        Commands::Extract { paper_id } => {
            let paper = if let Ok(uuid) = Uuid::parse_str(&paper_id) {
                db.get_paper(uuid).await?
            } else {
                db.get_paper_by_ntrs_id(&paper_id).await?
            }
            .ok_or_else(|| anyhow::anyhow!("paper '{}' not found", paper_id))?;

            let updated = pipeline.process_paper(paper.id, true).await?;
            let conds = db.conditions_for_paper(updated.id).await?;
            let finds = db.findings_for_paper(updated.id).await?;
            println!(
                "Extracted {} conditions and {} findings for NTRS {} ({})",
                conds.len(),
                finds.len(),
                updated.ntrs_id,
                updated.title
            );
        }
        Commands::ImportSheet { url_or_path } => {
            let ntrs = NtrsClient::new(&config)?;
            let rec = ntrs.import_google_sheet(&db, &url_or_path).await?;
            println!("{}", serde_json::to_string_pretty(&rec)?);
        }
        Commands::Summarize { experiment } => {
            let ans = rag.summarize_experiment(&experiment).await?;
            println!("{}", serde_json::to_string_pretty(&ans)?);
        }
        Commands::Query {
            question,
            experiment,
            top_k,
        } => {
            let ans = rag
                .query_research(&question, experiment.as_deref(), top_k)
                .await?;
            println!("{}", serde_json::to_string_pretty(&ans)?);
        }
        Commands::Serve => {
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
        }
    }

    Ok(())
}
