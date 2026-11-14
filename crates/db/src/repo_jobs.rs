use crate::Db;
use chrono::{DateTime, Utc};
use serde::Serialize;
use shared::Result;
use sqlx::{FromRow, Row};
use uuid::Uuid;

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct PipelineJob {
    pub id: Uuid,
    pub kind: String,
    pub paper_id: Option<Uuid>,
    pub payload: serde_json::Value,
    pub status: String,
    pub attempts: i32,
    pub max_attempts: i32,
    pub error: Option<String>,
    pub available_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl Db {
    pub async fn enqueue_job(
        &self,
        kind: &str,
        paper_id: Option<Uuid>,
        payload: serde_json::Value,
    ) -> Result<PipelineJob> {
        // Avoid duplicate queued/running jobs of the same kind for the same paper
        if let Some(pid) = paper_id {
            if let Some(existing) = sqlx::query_as::<_, PipelineJob>(
                "SELECT * FROM pipeline_jobs
                 WHERE kind = $1 AND paper_id = $2 AND status IN ('queued','running')
                 LIMIT 1",
            )
            .bind(kind)
            .bind(pid)
            .fetch_optional(self.pool())
            .await?
            {
                return Ok(existing);
            }
        }

        Ok(sqlx::query_as::<_, PipelineJob>(
            "INSERT INTO pipeline_jobs (kind, paper_id, payload) VALUES ($1, $2, $3) RETURNING *",
        )
        .bind(kind)
        .bind(paper_id)
        .bind(payload)
        .fetch_one(self.pool())
        .await?)
    }

    pub async fn claim_next_job(&self) -> Result<Option<PipelineJob>> {
        Ok(sqlx::query_as::<_, PipelineJob>(
            "UPDATE pipeline_jobs
             SET status = 'running', attempts = attempts + 1, started_at = now()
             WHERE id = (
                 SELECT id FROM pipeline_jobs
                 WHERE (status = 'queued' OR (status = 'failed' AND attempts < max_attempts))
                   AND available_at <= now()
                 ORDER BY created_at ASC
                 FOR UPDATE SKIP LOCKED
                 LIMIT 1
             )
             RETURNING *",
        )
        .fetch_optional(self.pool())
        .await?)
    }

    pub async fn complete_job(&self, id: Uuid) -> Result<()> {
        sqlx::query(
            "UPDATE pipeline_jobs SET status = 'done', error = NULL, finished_at = now() WHERE id = $1",
        )
        .bind(id)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn fail_job(&self, id: Uuid, err: &str, backoff_secs: i64) -> Result<()> {
        sqlx::query(
            "UPDATE pipeline_jobs
             SET status = 'failed',
                 error = $2,
                 available_at = now() + make_interval(secs => $3::double precision),
                 finished_at = now()
             WHERE id = $1",
        )
        .bind(id)
        .bind(err)
        .bind(backoff_secs as f64)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn retry_failed_jobs(&self) -> Result<u64> {
        let res = sqlx::query(
            "UPDATE pipeline_jobs
             SET status = 'queued', attempts = 0, error = NULL, available_at = now()
             WHERE status = 'failed'",
        )
        .execute(self.pool())
        .await?;
        Ok(res.rows_affected())
    }

    pub async fn retry_failed_papers(&self) -> Result<u64> {
        let res = sqlx::query(
            "UPDATE papers SET status = 'metadata', error = NULL, updated_at = now()
             WHERE status = 'failed'",
        )
        .execute(self.pool())
        .await?;
        Ok(res.rows_affected())
    }

    pub async fn job_status_counts(&self) -> Result<Vec<(String, String, i64)>> {
        let rows = sqlx::query(
            "SELECT kind, status, count(*) AS cnt FROM pipeline_jobs GROUP BY kind, status ORDER BY kind, status",
        )
        .fetch_all(self.pool())
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| (r.get("kind"), r.get("status"), r.get("cnt")))
            .collect())
    }

    pub async fn log_ai_call(
        &self,
        route: &str,
        model: Option<&str>,
        http_status: Option<i32>,
        ok: bool,
        in_tokens: Option<i32>,
        out_tokens: Option<i32>,
        latency_ms: i32,
        error: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO ai_call_log (route, model, http_status, ok, in_tokens, out_tokens, latency_ms, error)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
        )
        .bind(route)
        .bind(model)
        .bind(http_status)
        .bind(ok)
        .bind(in_tokens)
        .bind(out_tokens)
        .bind(latency_ms)
        .bind(error)
        .execute(self.pool())
        .await?;
        Ok(())
    }
}
