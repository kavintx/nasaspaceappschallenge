use crate::Db;
use chrono::{DateTime, Utc};
use serde::Serialize;
use shared::Result;
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct SheetImportRecord {
    pub id: Uuid,
    pub sheet_url: String,
    pub total_rows: i32,
    pub ok_rows: i32,
    pub errors: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

impl Db {
    pub async fn record_sheet_import(
        &self,
        sheet_url: &str,
        total_rows: i32,
        ok_rows: i32,
        errors: &[String],
    ) -> Result<SheetImportRecord> {
        let errs = serde_json::to_value(errors)?;
        Ok(sqlx::query_as::<_, SheetImportRecord>(
            "INSERT INTO sheet_imports (sheet_url, total_rows, ok_rows, errors)
             VALUES ($1, $2, $3, $4)
             RETURNING *",
        )
        .bind(sheet_url)
        .bind(total_rows)
        .bind(ok_rows)
        .bind(errs)
        .fetch_one(self.pool())
        .await?)
    }

    pub async fn list_sheet_imports(&self, limit: i64) -> Result<Vec<SheetImportRecord>> {
        Ok(sqlx::query_as::<_, SheetImportRecord>(
            "SELECT * FROM sheet_imports ORDER BY created_at DESC LIMIT $1",
        )
        .bind(limit)
        .fetch_all(self.pool())
        .await?)
    }
}
