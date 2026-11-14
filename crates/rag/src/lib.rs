use ai_engine::{AiClient, TextStream};
use db::{repo_chunks::ScoredChunk, Db};
use serde::{Deserialize, Serialize};
use shared::{
    types::{ChatTurn, Citation, Evidence},
    AppConfig, Error, Result,
};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Clone)]
pub struct RagEngine {
    pub config: AppConfig,
    pub db: Db,
    pub ai: AiClient,
}

#[derive(Debug, Clone, Serialize)]
pub struct GroundedAnswer {
    pub answer: String,
    pub citations: Vec<Citation>,
    pub evidence_count: usize,
    pub insufficient_evidence: bool,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<Uuid>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HabitatScenario {
    pub habitat_name: Option<String>,
    pub oxygen_percent: Option<f64>,
    pub pressure_kpa: Option<f64>,
    pub airflow_cm_s: Option<f64>,
    pub materials: Vec<String>,
    pub sensors: Option<Vec<String>>,
    pub suppression_system: Option<String>,
    pub question: Option<String>,
}

const GROUNDED_SYSTEM_PROMPT: &str = r#"You are a NASA microgravity spacecraft fire safety research assistant.
Answer strictly from the retrieved NASA NTRS evidence provided in the prompt.
RULES:
1. Cite every factual claim inline using [NTRS <ntrs_id>, p.<page>] (or [NTRS <ntrs_id>] if page is unknown).
2. Distinguish reported experimental values from inferred extrapolations.
3. If the retrieved evidence is missing or insufficient to answer part or all of the question, explicitly state "INSUFFICIENT EVIDENCE:" and explain what data is missing. NEVER invent or fabricate experimental numbers."#;

impl RagEngine {
    pub fn new(config: AppConfig, db: Db, ai: AiClient) -> Self {
        Self { config, db, ai }
    }

    /// Hybrid retrieval: combines structured DB records (conditions + findings),
    /// PostgreSQL full-text search, and pgvector cosine similarity (when available).
    pub async fn retrieve(
        &self,
        query: &str,
        experiment_acronym: Option<&str>,
        top_k: usize,
    ) -> Result<Vec<Evidence>> {
        let mut evidence: Vec<Evidence> = Vec::new();

        // 1. Structured records for target experiment(s)
        if let Some(acr) = experiment_acronym {
            if let Some(exp) = self.db.get_experiment_by_acronym(acr).await? {
                let papers = self.db.papers_for_experiment(exp.id).await?;
                let paper_map: HashMap<Uuid, (String, String)> = papers
                    .into_iter()
                    .map(|p| (p.id, (p.ntrs_id, p.title)))
                    .collect();

                for c in self.db.conditions_for_experiment(exp.id).await?.into_iter().take(8) {
                    if let Some((ntrs_id, title)) = paper_map.get(&c.paper_id) {
                        let summary = format!(
                            "[Structured Condition | {} | status={} | provenance={} | conf={:.2}] fuel={:?}, geometry={:?}, O2={:?}, pressure={:?}, airflow={:?}, sensors={}, objective={:?}",
                            exp.acronym,
                            c.verification_status,
                            c.provenance_kind,
                            c.confidence,
                            c.fuel_type,
                            c.fuel_geometry,
                            c.oxygen_concentration,
                            c.pressure,
                            c.airflow,
                            c.sensors.unwrap_or_default(),
                            c.objective
                        );
                        evidence.push(Evidence {
                            paper_id: c.paper_id,
                            ntrs_id: ntrs_id.clone(),
                            title: title.clone(),
                            page: c.source_page,
                            text: summary,
                            score: 0.90 + (c.confidence as f64 * 0.10),
                            origin: "structured".to_string(),
                        });
                    }
                }

                for f in self.db.findings_for_experiment(exp.id).await?.into_iter().take(10) {
                    if let Some((ntrs_id, title)) = paper_map.get(&f.paper_id) {
                        let summary = format!(
                            "[Structured Finding | {} | category={:?} | status={} | provenance={}] {}",
                            exp.acronym, f.category, f.verification_status, f.provenance_kind, f.text
                        );
                        evidence.push(Evidence {
                            paper_id: f.paper_id,
                            ntrs_id: ntrs_id.clone(),
                            title: title.clone(),
                            page: f.source_page,
                            text: summary,
                            score: 0.88 + (f.confidence as f64 * 0.10),
                            origin: "structured".to_string(),
                        });
                    }
                }
            }
        }

        // 2. Keyword full-text search over paper_chunks
        let kw_hits = self
            .db
            .search_chunks_keyword(query, experiment_acronym, (top_k * 2) as i64)
            .await
            .unwrap_or_default();

        // 3. Vector search via pgvector if vLLM supports /v1/embeddings
        let vec_hits = match self.ai.embed(&[query.to_string()]).await {
            Ok(Some(mut vecs)) if !vecs.is_empty() => {
                let q_vec = vecs.remove(0);
                self.db
                    .search_chunks_vector(
                        self.config.vllm_embed_dim,
                        &q_vec,
                        experiment_acronym,
                        (top_k * 2) as i64,
                    )
                    .await
                    .unwrap_or_default()
            }
            _ => vec![],
        };

        // Reciprocal Rank Fusion over chunk hits
        let mut rrf_scores: HashMap<Uuid, (f64, ScoredChunk)> = HashMap::new();
        for (rank, chunk) in kw_hits.into_iter().enumerate() {
            let rrf = 1.0 / (60.0 + (rank as f64) + 1.0);
            rrf_scores
                .entry(chunk.chunk_id)
                .and_modify(|(s, _)| *s += rrf)
                .or_insert((rrf, chunk));
        }
        for (rank, chunk) in vec_hits.into_iter().enumerate() {
            let rrf = 1.2 / (60.0 + (rank as f64) + 1.0);
            rrf_scores
                .entry(chunk.chunk_id)
                .and_modify(|(s, c)| {
                    *s += rrf;
                    c.origin = "hybrid".to_string();
                })
                .or_insert((rrf, chunk));
        }

        let mut ranked_chunks: Vec<(f64, ScoredChunk)> = rrf_scores.into_values().collect();
        ranked_chunks.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        for (score, c) in ranked_chunks.into_iter().take(top_k) {
            evidence.push(Evidence {
                paper_id: c.paper_id,
                ntrs_id: c.ntrs_id,
                title: c.title,
                page: c.page_start,
                text: c.text,
                score,
                origin: c.origin,
            });
        }

        evidence.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        evidence.truncate(top_k);
        Ok(evidence)
    }

