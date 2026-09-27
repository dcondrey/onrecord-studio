use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct ResearchPolicy {
    pub enabled: bool,
    pub auto_collect: bool,
    pub interval_hours: u32,
    pub provider: String,
    pub model: String,
    pub daily_call_limit: u32,
    pub research_passes: u32,
    pub discover_sources: bool,
}
impl ResearchPolicy {
    pub fn minimum_run_calls(&self) -> u32 {
        self.research_passes
            .saturating_add(3)
            .saturating_add(u32::from(
                self.auto_collect
                    && self.discover_sources
                    && matches!(self.provider.as_str(), "openai" | "openrouter"),
            ))
    }
}
impl Default for ResearchPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_collect: true,
            interval_hours: 6,
            provider: "openai".into(),
            model: "gpt-5.4".into(),
            daily_call_limit: 24,
            research_passes: 4,
            discover_sources: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchEvidence {
    pub id: String,
    pub label: String,
    pub source_url: Option<String>,
    pub portal_key: Option<String>,
    pub request_id: Option<u64>,
    pub document_id: Option<u64>,
    pub character_offset: Option<usize>,
    pub source_sha256: String,
    pub text: String,
    pub sql: Option<String>,
    #[serde(default)]
    pub snapshot_provenance: Vec<String>,
    #[serde(default)]
    pub source_references: Vec<ResearchSourceReference>,
    pub limitations: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchSourceReference {
    pub url: String,
    pub snapshot: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResearchCitation {
    pub evidence_id: String,
    pub quote: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResearchFinding {
    pub title: String,
    pub interpretation: String,
    pub why_it_matters: String,
    pub citations: Vec<ResearchCitation>,
    pub counter_evidence: Vec<ResearchCitation>,
    pub alternative_explanations: Vec<String>,
    pub next_test: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResearchQuestion {
    pub question: String,
    pub why: String,
    pub evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResearchRequest {
    pub title: String,
    pub custodian: String,
    pub gap: String,
    pub resolves: String,
    pub draft: String,
    pub evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResearchMeeting {
    pub body: String,
    pub date: String,
    pub agenda_item: String,
    pub relevance: String,
    pub action: String,
    pub citations: Vec<ResearchCitation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResearchSynthesis {
    pub title: String,
    pub summary: String,
    pub findings: Vec<ResearchFinding>,
    pub questions: Vec<ResearchQuestion>,
    pub requests: Vec<ResearchRequest>,
    pub meetings: Vec<ResearchMeeting>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchReport {
    pub id: String,
    pub topic: String,
    pub created_at: String,
    pub model: String,
    pub evidence: Vec<ResearchEvidence>,
    pub synthesis: ResearchSynthesis,
    pub investigation_log: Vec<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchSourceStatus {
    pub label: String,
    pub url: String,
    pub state: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchStatus {
    pub policy: ResearchPolicy,
    pub credential_available: bool,
    pub phase: String,
    pub last_error: Option<String>,
    pub last_finished_at: Option<String>,
    pub calls_today: u32,
    pub reports: Vec<ResearchReport>,
    pub sources: Vec<ResearchSourceStatus>,
    pub action_states: std::collections::BTreeMap<String, String>,
}
