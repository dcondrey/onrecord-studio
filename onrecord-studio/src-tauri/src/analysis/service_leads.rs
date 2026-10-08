//! Service-report leads stored as findings.
//!
//! The counts come from the discovery feed's SQL (`src/utils/serviceAccountability.ts`), run
//! through the restricted analytical SQL boundary. This side checks every snapshot a lead names
//! against the catalog, writes each source line from the catalog's own record, appends the SQL as
//! sent, and stores the lead with no confidence score: nothing here recounts the rows. A lead's
//! figure is checked against its schema and bounds and stored beside the finding.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use duckdb::OptionalExt;
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
    DisclosureFilings,
    CapacityComparison,
    SpendingRecords,
    PoliceCalls,
    ParkingCitations,
    HousingApprovals,
}

impl LeadFamily {
    const fn kind(self) -> FindingKind {
        match self {
            Self::ServiceReports => FindingKind::ServiceReportLead,
            Self::PoliceStops => FindingKind::PoliceStopLead,
            Self::AbatementLogs => FindingKind::AbatementLogLead,
            Self::ChargeLogs => FindingKind::ChargeLogLead,
            Self::DisclosureFilings => FindingKind::DisclosureFilingLead,
            Self::CapacityComparison => FindingKind::CapacityComparisonLead,
            Self::SpendingRecords => FindingKind::SpendingRecordLead,
            Self::PoliceCalls => FindingKind::PoliceCallLead,
            Self::ParkingCitations => FindingKind::ParkingCitationLead,
            Self::HousingApprovals => FindingKind::HousingApprovalLead,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
    /// Drawn from the same rows as `detail`; none when the result was cut short.
    pub figure: Option<LeadFigure>,
}

// ---- Lead figures ------------------------------------------------------------------------------
//
// Mirrors `LeadFigure` in `src/types/analysis.ts`; scripts/check-ipc-contract.mjs compares the
// kinds, units and limits. A figure is checked here and stored re-serialized from these types, so
// a field this side does not know is never stored.

const MAX_FIGURE_ROWS: usize = 60;
const MAX_FIGURE_SEGMENTS: usize = 8;
const MAX_FIGURE_PANELS: usize = 4;
const MAX_FIGURE_POINTS: usize = 40;
const MAX_FIGURE_NOTES: usize = 6;
const MAX_FIGURE_LABEL_CHARS: usize = 300;
const MAX_FIGURE_NOTE_CHARS: usize = 600;
const MAX_FIGURE_BYTES: usize = 24_000;

/// A measurement, or why there is none. Absence is never 0.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "state")]
pub enum LeadFigureValue {
    #[serde(rename = "value")]
    Value { value: f64 },
    #[serde(rename = "not-recorded")]
    NotRecorded { reason: String },
    #[serde(rename = "not-covered")]
    NotCovered { reason: String },
    #[serde(rename = "excluded")]
    Excluded { reason: String },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LeadFigureUnit {
    Count,
    Currency,
    Percent,
    Hours,
    Days,
}

/// A lead is never a counted legal finding, so a figure cannot claim to be one.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LeadFigureClaim {
    Descriptive,
    Statistical,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LeadFigureExcluded {
    pub key: String,
    pub label: String,
    pub count: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SeriesPoint {
    pub key: String,
    pub label: String,
    pub partial: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SeriesContext {
    pub label: String,
    pub values: Vec<LeadFigureValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SeriesPanel {
    pub key: String,
    pub label: String,
    pub unit: LeadFigureUnit,
    pub values: Vec<LeadFigureValue>,
    pub context: Option<SeriesContext>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SeriesFigure {
    pub measure: String,
    pub claim: LeadFigureClaim,
    pub caption: Option<String>,
    pub notes: Vec<String>,
    pub points: Vec<SeriesPoint>,
    pub panels: Vec<SeriesPanel>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SegmentTone {
    #[serde(rename = "lead")]
    Lead,
    #[serde(rename = "ord-1")]
    Ord1,
    #[serde(rename = "ord-2")]
    Ord2,
    #[serde(rename = "ord-3")]
    Ord3,
    #[serde(rename = "ord-4")]
    Ord4,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StackedSegment {
    pub key: String,
    pub label: String,
    pub tone: SegmentTone,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StackedOverlay {
    pub label: String,
    pub value: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StackedRow {
    pub key: String,
    pub group: String,
    pub label: String,
    pub partial: bool,
    pub values: Vec<f64>,
    pub excluded: Vec<LeadFigureExcluded>,
    pub overlay: Option<StackedOverlay>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StackedFigure {
    pub measure: String,
    pub claim: LeadFigureClaim,
    pub caption: Option<String>,
    pub notes: Vec<String>,
    pub unit: LeadFigureUnit,
    pub normalise: bool,
    pub segments: Vec<StackedSegment>,
    pub rows: Vec<StackedRow>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PairedDisplay {
    Dots,
    Bullet,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PairedBand {
    pub percent: f64,
    pub minimum: f64,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PairedRow {
    pub key: String,
    pub label: String,
    pub subject: LeadFigureValue,
    pub comparison: LeadFigureValue,
    pub subject_basis: Option<String>,
    pub comparison_basis: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PairedFigure {
    pub measure: String,
    pub claim: LeadFigureClaim,
    pub caption: Option<String>,
    pub notes: Vec<String>,
    pub unit: LeadFigureUnit,
    pub display: PairedDisplay,
    pub subject_label: String,
    pub comparison_label: String,
    pub band: Option<PairedBand>,
    pub rows: Vec<PairedRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FigureBin {
    pub key: String,
    pub label: String,
    pub from: f64,
    pub to: Option<f64>,
    pub count: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BinsReference {
    pub at: f64,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BinsFigure {
    pub measure: String,
    pub claim: LeadFigureClaim,
    pub caption: Option<String>,
    pub notes: Vec<String>,
    pub unit: LeadFigureUnit,
    pub bins: Vec<FigureBin>,
    pub before: Option<LeadFigureExcluded>,
    pub median: Option<f64>,
    pub reference: Option<BinsReference>,
    pub denominator: Vec<LeadFigureExcluded>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RankedRow {
    pub key: String,
    pub label: String,
    pub value: LeadFigureValue,
    pub subject: bool,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RankedFigure {
    pub measure: String,
    pub claim: LeadFigureClaim,
    pub caption: Option<String>,
    pub notes: Vec<String>,
    pub unit: LeadFigureUnit,
    pub rows: Vec<RankedRow>,
    pub excluded: Vec<LeadFigureExcluded>,
}

/// The figure a lead draws, built by the feed from the same rows as its detail text.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LeadFigure {
    Series(SeriesFigure),
    Stacked(StackedFigure),
    Paired(PairedFigure),
    Bins(BinsFigure),
    Ranked(RankedFigure),
}

fn refused(reason: String) -> AppError {
    AppError::Analysis(format!("A lead's figure was refused: {reason}"))
}

fn text(name: &str, value: &str, limit: usize) -> Result<(), AppError> {
    if value.chars().count() > limit {
        return Err(refused(format!(
            "its {name} is longer than {limit} characters."
        )));
    }
    Ok(())
}

fn finite(name: &str, value: f64) -> Result<(), AppError> {
    if !value.is_finite() {
        return Err(refused(format!("its {name} is not a finite number.")));
    }
    Ok(())
}

fn count(name: &str, value: f64) -> Result<(), AppError> {
    finite(name, value)?;
    if value < 0.0 {
        return Err(refused(format!("its {name} is negative.")));
    }
    Ok(())
}

fn length(name: &str, actual: usize, minimum: usize, maximum: usize) -> Result<(), AppError> {
    if actual < minimum || actual > maximum {
        return Err(refused(format!(
            "it has {actual} {name}; between {minimum} and {maximum} are drawn."
        )));
    }
    Ok(())
}

fn value(name: &str, value: &LeadFigureValue) -> Result<(), AppError> {
    match value {
        LeadFigureValue::Value { value } => finite(name, *value),
        LeadFigureValue::NotRecorded { reason }
        | LeadFigureValue::NotCovered { reason }
        | LeadFigureValue::Excluded { reason } => {
            text("absence reason", reason, MAX_FIGURE_NOTE_CHARS)
        }
    }
}

fn excluded(stubs: &[LeadFigureExcluded]) -> Result<(), AppError> {
    length("excluded counts", stubs.len(), 0, MAX_FIGURE_SEGMENTS)?;
    for stub in stubs {
        text("excluded key", &stub.key, MAX_FIGURE_LABEL_CHARS)?;
        text("excluded label", &stub.label, MAX_FIGURE_LABEL_CHARS)?;
        count("excluded count", stub.count)?;
    }
    Ok(())
}

fn heading(measure: &str, caption: Option<&str>, notes: &[String]) -> Result<(), AppError> {
    if measure.trim().is_empty() {
        return Err(refused("it names no measure.".to_string()));
    }
    text("measure", measure, MAX_FIGURE_LABEL_CHARS)?;
    text("caption", caption.unwrap_or(""), MAX_FIGURE_NOTE_CHARS)?;
    length("notes", notes.len(), 0, MAX_FIGURE_NOTES)?;
    notes
        .iter()
        .try_for_each(|note| text("note", note, MAX_FIGURE_NOTE_CHARS))
}

fn optional_text(name: &str, value: Option<&str>) -> Result<(), AppError> {
    text(name, value.unwrap_or(""), MAX_FIGURE_LABEL_CHARS)
}

/// Checks a figure's shape and bounds, and returns it as the JSON that is stored.
pub fn checked_figure(figure: &LeadFigure) -> Result<String, AppError> {
    match figure {
        LeadFigure::Series(series) => {
            heading(&series.measure, series.caption.as_deref(), &series.notes)?;
            length("points", series.points.len(), 1, MAX_FIGURE_POINTS)?;
            length("panels", series.panels.len(), 1, MAX_FIGURE_PANELS)?;
            for point in &series.points {
                text("point key", &point.key, MAX_FIGURE_LABEL_CHARS)?;
                text("point label", &point.label, MAX_FIGURE_LABEL_CHARS)?;
            }
            for panel in &series.panels {
                text("panel key", &panel.key, MAX_FIGURE_LABEL_CHARS)?;
                text("panel label", &panel.label, MAX_FIGURE_LABEL_CHARS)?;
                let mut lines = vec![&panel.values];
                if let Some(context) = &panel.context {
                    text("context label", &context.label, MAX_FIGURE_LABEL_CHARS)?;
                    lines.push(&context.values);
                }
                for values in lines {
                    if values.len() != series.points.len() {
                        return Err(refused(
                            "a panel's values do not line up with its points.".to_string(),
                        ));
                    }
                    values
                        .iter()
                        .try_for_each(|item| value("panel value", item))?;
                }
            }
        }
        LeadFigure::Stacked(stacked) => {
            heading(&stacked.measure, stacked.caption.as_deref(), &stacked.notes)?;
            length("segments", stacked.segments.len(), 1, MAX_FIGURE_SEGMENTS)?;
            length("rows", stacked.rows.len(), 1, MAX_FIGURE_ROWS)?;
            for segment in &stacked.segments {
                text("segment key", &segment.key, MAX_FIGURE_LABEL_CHARS)?;
                text("segment label", &segment.label, MAX_FIGURE_LABEL_CHARS)?;
            }
            for row in &stacked.rows {
                text("row key", &row.key, MAX_FIGURE_LABEL_CHARS)?;
                text("row group", &row.group, MAX_FIGURE_LABEL_CHARS)?;
                text("row label", &row.label, MAX_FIGURE_LABEL_CHARS)?;
                optional_text("row note", row.note.as_deref())?;
                if row.values.len() != stacked.segments.len() {
                    return Err(refused(
                        "a row's values do not line up with its segments.".to_string(),
                    ));
                }
                row.values
                    .iter()
                    .try_for_each(|item| finite("row value", *item))?;
                excluded(&row.excluded)?;
                if let Some(overlay) = &row.overlay {
                    text("overlay label", &overlay.label, MAX_FIGURE_LABEL_CHARS)?;
                    count("overlay value", overlay.value)?;
                }
            }
        }
        LeadFigure::Paired(paired) => {
            heading(&paired.measure, paired.caption.as_deref(), &paired.notes)?;
            length("rows", paired.rows.len(), 1, MAX_FIGURE_ROWS)?;
            text(
                "subject label",
                &paired.subject_label,
                MAX_FIGURE_LABEL_CHARS,
            )?;
            text(
                "comparison label",
                &paired.comparison_label,
                MAX_FIGURE_LABEL_CHARS,
            )?;
            if let Some(band) = &paired.band {
                count("band percent", band.percent)?;
                count("band minimum", band.minimum)?;
                text("band label", &band.label, MAX_FIGURE_LABEL_CHARS)?;
            }
            for row in &paired.rows {
                text("row key", &row.key, MAX_FIGURE_LABEL_CHARS)?;
                text("row label", &row.label, MAX_FIGURE_LABEL_CHARS)?;
                value("subject", &row.subject)?;
                value("comparison", &row.comparison)?;
                optional_text("subject basis", row.subject_basis.as_deref())?;
                optional_text("comparison basis", row.comparison_basis.as_deref())?;
            }
        }
        LeadFigure::Bins(bins) => {
            heading(&bins.measure, bins.caption.as_deref(), &bins.notes)?;
            length("bins", bins.bins.len(), 1, MAX_FIGURE_ROWS)?;
            for bin in &bins.bins {
                text("bin key", &bin.key, MAX_FIGURE_LABEL_CHARS)?;
                text("bin label", &bin.label, MAX_FIGURE_LABEL_CHARS)?;
                finite("bin start", bin.from)?;
                if let Some(to) = bin.to {
                    finite("bin end", to)?;
                    if to <= bin.from {
                        return Err(refused("a bin ends before it starts.".to_string()));
                    }
                }
                count("bin count", bin.count)?;
            }
            if let Some(before) = &bins.before {
                excluded(std::slice::from_ref(before))?;
            }
            if let Some(median) = bins.median {
                finite("median", median)?;
            }
            if let Some(reference) = &bins.reference {
                finite("reference", reference.at)?;
                text("reference label", &reference.label, MAX_FIGURE_LABEL_CHARS)?;
            }
            excluded(&bins.denominator)?;
        }
        LeadFigure::Ranked(ranked) => {
            heading(&ranked.measure, ranked.caption.as_deref(), &ranked.notes)?;
            length("rows", ranked.rows.len(), 1, MAX_FIGURE_ROWS)?;
            for row in &ranked.rows {
                text("row key", &row.key, MAX_FIGURE_LABEL_CHARS)?;
                text("row label", &row.label, MAX_FIGURE_LABEL_CHARS)?;
                value("row value", &row.value)?;
                optional_text("row detail", row.detail.as_deref())?;
            }
            excluded(&ranked.excluded)?;
        }
    }
    let json = serde_json::to_string(figure)
        .map_err(|error| refused(format!("it could not be written: {error}")))?;
    if json.len() > MAX_FIGURE_BYTES {
        return Err(refused(format!(
            "it is {} bytes; at most {MAX_FIGURE_BYTES} are stored.",
            json.len()
        )));
    }
    Ok(json)
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

/// A lead as it is stored: its finding and its checked figure, if it drew one.
#[derive(Debug, Clone)]
pub struct StoredLead {
    pub finding: Finding,
    pub figure_json: Option<String>,
}

/// One finding per lead. A lead naming a snapshot the catalog does not hold, or sending a figure
/// outside the schema or its bounds, is refused along with the whole batch: its counts would have
/// no source to cite, or its figure would draw something its rows do not say.
pub fn to_findings(
    leads: &[ServiceLeadInput],
    datasets: &[AnalyticalDatasetSummary],
    now: DateTime<Utc>,
) -> Result<Vec<StoredLead>, AppError> {
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
            let figure_json = lead.figure.as_ref().map(checked_figure).transpose()?;
            let sources = sources(lead, &catalog)?;
            let dataset = sources[0];
            let subject = FindingSubject::default();
            let finding = Finding {
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
            };
            Ok(StoredLead {
                finding,
                figure_json,
            })
        })
        .collect()
}

/// Upserts the leads, then sets every one's figure, clearing it for a lead sent without one so a
/// figure from an earlier, fuller result never outlives it. Earlier leads that were not sent again
/// are removed only when `complete` says every imported table was checked without error; a table
/// not yet checked is not a table whose lead no longer holds.
pub async fn store(
    repository: &DuckDbRepository,
    leads: Vec<StoredLead>,
    complete: bool,
    run_started: DateTime<Utc>,
) -> Result<ServiceLeadsRecorded, AppError> {
    let figures: Vec<(String, Option<String>)> = leads
        .iter()
        .map(|lead| (lead.finding.id.clone(), lead.figure_json.clone()))
        .collect();
    let findings_recorded = repository
        .record_findings(leads.into_iter().map(|lead| lead.finding).collect())
        .await?;
    repository
        .query_connection("Record lead figures", move |conn| {
            let tx = conn.transaction().map_err(storage_error)?;
            for (id, figure_json) in figures {
                tx.execute(
                    "UPDATE findings SET figure_json = ?1 WHERE id = ?2",
                    duckdb::params![figure_json, id],
                )
                .map_err(storage_error)?;
            }
            tx.commit().map_err(storage_error)
        })
        .await?;
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

fn storage_error(error: duckdb::Error) -> AppError {
    AppError::Storage(error.to_string())
}

/// The stored figure of one finding, or none: not a lead, a lead whose result was cut short, a
/// lead stored before figures, or a figure this build's schema no longer reads.
pub async fn figure(
    repository: &DuckDbRepository,
    finding_id: String,
) -> Result<Option<LeadFigure>, AppError> {
    let stored: Option<Option<String>> = repository
        .query_connection("Read a lead figure", move |conn| {
            conn.query_row(
                "SELECT figure_json FROM findings WHERE id = ?1",
                duckdb::params![finding_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(storage_error)
        })
        .await?;
    Ok(stored
        .flatten()
        .and_then(|json| serde_json::from_str(&json).ok()))
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
            figure: None,
        }
    }

    /// One figure of each kind, as `src/utils/*Leads.ts` sends it.
    const SENT_FIGURES: [&str; 5] = [
        r#"{"kind":"series","measure":"Adopted budget, Encampment reports and shelter beds, by fiscal year","claim":"descriptive","caption":"Three publishers.","notes":["12 reports have no readable filing date and are in no year."],"points":[{"key":"2024","label":"FY 2024","partial":false},{"key":"2025","label":"FY 2025","partial":false}],"panels":[{"key":"budget","label":"Adopted operating expense","unit":"currency","context":null,"values":[{"state":"not-recorded","reason":"no department with \"homeless\" in its name; not an amount of zero"},{"state":"value","value":53916571}]},{"key":"beds","label":"Year-round emergency shelter beds","unit":"count","values":[{"state":"value","value":2476},{"state":"excluded","reason":"fewer than 30 reports filed (12)"}],"context":{"label":"Beds of every kind","values":[{"state":"value","value":2600},{"state":"not-covered","reason":"not counted"}]}}]}"#,
        r#"{"kind":"stacked","measure":"How the calls ended, by call type and year","claim":"descriptive","unit":"count","normalise":true,"caption":null,"notes":[],"segments":[{"key":"arrest","label":"Arrest","tone":"lead"},{"key":"report","label":"Report","tone":"ord-4"}],"rows":[{"key":"SLEEPER:2025","group":"SLEEPER","label":"2025","partial":false,"values":[12,81],"excluded":[{"key":"blank","label":"no disposition","count":5}],"overlay":{"label":"Crisis Response Team (also counted under its outcome)","value":5},"note":null}]}"#,
        r#"{"kind":"paired","display":"bullet","measure":"Adopted against actual operating expense: Police","claim":"statistical","unit":"currency","subjectLabel":"Actual expense","comparisonLabel":"Adopted","caption":"Adopted is the original appropriation.","notes":[],"band":{"percent":10,"minimum":1000000,"label":"Lead threshold (10% and $1M below): app yardstick, not a budget rule"},"rows":[{"key":"2025","label":"FY 2025","subject":{"state":"not-recorded","reason":"no rows in the actuals table; not spending of zero"},"subjectBasis":null,"comparisonBasis":null,"comparison":{"state":"value","value":5000000.5}}]}"#,
        r#"{"kind":"bins","measure":"Hours from a posted notice to the abatement","claim":"descriptive","unit":"hours","caption":null,"notes":[],"bins":[{"key":"under_3","label":"under 3 h","from":0,"to":3,"count":22},{"key":"over_72","label":"72 h or more","from":72,"to":null,"count":144}],"before":{"key":"before","label":"completed before the notice was logged","count":11},"median":29.3,"reference":{"at":72,"label":"72 h reference (app yardstick, not the agency's stated requirement)"},"denominator":[{"key":"paired","label":"places matched to one notice","count":2067}]}"#,
        r#"{"kind":"ranked","measure":"SLEEPER calls by police beat","claim":"descriptive","unit":"count","caption":null,"notes":[],"rows":[{"key":"524","label":"Beat 524 (Core-Columbia)","value":{"state":"value","value":40},"subject":true,"detail":"8.0% of citywide calls"}],"excluded":[{"key":"no_beat","label":"no beat recorded","count":2}]}"#,
    ];

    fn ranked(rows: Vec<RankedRow>) -> LeadFigure {
        LeadFigure::Ranked(RankedFigure {
            measure: "Reports".into(),
            claim: LeadFigureClaim::Descriptive,
            caption: None,
            notes: vec![],
            unit: LeadFigureUnit::Count,
            rows,
            excluded: vec![],
        })
    }

    fn row(index: usize, label: String, detail: String) -> RankedRow {
        RankedRow {
            key: format!("r{index}"),
            label,
            value: LeadFigureValue::Value { value: 1.0 },
            subject: false,
            detail: Some(detail),
        }
    }

    /// A ranked figure whose stored JSON is exactly `bytes` long, padded with full rows and then
    /// one partial row. Labels and details are ASCII, so characters are bytes.
    fn sized(bytes: usize) -> LeadFigure {
        let mut rows = Vec::new();
        loop {
            let current = serde_json::to_string(&ranked(rows.clone())).unwrap().len();
            let comma = usize::from(!rows.is_empty());
            let empty = serde_json::to_string(&row(rows.len(), String::new(), String::new()))
                .unwrap()
                .len()
                + comma;
            let room = bytes
                .checked_sub(current + empty)
                .expect("the padding never passes the target");
            let full = 2 * MAX_FIGURE_LABEL_CHARS;
            // Leave the next row its own overhead, so the last row can land exactly on `bytes`.
            let padding = if room <= full {
                room
            } else {
                full.min(room - empty - 2)
            };
            let label = padding.min(MAX_FIGURE_LABEL_CHARS);
            rows.push(row(
                rows.len(),
                "x".repeat(label),
                "y".repeat(padding - label),
            ));
            if padding == room {
                return ranked(rows);
            }
        }
    }

    #[test]
    fn every_figure_kind_the_feed_sends_is_read_and_stored_as_sent() {
        for sent in SENT_FIGURES {
            let figure: LeadFigure = serde_json::from_str(sent).expect("the feed's figure reads");
            let stored = checked_figure(&figure).expect("within bounds");
            let reread: LeadFigure = serde_json::from_str(&stored).expect("stored figure reads");
            assert_eq!(reread, figure);
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&stored).unwrap()["kind"],
                serde_json::from_str::<serde_json::Value>(sent).unwrap()["kind"]
            );
        }
        // A claim the payload cannot make is refused at the boundary, not drawn.
        let counted = SENT_FIGURES[4].replace("\"claim\":\"descriptive\"", "\"claim\":\"counted\"");
        assert!(serde_json::from_str::<LeadFigure>(&counted).is_err());
    }

    #[test]
    fn a_figure_is_refused_past_its_byte_cap_or_with_misaligned_values() {
        assert_eq!(
            checked_figure(&sized(MAX_FIGURE_BYTES - 1)).unwrap().len(),
            MAX_FIGURE_BYTES - 1
        );
        assert_eq!(
            checked_figure(&sized(MAX_FIGURE_BYTES)).unwrap().len(),
            MAX_FIGURE_BYTES
        );
        assert!(checked_figure(&sized(MAX_FIGURE_BYTES + 1)).is_err());

        let LeadFigure::Series(mut series) = serde_json::from_str(SENT_FIGURES[0]).unwrap() else {
            panic!("the first sent figure is a series");
        };
        series.panels[0].values.pop();
        assert!(checked_figure(&LeadFigure::Series(series)).is_err());
        let LeadFigure::Stacked(mut stacked) = serde_json::from_str(SENT_FIGURES[1]).unwrap()
        else {
            panic!("the second sent figure is stacked");
        };
        stacked.rows[0].excluded[0].count = -1.0;
        assert!(checked_figure(&LeadFigure::Stacked(stacked)).is_err());
    }

    /// A core written before figures holds `findings` without `figure_json`. Opened by this build
    /// it gains the column, its old leads read as having no figure, and a lead sent again without
    /// a figure clears the one stored before.
    #[tokio::test]
    async fn a_core_written_before_figures_gains_the_column_and_clears_a_figure_not_sent_again() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("core.duckdb");
        {
            let legacy = duckdb::Connection::open(&path).unwrap();
            legacy
                .execute_batch(
                    "CREATE TABLE findings (
                        id TEXT PRIMARY KEY,
                        kind TEXT NOT NULL,
                        severity TEXT NOT NULL,
                        confidence_basis_points INTEGER,
                        subject_portal_key TEXT,
                        subject_agency_id BIGINT,
                        subject_request_id BIGINT,
                        subject_document_id BIGINT,
                        subject_entity_id TEXT,
                        headline TEXT NOT NULL,
                        explanation TEXT NOT NULL,
                        citations_json TEXT NOT NULL DEFAULT '[]',
                        discovered_at TEXT NOT NULL,
                        last_seen_at TEXT NOT NULL,
                        dismissed_at TEXT,
                        source_detector TEXT NOT NULL
                    );
                    INSERT INTO findings (id, kind, severity, headline, explanation,
                        discovered_at, last_seen_at, source_detector)
                    VALUES ('old', 'SERVICE_REPORT_LEAD', 'NOTABLE', 'An old lead',
                        'Stored before figures.', '2026-01-01T00:00:00Z',
                        '2026-01-01T00:00:00Z', 'service_report_leads');",
                )
                .unwrap();
        }
        let repository = DuckDbRepository::new(&path).expect("open the older core");
        repository
            .run_migrations(include_str!("../../migrations/0001_initial_schema.sql"))
            .await
            .expect("an older core migrates");
        assert_eq!(figure(&repository, "old".into()).await.unwrap(), None);

        let datasets = [dataset("a")];
        let drawn: LeadFigure = serde_json::from_str(SENT_FIGURES[4]).unwrap();
        let start = Utc::now();
        let with = to_findings(
            &[ServiceLeadInput {
                figure: Some(drawn.clone()),
                ..lead("a:backlog", "a")
            }],
            &datasets,
            start,
        )
        .unwrap();
        let id = with[0].finding.id.clone();
        store(&repository, with, false, start).await.unwrap();
        assert_eq!(figure(&repository, id.clone()).await.unwrap(), Some(drawn));

        let without = to_findings(&[lead("a:backlog", "a")], &datasets, start).unwrap();
        assert_eq!(without[0].finding.id, id);
        store(&repository, without, false, start).await.unwrap();
        assert_eq!(figure(&repository, id).await.unwrap(), None);
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
        assert!(finding.finding.explanation.ends_with(&format!(
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

    #[test]
    fn each_family_the_feed_sends_is_stored_as_its_lead_kind() {
        for (wire, family, kind) in [
            ("POLICE_CALLS", LeadFamily::PoliceCalls, "POLICE_CALL_LEAD"),
            (
                "PARKING_CITATIONS",
                LeadFamily::ParkingCitations,
                "PARKING_CITATION_LEAD",
            ),
            (
                "HOUSING_APPROVALS",
                LeadFamily::HousingApprovals,
                "HOUSING_APPROVAL_LEAD",
            ),
        ] {
            let parsed: LeadFamily = serde_json::from_value(serde_json::json!(wire)).unwrap();
            assert_eq!(parsed, family);
            assert_eq!(
                serde_json::to_value(family).unwrap(),
                serde_json::json!(wire)
            );
            assert_eq!(family.kind().as_str(), kind);
        }
    }
}
