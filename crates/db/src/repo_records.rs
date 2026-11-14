use crate::Db;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use shared::{types::{ExtractedCondition, ExtractedFinding}, Result};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct Experiment {
    pub id: Uuid,
    pub acronym: String,
    pub name: String,
    pub aliases: serde_json::Value,
    pub description: String,
    pub source: String,
    pub verification_status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct ConditionRow {
    pub id: Uuid,
    pub paper_id: Uuid,
    pub experiment_id: Option<Uuid>,
    pub fuel_type: Option<String>,
    pub fuel_geometry: Option<String>,
    pub oxygen_concentration: Option<String>,
    pub pressure: Option<String>,
    pub airflow: Option<String>,
    pub sensors: Option<serde_json::Value>,
    pub objective: Option<String>,
    pub source_page: Option<i32>,
    pub source_quote: Option<String>,
    pub provenance_kind: String,
    pub confidence: f32,
    pub verification_status: String,
    pub correction_note: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct FindingRow {
    pub id: Uuid,
    pub paper_id: Uuid,
    pub experiment_id: Option<Uuid>,
    pub text: String,
    pub category: Option<String>,
    pub source_page: Option<i32>,
    pub source_quote: Option<String>,
    pub provenance_kind: String,
    pub confidence: f32,
    pub verification_status: String,
    pub correction_note: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ConditionPatch {
    pub fuel_type: Option<String>,
    pub fuel_geometry: Option<String>,
    pub oxygen_concentration: Option<String>,
    pub pressure: Option<String>,
    pub airflow: Option<String>,
    pub sensors: Option<serde_json::Value>,
    pub objective: Option<String>,
    pub source_page: Option<i32>,
    pub verification_status: Option<String>,
    pub correction_note: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct FindingPatch {
    pub text: Option<String>,
    pub category: Option<String>,
    pub source_page: Option<i32>,
    pub verification_status: Option<String>,
    pub correction_note: Option<String>,
}

impl Db {
    pub async fn list_experiments(&self) -> Result<Vec<Experiment>> {
        Ok(
            sqlx::query_as::<_, Experiment>("SELECT * FROM experiments ORDER BY acronym ASC")
                .fetch_all(self.pool())
                .await?,
        )
    }

    pub async fn get_experiment(&self, id: Uuid) -> Result<Option<Experiment>> {
        Ok(
            sqlx::query_as::<_, Experiment>("SELECT * FROM experiments WHERE id = $1")
                .bind(id)
                .fetch_optional(self.pool())
                .await?,
        )
    }

    pub async fn get_experiment_by_acronym(&self, acronym: &str) -> Result<Option<Experiment>> {
        Ok(sqlx::query_as::<_, Experiment>(
            "SELECT * FROM experiments WHERE upper(acronym) = upper($1)",
        )
        .bind(acronym.trim())
        .fetch_optional(self.pool())
        .await?)
    }

    pub async fn upsert_experiment(
        &self,
        acronym: &str,
        name: &str,
        description: &str,
        source: &str,
    ) -> Result<Experiment> {
        let clean_acronym = acronym.trim().to_uppercase();
        let row = sqlx::query_as::<_, Experiment>(
            "INSERT INTO experiments (acronym, name, description, source)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (acronym) DO UPDATE SET
                 name = CASE WHEN experiments.name = '' THEN EXCLUDED.name ELSE experiments.name END,
                 description = CASE WHEN experiments.description = '' THEN EXCLUDED.description ELSE experiments.description END,
                 updated_at = now()
             RETURNING *",
        )
        .bind(&clean_acronym)
        .bind(name.trim())
        .bind(description.trim())
        .bind(source)
        .fetch_one(self.pool())
        .await?;
        Ok(row)
    }

    pub async fn upsert_experiment_overwrite(
        &self,
        acronym: &str,
        name: &str,
        description: &str,
        source: &str,
    ) -> Result<Experiment> {
        let clean_acronym = acronym.trim().to_uppercase();
        let row = sqlx::query_as::<_, Experiment>(
            "INSERT INTO experiments (acronym, name, description, source)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (acronym) DO UPDATE SET
                 name = CASE WHEN EXCLUDED.name <> '' THEN EXCLUDED.name ELSE experiments.name END,
                 description = CASE WHEN EXCLUDED.description <> '' THEN EXCLUDED.description ELSE experiments.description END,
                 source = EXCLUDED.source,
                 updated_at = now()
             RETURNING *",
        )
        .bind(&clean_acronym)
        .bind(name.trim())
        .bind(description.trim())
        .bind(source)
        .fetch_one(self.pool())
        .await?;
        Ok(row)
    }

    pub async fn link_experiment_paper(
        &self,
        experiment_id: Uuid,
        paper_id: Uuid,
        match_method: &str,
        confidence: f32,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO experiment_papers (experiment_id, paper_id, match_method, confidence)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (experiment_id, paper_id) DO UPDATE SET
                 confidence = GREATEST(experiment_papers.confidence, EXCLUDED.confidence)",
        )
        .bind(experiment_id)
        .bind(paper_id)
        .bind(match_method)
        .bind(confidence)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn experiments_for_paper(&self, paper_id: Uuid) -> Result<Vec<Experiment>> {
        Ok(sqlx::query_as::<_, Experiment>(
            "SELECT e.* FROM experiments e
             JOIN experiment_papers ep ON ep.experiment_id = e.id
             WHERE ep.paper_id = $1
             ORDER BY ep.confidence DESC, e.acronym ASC",
        )
        .bind(paper_id)
        .fetch_all(self.pool())
        .await?)
    }

    pub async fn papers_for_experiment(&self, experiment_id: Uuid) -> Result<Vec<crate::repo_papers::Paper>> {
        Ok(sqlx::query_as::<_, crate::repo_papers::Paper>(
            "SELECT p.* FROM papers p
             JOIN experiment_papers ep ON ep.paper_id = p.id
             WHERE ep.experiment_id = $1
             ORDER BY p.publication_date DESC NULLS LAST, p.title ASC",
        )
        .bind(experiment_id)
        .fetch_all(self.pool())
        .await?)
    }

    pub async fn replace_paper_extractions(
        &self,
        paper_id: Uuid,
        primary_experiment_id: Option<Uuid>,
        conditions: &[ExtractedCondition],
        findings: &[ExtractedFinding],
    ) -> Result<()> {
        let mut tx = self.pool().begin().await?;
        // Preserve manually verified or corrected records; replace pending/rejected ones.
        sqlx::query(
            "DELETE FROM conditions WHERE paper_id = $1 AND verification_status IN ('pending','rejected')",
        )
        .bind(paper_id)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "DELETE FROM findings WHERE paper_id = $1 AND verification_status IN ('pending','rejected')",
        )
        .bind(paper_id)
        .execute(&mut *tx)
        .await?;

        for c in conditions {
            let prov = serde_json::to_value(c.provenance)
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_else(|| "extracted".to_string());
            let sensors_json = serde_json::to_value(&c.sensors)?;
            sqlx::query(
                "INSERT INTO conditions (
                    paper_id, experiment_id, fuel_type, fuel_geometry, oxygen_concentration,
                    pressure, airflow, sensors, objective, source_page, source_quote,
                    provenance_kind, confidence, verification_status
                 ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,'pending')",
            )
            .bind(paper_id)
            .bind(primary_experiment_id)
            .bind(&c.fuel_type)
            .bind(&c.fuel_geometry)
            .bind(&c.oxygen_concentration)
            .bind(&c.pressure)
            .bind(&c.airflow)
            .bind(sensors_json)
            .bind(&c.objective)
            .bind(c.source_page)
            .bind(&c.source_quote)
            .bind(prov)
            .bind(c.confidence)
            .execute(&mut *tx)
            .await?;
        }

        for f in findings {
            let prov = serde_json::to_value(f.provenance)
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_else(|| "extracted".to_string());
            let cat = f.category.as_deref().and_then(|c| match c.to_lowercase().as_str() {
                "prevention" => Some("prevention"),
                "detection" => Some("detection"),
                "suppression" => Some("suppression"),
                "other" => Some("other"),
                _ => Some("other"),
            });
            sqlx::query(
                "INSERT INTO findings (
                    paper_id, experiment_id, text, category, source_page, source_quote,
                    provenance_kind, confidence, verification_status
                 ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,'pending')",
            )
            .bind(paper_id)
            .bind(primary_experiment_id)
            .bind(&f.text)
            .bind(cat)
            .bind(f.source_page)
            .bind(&f.source_quote)
            .bind(prov)
            .bind(f.confidence)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    pub async fn conditions_for_paper(&self, paper_id: Uuid) -> Result<Vec<ConditionRow>> {
        Ok(sqlx::query_as::<_, ConditionRow>(
            "SELECT * FROM conditions WHERE paper_id = $1 ORDER BY source_page NULLS LAST, created_at ASC",
        )
        .bind(paper_id)
        .fetch_all(self.pool())
        .await?)
    }

    pub async fn findings_for_paper(&self, paper_id: Uuid) -> Result<Vec<FindingRow>> {
        Ok(sqlx::query_as::<_, FindingRow>(
            "SELECT * FROM findings WHERE paper_id = $1 ORDER BY source_page NULLS LAST, created_at ASC",
        )
        .bind(paper_id)
        .fetch_all(self.pool())
        .await?)
    }

    pub async fn conditions_for_experiment(&self, experiment_id: Uuid) -> Result<Vec<ConditionRow>> {
        Ok(sqlx::query_as::<_, ConditionRow>(
            "SELECT DISTINCT c.* FROM conditions c
             LEFT JOIN experiment_papers ep ON ep.paper_id = c.paper_id
             WHERE c.experiment_id = $1 OR ep.experiment_id = $1
             ORDER BY c.confidence DESC, c.created_at ASC",
        )
        .bind(experiment_id)
        .fetch_all(self.pool())
        .await?)
    }

    pub async fn findings_for_experiment(&self, experiment_id: Uuid) -> Result<Vec<FindingRow>> {
        Ok(sqlx::query_as::<_, FindingRow>(
            "SELECT DISTINCT f.* FROM findings f
             LEFT JOIN experiment_papers ep ON ep.paper_id = f.paper_id
             WHERE f.experiment_id = $1 OR ep.experiment_id = $1
             ORDER BY f.confidence DESC, f.created_at ASC",
        )
        .bind(experiment_id)
        .fetch_all(self.pool())
        .await?)
    }

    pub async fn update_condition(&self, id: Uuid, patch: &ConditionPatch) -> Result<Option<ConditionRow>> {
        Ok(sqlx::query_as::<_, ConditionRow>(
            "UPDATE conditions SET
                fuel_type = COALESCE($2, fuel_type),
                fuel_geometry = COALESCE($3, fuel_geometry),
                oxygen_concentration = COALESCE($4, oxygen_concentration),
                pressure = COALESCE($5, pressure),
                airflow = COALESCE($6, airflow),
                sensors = COALESCE($7, sensors),
                objective = COALESCE($8, objective),
                source_page = COALESCE($9, source_page),
                verification_status = COALESCE($10, verification_status),
                correction_note = COALESCE($11, correction_note),
                updated_at = now()
             WHERE id = $1
             RETURNING *",
        )
        .bind(id)
        .bind(&patch.fuel_type)
        .bind(&patch.fuel_geometry)
        .bind(&patch.oxygen_concentration)
        .bind(&patch.pressure)
        .bind(&patch.airflow)
        .bind(&patch.sensors)
        .bind(&patch.objective)
        .bind(patch.source_page)
        .bind(&patch.verification_status)
        .bind(&patch.correction_note)
        .fetch_optional(self.pool())
        .await?)
    }

    pub async fn update_finding(&self, id: Uuid, patch: &FindingPatch) -> Result<Option<FindingRow>> {
        Ok(sqlx::query_as::<_, FindingRow>(
            "UPDATE findings SET
                text = COALESCE($2, text),
                category = COALESCE($3, category),
                source_page = COALESCE($4, source_page),
                verification_status = COALESCE($5, verification_status),
                correction_note = COALESCE($6, correction_note),
                updated_at = now()
             WHERE id = $1
             RETURNING *",
        )
        .bind(id)
        .bind(&patch.text)
        .bind(&patch.category)
        .bind(patch.source_page)
        .bind(&patch.verification_status)
        .bind(&patch.correction_note)
        .fetch_optional(self.pool())
        .await?)
    }
}
