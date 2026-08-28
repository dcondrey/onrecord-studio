//! Strongly-typed structures for deserializing NextRequest API endpoints.

use serde::{Deserialize, Serialize};

use super::StatutoryExtensionRecord;
use crate::error::AppError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NextRequestResponse {
    #[serde(default)]
    pub requests: Vec<PublicRequest>,
    pub meta: Option<PaginationMeta>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginationMeta {
    #[serde(alias = "currentPage", alias = "page")]
    pub current_page: usize,
    #[serde(alias = "nextPage")]
    pub next_page: Option<usize>,
    #[serde(alias = "prevPage", alias = "previousPage")]
    pub prev_page: Option<usize>,
    #[serde(alias = "totalPages", alias = "pages")]
    pub total_pages: usize,
    #[serde(alias = "totalCount", alias = "count")]
    pub total_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicRequest {
    pub id: u64,
    pub rpid: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub status: String,
    #[serde(alias = "currentStatusId")]
    pub current_status_id: Option<u64>,

    // ISO-8601 Timestamp Strings
    #[serde(alias = "createdAt")]
    pub created_at: String,
    #[serde(alias = "updatedAt")]
    pub updated_at: String,
    #[serde(alias = "closedAt")]
    pub closed_at: Option<String>,
    #[serde(alias = "dueDate")]
    pub due_date: Option<String>,
    /// A source-reported determination event, distinct from generic request activity.
    #[serde(default, alias = "determinationAt")]
    pub determination_at: Option<String>,
    /// Human-readable source/notice identifier that attests `determination_at`.
    #[serde(default, alias = "determinationSource")]
    pub determination_source: Option<String>,

    #[serde(default, alias = "neighborhoodBlock")]
    pub neighborhood_block: Option<String>,

    #[serde(default, alias = "statutoryExtension")]
    pub statutory_extension: Option<StatutoryExtensionRecord>,

    // Relations
    #[serde(default)]
    pub documents: Vec<DocumentMetadata>,
    pub agency: Option<AgencyMetadata>,
    pub category: Option<CategoryMetadata>,

    // Operational metrics derived during parsing
    #[serde(skip_deserializing)]
    pub timeline_anomalies: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentMetadata {
    pub id: u64,
    #[serde(alias = "fileName")]
    pub file_name: Option<String>,
    #[serde(alias = "fileSize")]
    pub file_size: Option<u64>,
    pub description: Option<String>,
    #[serde(alias = "contentType")]
    pub content_type: Option<String>,
    #[serde(alias = "downloadUrl")]
    pub download_url: Option<String>,
    #[serde(alias = "createdAt")]
    pub created_at: String,
    #[serde(alias = "updatedAt")]
    pub updated_at: String,
    pub visibility: String, // e.g., "public", "staff_only"

    // Fields tracking deep offline extraction metrics
    #[serde(skip_deserializing)]
    pub local_path: Option<String>,
    #[serde(skip_deserializing)]
    pub extracted_text_sha256: Option<String>,
    #[serde(skip_deserializing)]
    pub was_ocr_processed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmailParticipantMetadata {
    pub name: Option<String>,
    pub email: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmailMetadata {
    pub sender: Option<EmailParticipantMetadata>,
    #[serde(default)]
    pub to: Vec<EmailParticipantMetadata>,
    #[serde(default)]
    pub cc: Vec<EmailParticipantMetadata>,
    #[serde(default)]
    pub bcc: Vec<EmailParticipantMetadata>,
    pub subject: Option<String>,
    #[serde(alias = "sentAt")]
    pub sent_at: Option<String>,
    #[serde(alias = "messageId")]
    pub message_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgencyMetadata {
    pub id: u64,
    pub name: String,
    #[serde(alias = "departmentName")]
    pub department_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryMetadata {
    pub id: u64,
    pub name: String,
}

impl PublicRequest {
    pub fn compute_timeline_anomalies(&mut self) -> Result<(), AppError> {
        let mut anomalies = Vec::new();
        let created = parse_request_timestamp(self.id, "created", &self.created_at)?;
        let updated = parse_request_timestamp(self.id, "updated", &self.updated_at)?;
        if updated < created {
            anomalies.push("UPDATE_BEFORE_CREATION".to_string());
        }

        let operational = crate::analysis::calculate_operational_latency_for_status(
            self.id,
            &self.status,
            &self.created_at,
            self.closed_at.as_deref(),
        )?;
        anomalies.extend(
            operational
                .anomaly_markers
                .into_iter()
                .map(|marker| marker.as_str().to_string()),
        );

        let statutory = crate::analysis::evaluate_statutory_compliance_with_determination_at(
            self.id,
            &self.created_at,
            Some(&self.updated_at),
            self.closed_at.as_deref(),
            self.determination_at.as_deref(),
            self.determination_source.as_deref(),
            self.statutory_extension.as_ref(),
            &chrono::Utc::now().to_rfc3339(),
        )?;
        anomalies.extend(
            statutory
                .anomaly_markers
                .into_iter()
                .map(|marker| marker.as_str().to_string()),
        );
        anomalies.sort();
        anomalies.dedup();
        self.timeline_anomalies = anomalies;
        Ok(())
    }
}

fn parse_request_timestamp(
    request_id: u64,
    field: &str,
    value: &str,
) -> Result<chrono::DateTime<chrono::FixedOffset>, AppError> {
    chrono::DateTime::parse_from_rfc3339(value).map_err(|error| {
        AppError::DataParsing(format!(
            "Request {request_id} has an invalid {field} timestamp '{value}': {error}"
        ))
    })
}
