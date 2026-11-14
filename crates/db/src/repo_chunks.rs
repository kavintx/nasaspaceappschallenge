use crate::{pgvector_dim_check, vector_to_pg, Db};
use serde::Serialize;
use shared::{types::PageChunk, Result};
use sqlx::Row;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
pub struct ChunkRow {
    pub id: Uuid,
    pub paper_id: Uuid,
    pub ordinal: i32,
    pub page_start: Option<i32>,
    pub page_end: Option<i32>,
    pub char_start: i32,
    pub char_end: i32,
    pub text: String,
    pub embedding_status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScoredChunk {
    pub chunk_id: Uuid,
    pub paper_id: Uuid,
    pub ntrs_id: String,
    pub title: String,
    pub page_start: Option<i32>,
    pub page_end: Option<i32>,
    pub text: String,
    pub score: f64,
    pub origin: String,
}

impl Db {
    pub async fn replace_paper_chunks(&self, paper_id: Uuid, chunks: &[PageChunk]) -> Result<usize> {
        let mut tx = self.pool().begin().await?;
        sqlx::query("DELETE FROM paper_chunks WHERE paper_id = $1")
            .bind(paper_id)
            .execute(&mut *tx)
            .await?;

        for c in chunks {
            sqlx::query(
                "INSERT INTO paper_chunks (paper_id, ordinal, page_start, page_end, char_start, char_end, text)
                 VALUES ($1,$2,$3,$4,$5,$6,$7)",
            )
            .bind(paper_id)
            .bind(c.ordinal)
            .bind(c.page_start)
            .bind(c.page_end)
            .bind(c.char_start)
            .bind(c.char_end)
            .bind(&c.text)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(chunks.len())
    }

    pub async fn list_paper_chunks(&self, paper_id: Uuid) -> Result<Vec<ChunkRow>> {
        let rows = sqlx::query(
            "SELECT id, paper_id, ordinal, page_start, page_end, char_start, char_end, text, embedding_status
             FROM paper_chunks WHERE paper_id = $1 ORDER BY ordinal ASC",
        )
        .bind(paper_id)
        .fetch_all(self.pool())
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| ChunkRow {
                id: r.get("id"),
                paper_id: r.get("paper_id"),
                ordinal: r.get("ordinal"),
                page_start: r.get("page_start"),
                page_end: r.get("page_end"),
                char_start: r.get("char_start"),
                char_end: r.get("char_end"),
                text: r.get("text"),
                embedding_status: r.get("embedding_status"),
            })
            .collect())
    }

    pub async fn chunks_needing_embeddings(&self, limit: i64) -> Result<Vec<ChunkRow>> {
        let rows = sqlx::query(
            "SELECT id, paper_id, ordinal, page_start, page_end, char_start, char_end, text, embedding_status
             FROM paper_chunks
             WHERE embedding_status IN ('pending','unavailable','failed')
             ORDER BY paper_id, ordinal ASC LIMIT $1",
        )
        .bind(limit)
        .fetch_all(self.pool())
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| ChunkRow {
                id: r.get("id"),
                paper_id: r.get("paper_id"),
                ordinal: r.get("ordinal"),
                page_start: r.get("page_start"),
                page_end: r.get("page_end"),
                char_start: r.get("char_start"),
                char_end: r.get("char_end"),
                text: r.get("text"),
                embedding_status: r.get("embedding_status"),
            })
            .collect())
    }

    pub async fn set_chunk_embedding(
        &self,
        chunk_id: Uuid,
        expected_dim: usize,
        embedding: &[f32],
        model: &str,
    ) -> Result<()> {
        pgvector_dim_check(expected_dim, embedding.len())?;
        let pg_vec = vector_to_pg(embedding);
        sqlx::query(
            "UPDATE paper_chunks
             SET embedding = $2::vector, embedding_model = $3, embedding_status = 'ready'
             WHERE id = $1",
        )
        .bind(chunk_id)
        .bind(pg_vec)
        .bind(model)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn set_chunk_embedding_status(&self, chunk_id: Uuid, status: &str) -> Result<()> {
        sqlx::query("UPDATE paper_chunks SET embedding_status = $2 WHERE id = $1")
            .bind(chunk_id)
            .bind(status)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn search_chunks_keyword(
        &self,
        query_text: &str,
        experiment_acronym: Option<&str>,
        limit: i64,
    ) -> Result<Vec<ScoredChunk>> {
        let rows = sqlx::query(
            "SELECT c.id AS chunk_id, c.paper_id, p.ntrs_id, p.title, c.page_start, c.page_end, c.text,
                    ts_rank_cd(c.text_tsv, websearch_to_tsquery('english', $1))::float8 AS score
             FROM paper_chunks c
             JOIN papers p ON p.id = c.paper_id
             WHERE c.text_tsv @@ websearch_to_tsquery('english', $1)
               AND ($2::text IS NULL OR EXISTS (
                   SELECT 1 FROM experiment_papers ep
                   JOIN experiments e ON e.id = ep.experiment_id
                   WHERE ep.paper_id = p.id AND upper(e.acronym) = upper($2)
               ))
             ORDER BY score DESC
             LIMIT $3",
        )
        .bind(query_text)
        .bind(experiment_acronym)
        .bind(limit)
        .fetch_all(self.pool())
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| ScoredChunk {
                chunk_id: r.get("chunk_id"),
                paper_id: r.get("paper_id"),
                ntrs_id: r.get("ntrs_id"),
                title: r.get("title"),
                page_start: r.get("page_start"),
                page_end: r.get("page_end"),
                text: r.get("text"),
                score: r.get::<f64, _>("score"),
                origin: "keyword".to_string(),
            })
            .collect())
    }

    pub async fn search_chunks_vector(
        &self,
        expected_dim: usize,
        embedding: &[f32],
        experiment_acronym: Option<&str>,
        limit: i64,
    ) -> Result<Vec<ScoredChunk>> {
        pgvector_dim_check(expected_dim, embedding.len())?;
        let pg_vec = vector_to_pg(embedding);
        let rows = sqlx::query(
            "SELECT c.id AS chunk_id, c.paper_id, p.ntrs_id, p.title, c.page_start, c.page_end, c.text,
                    (1.0 - (c.embedding <=> $1::vector))::float8 AS score
             FROM paper_chunks c
             JOIN papers p ON p.id = c.paper_id
             WHERE c.embedding IS NOT NULL AND c.embedding_status = 'ready'
               AND ($2::text IS NULL OR EXISTS (
                   SELECT 1 FROM experiment_papers ep
                   JOIN experiments e ON e.id = ep.experiment_id
                   WHERE ep.paper_id = p.id AND upper(e.acronym) = upper($2)
               ))
             ORDER BY c.embedding <=> $1::vector ASC
             LIMIT $3",
        )
        .bind(pg_vec)
        .bind(experiment_acronym)
        .bind(limit)
        .fetch_all(self.pool())
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| ScoredChunk {
                chunk_id: r.get("chunk_id"),
                paper_id: r.get("paper_id"),
                ntrs_id: r.get("ntrs_id"),
                title: r.get("title"),
                page_start: r.get("page_start"),
                page_end: r.get("page_end"),
                text: r.get("text"),
                score: r.get::<f64, _>("score"),
                origin: "vector".to_string(),
            })
            .collect())
    }
}
