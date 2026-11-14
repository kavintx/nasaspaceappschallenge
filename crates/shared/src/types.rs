use serde::{Deserialize, Serialize};

/// Where a value came from — spec: never fabricate; distinguish these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProvenanceKind {
    /// Directly stated by the source paper.
    Reported,
    /// Pulled from structured text with high certainty.
    Extracted,
    /// Derived by the model from context (flagged, never presented as fact).
    Inferred,
    /// Explicitly not known from the source.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VerificationStatus {
    Pending,
    Verified,
    Corrected,
    Rejected,
}

/// One extracted experimental condition record, as produced by AI extraction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedCondition {
    pub fuel_type: Option<String>,
    pub fuel_geometry: Option<String>,
    pub oxygen_concentration: Option<String>,
    pub pressure: Option<String>,
    pub airflow: Option<String>,
    pub sensors: Vec<String>,
    pub objective: Option<String>,
    /// 1-based page in the source PDF, None if not determinable.
    pub source_page: Option<i32>,
    pub source_quote: Option<String>,
    pub provenance: ProvenanceKind,
    pub confidence: f32,
}

/// One extracted finding/claim.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedFinding {
    pub text: String,
    pub category: Option<String>, // prevention|detection|suppression|other
    pub source_page: Option<i32>,
    pub source_quote: Option<String>,
    pub provenance: ProvenanceKind,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionResult {
    pub experiment_acronyms: Vec<String>,
    pub conditions: Vec<ExtractedCondition>,
    pub findings: Vec<ExtractedFinding>,
    /// Model's own statement of missing data; surfaced, never filled in.
    pub data_gaps: Vec<String>,
}

/// Citation attached to AI answers: links back to the NTRS publication.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Citation {
    pub paper_id: uuid::Uuid,
    pub ntrs_id: String,
    pub title: String,
    pub ntrs_url: String,
    pub page: Option<i32>,
    pub quote: String,
}

/// One piece of retrieved evidence fed to the model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub paper_id: uuid::Uuid,
    pub ntrs_id: String,
    pub title: String,
    pub page: Option<i32>,
    pub text: String,
    pub score: f64,
    pub origin: String, // "vector" | "keyword" | "structured"
}

impl Evidence {
    pub fn citation(&self, quote_len: usize) -> Citation {
        Citation {
            paper_id: self.paper_id,
            ntrs_id: self.ntrs_id.clone(),
            title: self.title.clone(),
            ntrs_url: format!("https://ntrs.nasa.gov/citations/{}", self.ntrs_id),
            page: self.page,
            quote: self.text.chars().take(quote_len).collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatTurn {
    pub role: String,
    pub content: String,
}

/// A page-tagged chunk produced by the document processor.
#[derive(Debug, Clone)]
pub struct PageChunk {
    pub ordinal: i32,
    pub page_start: i32,
    pub page_end: i32,
    pub char_start: i32,
    pub char_end: i32,
    pub text: String,
}
