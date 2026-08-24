//! Strongly-typed structures for deserializing NextRequest API endpoints.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NextRequestResponse {
    #[serde(default)]
    pub requests: Vec<PublicRequest>,
    pub meta: Option<PaginationMeta>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginationMeta {
    pub current_page: usize,
    pub next_page: Option<usize>,
    pub prev_page: Option<usize>,
    pub total_pages: usize,
    pub total_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicRequest {
    pub id: u64,
    pub rpid: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub status: String,
    pub current_status_id: Option<u64>,
    
    // ISO-8601 Timestamp Strings
    pub created_at: String,
    pub updated_at: String,
    pub closed_at: Option<String>,
    pub due_date: Option<String>,
    
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
    pub file_name: Option<String>,
    pub file_size: Option<u64>,
    pub description: Option<String>,
    pub content_type: Option<String>,
    pub download_url: Option<String>,
    pub created_at: String,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgencyMetadata {
    pub id: u64,
    pub name: String,
    pub department_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryMetadata {
    pub id: u64,
    pub name: String,
}

impl PublicRequest {
    /// Validates internal chronology states to detect structural timestamp anomalies.
    pub fn compute_timeline_anomalies(&mut self) {
        let mut anomalies = Vec::new();

        if let (Ok(created), Ok(updated)) = (
            chrono::DateTime::parse_from_rfc3339(&self.created_at),
            chrono::DateTime::parse_from_rfc3339(&self.updated_at)
        ) {
            if updated < created {
                anomalies.push("UPDATE_BEFORE_CREATION".to_string());
            }
        }

        if let Some(ref closed_str) = self.closed_at {
            if let (Ok(created), Ok(closed)) = (
                chrono::DateTime::parse_from_rfc3339(&self.created_at),
                chrono::DateTime::parse_from_rfc3339(closed_str)
            ) {
                if closed < created {
                    anomalies.push("CLOSE_BEFORE_CREATION".to_string());
                }
                
                // Flag requests closed in less than 5 minutes as high risk for arbitrary closure
                if (closed - created).num_minutes() < 5 && self.status.to_lowercase() == "closed" {
                    anomalies.push("SUSPICIOUSLY_FAST_CLOSURE".to_string());
                }
            }
        }

        self.timeline_anomalies = anomalies;
    }
}
