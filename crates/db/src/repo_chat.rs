use crate::Db;
use chrono::{DateTime, Utc};
use serde::Serialize;
use shared::{types::{ChatTurn, Citation}, Result};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct Conversation {
    pub id: Uuid,
    pub title: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct ChatMessage {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub role: String,
    pub content: String,
    pub citations: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

impl Db {
    pub async fn create_conversation(&self, title: &str) -> Result<Conversation> {
        Ok(sqlx::query_as::<_, Conversation>(
            "INSERT INTO conversations (title) VALUES ($1) RETURNING *",
        )
        .bind(title)
        .fetch_one(self.pool())
        .await?)
    }

    pub async fn get_or_create_conversation(&self, id: Option<Uuid>, title: &str) -> Result<Conversation> {
        if let Some(cid) = id {
            if let Some(existing) = sqlx::query_as::<_, Conversation>(
                "SELECT * FROM conversations WHERE id = $1",
            )
            .bind(cid)
            .fetch_optional(self.pool())
            .await?
            {
                return Ok(existing);
            }
        }
        self.create_conversation(title).await
    }

    pub async fn append_message(
        &self,
        conversation_id: Uuid,
        role: &str,
        content: &str,
        citations: &[Citation],
    ) -> Result<ChatMessage> {
        let cits = serde_json::to_value(citations)?;
        Ok(sqlx::query_as::<_, ChatMessage>(
            "INSERT INTO messages (conversation_id, role, content, citations)
             VALUES ($1, $2, $3, $4)
             RETURNING *",
        )
        .bind(conversation_id)
        .bind(role)
        .bind(content)
        .bind(cits)
        .fetch_one(self.pool())
        .await?)
    }

    pub async fn list_conversation_turns(&self, conversation_id: Uuid, limit: i64) -> Result<Vec<ChatTurn>> {
        let rows = sqlx::query_as::<_, ChatMessage>(
            "SELECT * FROM (
                SELECT * FROM messages WHERE conversation_id = $1 ORDER BY created_at DESC LIMIT $2
             ) sub ORDER BY created_at ASC",
        )
        .bind(conversation_id)
        .bind(limit)
        .fetch_all(self.pool())
        .await?;

        Ok(rows
            .into_iter()
            .map(|m| ChatTurn {
                role: m.role,
                content: m.content,
            })
            .collect())
    }
}
