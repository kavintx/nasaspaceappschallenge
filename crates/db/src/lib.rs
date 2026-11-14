use shared::{Error, Result};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

pub mod repo_chat;
pub mod repo_chunks;
pub mod repo_jobs;
pub mod repo_papers;
pub mod repo_records;
pub mod repo_sheet;
pub mod repo_workflow;

#[derive(Clone)]
pub struct Db(pub PgPool);

impl Db {
    pub async fn connect(url: &str) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .acquire_timeout(std::time::Duration::from_secs(10))
            .connect(url)
            .await?;
        // Embed migrations from the repo root so containers need no extra files.
        sqlx::migrate!("../../migrations")
            .run(&pool)
            .await
            .map_err(|e| Error::Config(format!("migrations failed: {e}")))?;
        Ok(Self(pool))
    }

    pub fn pool(&self) -> &PgPool {
        &self.0
    }
}

/// Serialize a Vec<f32> into pgvector text format "[0.1,0.2,...]".
pub fn vector_to_pg(v: &[f32]) -> String {
    let inner = v
        .iter()
        .map(|x| format!("{x:.6}"))
        .collect::<Vec<_>>()
        .join(",");
    format!("[{inner}]")
}

pub fn pgvector_dim_check(expected: usize, got: usize) -> Result<()> {
    if expected != got {
        return Err(Error::Config(format!(
            "embedding dimension mismatch: expected {expected} (VLLM_EMBED_DIM), got {got}"
        )));
    }
    Ok(())
}