    /// Summarize a specific experiment from its linked NTRS papers, extracted conditions, and findings.
    pub async fn summarize_experiment(&self, experiment_id_or_acronym: &str) -> Result<GroundedAnswer> {
        let exp = if let Ok(uuid) = Uuid::parse_str(experiment_id_or_acronym) {
            self.db.get_experiment(uuid).await?
        } else {
            self.db.get_experiment_by_acronym(experiment_id_or_acronym).await?
        }
        .ok_or_else(|| Error::NotFound(format!("experiment '{experiment_id_or_acronym}'")))?;

        let evidence = self
            .retrieve(
                &format!("{} {} microgravity combustion flammability findings", exp.acronym, exp.name),
                Some(&exp.acronym),
                12,
            )
            .await?;

        if evidence.is_empty() {
            return Ok(GroundedAnswer {
                answer: format!(
                    "INSUFFICIENT EVIDENCE: No ingested NASA NTRS papers or extracted records are linked to {} ({}) yet.",
                    exp.acronym, exp.name
                ),
                citations: vec![],
                evidence_count: 0,
                insufficient_evidence: true,
                model: self.config.vllm_model.clone(),
                conversation_id: None,
            });
        }

        let context_block = format_evidence_block(&evidence);
        let prompt = format!(
            "Provide a comprehensive scientific summary of NASA experiment {} ({}).\n\
             Cover: (1) Experimental objective & hardware, (2) Tested fuels, geometry, oxygen concentration, pressure, and airflow, \
             (3) Sensors & diagnostics, (4) Key findings categorized by prevention, detection, and suppression, and (5) Any reported data gaps.\n\n\
             Retrieved Evidence:\n{}",
            exp.acronym, exp.name, context_block
        );

        let resp = self
            .ai
            .complete(
                Some(GROUNDED_SYSTEM_PROMPT),
                &[ChatTurn {
                    role: "user".to_string(),
                    content: prompt,
                }],
                2048,
                0.2,
            )
            .await?;

        let insufficient = resp.text.contains("INSUFFICIENT EVIDENCE");
        let citations: Vec<Citation> = evidence.iter().map(|e| e.citation(220)).collect();

        Ok(GroundedAnswer {
            answer: resp.text,
            citations,
            evidence_count: evidence.len(),
            insufficient_evidence: insufficient,
            model: resp.model,
            conversation_id: None,
        })
    }

