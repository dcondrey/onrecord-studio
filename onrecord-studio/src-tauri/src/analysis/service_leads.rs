//! Service-report leads stored as findings.
//!
//! The counts come from the discovery feed's SQL (`src/utils/serviceAccountability.ts`), run
//! through the restricted analytical SQL boundary. This side checks the snapshot each lead names
//! against the catalog, writes the source line from the catalog's own record, and stores the
//! lead with no confidence score: nothing here recounts the rows.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    analysis::finding::{
        finding_id, Finding, FindingConfidence, FindingKind, FindingSeverity, FindingSubject,
    },
    error::AppError,
    storage::DuckDbRepository,
    types::AnalyticalDatasetSummary,
};

pub const SLUG: &str = "service_report_leads";

const MAX_LEADS: usize = 200;
const MAX_KEY_CHARS: usize = 400;
const MAX_TITLE_CHARS: usize = 600;
const MAX_TEXT_CHARS: usize = 6_000;

/// Which published records a lead was counted from; decides the finding kind it is stored as.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LeadFamily {
    ServiceReports,
    PoliceStops,
}

impl LeadFamily {
    const fn kind(self) -> FindingKind {
        match self {
            Self::ServiceReports => FindingKind::ServiceReportLead,
            Self::PoliceStops => FindingKind::PoliceStopLead,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ServiceLeadInput {
    pub family: LeadFamily,
    /// The feed's own identifier for the lead, stable while its evidence is.
    pub key: String,
    pub snapshot_id: String,
    pub title: String,
    pub detail: String,
    pub question: String,
    pub method: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ServiceLeadsRecorded {
    pub findings_recorded: usize,
    pub stale_cleared: usize,
}

fn bounded(name: &str, value: &str, limit: usize) -> Result<(), AppError> {
    if value.trim().is_empty() {
        return Err(AppError::Analysis(format!("A service lead has no {name}.")));
    }
    if value.chars().count() > limit {
        return Err(AppError::Analysis(format!(
            "A service lead's {name} is longer than {limit} characters."
        )));
    }
    Ok(())
}

/// One finding per lead. A lead naming a snapshot the catalog does not hold is refused, along
/// with the whole batch: its counts would have no source to cite.
pub fn to_findings(
    leads: &[ServiceLeadInput],
    datasets: &[AnalyticalDatasetSummary],
    now: DateTime<Utc>,
) -> Result<Vec<Finding>, AppError> {
    if leads.len() > MAX_LEADS {
        return Err(AppError::Analysis(format!(
            "{} service leads were sent; at most {MAX_LEADS} are stored at once.",
            leads.len()
        )));
    }
    let catalog: HashMap<&str, &AnalyticalDatasetSummary> = datasets
        .iter()
        .map(|dataset| (dataset.snapshot_id.as_str(), dataset))
        .collect();
    leads
        .iter()
        .map(|lead| {
            bounded("key", &lead.key, MAX_KEY_CHARS)?;
            bounded("title", &lead.title, MAX_TITLE_CHARS)?;
            bounded("detail", &lead.detail, MAX_TEXT_CHARS)?;
            bounded("question", &lead.question, MAX_TEXT_CHARS)?;
            bounded("method", &lead.method, MAX_TEXT_CHARS)?;
            let dataset = catalog.get(lead.snapshot_id.as_str()).ok_or_else(|| {
                AppError::Analysis(
                    "A service lead names a table that is no longer imported.".to_string(),
                )
            })?;
            let subject = FindingSubject::default();
            Ok(Finding {
                id: finding_id(
                    lead.family.kind(),
                    SLUG,
                    &subject,
                    &[
                        &dataset.snapshot_id,
                        &dataset.source_sha256,
                        &lead.key,
                        &lead.title,
                        &lead.detail,
                    ],
                ),
                kind: lead.family.kind(),
                severity: FindingSeverity::Notable,
                confidence: FindingConfidence::Unassessed,
                subject,
                headline: lead.title.trim().to_string(),
                explanation: format!(
                    "{} Method: {} Worth asking: {} Source: the dataset '{}' (sha256 {}), retrieved {}.",
                    lead.detail.trim(),
                    lead.method.trim(),
                    lead.question.trim(),
                    dataset.display_name,
                    dataset
                        .source_sha256
                        .get(..16)
                        .unwrap_or(&dataset.source_sha256),
                    dataset.retrieved_at.get(..10).unwrap_or(&dataset.retrieved_at),
                ),
                citations: vec![],
                discovered_at: now,
                last_seen_at: now,
                dismissed_at: None,
                source_detector: SLUG.into(),
            })
        })
        .collect()
}

/// Upserts the leads. Earlier leads that were not sent again are removed only when `complete`
/// says every imported table was checked without error; a table not yet checked is not a table
/// whose lead no longer holds.
pub async fn store(
    repository: &DuckDbRepository,
    findings: Vec<Finding>,
    complete: bool,
    run_started: DateTime<Utc>,
) -> Result<ServiceLeadsRecorded, AppError> {
    let findings_recorded = repository.record_findings(findings).await?;
    let stale_cleared = if complete {
        repository
            .clear_stale_findings(SLUG.to_string(), run_started)
            .await?
    } else {
        0
    };
    Ok(ServiceLeadsRecorded {
        findings_recorded,
        stale_cleared,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{storage::findings::FindingsFilter, types::AnalyticalDatasetKind};
    use chrono::Duration;

    fn dataset(snapshot_id: &str) -> AnalyticalDatasetSummary {
        AnalyticalDatasetSummary {
            snapshot_id: snapshot_id.into(),
            dataset_kind: AnalyticalDatasetKind::RegionalData,
            display_name: "Get It Done open reports".into(),
            table_name: format!("analysis_{snapshot_id}"),
            imported_rows: 10,
            rejected_rows: 0,
            source_uri: Some("https://example.test/open.csv".into()),
            source_sha256: "ab".repeat(32),
            retrieved_at: "2026-10-04T00:00:00Z".into(),
            observation_period_start: None,
            observation_period_end: None,
            partial_dataset: false,
            validation_summary: "{}".into(),
        }
    }

    fn lead(key: &str, snapshot_id: &str) -> ServiceLeadInput {
        ServiceLeadInput {
            family: if key.starts_with("b:") {
                LeadFamily::PoliceStops
            } else {
                LeadFamily::ServiceReports
            },
            key: key.into(),
            snapshot_id: snapshot_id.into(),
            title: format!("{key}: 12 of 40 open reports have been open more than a year"),
            detail: "Street lights: 12 of 40.".into(),
            question: "Which department owns street lights?".into(),
            method: "Reports with no close date, grouped by service.".into(),
        }
    }

    #[tokio::test]
    async fn an_incomplete_run_keeps_unsent_leads_and_a_complete_one_drops_them() {
        let repository = DuckDbRepository::new(":memory:").expect("open in-memory core");
        repository
            .run_migrations(include_str!("../../migrations/0001_initial_schema.sql"))
            .await
            .expect("run migrations");
        let datasets = [dataset("a"), dataset("b")];
        let start = Utc::now();
        let stored = |repository: &DuckDbRepository| {
            let repository = repository.clone();
            async move {
                let mut headlines: Vec<String> = repository
                    .list_findings(FindingsFilter::default(), 50)
                    .await
                    .expect("list")
                    .into_iter()
                    .map(|finding| {
                        assert_eq!(
                            finding.kind == FindingKind::PoliceStopLead,
                            finding.headline.starts_with("b:")
                        );
                        assert_eq!(finding.confidence, FindingConfidence::Unassessed);
                        assert!(finding.explanation.contains("sha256 abababababababab"));
                        finding.headline.split(':').next().unwrap().to_string()
                    })
                    .collect();
                headlines.sort();
                headlines
            }
        };

        let both = to_findings(
            &[lead("a:backlog", "a"), lead("b:backlog", "b")],
            &datasets,
            start,
        )
        .expect("both snapshots are imported");
        store(&repository, both, true, start).await.expect("store");
        assert_eq!(stored(&repository).await, ["a", "b"]);

        let later = start + Duration::seconds(1);
        let only_a = to_findings(&[lead("a:backlog", "a")], &datasets, later).expect("a");
        let outcome = store(&repository, only_a.clone(), false, later)
            .await
            .expect("store");
        assert_eq!(outcome.stale_cleared, 0);
        assert_eq!(stored(&repository).await, ["a", "b"]);

        let last = later + Duration::seconds(1);
        let only_a = to_findings(&[lead("a:backlog", "a")], &datasets, last).expect("a");
        let outcome = store(&repository, only_a, true, last).await.expect("store");
        assert_eq!(outcome.stale_cleared, 1);
        assert_eq!(stored(&repository).await, ["a"]);

        assert!(to_findings(&[lead("c:backlog", "c")], &datasets, last).is_err());
    }
}
