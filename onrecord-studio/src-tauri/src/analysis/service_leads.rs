//! Service-report leads stored as findings.
//!
//! The counts come from the discovery feed's SQL (`src/utils/serviceAccountability.ts`), run
//! through the restricted analytical SQL boundary. This side checks every snapshot a lead names
//! against the catalog, writes each source line from the catalog's own record, appends the SQL as
//! sent, and stores the lead with no confidence score: nothing here recounts the rows.

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
/// A comparison across yearly files reads one table per year plus its companions.
const MAX_SOURCE_TABLES: usize = 40;
const MAX_SQL_CHARS: usize = 40_000;

/// Which published records a lead was counted from; decides the finding kind it is stored as.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LeadFamily {
    ServiceReports,
    PoliceStops,
    AbatementLogs,
    ChargeLogs,
    CapacityComparison,
    SpendingRecords,
}

impl LeadFamily {
    const fn kind(self) -> FindingKind {
        match self {
            Self::ServiceReports => FindingKind::ServiceReportLead,
            Self::PoliceStops => FindingKind::PoliceStopLead,
            Self::AbatementLogs => FindingKind::AbatementLogLead,
            Self::ChargeLogs => FindingKind::ChargeLogLead,
            Self::CapacityComparison => FindingKind::CapacityComparisonLead,
            Self::SpendingRecords => FindingKind::SpendingRecordLead,
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
    /// Every table the SQL read, `snapshot_id` first.
    pub snapshot_ids: Vec<String>,
    pub title: String,
    pub detail: String,
    pub question: String,
    pub method: String,
    /// The statement whose result the lead's figures were read from.
    pub sql: String,
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

/// The tables a lead read, in the order sent: its own first, none twice, all still imported.
fn sources<'a>(
    lead: &ServiceLeadInput,
    catalog: &HashMap<&str, &'a AnalyticalDatasetSummary>,
) -> Result<Vec<&'a AnalyticalDatasetSummary>, AppError> {
    if lead.snapshot_ids.len() > MAX_SOURCE_TABLES {
        return Err(AppError::Analysis(format!(
            "A service lead names {} tables; at most {MAX_SOURCE_TABLES} are cited.",
            lead.snapshot_ids.len()
        )));
    }
    if lead.snapshot_ids.first() != Some(&lead.snapshot_id) {
        return Err(AppError::Analysis(
            "A service lead does not list its own table first among the tables it read."
                .to_string(),
        ));
    }
    let mut listed = std::collections::HashSet::new();
    lead.snapshot_ids
        .iter()
        .map(|snapshot_id| {
            if !listed.insert(snapshot_id.as_str()) {
                return Err(AppError::Analysis(
                    "A service lead lists a table twice.".to_string(),
                ));
            }
            catalog.get(snapshot_id.as_str()).copied().ok_or_else(|| {
                AppError::Analysis(
                    "A service lead names a table that is no longer imported.".to_string(),
                )
            })
        })
        .collect()
}

fn source_line(position: usize, dataset: &AnalyticalDatasetSummary) -> String {
    format!(
        "{}. '{}' ({}), retrieved {}, sha256 {}",
        position + 1,
        dataset.display_name,
        dataset
            .source_uri
            .as_deref()
            .unwrap_or("no source address recorded"),
        dataset
            .retrieved_at
            .get(..10)
            .unwrap_or(&dataset.retrieved_at),
        dataset.source_sha256,
    )
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
            bounded("SQL", &lead.sql, MAX_SQL_CHARS)?;
            let sources = sources(lead, &catalog)?;
            let dataset = sources[0];
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
                    "{} Method: {} Worth asking: {}\n\nTables read:\n{}\n\nSQL:\n{}",
                    lead.detail.trim(),
                    lead.method.trim(),
                    lead.question.trim(),
                    sources
                        .iter()
                        .enumerate()
                        .map(|(position, dataset)| source_line(position, dataset))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    lead.sql.trim(),
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
            unconverted_by_column: Default::default(),
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
            snapshot_ids: vec![snapshot_id.into()],
            sql: "SELECT category FROM service_backlog".into(),
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

    #[test]
    fn a_lead_cites_every_table_it_read_and_its_sql_or_is_refused() {
        let budget = AnalyticalDatasetSummary {
            display_name: "City operating budget".into(),
            source_uri: None,
            source_sha256: "cd".repeat(32),
            ..dataset("b")
        };
        let datasets = [dataset("a"), budget];
        let reading = |snapshot_ids: &[&str]| ServiceLeadInput {
            snapshot_ids: snapshot_ids.iter().map(|id| (*id).to_string()).collect(),
            ..lead("a:backlog", "a")
        };
        let [finding] = &to_findings(&[reading(&["a", "b"])], &datasets, Utc::now()).unwrap()[..]
        else {
            panic!("one lead, one finding");
        };
        assert!(finding.explanation.ends_with(&format!(
            "Tables read:\n1. 'Get It Done open reports' (https://example.test/open.csv), retrieved 2026-10-04, sha256 {}\n2. 'City operating budget' (no source address recorded), retrieved 2026-10-04, sha256 {}\n\nSQL:\nSELECT category FROM service_backlog",
            "ab".repeat(32),
            "cd".repeat(32)
        )));
        for refused in [
            reading(&[]),
            reading(&["b", "a"]),
            reading(&["a", "a"]),
            reading(&["a", "gone"]),
            ServiceLeadInput {
                sql: " ".into(),
                ..reading(&["a"])
            },
        ] {
            assert!(to_findings(&[refused], &datasets, Utc::now()).is_err());
        }
        let ids: Vec<String> = (0..=MAX_SOURCE_TABLES).map(|n| format!("t{n}")).collect();
        let many: Vec<AnalyticalDatasetSummary> = ids.iter().map(|id| dataset(id)).collect();
        let listing = |count: usize| ServiceLeadInput {
            snapshot_ids: ids[..count].to_vec(),
            ..lead("t0:backlog", "t0")
        };
        assert!(to_findings(&[listing(MAX_SOURCE_TABLES)], &many, Utc::now()).is_ok());
        assert!(to_findings(&[listing(MAX_SOURCE_TABLES + 1)], &many, Utc::now()).is_err());
    }
}
