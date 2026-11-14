use ai_engine::{extract_json_block, AiClient};
use db::{repo_papers::Paper, Db};
use lopdf::Document;
use ntrs::{NtrsClient, DEFAULT_PROGRAM_QUERIES};
use serde::Serialize;
use serde_json::json;
use shared::{
    types::{ChatTurn, ExtractedCondition, ExtractedFinding, ExtractionResult, PageChunk, ProvenanceKind},
    AppConfig, Error, Result,
};
use std::path::Path;
use tracing::{info, warn};
use uuid::Uuid;

#[derive(Clone)]
pub struct PipelineService {
    pub config: AppConfig,
    pub db: Db,
    pub ntrs: NtrsClient,
    pub ai: AiClient,
}

#[derive(Debug, Clone, Serialize)]
pub struct IngestSummary {
    pub discovered: usize,
    pub processed: usize,
    pub errors: Vec<String>,
}

impl PipelineService {
    pub fn new(config: AppConfig, db: Db, ai: AiClient) -> Result<Self> {
        let ntrs = NtrsClient::new(&config)?;
        Ok(Self {
            config,
            db,
            ntrs,
            ai,
        })
    }

    /// Stage 1 & 2: Discover NASA NTRS publications for a specific program or all 13 target programs.
    pub async fn discover(&self, program_filter: Option<&str>, limit_per_program: usize) -> Result<IngestSummary> {
        let mut discovered = 0usize;
        let mut errors = Vec::new();

        for (acronym, query) in DEFAULT_PROGRAM_QUERIES {
            if let Some(filter) = program_filter {
                if !acronym.eq_ignore_ascii_case(filter.trim()) {
                    continue;
                }
            }

            info!(program = acronym, query, "discovering NASA NTRS publications");
            match self.ntrs.search(query, limit_per_program, 0).await {
                Ok(pubs) => {
                    let exp = self.db.get_experiment_by_acronym(acronym).await?;
                    for p in pubs {
                        let authors_json = serde_json::to_value(&p.authors).unwrap_or_default();
                        match self
                            .db
                            .upsert_paper(
                                &p.ntrs_id,
                                &p.title,
                                &p.r#abstract,
                                &authors_json,
                                p.publication_date.as_deref(),
                                p.doi.as_deref(),
                                &p.ntrs_url,
                                p.pdf_url.as_deref(),
                                &p.pub_type,
                                query,
                            )
                            .await
                        {
                            Ok((paper, _)) => {
                                discovered += 1;
                                if let Some(ref e) = exp {
                                    let _ = self
                                        .db
                                        .link_experiment_paper(e.id, paper.id, "acronym", 0.95)
                                        .await;
                                }
                                if matches!(paper.status.as_str(), "discovered" | "metadata") {
                                    let _ = self
                                        .db
                                        .enqueue_job("ingest_paper", Some(paper.id), json!({}))
                                        .await;
                                }
                            }
                            Err(e) => errors.push(format!("upsert {}: {e}", p.ntrs_id)),
                        }
                    }
                }
                Err(e) => {
                    warn!(program = acronym, error = %e, "NTRS search failed");
                    errors.push(format!("search {acronym}: {e}"));
                }
            }
        }

        Ok(IngestSummary {
            discovered,
            processed: 0,
            errors,
        })
    }

    /// Run stages 2..11 for a single paper idempotently.
    pub async fn process_paper(&self, paper_id: Uuid, run_ai_extract: bool) -> Result<Paper> {
        let mut paper = self
            .db
            .get_paper(paper_id)
            .await?
            .ok_or_else(|| Error::NotFound(format!("paper {paper_id}")))?;

        // Stage 2: Enrich metadata from NTRS if abstract or pdf_url is missing
        if paper.r#abstract.is_empty() || paper.pdf_url.is_none() {
            if let Ok(meta) = self.ntrs.fetch_citation(&paper.ntrs_id).await {
                let authors_json = serde_json::to_value(&meta.authors).unwrap_or_default();
                let (updated, _) = self
                    .db
                    .upsert_paper(
                        &meta.ntrs_id,
                        if meta.title.is_empty() { &paper.title } else { &meta.title },
                        if meta.r#abstract.is_empty() { &paper.r#abstract } else { &meta.r#abstract },
                        &authors_json,
                        meta.publication_date.as_deref().or(paper.publication_date.as_deref()),
                        meta.doi.as_deref().or(paper.doi.as_deref()),
                        &meta.ntrs_url,
                        meta.pdf_url.as_deref().or(paper.pdf_url.as_deref()),
                        &meta.pub_type,
                        &paper.discovery_query,
                    )
                    .await?;
                paper = updated;
            }
        }