    /// Research assistant chat with persistent conversation history and RAG citations.
    pub async fn chat(
        &self,
        conversation_id: Option<Uuid>,
        message: &str,
        experiment_acronym: Option<&str>,
    ) -> Result<GroundedAnswer> {
        let title: String = message.chars().take(60).collect();
        let conv = self.db.get_or_create_conversation(conversation_id, &title).await?;
        let mut history = self.db.list_conversation_turns(conv.id, 10).await?;

        let evidence = self.retrieve(message, experiment_acronym, 10).await?;
        let citations: Vec<Citation> = evidence.iter().map(|e| e.citation(220)).collect();

        self.db
            .append_message(conv.id, "user", message, &[])
            .await?;

        let user_with_rag = format!(
            "Question: {}\n\nRetrieved NASA NTRS Evidence (cite using [NTRS <id>, p.<page>]):\n{}",
            message,
            if evidence.is_empty() {
                "NO MATCHING RECORDS FOUND IN DATABASE.".to_string()
            } else {
                format_evidence_block(&evidence)
            }
        );
        history.push(ChatTurn {
            role: "user".to_string(),
            content: user_with_rag,
        });

        let resp = self
            .ai
            .complete(Some(GROUNDED_SYSTEM_PROMPT), &history, 2048, 0.2)
            .await?;

        self.db
            .append_message(conv.id, "assistant", &resp.text, &citations)
            .await?;

        let insufficient = evidence.is_empty() || resp.text.contains("INSUFFICIENT EVIDENCE");
        Ok(GroundedAnswer {
            answer: resp.text,
            citations,
            evidence_count: evidence.len(),
            insufficient_evidence: insufficient,
            model: resp.model,
            conversation_id: Some(conv.id),
        })
    }

    /// Streaming research assistant chat returning (conversation_id, citations, TextStream).
    pub async fn chat_stream(
        &self,
        conversation_id: Option<Uuid>,
        message: &str,
        experiment_acronym: Option<&str>,
    ) -> Result<(Uuid, Vec<Citation>, TextStream)> {
        let title: String = message.chars().take(60).collect();
        let conv = self.db.get_or_create_conversation(conversation_id, &title).await?;
        let mut history = self.db.list_conversation_turns(conv.id, 10).await?;

        let evidence = self.retrieve(message, experiment_acronym, 10).await?;
        let citations: Vec<Citation> = evidence.iter().map(|e| e.citation(220)).collect();

        self.db
            .append_message(conv.id, "user", message, &[])
            .await?;

        let user_with_rag = format!(
            "Question: {}\n\nRetrieved NASA NTRS Evidence (cite using [NTRS <id>, p.<page>]):\n{}",
            message,
            if evidence.is_empty() {
                "NO MATCHING RECORDS FOUND IN DATABASE.".to_string()
            } else {
                format_evidence_block(&evidence)
            }
        );
        history.push(ChatTurn {
            role: "user".to_string(),
            content: user_with_rag,
        });

        let stream = self
            .ai
            .stream_messages(Some(GROUNDED_SYSTEM_PROMPT), &history, 2048, 0.2)
            .await?;

        Ok((conv.id, citations, stream))
    }

    /// Compare two or more NASA microgravity combustion experiments.
    pub async fn compare_experiments(
        &self,
        experiments: &[String],
        focus: Option<&str>,
    ) -> Result<GroundedAnswer> {
        if experiments.len() < 2 {
            return Err(Error::BadRequest(
                "compare requires at least two experiment acronyms or IDs".into(),
            ));
        }

        let mut all_evidence = Vec::new();
        let mut resolved_names = Vec::new();
        for item in experiments {
            let exp = if let Ok(uuid) = Uuid::parse_str(item) {
                self.db.get_experiment(uuid).await?
            } else {
                self.db.get_experiment_by_acronym(item).await?
            };
            if let Some(e) = exp {
                resolved_names.push(format!("{} ({})", e.acronym, e.name));
                let q = focus.unwrap_or("experimental conditions fuel oxygen pressure airflow findings");
                let mut ev = self.retrieve(q, Some(&e.acronym), 6).await?;
                all_evidence.append(&mut ev);
            }
        }

        if all_evidence.is_empty() {
            return Ok(GroundedAnswer {
                answer: "INSUFFICIENT EVIDENCE: No ingested records found for the requested experiments.".into(),
                citations: vec![],
                evidence_count: 0,
                insufficient_evidence: true,
                model: self.config.vllm_model.clone(),
                conversation_id: None,
            });
        }

        let prompt = format!(
            "Compare the following NASA microgravity combustion experiments: {}.\n\
             Focus area: {}\n\
             Compare fuels, geometry, oxygen concentration, pressure, airflow, sensors, and prevention/detection/suppression findings. \
             Highlight similarities, differences, and missing/unreported parameters.\n\n\
             Retrieved Evidence:\n{}",
            resolved_names.join(" vs "),
            focus.unwrap_or("all experimental parameters and fire safety findings"),
            format_evidence_block(&all_evidence)
        );

        let resp = self
            .ai
            .complete(
                Some(GROUNDED_SYSTEM_PROMPT),
                &[ChatTurn {
                    role: "user".to_string(),
                    content: prompt,
                }],
                2048,
                0.2,
            )
            .await?;

        let insufficient = resp.text.contains("INSUFFICIENT EVIDENCE");
        let citations = all_evidence.iter().map(|e| e.citation(220)).collect();

        Ok(GroundedAnswer {
            answer: resp.text,
            citations,
            evidence_count: all_evidence.len(),
            insufficient_evidence: insufficient,
            model: resp.model,
            conversation_id: None,
        })
    }

