use crate::Db;
use chrono::{DateTime, Utc};
use serde::Serialize;
use shared::Result;
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct WorkflowRun {
    pub id: Uuid,
    pub goal: String,
    pub status: String,
    pub max_steps: i32,
    pub result_summary: Option<String>,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

impl Db {
    pub async fn create_workflow_run(&self, goal: &str, max_steps: i32) -> Result<WorkflowRun> {
        Ok(sqlx::query_as::<_, WorkflowRun>(
            "INSERT INTO workflow_runs (goal, max_steps) VALUES ($1, $2) RETURNING *",
        )
        .bind(goal)
        .bind(max_steps)
        .fetch_one(self.pool())
        .await?)
    }

    pub async fn finish_workflow_run(
        &self,
        id: Uuid,
        status: &str,
        summary: Option<&str>,
        error: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE workflow_runs SET status = $2, result_summary = $3, error = $4, finished_at = now() WHERE id = $1",
        )
        .bind(id)
        .bind(status)
        .bind(summary)
        .bind(error)
        .execute(self.pool())
        .await?;
        Ok(())
    }
}