        // Stage 3 & 4: Download PDF (if available) and extract normalized page-tagged text
        let pages: Vec<(i32, String)> = if let Some(ref pdf_url) = paper.pdf_url {
            match self.ntrs.download_pdf(&paper.ntrs_id, pdf_url).await {
                Ok(path) => {
                    let extracted_pages = extract_pdf_pages(&path)?;
                    let total_pages = extracted_pages.len() as i32;
                    let path_str = path.to_string_lossy().to_string();
                    self.db.set_paper_pdf(paper.id, &path_str, total_pages).await?;
                    if extracted_pages.is_empty() && !paper.r#abstract.is_empty() {
                        vec![(1, normalize_text(&paper.r#abstract))]
                    } else {
                        extracted_pages
                    }
                }
                Err(e) => {
                    warn!(ntrs_id = %paper.ntrs_id, error = %e, "PDF download failed; falling back to NTRS abstract");
                    if !paper.r#abstract.is_empty() {
                        vec![(1, normalize_text(&paper.r#abstract))]
                    } else {
                        self.db.set_paper_error(paper.id, &e.to_string()).await?;
                        return Err(e);
                    }
                }
            }
        } else if !paper.r#abstract.is_empty() {
            vec![(1, normalize_text(&paper.r#abstract))]
        } else {
            let msg = "no PDF URL and empty abstract on NTRS".to_string();
            self.db.set_paper_error(paper.id, &msg).await?;
            return Err(Error::Pipeline(msg));
        };

        let combined_text = format_pages_with_markers(&pages);
        self.db.set_paper_fulltext(paper.id, &combined_text).await?;

        // Stage 5: Split into page-tracked searchable chunks
        let chunks = chunk_pages(&pages, 1400, 200);
        self.db.replace_paper_chunks(paper.id, &chunks).await?;
        self.db.update_paper_status(paper.id, "chunked").await?;

        // Stage 6: Match experiments discussed in title, abstract, and text
        self.match_experiments_for_paper(paper.id, &paper.title, &paper.r#abstract, &combined_text)
            .await?;
        self.db.update_paper_status(paper.id, "matched").await?;

        // Stage 11: Embed chunks if vLLM supports /v1/embeddings
        let _ = self.embed_pending_chunks(32).await;

        // Stage 7, 8, 9, 10: AI extraction + validation + PostgreSQL persistence
        if run_ai_extract {
            self.extract_and_store_claims(paper.id).await?;
            self.db.update_paper_status(paper.id, "complete").await?;
        }

        self.db
            .get_paper(paper.id)
            .await?
            .ok_or_else(|| Error::NotFound(format!("paper {paper_id}")))
    }

    /// Stage 6: Deterministic acronym + alias matching across all experiments.
    pub async fn match_experiments_for_paper(
        &self,
        paper_id: Uuid,
        title: &str,
        abstract_text: &str,
        fulltext: &str,
    ) -> Result<Vec<String>> {
        let experiments = self.db.list_experiments().await?;
        let header_haystack = format!("{} {}", title, abstract_text).to_uppercase();
        let body_haystack = fulltext.chars().take(25_000).collect::<String>().to_uppercase();
        let mut matched = Vec::new();

        for exp in experiments {
            let mut terms = vec![exp.acronym.to_uppercase(), exp.name.to_uppercase()];
            if let Some(arr) = exp.aliases.as_array() {
                for a in arr {
                    if let Some(s) = a.as_str() {
                        terms.push(s.to_uppercase());
                    }
                }
            }

            let in_header = terms
                .iter()
                .filter(|t| t.len() >= 3)
                .any(|t| contains_token(&header_haystack, t));
            let in_body = !in_header
                && terms
                    .iter()
                    .filter(|t| t.len() >= 4)
                    .any(|t| contains_token(&body_haystack, t));

            if in_header || in_body {
                let conf = if in_header { 0.95 } else { 0.75 };
                self.db
                    .link_experiment_paper(exp.id, paper_id, "acronym", conf)
                    .await?;
                matched.push(exp.acronym);
            }
        }
        Ok(matched)
    }

    /// Stages 7, 8, 9, 10: Call vLLM to extract structured experimental conditions & findings,
    /// validate every claim and page number, and store in PostgreSQL.
    pub async fn extract_and_store_claims(&self, paper_id: Uuid) -> Result<ExtractionResult> {
        let paper = self
            .db
            .get_paper(paper_id)
            .await?
            .ok_or_else(|| Error::NotFound(format!("paper {paper_id}")))?;

        let linked_exps = self.db.experiments_for_paper(paper_id).await?;
        let primary_exp_id = linked_exps.first().map(|e| e.id);
        let known_acronyms: Vec<String> = linked_exps.iter().map(|e| e.acronym.clone()).collect();

        let chunks = self.db.list_paper_chunks(paper_id).await?;
        let evidence_text = if chunks.is_empty() {
            format!("[PAGE 1]\
Title: {}\
Abstract: {}", paper.title, paper.r#abstract)
        } else {
            let mut acc = String::new();
            for c in chunks.iter().take(12) {
                let p_label = match (c.page_start, c.page_end) {
                    (Some(s), Some(e)) if s != e => format!("[PAGES {s}-{e}]"),
                    (Some(s), _) => format!("[PAGE {s}]"),
                    _ => "[PAGE 1]".to_string(),
                };
                let block = format!("{p_label}\
{}\
\
", c.text);
                if acc.len() + block.len() > 14_000 {
                    break;
                }
                acc.push_str(&block);
            }
            acc
        };

        let system_prompt = r#"You are a NASA microgravity combustion research extraction engine.
Extract ONLY facts explicitly supported by the provided NASA NTRS publication text.
CRITICAL RULES:
1. NEVER fabricate, guess, or fill in missing experimental parameters. Use null when a parameter is not reported.
2. Distinguish provenance_kind strictly:
   - "reported": explicitly stated with numerical or direct factual values in the text.
   - "extracted": directly pulled from prose/tables with high certainty.
   - "inferred": synthesized from context (must be flagged as inferred and given lower confidence <= 0.65).
   - "unknown": explicitly noted as unmeasured or unknown.
3. Every condition and finding MUST cite the exact 1-based `source_page` from the [PAGE N] markers and include a short verbatim `source_quote`.
4. Classify each finding `category` into one of: "prevention", "detection", "suppression", or "other".
5. Output ONLY valid JSON matching the exact schema below.

Schema:
{
  "experiment_acronyms": ["BASS"],
  "conditions": [
    {
      "fuel_type": "string or null",
      "fuel_geometry": "string or null",
      "oxygen_concentration": "string or null",
      "pressure": "string or null",
      "airflow": "string or null",
      "sensors": ["string"],
      "objective": "string or null",
      "source_page": 1,
      "source_quote": "verbatim quote from text",
      "provenance": "reported",
      "confidence": 0.92
    }
  ],
  "findings": [
    {
      "text": "concise factual claim",
      "category": "prevention|detection|suppression|other",
      "source_page": 1,
      "source_quote": "verbatim quote from text",
      "provenance": "reported",
      "confidence": 0.90
    }
  ],
  "data_gaps": ["list of parameters not reported in this paper"]
}"#;

        let user_prompt = format!(
            "NTRS ID: {}\
Title: {}\
Linked Experiments: {:?}\
\
Source Text:\
{}",
            paper.ntrs_id, paper.title, known_acronyms, evidence_text
        );

        let resp = self
            .ai
            .complete(
                Some(system_prompt),
                &[ChatTurn {
                    role: "user".to_string(),
                    content: user_prompt,
                }],
                2048,
                0.1,
            )
            .await?;

        let json_str = extract_json_block(&resp.text);
        let mut parsed: ExtractionResult = serde_json::from_str(json_str).map_err(|e| {
            Error::Pipeline(format!(
                "failed to parse AI extraction JSON for NTRS {}: {e}; raw: {}",
                paper.ntrs_id,
                resp.text.chars().take(400).collect::<String>()
            ))
        })?;

        // Stage 9: Validate extracted records against page bounds & non-fabrication rules
        validate_extraction(&mut parsed, paper.pages_total.unwrap_or(1));

        // Link any newly discovered experiment acronyms returned by AI
        for acr in &parsed.experiment_acronyms {
            if let Some(exp) = self.db.get_experiment_by_acronym(acr).await? {
                let _ = self
                    .db
                    .link_experiment_paper(exp.id, paper.id, "ai", 0.85)
                    .await;
            }
        }

        // Stage 10: Persist validated records to PostgreSQL (status = 'pending' for human review)
        self.db
            .replace_paper_extractions(paper.id, primary_exp_id, &parsed.conditions, &parsed.findings)
            .await?;
        self.db.update_paper_status(paper.id, "extracted_claims").await?;

        Ok(parsed)
    }

    /// Stage 11: Embed pending chunks if vLLM serves /v1/embeddings.
    pub async fn embed_pending_chunks(&self, batch_size: usize) -> Result<usize> {
        let chunks = self.db.chunks_needing_embeddings(batch_size as i64).await?;
        if chunks.is_empty() {
            return Ok(0);
        }
        let texts: Vec<String> = chunks.iter().map(|c| c.text.clone()).collect();
        match self.ai.embed(&texts).await? {
            Some(vecs) => {
                let mut count = 0;
                for (chunk, vec) in chunks.iter().zip(vecs.iter()) {
                    self.db
                        .set_chunk_embedding(
                            chunk.id,
                            self.config.vllm_embed_dim,
                            vec,
                            &self.config.vllm_embed_model,
                        )
                        .await?;
                    count += 1;
                }
                Ok(count)
            }
            None => {
                for chunk in &chunks {
                    self.db
                        .set_chunk_embedding_status(chunk.id, "unavailable")
                        .await?;
                }
                Ok(0)
            }
        }
    }

    /// Execute one queued/retryable pipeline job.
    pub async fn process_next_job(&self) -> Result<bool> {
        let Some(job) = self.db.claim_next_job().await? else {
            return Ok(false);
        };

        let res = match job.kind.as_str() {
            "ingest_paper" => {
                if let Some(pid) = job.paper_id {
                    self.process_paper(pid, true).await.map(|_| ())
                } else {
                    Err(Error::Pipeline("ingest_paper job missing paper_id".into()))
                }
            }
            "extract_claims" => {
                if let Some(pid) = job.paper_id {
                    self.extract_and_store_claims(pid).await.map(|_| ())
                } else {
                    Err(Error::Pipeline("extract_claims job missing paper_id".into()))
                }
            }
            "embed" => self.embed_pending_chunks(32).await.map(|_| ()),
            other => Err(Error::Pipeline(format!("unknown job kind: {other}"))),
        };

        match res {
            Ok(()) => {
                self.db.complete_job(job.id).await?;
            }
            Err(e) => {
                let backoff = 15 * (1i64 << job.attempts.min(5));
                self.db.fail_job(job.id, &e.to_string(), backoff).await?;
            }
        }
        Ok(true)
    }

    pub async fn run_worker_until_empty(&self, max_jobs: usize) -> Result<usize> {
        let mut ran = 0usize;
        while ran < max_jobs {
            if !self.process_next_job().await? {
                break;
            }
            ran += 1;
        }
        Ok(ran)
    }
}

/// Extract normalized text per 1-based PDF page using `lopdf`.
pub fn extract_pdf_pages(path: &Path) -> Result<Vec<(i32, String)>> {
    let doc = Document::load(path).map_err(|e| Error::Pdf(format!("lopdf load {}: {e}", path.display())))?;
    let pages = doc.get_pages();
    let mut out = Vec::with_capacity(pages.len());
    for (page_num, _object_id) in pages {
        let raw = doc.extract_text(&[page_num]).unwrap_or_default();
        let norm = normalize_text(&raw);
        if !norm.is_empty() {
            out.push((page_num as i32, norm));
        }
    }
    Ok(out)
}

pub fn normalize_text(raw: &str) -> String {
    raw.replace("\r\n", "\n")
        .replace("\u{0000}", "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn format_pages_with_markers(pages: &[(i32, String)]) -> String {
    pages
        .iter()
        .map(|(p, t)| format!("[PAGE {p}]\
{t}"))
        .collect::<Vec<_>>()
        .join("\
\
")
}

/// Split page-tagged text into overlapping character chunks preserving `page_start` and `page_end`.
pub fn chunk_pages(pages: &[(i32, String)], target_chars: usize, overlap_chars: usize) -> Vec<PageChunk> {
    let mut full = String::new();
    let mut page_spans: Vec<(i32, usize, usize)> = Vec::new();

    for (page_num, text) in pages {
        if text.trim().is_empty() {
            continue;
        }
        let start = full.len();
        if !full.is_empty() {
            full.push_str("\
\
");
        }
        full.push_str(text.trim());
        let end = full.len();
        page_spans.push((*page_num, start, end));
    }

    if full.is_empty() {
        return vec![];
    }

    let char_indices: Vec<(usize, char)> = full.char_indices().collect();
    let total_chars = char_indices.len();
    let step = target_chars.saturating_sub(overlap_chars).max(1);
    let mut chunks = Vec::new();
    let mut c_start = 0usize;
    let mut ordinal = 0i32;

    while c_start < total_chars {
        let c_end = (c_start + target_chars).min(total_chars);
        let byte_start = char_indices[c_start].0;
        let byte_end = if c_end < total_chars {
            char_indices[c_end].0
        } else {
            full.len()
        };

        let slice = full[byte_start..byte_end].trim().to_string();
        if !slice.is_empty() {
            let mut p_start = page_spans.first().map(|p| p.0).unwrap_or(1);
            let mut p_end = p_start;
            for (p_num, s, e) in &page_spans {
                if *e > byte_start && *s < byte_end {
                    if byte_start >= *s {
                        p_start = *p_num;
                    }
                    p_end = *p_num;
                }
            }

            chunks.push(PageChunk {
                ordinal,
                page_start: p_start,
                page_end: p_end,
                char_start: c_start as i32,
                char_end: c_end as i32,
                text: slice,
            });
            ordinal += 1;
        }

        if c_end == total_chars {
            break;
        }
        c_start += step;
    }

    chunks
}

fn contains_token(haystack_upper: &str, token_upper: &str) -> bool {
    if let Some(pos) = haystack_upper.find(token_upper) {
        let before_ok = pos == 0
            || !haystack_upper[..pos]
                .chars()
                .next_back()
                .map(|c| c.is_ascii_alphanumeric())
                .unwrap_or(false);
        let after_idx = pos + token_upper.len();
        let after_ok = after_idx >= haystack_upper.len()
            || !haystack_upper[after_idx..]
                .chars()
                .next()
                .map(|c| c.is_ascii_alphanumeric())
                .unwrap_or(false);
        return before_ok && after_ok;
    }
    false
}

/// Stage 9: Validate extracted conditions & findings against page bounds and non-fabrication rules.
pub fn validate_extraction(res: &mut ExtractionResult, max_pages: i32) {
    let upper_page = max_pages.max(1);

    res.conditions.retain_mut(|c: &mut ExtractedCondition| {
        let has_any = c.fuel_type.is_some()
            || c.fuel_geometry.is_some()
            || c.oxygen_concentration.is_some()
            || c.pressure.is_some()
            || c.airflow.is_some()
            || !c.sensors.is_empty()
            || c.objective.is_some();
        if !has_any {
            return false;
        }
        if let Some(p) = c.source_page {
            if p < 1 || p > upper_page {
                c.source_page = None;
            }
        }
        c.confidence = c.confidence.clamp(0.0, 1.0);
        if c.provenance == ProvenanceKind::Inferred && c.confidence > 0.65 {
            c.confidence = 0.65;
        }
        true
    });

    res.findings.retain_mut(|f: &mut ExtractedFinding| {
        if f.text.trim().is_empty() {
            return false;
        }
        if let Some(p) = f.source_page {
            if p < 1 || p > upper_page {
                f.source_page = None;
            }
        }
        f.confidence = f.confidence.clamp(0.0, 1.0);
        if f.provenance == ProvenanceKind::Inferred && f.confidence > 0.65 {
            f.confidence = 0.65;
        }
        true
    });
}
