use crate::Db;
use chrono::{DateTime, Utc};
use serde::Serialize;
use shared::Result;
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct Paper {
    pub id: Uuid,
    pub ntrs_id: String,
    pub title: String,
    pub r#abstract: String,
    pub authors: serde_json::Value,
    pub publication_date: Option<String>,
    pub doi: Option<String>,
    pub ntrs_url: String,
    pub pdf_url: Option<String>,
    pub pdf_path: Option<String>,
    pub pub_type: String,
    pub fulltext: String,
    pub pages_total: Option<i32>,
    pub discovery_query: String,
    pub status: String,
    pub error: Option<String>,
    pub retry_count: i32,
    pub first_seen_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Db {
    /// Insert-or-ignore a discovered paper. Returns (paper, newly_inserted).
    pub async fn upsert_paper(
        &self,
        ntrs_id: &str,
        title: &str,
        r#abstract: &str,
        authors: &serde_json::Value,
        publication_date: Option<&str>,
        doi: Option<&str>,
        ntrs_url: &str,
        pdf_url: Option<&str>,
        pub_type: &str,
        discovery_query: &str,
    ) -> Result<(Paper, bool)> {
        let row: Option<Paper> = sqlx::query_as(
            "INSERT INTO papers (ntrs_id, title, abstract, authors, publication_date, doi,
                                 ntrs_url, pdf_url, pub_type, discovery_query, status)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,'metadata')
             ON CONFLICT (ntrs_id) DO UPDATE SET
                 title = EXCLUDED.title,
                 abstract = EXCLUDED.abstract,
                 authors = EXCLUDED.authors,
                 doi = COALESCE(papers.doi, EXCLUDED.doi),
                 pdf_url = COALESCE(papers.pdf_url, EXCLUDED.pdf_url),
                 updated_at = now()
             RETURNING *, (xmax = 0) AS inserted",
        )
        .bind(ntrs_id)
        .bind(title)
        .bind(r#abstract)
        .bind(authors)
        .bind(publication_date)
        .bind(doi)
        .bind(ntrs_url)
        .bind(pdf_url)
        .bind(pub_type)
        .bind(discovery_query)
        .fetch_optional(self.pool())
        .await?;
        let paper = row.ok_or_else(|| {
            shared::Error::Pipeline(format!("upsert_paper returned no row for {ntrs_id}"))
        })?;
        let is_new = paper.retry_count == 0;
        Ok((paper, is_new))
    }

    pub async fn get_paper(&self, id: Uuid) -> Result<Option<Paper>> {
        Ok(sqlx::query_as::<_, Paper>("SELECT * FROM papers WHERE id = $1")
            .bind(id)
            .fetch_optional(self.pool())
            .await?)
    }

    pub async fn get_paper_by_ntrs_id(&self, ntrs_id: &str) -> Result<Option<Paper>> {
        Ok(sqlx::query_as::<_, Paper>("SELECT * FROM papers WHERE ntrs_id = $1")
            .bind(ntrs_id)
            .fetch_optional(self.pool())
            .await?)
    }

    pub async fn list_papers(&self, status: Option<&str>, limit: i64) -> Result<Vec<Paper>> {
        let rows = sqlx::query_as::<_, Paper>(
            "SELECT * FROM papers WHERE ($1::text IS NULL OR status = $1)
             ORDER BY updated_at ASC LIMIT $2",
        )
        .bind(status)
        .bind(limit)
        .fetch_all(self.pool())
        .await?;
        Ok(rows)
    }

    pub async fn update_paper_status(&self, id: Uuid, status: &str) -> Result<()> {
        sqlx::query("UPDATE papers SET status = $2, updated_at = now() WHERE id = $1")
            .bind(id)
            .bind(status)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn set_paper_error(&self, id: Uuid, err: &str) -> Result<()> {
        sqlx::query(
            "UPDATE papers SET status='failed', error=$2, retry_count=retry_count+1,
             updated_at=now() WHERE id=$1",
        )
        .bind(id)
        .bind(err)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn set_paper_pdf(&self, id: Uuid, pdf_path: &str, pages: i32) -> Result<()> {
        sqlx::query(
            "UPDATE papers SET pdf_path=$2, pages_total=$3, status='downloaded',
             updated_at=now() WHERE id=$1",
        )
        .bind(id)
        .bind(pdf_path)
        .bind(pages)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn set_paper_fulltext(&self, id: Uuid, text: &str) -> Result<()> {
        sqlx::query("UPDATE papers SET fulltext=$2, status='extracted', updated_at=now() WHERE id=$1")
            .bind(id)
            .bind(text)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn paper_status_counts(&self) -> Result<Vec<(String, i64)>> {
        let rows = sqlx::query("SELECT status, count(*) FROM papers GROUP BY status")
            .fetch_all(self.pool())
            .await?;
        Ok(rows
            .iter()
            .map(|r: &sqlx::postgres::PgRow| {
                use sqlx::Row;
                (
                    r.try_get::<String, _>("status").unwrap_or_default(),
                    r.try_get::<i64, _>("count").unwrap_or(0),
                )
            })
            .collect())
    }

    /// Papers that have text but no chunks yet.
    pub async fn papers_needing_chunks(&self, limit: i64) -> Result<Vec<Paper>> {
        Ok(sqlx::query_as::<_, Paper>(
            "SELECT p.* FROM papers p
             WHERE p.fulltext <> '' AND p.status IN ('extracted','chunked')
               AND NOT EXISTS (SELECT 1 FROM paper_chunks c WHERE c.paper_id = p.id)
             ORDER BY p.updated_at LIMIT $1",
        )
        .bind(limit)
        .fetch_all(self.pool())
        .await?)
    }

    /// Papers whose claims have not been AI-extracted yet (status 'matched').
    pub async fn papers_needing_claims(&self, limit: i64) -> Result<Vec<Paper>> {
        Ok(sqlx::query_as::<_, Paper>(
            "SELECT * FROM papers WHERE status = 'matched' ORDER BY updated_at LIMIT $1",
        )
        .bind(limit)
        .fetch_all(self.pool())
        .await?)
    }
}