    /// Analyze spacecraft habitat fire safety against NASA microgravity combustion findings.
    pub async fn analyze_habitat(&self, scenario: &HabitatScenario) -> Result<GroundedAnswer> {
        let search_query = format!(
            "microgravity flammability oxygen {} kPa {} materials {} detection suppression",
            scenario.oxygen_percent.map(|o| format!("{o}%")).unwrap_or_default(),
            scenario.pressure_kpa.map(|p| format!("{p}")).unwrap_or_default(),
            scenario.materials.join(" ")
        );

        let evidence = self.retrieve(&search_query, None, 12).await?;
        let citations = evidence.iter().map(|e| e.citation(220)).collect();

        let prompt = format!(
            "Analyze the spacecraft habitat fire risk and mitigation posture for the following configuration using ONLY retrieved NASA microgravity combustion evidence:\n\
             - Habitat: {}\n\
             - Oxygen Concentration (%): {:?}\n\
             - Total Pressure (kPa): {:?}\n\
             - Ventilation Airflow (cm/s): {:?}\n\
             - Materials Present: {:?}\n\
             - Sensors Configured: {:?}\n\
             - Suppression System: {:?}\n\
             - Specific Question: {}\n\n\
             Structure your analysis into: (1) Ignition & Flame Spread Risk, (2) Prevention Recommendations, \
             (3) Detection Recommendations, (4) Suppression Recommendations, and (5) Evidence Gaps / Unverified Regime Extrapolations.\n\n\
             Retrieved Evidence:\n{}",
            scenario.habitat_name.as_deref().unwrap_or("Unspecified Spacecraft Habitat"),
            scenario.oxygen_percent,
            scenario.pressure_kpa,
            scenario.airflow_cm_s,
            scenario.materials,
            scenario.sensors,
            scenario.suppression_system,
            scenario.question.as_deref().unwrap_or("Evaluate overall fire safety posture."),
            if evidence.is_empty() {
                "NO MATCHING RECORDS FOUND IN DATABASE.".to_string()
            } else {
                format_evidence_block(&evidence)
            }
        );

        let resp = self
            .ai
            .complete(
                Some(GROUNDED_SYSTEM_PROMPT),
                &[ChatTurn {
                    role: "user".to_string(),
                    content: prompt,
                }],
                2048,
                0.2,
            )
            .await?;

        let insufficient = evidence.is_empty() || resp.text.contains("INSUFFICIENT EVIDENCE");
        Ok(GroundedAnswer {
            answer: resp.text,
            citations,
            evidence_count: evidence.len(),
            insufficient_evidence: insufficient,
            model: resp.model,
            conversation_id: None,
        })
    }

    /// Single-turn grounded research question answering.
    pub async fn query_research(
        &self,
        question: &str,
        experiment_acronym: Option<&str>,
        top_k: usize,
    ) -> Result<GroundedAnswer> {
        let evidence = self.retrieve(question, experiment_acronym, top_k).await?;
        let citations = evidence.iter().map(|e| e.citation(220)).collect();

        let prompt = format!(
            "Research Question: {}\n\nRetrieved NASA NTRS Evidence:\n{}",
            question,
            if evidence.is_empty() {
                "NO MATCHING RECORDS FOUND IN DATABASE.".to_string()
            } else {
                format_evidence_block(&evidence)
            }
        );

        let resp = self
            .ai
            .complete(
                Some(GROUNDED_SYSTEM_PROMPT),
                &[ChatTurn {
                    role: "user".to_string(),
                    content: prompt,
                }],
                2048,
                0.15,
            )
            .await?;

        let insufficient = evidence.is_empty() || resp.text.contains("INSUFFICIENT EVIDENCE");
        Ok(GroundedAnswer {
            answer: resp.text,
            citations,
            evidence_count: evidence.len(),
            insufficient_evidence: insufficient,
            model: resp.model,
            conversation_id: None,
        })
    }
}

fn format_evidence_block(evidence: &[Evidence]) -> String {
    evidence
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let page_str = e
                .page
                .map(|p| format!("p.{p}"))
                .unwrap_or_else(|| "p.unknown".to_string());
            format!(
                "[Evidence {} | NTRS {} | {} | {} | origin={}]\n{}",
                i + 1, e.ntrs_id, e.title, page_str, e.origin, e.text
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}
