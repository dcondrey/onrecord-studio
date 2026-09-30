//! Executes a parsed investigative intent against stored findings, the corpus graph, the document
//! text index and the request timeline. No model is involved anywhere on this path.
//!
//! Every row states the evidence basis it rests on and carries citations, or says why it cannot.
//! Three rules shape the wording, and each has a test:
//!
//! * a `CO_OCCURRENCE` edge is proximity in text and is never phrased as a relationship;
//! * an empty result says what was searched and does not say the thing is absent, because a
//!   detector that has not run, a graph window that was cut, and a search over extracted text
//!   only, all return nothing without meaning nothing exists;
//! * a statutory answer reports whether the determination date was sourced, a proxy, or still
//!   running, because a proxy presented as a measurement is a false statement about an agency.
//!
//! Aggregation is SQL. The only per-row work is looking up a citation for a row that is already
//! inside the row limit.

use super::{corpus::CorpusIndex, fts::core_query_tokens, sqlite::DuckDbRepository};
use crate::{
    analysis::{
        finding::{FindingCitation, FindingKind},
        intent::{self, ParsedIntent, MAX_UTTERANCE_CHARS},
        statute::{DeterminationBasis, RECOGNIZED_EXTENSION_REASONS},
        timeline::{evaluate_statutory_compliance_with_determination_at, StatutoryExtensionStatus},
    },
    error::AppError,
    ingestion::regional::source_jurisdiction,
    types::{
        intent::{
            CrossCorpusSignal, EvidenceBasis, IntentAnswer, IntentCitation, IntentOption,
            IntentRow, IntentStatus, QueryPlan,
        },
        CorpusEntityNode, RelationshipKind, StatutoryExtensionRecord,
    },
};
use chrono::Utc;
use duckdb::{params, Connection};
use std::collections::{BTreeSet, HashMap};

pub const DEFAULT_ROW_LIMIT: usize = 10;
pub const MAX_ROW_LIMIT: usize = 25;
const MAX_CITATIONS_PER_ROW: usize = 5;
/// Names are matched among this many entities, ranked by document count. The corpus API offers no
/// lookup by name, so a rarer entity is outside the window and the answer says so.
const ENTITY_WINDOW: usize = 200;
const MAX_ENTITY_CANDIDATES: usize = 3;
/// The largest neighbour list the corpus graph will return for one entity.
const NEIGHBOR_WINDOW: usize = 200;
const MAX_PATH_HOPS: usize = 4;
/// A name in this many distinct portals is a cross-jurisdiction pattern rather than a coincidence.
const CROSS_PORTAL_THRESHOLD: usize = 3;
const QUOTE_LEAD_CHARACTERS: i64 = 80;
const QUOTE_CHARACTERS: i64 = 320;
/// Two names closer than this are read as one passage.
const SAME_PASSAGE_CHARACTERS: i64 = QUOTE_CHARACTERS - QUOTE_LEAD_CHARACTERS;

const CONTRADICTION_KINDS: &[FindingKind] = &[
    FindingKind::ContradictoryStatement,
    FindingKind::NoRecordsContradictedByRelease,
    FindingKind::MetadataContradictsContent,
    FindingKind::PolicyViolationDetected,
];

/// Parses and answers one utterance.
pub async fn answer_utterance(
    repository: &DuckDbRepository,
    corpus: Option<&CorpusIndex>,
    utterance: &str,
    limit: usize,
) -> Result<IntentAnswer, AppError> {
    if !(1..=MAX_ROW_LIMIT).contains(&limit) {
        return Err(AppError::DataParsing(format!(
            "An intent query must request between 1 and {MAX_ROW_LIMIT} rows"
        )));
    }
    if utterance.chars().count() > MAX_UTTERANCE_CHARS {
        return Err(AppError::DataParsing(format!(
            "An intent query is limited to {MAX_UTTERANCE_CHARS} characters"
        )));
    }
    let utterance = utterance.trim();
    let parsed = intent::parse(utterance);
    let answer = |status, summary: String, options| IntentAnswer {
        utterance: utterance.to_string(),
        status,
        plan: None,
        summary,
        rows: Vec::new(),
        truncated: false,
        row_limit: limit,
        notes: Vec::new(),
        options,
    };
    match parsed {
        ParsedIntent::Plan(plan) => execute_plan(repository, corpus, utterance, plan, limit).await,
        ParsedIntent::NotUnderstood => Ok(answer(
            IntentStatus::NotUnderstood,
            "Not understood. This router answers only the questions listed below and does not guess."
                .to_string(),
            parsed.options(),
        )),
        ParsedIntent::NeedsSubject { hint, .. } => Ok(answer(
            IntentStatus::NeedsSubject,
            hint.to_string(),
            parsed.options(),
        )),
        ParsedIntent::Ambiguous(_) => Ok(answer(
            IntentStatus::Ambiguous,
            "More than one question fits. Rephrase to name one of these.".to_string(),
            parsed.options(),
        )),
    }
}

pub async fn execute_plan(
    repository: &DuckDbRepository,
    corpus: Option<&CorpusIndex>,
    utterance: &str,
    plan: QueryPlan,
    limit: usize,
) -> Result<IntentAnswer, AppError> {
    let mut answer = IntentAnswer {
        utterance: utterance.to_string(),
        status: IntentStatus::NoMatch,
        plan: Some(plan.clone()),
        summary: String::new(),
        rows: Vec::new(),
        truncated: false,
        row_limit: limit,
        notes: Vec::new(),
        options: Vec::new(),
    };
    match &plan {
        QueryPlan::ContradictionsInAgency { agency } => {
            findings_answer(
                repository,
                &mut answer,
                CONTRADICTION_KINDS,
                agency.clone(),
                "contradiction",
            )
            .await?
        }
        QueryPlan::FindingsForAgency { agency } => {
            findings_answer(repository, &mut answer, &[], agency.clone(), "stored").await?
        }
        QueryPlan::CrossCorpusSignal { signal } => {
            signal_answer(repository, &mut answer, *signal).await?
        }
        QueryPlan::WhyRequestMissedDeadline {
            request_id,
            portal_key,
        } => deadline_answer(repository, &mut answer, *request_id, portal_key.clone()).await?,
        QueryPlan::AppearanceAcrossAgencies { subject } => {
            appearance_answer(repository, &mut answer, subject).await?
        }
        QueryPlan::SignatoriesFor { subject } => {
            signatories_answer(repository, corpus, &mut answer, subject).await?
        }
        QueryPlan::ConnectionBetween { from, to } => {
            connection_answer(repository, corpus, &mut answer, from, to).await?
        }
    }
    Ok(answer)
}

fn storage(error: impl std::fmt::Display) -> AppError {
    AppError::Storage(error.to_string())
}

fn sql_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// Builds a row and enforces the citation invariant: citations are capped, and a row with none
/// says why instead of showing nothing.
#[allow(clippy::too_many_arguments)]
fn make_row(
    id: String,
    headline: String,
    detail: Option<String>,
    basis: EvidenceBasis,
    relationship_kind: Option<String>,
    confidence_basis_points: Option<u16>,
    agency_name: Option<String>,
    mut portal_keys: Vec<String>,
    mut citations: Vec<IntentCitation>,
    unavailable_reason: &str,
) -> IntentRow {
    let omitted_citation_count = citations.len().saturating_sub(MAX_CITATIONS_PER_ROW);
    citations.truncate(MAX_CITATIONS_PER_ROW);
    portal_keys.extend(citations.iter().map(|citation| citation.portal_key.clone()));
    let portal_keys = portal_keys
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    IntentRow {
        id,
        headline,
        detail,
        basis,
        basis_statement: basis.statement().to_string(),
        relationship_kind,
        confidence_basis_points,
        agency_name,
        portal_keys,
        citations_unavailable_reason: citations.is_empty().then(|| unavailable_reason.to_string()),
        citations,
        omitted_citation_count,
    }
}

// ---------------------------------------------------------------------------------------------
// Stored findings
// ---------------------------------------------------------------------------------------------

struct FindingHit {
    id: String,
    kind: String,
    severity: String,
    confidence: Option<i64>,
    portal_key: Option<String>,
    headline: String,
    explanation: String,
    citations_json: String,
    agency_name: Option<String>,
}

/// Findings joined to the agency of their request. Contradiction and cross-corpus detectors
/// anchor a finding on a document or request, not on an agency, so the agency is reached through
/// the document, then the request. Agency ids repeat across portals, hence the portal in every
/// join.
fn query_findings(
    connection: &Connection,
    kinds: &[FindingKind],
    agency: Option<&str>,
    limit: usize,
) -> Result<(Vec<FindingHit>, i64), AppError> {
    let kind_clause = if kinds.is_empty() {
        String::new()
    } else {
        format!(
            "AND f.kind IN ({})",
            kinds
                .iter()
                .map(|kind| sql_literal(kind.as_str()))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let sql = format!(
        "SELECT f.id, f.kind, f.severity, f.confidence_basis_points, f.subject_portal_key,
                f.headline, f.explanation, f.citations_json, a.name
         FROM findings f
         LEFT JOIN documents d
           ON d.id = f.subject_document_id AND d.portal_subdomain = f.subject_portal_key
         LEFT JOIN requests r
           ON r.id = COALESCE(f.subject_request_id, d.request_id)
          AND r.portal_subdomain = f.subject_portal_key
         LEFT JOIN agencies a
           ON a.id = COALESCE(f.subject_agency_id, r.agency_id)
          AND a.portal_subdomain = f.subject_portal_key
         WHERE f.dismissed_at IS NULL {kind_clause}
           AND (?1 IS NULL
                OR contains(lower(a.name), ?1)
                OR contains(lower(COALESCE(a.department_name, '')), ?1))
         ORDER BY CASE f.severity
                    WHEN 'CRITICAL' THEN 4 WHEN 'SERIOUS' THEN 3
                    WHEN 'NOTABLE' THEN 2 WHEN 'INFO' THEN 1 ELSE 0 END DESC,
                  COALESCE(f.confidence_basis_points, -1) DESC,
                  f.last_seen_at DESC, f.id
         LIMIT ?2"
    );
    let needle = agency.map(str::to_lowercase);
    let mut statement = connection.prepare(&sql).map_err(storage)?;
    let hits = statement
        .query_map(params![needle, (limit + 1) as i64], |row| {
            Ok(FindingHit {
                id: row.get(0)?,
                kind: row.get(1)?,
                severity: row.get(2)?,
                confidence: row.get(3)?,
                portal_key: row.get(4)?,
                headline: row.get(5)?,
                explanation: row.get(6)?,
                citations_json: row.get(7)?,
                agency_name: row.get(8)?,
            })
        })
        .map_err(storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage)?;
    let stored: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM findings WHERE dismissed_at IS NULL",
            [],
            |row| row.get(0),
        )
        .map_err(storage)?;
    Ok((hits, stored))
}

fn finding_row(hit: FindingHit) -> Result<IntentRow, AppError> {
    let citations: Vec<IntentCitation> =
        serde_json::from_str::<Vec<FindingCitation>>(&hit.citations_json)
            .map_err(|error| {
                AppError::Storage(format!(
                    "Finding {} has unreadable citations: {error}",
                    hit.id
                ))
            })?
            .into_iter()
            .map(|citation| IntentCitation {
                portal_key: citation.portal_key,
                document_id: Some(citation.document_id),
                character_start: citation.character_start,
                character_end: citation.character_end,
                quote: citation.quote,
                label: citation.label,
            })
            .collect();
    let confidence = hit
        .confidence
        .map(|value| u16::try_from(value.clamp(0, 10_000)).unwrap_or(0));
    Ok(make_row(
        hit.id,
        hit.headline,
        Some(format!(
            "{} ({}). {}",
            hit.kind, hit.severity, hit.explanation
        )),
        EvidenceBasis::StoredFinding,
        None,
        confidence,
        hit.agency_name,
        hit.portal_key.into_iter().collect(),
        citations,
        "The detector stored this finding without citations.",
    ))
}

async fn load_findings(
    repository: &DuckDbRepository,
    kinds: &'static [FindingKind],
    agency: Option<String>,
    limit: usize,
) -> Result<(Vec<FindingHit>, i64), AppError> {
    repository
        .query_connection("Intent findings query", move |connection| {
            query_findings(connection, kinds, agency.as_deref(), limit)
        })
        .await
}

const NOT_ATTRIBUTABLE: &str = "A finding is attributed to an agency through its request or document; one anchored on neither is not shown for an agency query.";
const REFRESH_REMINDER: &str =
    "Detectors report only what meets their thresholds, and only after a findings refresh.";

async fn findings_answer(
    repository: &DuckDbRepository,
    answer: &mut IntentAnswer,
    kinds: &'static [FindingKind],
    agency: Option<String>,
    adjective: &str,
) -> Result<(), AppError> {
    let limit = answer.row_limit;
    let scope = agency
        .as_deref()
        .map_or("all agencies".to_string(), |name| {
            format!("agencies matching \"{name}\"")
        });
    let (mut hits, stored) = load_findings(repository, kinds, agency.clone(), limit).await?;
    answer.truncated = hits.len() > limit;
    hits.truncate(limit);
    if agency.is_some() {
        answer.notes.push(NOT_ATTRIBUTABLE.to_string());
    }
    answer.rows = hits
        .into_iter()
        .map(finding_row)
        .collect::<Result<_, _>>()?;
    finish_findings(answer, stored, &scope, adjective);
    Ok(())
}

fn finish_findings(answer: &mut IntentAnswer, stored: i64, scope: &str, adjective: &str) {
    if !answer.rows.is_empty() {
        answer.status = IntentStatus::Answered;
        answer.summary = format!(
            "{}{} {adjective} finding(s) for {scope}. Each is a pattern signal for review, not a legal conclusion.",
            answer.rows.len(),
            if answer.truncated { "+" } else { "" },
        );
    } else if stored == 0 {
        answer.status = IntentStatus::Unavailable;
        answer.summary =
            "No findings are stored yet, so nothing was searched. Run a findings refresh first."
                .to_string();
    } else {
        answer.status = IntentStatus::NoMatch;
        answer.summary = format!(
            "No stored {adjective} finding matches {scope}. This does not show none exists."
        );
        answer.notes.push(REFRESH_REMINDER.to_string());
    }
}

async fn signal_answer(
    repository: &DuckDbRepository,
    answer: &mut IntentAnswer,
    signal: CrossCorpusSignal,
) -> Result<(), AppError> {
    let (kinds, label, gate): (&'static [FindingKind], &str, &str) = match signal {
        CrossCorpusSignal::SharedDenialLanguage => (
            &[FindingKind::CrossCorpusSharedLanguage],
            "shared denial language",
            "One denial paragraph reused by several agencies. The detector reports only clusters that meet its agency-count and near-duplicate gates.",
        ),
        CrossCorpusSignal::ReusedExtensionGrounds => (
            &[FindingKind::CrossCorpusReusedDenialGrounds],
            "reused extension ground",
            "One statutory extension ground used across topically diverse requests within an agency. Concentration alone does not qualify.",
        ),
        CrossCorpusSignal::BehaviorShift => (
            &[FindingKind::CrossCorpusTemporalShift],
            "behavior shift",
            "An agency's behavior stepping over a specific date. The detector needs a minimum window on each side.",
        ),
        CrossCorpusSignal::OrphanTopic => (
            &[FindingKind::CrossCorpusOrphanTopic],
            "orphan topic",
            "A topic prominent at one agency and absent at another that has enough requests to trust the absence.",
        ),
    };
    let limit = answer.row_limit;
    let (mut hits, stored) = load_findings(repository, kinds, None, limit).await?;
    answer.truncated = hits.len() > limit;
    hits.truncate(limit);
    answer.rows = hits
        .into_iter()
        .map(finding_row)
        .collect::<Result<_, _>>()?;
    for row in &mut answer.rows {
        let portals = row.portal_keys.len();
        row.detail = Some(format!(
            "{} Cites {portals} distinct portal(s).",
            row.detail.clone().unwrap_or_default()
        ));
    }
    finish_findings(answer, stored, "all agencies", label);
    answer.notes.push(gate.to_string());
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Request deadlines
// ---------------------------------------------------------------------------------------------

struct RequestRecord {
    portal_key: String,
    portal_name: Option<String>,
    title: Option<String>,
    agency_name: Option<String>,
    created_at: String,
    updated_at: String,
    closed_at: Option<String>,
    determination_at: Option<String>,
    determination_source: Option<String>,
    extension: Option<(String, String, String, String)>,
}

fn extension_phrase(status: StatutoryExtensionStatus) -> &'static str {
    match status {
        StatutoryExtensionStatus::NotInvoked => "no extension was invoked",
        StatutoryExtensionStatus::Valid => "a valid extension was recorded",
        StatutoryExtensionStatus::NoticeBeforeReceipt => "the extension notice predates receipt",
        StatutoryExtensionStatus::NoticeAfterInitialDeadline => {
            "the extension notice came after the initial deadline"
        }
        StatutoryExtensionStatus::ExpectedDateNotExtended => {
            "the extension's expected date does not extend the deadline"
        }
        StatutoryExtensionStatus::ExtensionExceedsFourteenDays => {
            "the extension exceeds the 14-day maximum"
        }
        StatutoryExtensionStatus::MissingNoticeSource => {
            "the extension has no recorded notice source"
        }
    }
}

async fn deadline_answer(
    repository: &DuckDbRepository,
    answer: &mut IntentAnswer,
    request_id: u64,
    portal_key: Option<String>,
) -> Result<(), AppError> {
    let Ok(id) = i64::try_from(request_id) else {
        answer.summary = format!("No request numbered {request_id} is recorded.");
        return Ok(());
    };
    let scope_portal = portal_key.clone();
    let records = repository
        .query_connection("Intent request lookup", move |connection| {
            let mut statement = connection
                .prepare(
                    "SELECT r.portal_subdomain, p.display_name, r.title, a.name,
                            r.created_at, r.updated_at, r.closed_at, r.determination_at,
                            r.determination_source, e.notice_at, e.expected_determination_at,
                            e.reason, e.notice_source
                     FROM requests r
                     LEFT JOIN agencies a
                       ON a.id = r.agency_id AND a.portal_subdomain = r.portal_subdomain
                     LEFT JOIN portals p ON p.subdomain = r.portal_subdomain
                     LEFT JOIN statutory_extensions e
                       ON e.request_id = r.id AND e.portal_subdomain = r.portal_subdomain
                     WHERE r.id = ?1 AND (?2 IS NULL OR r.portal_subdomain = ?2)
                     ORDER BY r.portal_subdomain
                     LIMIT ?3",
                )
                .map_err(storage)?;
            let rows = statement
                .query_map(
                    params![id, scope_portal, (MAX_ROW_LIMIT + 1) as i64],
                    |row| {
                        let notice: Option<String> = row.get(9)?;
                        let extension = match notice {
                            Some(notice_at) => Some((
                                notice_at,
                                row.get::<_, String>(10)?,
                                row.get::<_, String>(11)?,
                                row.get::<_, String>(12)?,
                            )),
                            None => None,
                        };
                        Ok(RequestRecord {
                            portal_key: row.get(0)?,
                            portal_name: row.get(1)?,
                            title: row.get(2)?,
                            agency_name: row.get(3)?,
                            created_at: row.get(4)?,
                            updated_at: row.get(5)?,
                            closed_at: row.get(6)?,
                            determination_at: row.get(7)?,
                            determination_source: row.get(8)?,
                            extension,
                        })
                    },
                )
                .map_err(storage)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(storage)?;
            Ok(rows)
        })
        .await?;

    match records.len() {
        0 => {
            answer.summary = match &portal_key {
                Some(portal) => {
                    format!("No request {request_id} is recorded in portal \"{portal}\".")
                }
                None => format!("No request {request_id} is recorded in any portal."),
            };
        }
        1 => evaluate_request(answer, request_id, records.into_iter().next())?,
        _ => {
            answer.status = IntentStatus::Ambiguous;
            answer.summary = format!(
                "Request {request_id} exists in {} portals. Name one in quotes.",
                records.len()
            );
            answer.options = records
                .iter()
                .take(MAX_ROW_LIMIT)
                .map(|record| IntentOption {
                    label: format!(
                        "Request {request_id} in {}",
                        record.portal_name.as_deref().unwrap_or(&record.portal_key)
                    ),
                    description: [record.title.as_deref(), record.agency_name.as_deref()]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                        .join(" | "),
                    example: format!(
                        "why did request {request_id} miss its deadline \"{}\"",
                        record.portal_key
                    ),
                })
                .collect();
        }
    }
    Ok(())
}

fn evaluate_request(
    answer: &mut IntentAnswer,
    request_id: u64,
    record: Option<RequestRecord>,
) -> Result<(), AppError> {
    let Some(record) = record else {
        return Ok(());
    };
    let extension = record
        .extension
        .as_ref()
        .map(|(notice_at, expected, reason, source)| {
            RECOGNIZED_EXTENSION_REASONS
                .iter()
                .copied()
                .find(|candidate| candidate.as_str() == reason)
                .map(|reason| StatutoryExtensionRecord {
                    notice_at: notice_at.clone(),
                    expected_determination_at: expected.clone(),
                    reason,
                    notice_source: source.clone(),
                })
                .ok_or_else(|| {
                    AppError::Storage(format!(
                        "Request {request_id} carries an unrecognized extension reason"
                    ))
                })
        })
        .transpose()?;
    let evaluation = evaluate_statutory_compliance_with_determination_at(
        request_id,
        &record.created_at,
        Some(&record.updated_at),
        record.closed_at.as_deref(),
        record.determination_at.as_deref(),
        record.determination_source.as_deref(),
        extension.as_ref(),
        &Utc::now().to_rfc3339(),
    );
    let analysis = match evaluation {
        Ok(analysis) => analysis,
        Err(error) => {
            answer.status = IntentStatus::Unavailable;
            answer.summary = format!("Request {request_id} could not be evaluated: {error}");
            return Ok(());
        }
    };
    let (basis, measured_to) = match analysis.determination_basis {
        DeterminationBasis::SourcedDeterminationEvent => (
            EvidenceBasis::StatutorySourcedDetermination,
            "the sourced determination event",
        ),
        DeterminationBasis::EarliestRecordedActivityProxy => (
            EvidenceBasis::StatutoryActivityProxy,
            "the earliest recorded portal activity, which stands in for the determination",
        ),
        DeterminationBasis::ObservationWithoutRecordedActivity => (
            EvidenceBasis::StatutoryProvisionalObservation,
            "today, because no activity was recorded after receipt",
        ),
    };
    let counts = format!(
        "{} calendar days elapsed against {} allowed",
        analysis.calendar_days_elapsed, analysis.allowed_calendar_days
    );
    let headline = match (analysis.is_violation, analysis.determination_basis) {
        (true, DeterminationBasis::ObservationWithoutRecordedActivity) => format!(
            "Request {request_id} is past its statutory deadline with no recorded response ({counts}); the period is still running."
        ),
        (true, _) => format!(
            "Request {request_id} exceeded its statutory deadline: {counts}, measured to {measured_to}."
        ),
        (false, DeterminationBasis::ObservationWithoutRecordedActivity) => format!(
            "Request {request_id} is still inside its statutory deadline as of today ({counts}); no response is recorded."
        ),
        (false, _) => format!(
            "Request {request_id} did not miss its deadline under this model: {counts}, measured to {measured_to}."
        ),
    };
    let mut detail = format!(
        "Received {}; effective deadline {}; {}.",
        analysis.created_at,
        analysis.effective_deadline_at,
        extension_phrase(analysis.extension_status)
    );
    if let Some(source) = &analysis.determination_source {
        detail.push_str(&format!(" Determination source: {source}."));
    }
    if !analysis.is_violation {
        detail.push_str(
            " The recorded dates do not support the premise that the deadline was missed.",
        );
    }
    let citation = IntentCitation {
        portal_key: record.portal_key.clone(),
        document_id: None,
        character_start: None,
        character_end: None,
        quote: Some(analysis.evidence_summary.clone()),
        label: Some(format!(
            "Recorded timestamps of request {request_id}, evaluated under {}",
            analysis.statute_citation
        )),
    };
    answer.status = IntentStatus::Answered;
    answer.summary = headline.clone();
    answer.rows.push(make_row(
        format!("request:{}:{request_id}", record.portal_key),
        headline,
        Some(detail),
        basis,
        None,
        None,
        record.agency_name,
        vec![record.portal_key],
        vec![citation],
        "",
    ));
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Full-text passages
// ---------------------------------------------------------------------------------------------

/// Locates `anchors` in each document and returns a quote window around the first one found.
/// Offsets are Unicode character offsets into the stored text, matching `SUBSTRING`.
fn passages_for_documents(
    connection: &Connection,
    documents: &[(String, i64)],
    anchors: &[String],
) -> Result<HashMap<(String, i64), IntentCitation>, AppError> {
    if documents.is_empty() {
        return Ok(HashMap::new());
    }
    let wanted = documents
        .iter()
        .map(|(portal, document)| format!("({}, {document})", sql_literal(portal)))
        .collect::<Vec<_>>()
        .join(", ");
    let first = anchors.first().map(|anchor| anchor.to_lowercase());
    let second = anchors
        .get(1)
        .map(|anchor| anchor.to_lowercase())
        .or_else(|| first.clone());
    let sql = format!(
        "SELECT portal, document_id, file_name, first_position, second_position, text_length,
                start_offset, SUBSTRING(extracted_text, start_offset + 1, {QUOTE_CHARACTERS}) AS quote
         FROM (
            SELECT wanted.portal AS portal, wanted.document_id AS document_id, f.file_name,
                   f.extracted_text, LENGTH(f.extracted_text) AS text_length,
                   COALESCE(STRPOS(LOWER(f.extracted_text), ?1), 0) AS first_position,
                   COALESCE(STRPOS(LOWER(f.extracted_text), ?2), 0) AS second_position,
                   GREATEST(
                     COALESCE(LEAST(NULLIF(STRPOS(LOWER(f.extracted_text), ?1), 0),
                                    NULLIF(STRPOS(LOWER(f.extracted_text), ?2), 0)), 1)
                       - 1 - {QUOTE_LEAD_CHARACTERS}, 0) AS start_offset
            FROM (VALUES {wanted}) AS wanted(portal, document_id)
            JOIN documents_fts f
              ON f.portal_subdomain = wanted.portal AND f.document_id = wanted.document_id
         )"
    );
    let mut statement = connection.prepare(&sql).map_err(storage)?;
    let rows = statement
        .query_map(params![first, second], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, String>(7)?,
            ))
        })
        .map_err(storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage)?;
    let mut passages = HashMap::new();
    for (portal, document, file_name, first_at, second_at, length, start, quote) in rows {
        let located = [first_at, second_at]
            .into_iter()
            .filter(|position| *position > 0)
            .count();
        let wanted_both = anchors.len() >= 2;
        let together =
            first_at > 0 && second_at > 0 && (first_at - second_at).abs() < SAME_PASSAGE_CHARACTERS;
        let label = match (located, wanted_both, together) {
            (0, _, _) => "Search terms are present in this document; the exact wording was not located, so no passage is quoted.",
            (_, true, true) => "One passage contains both names.",
            (2, true, false) => "Both names appear in this document, but not in one passage.",
            (_, true, false) => "Only one of the two names could be located as written; the terms are present in this document.",
            (_, false, _) => "Passage containing the search text.",
        };
        let has_quote = located > 0;
        passages.insert(
            (portal.clone(), document),
            IntentCitation {
                portal_key: portal,
                document_id: Some(document),
                character_start: has_quote.then_some(start),
                character_end: has_quote.then_some((start + QUOTE_CHARACTERS).min(length)),
                quote: has_quote.then(|| quote.split_whitespace().collect::<Vec<_>>().join(" ")),
                label: Some(match file_name {
                    Some(name) => format!("{label} ({name})"),
                    None => label.to_string(),
                }),
            },
        );
    }
    Ok(passages)
}

/// The tokens a document must contain, as a SQL `IN` list and the count that must all match.
fn token_filter(text: &str) -> Result<Option<(String, usize)>, AppError> {
    let tokens =
        core_query_tokens(text).map_err(|message| AppError::DataParsing(message.into()))?;
    if tokens.is_empty() {
        return Ok(None);
    }
    Ok(Some((
        tokens
            .iter()
            .map(|token| sql_literal(token))
            .collect::<Vec<_>>()
            .join(", "),
        tokens.len(),
    )))
}

/// The best public documents containing every token of `names`, quoted around the names.
async fn pair_passages(
    repository: &DuckDbRepository,
    names: Vec<String>,
    wanted: usize,
) -> Result<Vec<IntentCitation>, AppError> {
    let Some((tokens, token_count)) = token_filter(&names.join(" "))? else {
        return Ok(Vec::new());
    };
    repository
        .query_connection("Intent passage lookup", move |connection| {
            let sql = format!(
                "SELECT m.portal_subdomain, m.document_id
                 FROM (SELECT document_id, portal_subdomain, CAST(SUM(term_frequency) AS BIGINT) AS frequency
                       FROM document_search_tokens WHERE token IN ({tokens})
                       GROUP BY document_id, portal_subdomain
                       HAVING COUNT(DISTINCT token) = {token_count}) m
                 JOIN documents d
                   ON d.id = m.document_id AND d.portal_subdomain = m.portal_subdomain
                 WHERE LOWER(TRIM(d.visibility)) = 'public'
                 ORDER BY m.frequency DESC, m.portal_subdomain, m.document_id
                 LIMIT {wanted}"
            );
            let mut statement = connection.prepare(&sql).map_err(storage)?;
            let documents = statement
                .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))
                .map_err(storage)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(storage)?;
            let mut passages = passages_for_documents(connection, &documents, &names)?;
            Ok(documents
                .into_iter()
                .filter_map(|key| passages.remove(&key))
                .collect())
        })
        .await
}

// ---------------------------------------------------------------------------------------------
// Appearance across agencies
// ---------------------------------------------------------------------------------------------

struct PortalTotal {
    base_url: Option<String>,
    documents: i64,
    agencies: i64,
}

struct AgencyGroup {
    portal_key: String,
    agency_id: Option<i64>,
    agency_name: String,
    portal_name: String,
    documents: i64,
    occurrences: i64,
    strongest_document: i64,
}

async fn appearance_answer(
    repository: &DuckDbRepository,
    answer: &mut IntentAnswer,
    subject: &str,
) -> Result<(), AppError> {
    let Some((tokens, token_count)) = token_filter(subject)? else {
        answer.summary = "The search text contains no searchable word.".to_string();
        return Ok(());
    };
    let limit = answer.row_limit;
    let anchor = subject.trim().to_string();
    let (totals, groups, passages) = repository
        .query_connection("Intent appearance aggregate", move |connection| {
            let scoped = format!(
                "WITH matches AS (
                    SELECT document_id, portal_subdomain,
                           CAST(SUM(term_frequency) AS BIGINT) AS frequency
                    FROM document_search_tokens WHERE token IN ({tokens})
                    GROUP BY document_id, portal_subdomain
                    HAVING COUNT(DISTINCT token) = {token_count}
                 ), scoped AS (
                    SELECT d.portal_subdomain AS portal, r.agency_id AS agency_id,
                           m.document_id AS document_id, m.frequency AS frequency
                    FROM matches m
                    JOIN documents d
                      ON d.id = m.document_id AND d.portal_subdomain = m.portal_subdomain
                    JOIN requests r
                      ON r.id = d.request_id AND r.portal_subdomain = d.portal_subdomain
                    WHERE LOWER(TRIM(d.visibility)) = 'public'
                 )"
            );
            let mut statement = connection
                .prepare(&format!(
                    "{scoped}
                     SELECT s.portal, p.base_url, COUNT(*), COUNT(DISTINCT s.agency_id)
                     FROM scoped s LEFT JOIN portals p ON p.subdomain = s.portal
                     GROUP BY s.portal, p.base_url
                     ORDER BY s.portal"
                ))
                .map_err(storage)?;
            let totals = statement
                .query_map([], |row| {
                    Ok(PortalTotal {
                        base_url: row.get(1)?,
                        documents: row.get(2)?,
                        agencies: row.get(3)?,
                    })
                })
                .map_err(storage)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(storage)?;
            let mut statement = connection
                .prepare(&format!(
                    "{scoped}
                     SELECT s.portal, s.agency_id, COALESCE(a.name, 'Unmapped Department'),
                            COALESCE(p.display_name, s.portal), COUNT(*),
                            CAST(SUM(s.frequency) AS BIGINT), arg_max(s.document_id, s.frequency)
                     FROM scoped s
                     LEFT JOIN agencies a
                       ON a.id = s.agency_id AND a.portal_subdomain = s.portal
                     LEFT JOIN portals p ON p.subdomain = s.portal
                     GROUP BY s.portal, s.agency_id, a.name, p.display_name
                     ORDER BY COUNT(*) DESC, SUM(s.frequency) DESC, s.portal, s.agency_id
                     LIMIT {}",
                    limit + 1
                ))
                .map_err(storage)?;
            let groups = statement
                .query_map([], |row| {
                    Ok(AgencyGroup {
                        portal_key: row.get(0)?,
                        agency_id: row.get(1)?,
                        agency_name: row.get(2)?,
                        portal_name: row.get(3)?,
                        documents: row.get(4)?,
                        occurrences: row.get(5)?,
                        strongest_document: row.get(6)?,
                    })
                })
                .map_err(storage)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(storage)?;
            let shown = groups
                .iter()
                .take(limit)
                .map(|group| (group.portal_key.clone(), group.strongest_document))
                .collect::<Vec<_>>();
            let passages = passages_for_documents(connection, &shown, &[anchor])?;
            Ok((totals, groups, passages))
        })
        .await?;

    answer.notes.push(
        "Only extracted text is searched. A scanned document without extracted text, and text past the index length cap, are not searched."
            .to_string(),
    );
    if totals.is_empty() {
        answer.summary = format!(
            "No public document in the text index contains every word of \"{subject}\". This does not show it is absent from the records."
        );
        return Ok(());
    }
    let documents: i64 = totals.iter().map(|total| total.documents).sum();
    let agencies: i64 = totals.iter().map(|total| total.agencies).sum();
    let jurisdictions = totals
        .iter()
        .map(|total| {
            total
                .base_url
                .as_deref()
                .map_or("Unclassified source", source_jurisdiction)
        })
        .collect::<BTreeSet<_>>()
        .len();
    let portals = totals.len();
    let threshold = if portals >= CROSS_PORTAL_THRESHOLD {
        format!("This meets the threshold of {CROSS_PORTAL_THRESHOLD} distinct portals for a cross-jurisdiction pattern.")
    } else {
        format!("This is below the threshold of {CROSS_PORTAL_THRESHOLD} distinct portals for a cross-jurisdiction pattern.")
    };
    answer.status = IntentStatus::Answered;
    answer.summary = format!(
        "\"{subject}\" is present in {documents} document(s) across {portals} portal(s), {agencies} mapped agenc{}, {jurisdictions} jurisdiction group(s). {threshold} The corpus does not record whether the publishers are related.",
        if agencies == 1 { "y" } else { "ies" }
    );
    answer.truncated = groups.len() > limit;
    for group in groups.into_iter().take(limit) {
        let citation = passages
            .get(&(group.portal_key.clone(), group.strongest_document))
            .cloned();
        answer.rows.push(make_row(
            format!(
                "appearance:{}:{}",
                group.portal_key,
                group
                    .agency_id
                    .map_or("none".to_string(), |id| id.to_string())
            ),
            format!(
                "{} ({}): {} document(s)",
                group.agency_name, group.portal_name, group.documents
            ),
            Some(format!(
                "{} occurrence(s) of the search words; strongest match in document {}.",
                group.occurrences, group.strongest_document
            )),
            EvidenceBasis::FullTextTermMatch,
            None,
            None,
            Some(group.agency_name),
            vec![group.portal_key],
            citation.into_iter().collect(),
            "The text of the strongest document could not be read from the search index.",
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Corpus graph
// ---------------------------------------------------------------------------------------------

fn name_tokens(text: &str) -> Vec<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Entities whose display value contains every word of `subject`, exact matches first.
async fn resolve_entities(
    corpus: &CorpusIndex,
    subject: &str,
) -> Result<Vec<CorpusEntityNode>, AppError> {
    let wanted = name_tokens(subject);
    if wanted.is_empty() {
        return Ok(Vec::new());
    }
    let mut candidates = corpus
        .top_entities(ENTITY_WINDOW)
        .await?
        .into_iter()
        .filter_map(|node| {
            let have = name_tokens(&node.display_value);
            wanted
                .iter()
                .all(|token| have.contains(token))
                .then_some((have.len() != wanted.len(), node))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then(right.1.document_count.cmp(&left.1.document_count))
            .then(left.1.display_value.cmp(&right.1.display_value))
    });
    Ok(candidates
        .into_iter()
        .take(MAX_ENTITY_CANDIDATES)
        .map(|(_, node)| node)
        .collect())
}

/// The corpus, or the reason it cannot answer.
async fn usable_corpus<'a>(
    corpus: Option<&'a CorpusIndex>,
    answer: &mut IntentAnswer,
) -> Option<&'a CorpusIndex> {
    let Some(corpus) = corpus else {
        answer.status = IntentStatus::Unavailable;
        answer.summary = "The corpus graph is not available.".to_string();
        return None;
    };
    match corpus.status().await {
        Ok(status) if status.indexed_document_count > 0 => Some(corpus),
        Ok(_) => {
            answer.status = IntentStatus::Unavailable;
            answer.summary =
                "The corpus graph holds no indexed documents yet, so it was not searched."
                    .to_string();
            None
        }
        Err(error) => {
            answer.status = IntentStatus::Unavailable;
            answer.summary = format!("The corpus graph could not be read: {error}");
            None
        }
    }
}

fn window_note() -> String {
    format!(
        "Names are matched among the {ENTITY_WINDOW} most-documented entities in the corpus graph; a less-documented entity is not searched."
    )
}

async fn signatories_answer(
    repository: &DuckDbRepository,
    corpus: Option<&CorpusIndex>,
    answer: &mut IntentAnswer,
    subject: &str,
) -> Result<(), AppError> {
    let Some(corpus) = usable_corpus(corpus, answer).await else {
        return Ok(());
    };
    answer.notes.push(window_note());
    let centers = resolve_entities(corpus, subject).await?;
    if centers.is_empty() {
        answer.summary = format!(
            "No entity named \"{subject}\" was found in the corpus graph window. This does not show it is absent from the documents."
        );
        return Ok(());
    }
    let mut signed = Vec::new();
    let mut cut_off = false;
    let mut proximity_only = 0_usize;
    for center in &centers {
        let Some(neighborhood) = corpus
            .entity_neighborhood(center.entity_id.clone(), NEIGHBOR_WINDOW)
            .await?
        else {
            continue;
        };
        cut_off |= neighborhood.truncated;
        for edge in neighborhood.edges {
            match edge.kind {
                RelationshipKind::SignedBy => signed.push((neighborhood.center.clone(), edge)),
                RelationshipKind::CoOccurrence => proximity_only += 1,
                _ => {}
            }
        }
    }
    let limit = answer.row_limit;
    answer.truncated = signed.len() > limit;
    signed.truncate(limit);
    if proximity_only > 0 {
        answer.notes.push(format!(
            "{proximity_only} other entities appear near \"{subject}\" without a signature relationship. Proximity is not a signature and is not listed."
        ));
    }
    if cut_off {
        answer.notes.push(format!(
            "A neighbour list was cut at {NEIGHBOR_WINDOW} entries, so further SIGNED_BY relationships may exist."
        ));
    }
    if signed.is_empty() {
        answer.summary = if cut_off {
            format!(
                "No SIGNED_BY relationship for \"{subject}\" was found within the {NEIGHBOR_WINDOW} strongest neighbours. This does not show none exists."
            )
        } else {
            format!(
                "No SIGNED_BY relationship is recorded for \"{subject}\". Only signature lines the extractor recognized are recorded."
            )
        };
        return Ok(());
    }
    answer.status = IntentStatus::Answered;
    answer.summary = format!(
        "{} typed SIGNED_BY relationship(s) recorded around \"{subject}\".",
        signed.len()
    );
    for (center, edge) in signed {
        let names = vec![
            center.display_value.clone(),
            edge.neighbor.display_value.clone(),
        ];
        let citations = pair_passages(repository, names, MAX_CITATIONS_PER_ROW).await?;
        let mut detail = format!(
            "Recorded in {} document(s); relationship basis {}; neighbour verification {}. The neighbourhood read does not say which side signed for which.",
            edge.shared_document_count,
            edge.basis.as_str(),
            edge.neighbor.validation_grade.as_str()
        );
        if let Some(from) = &edge.valid_from {
            detail.push_str(&format!(" Valid from {from}."));
        }
        if let Some(until) = &edge.valid_until {
            detail.push_str(&format!(" Valid until {until}."));
        }
        answer.rows.push(make_row(
            format!("signed:{}:{}", center.entity_id, edge.neighbor.entity_id),
            format!(
                "{} ({}) has a SIGNED_BY relationship with {} ({})",
                edge.neighbor.display_value,
                edge.neighbor.kind,
                center.display_value,
                center.kind
            ),
            Some(detail),
            EvidenceBasis::TypedRelationship,
            Some(RelationshipKind::SignedBy.as_str().to_string()),
            Some(edge.confidence_basis_points),
            None,
            Vec::new(),
            citations,
            "No public indexed document contains both names; the graph edge has no stored span this API can return.",
        ));
    }
    Ok(())
}

async fn connection_answer(
    repository: &DuckDbRepository,
    corpus: Option<&CorpusIndex>,
    answer: &mut IntentAnswer,
    from: &str,
    to: &str,
) -> Result<(), AppError> {
    let Some(corpus) = usable_corpus(corpus, answer).await else {
        return Ok(());
    };
    answer.notes.push(window_note());
    let start = resolve_entities(corpus, from).await?.into_iter().next();
    let end = resolve_entities(corpus, to).await?.into_iter().next();
    let (Some(start), Some(end)) = (start, end) else {
        answer.summary = format!(
            "\"{from}\" or \"{to}\" was not found in the corpus graph window. This does not show either is absent from the documents."
        );
        return Ok(());
    };
    let Some(path) = corpus
        .find_entity_paths(
            start.entity_id.clone(),
            end.entity_id.clone(),
            MAX_PATH_HOPS,
        )
        .await?
    else {
        answer.summary = format!(
            "No path of up to {MAX_PATH_HOPS} hops joins {} and {} in the corpus graph. The graph is built from extracted entities and is bounded, so this does not show they are unconnected.",
            start.display_value, end.display_value
        );
        return Ok(());
    };
    if path.hops == 0 {
        answer.status = IntentStatus::Answered;
        answer.summary = format!(
            "\"{from}\" and \"{to}\" resolve to the same entity, {}.",
            start.display_value
        );
        return Ok(());
    }
    let hops = &path.steps[1..];
    let proximity_hops = hops
        .iter()
        .filter(|step| step.edge_kind == RelationshipKind::CoOccurrence)
        .count();
    let kinds = hops
        .iter()
        .map(|step| step.edge_kind.as_str())
        .collect::<BTreeSet<_>>();
    let (basis, headline) = if proximity_hops == hops.len() {
        (
            EvidenceBasis::CoOccurrenceOnly,
            format!(
                "{} and {} are linked in {} hop(s) only by co-occurrence: the names appear near each other in text. No relationship between them is stated.",
                start.display_value,
                end.display_value,
                hops.len()
            ),
        )
    } else if proximity_hops == 0 {
        (
            EvidenceBasis::TypedRelationship,
            format!(
                "{} is connected to {} in {} hop(s), each by a relationship the text stated.",
                start.display_value,
                end.display_value,
                hops.len()
            ),
        )
    } else {
        (
            EvidenceBasis::CoOccurrenceOnly,
            format!(
                "{} reaches {} in {} hop(s), but {proximity_hops} of them are only co-occurrence (names near each other), so the chain does not establish a stated relationship.",
                start.display_value,
                end.display_value,
                hops.len()
            ),
        )
    };
    let mut detail_parts = Vec::new();
    let mut citations = Vec::new();
    for (index, step) in hops.iter().enumerate() {
        let previous = &path.steps[index].neighbor;
        detail_parts.push(format!(
            "{} -[{}]- {} ({} shared document(s), basis {})",
            previous.display_value,
            step.edge_kind.as_str(),
            step.neighbor.display_value,
            step.shared_document_count,
            step.edge_basis.as_str()
        ));
        citations.extend(
            pair_passages(
                repository,
                vec![
                    previous.display_value.clone(),
                    step.neighbor.display_value.clone(),
                ],
                1,
            )
            .await?,
        );
    }
    answer.status = IntentStatus::Answered;
    answer.summary = headline.clone();
    answer.rows.push(make_row(
        format!("path:{}:{}", start.entity_id, end.entity_id),
        headline,
        Some(detail_parts.join("; ")),
        basis,
        Some(if kinds.len() == 1 {
            kinds.iter().next().copied().unwrap_or_default().to_string()
        } else {
            "MIXED".to_string()
        }),
        path.weakest_confidence_basis_points,
        None,
        Vec::new(),
        citations,
        "No public indexed document contains both names of any hop.",
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::finding::{
        finding_id, Finding, FindingConfidence, FindingSeverity, FindingSubject,
    };
    use crate::storage::corpus::CorpusDocumentInput;
    use tempfile::tempdir;

    async fn repository() -> DuckDbRepository {
        let repository = DuckDbRepository::new(":memory:").expect("open in-memory core");
        repository
            .run_migrations(include_str!("../../migrations/0001_initial_schema.sql"))
            .await
            .expect("run migrations");
        repository
    }

    async fn run(repository: &DuckDbRepository, sql: String) {
        repository
            .query_connection("Test seed", move |connection| {
                connection.execute_batch(&sql).map_err(storage)
            })
            .await
            .expect("seed");
    }

    /// One portal, one agency, one request, one public document whose text mentions `text`.
    fn seed_document(
        portal: &str,
        agency_id: i64,
        agency: &str,
        document: i64,
        text: &str,
    ) -> String {
        let tokens = name_tokens(text)
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(|token| format!("({document}, '{portal}', '{token}', 2)"))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "INSERT OR IGNORE INTO portals (subdomain, display_name, base_url)
               VALUES ('{portal}', '{portal} portal', 'https://{portal}.example.org');
             INSERT OR IGNORE INTO agencies (id, portal_subdomain, name)
               VALUES ({agency_id}, '{portal}', '{agency}');
             INSERT INTO requests (id, portal_subdomain, title, status, created_at, updated_at, agency_id)
               VALUES ({document}, '{portal}', 'Request {document}', 'closed',
                       '2026-01-01T00:00:00Z', '2026-01-05T00:00:00Z', {agency_id});
             INSERT INTO documents (id, request_id, portal_subdomain, file_name, visibility, created_at, updated_at)
               VALUES ({document}, {document}, '{portal}', 'letter-{document}.pdf', 'public',
                       '2026-01-02T00:00:00Z', '2026-01-02T00:00:00Z');
             INSERT INTO documents_fts (document_id, portal_subdomain, file_name, extracted_text)
               VALUES ({document}, '{portal}', 'letter-{document}.pdf', '{text}');
             INSERT INTO document_search_tokens (document_id, portal_subdomain, token, term_frequency)
               VALUES {tokens};"
        )
    }

    fn finding(kind: FindingKind, portal: &str, request: i64, document: i64) -> Finding {
        let subject = FindingSubject {
            portal_key: Some(portal.into()),
            request_id: Some(request),
            document_id: Some(document),
            ..Default::default()
        };
        let now = Utc::now();
        Finding {
            id: finding_id(kind, "test", &subject, &[]),
            kind,
            severity: FindingSeverity::Notable,
            confidence: FindingConfidence::measured(7000),
            subject,
            headline: format!("{} in {portal}", kind.as_str()),
            explanation: "explained".into(),
            citations: vec![FindingCitation {
                portal_key: portal.into(),
                document_id: document,
                character_start: Some(3),
                character_end: Some(9),
                quote: Some("quoted".into()),
                label: None,
            }],
            discovered_at: now,
            last_seen_at: now,
            dismissed_at: None,
            source_detector: "test".into(),
        }
    }

    /// The invariant every row must hold, checked on every answer below.
    fn assert_rows_are_cited(answer: &IntentAnswer) {
        assert!(answer.rows.len() <= answer.row_limit);
        for row in &answer.rows {
            assert_eq!(
                row.citations.is_empty(),
                row.citations_unavailable_reason.is_some(),
                "row {} must carry citations or say why not",
                row.id
            );
            assert!(row.citations.len() <= MAX_CITATIONS_PER_ROW);
            assert!(!row.basis_statement.is_empty());
        }
    }

    async fn ask(repository: &DuckDbRepository, text: &str, limit: usize) -> IntentAnswer {
        let answer = answer_utterance(repository, None, text, limit)
            .await
            .expect("answer");
        assert_rows_are_cited(&answer);
        answer
    }

    #[tokio::test]
    async fn appearance_counts_per_agency_and_quotes_the_exact_span() {
        let repo = repository().await;
        let text = "Payment approved to Acme Paving Inc for road work";
        for (index, portal) in ["a", "b", "c", "d"].iter().enumerate() {
            let document = index as i64 + 1;
            run(
                &repo,
                seed_document(portal, 7, &format!("Agency {portal}"), document, text),
            )
            .await;
        }
        let answer = ask(
            &repo,
            "where does \"Acme Paving\" appear across agencies",
            10,
        )
        .await;
        assert_eq!(answer.status, IntentStatus::Answered);
        assert_eq!(answer.rows.len(), 4);
        assert!(answer.summary.contains("4 portal(s)"), "{}", answer.summary);
        assert!(
            answer.summary.contains("meets the threshold"),
            "{}",
            answer.summary
        );
        for row in &answer.rows {
            assert_eq!(row.basis, EvidenceBasis::FullTextTermMatch);
            let citation = &row.citations[0];
            let quote = citation.quote.as_deref().expect("a located quote");
            assert!(quote.contains("Acme Paving"), "{quote}");
            let (start, end) = (
                citation.character_start.unwrap() as usize,
                citation.character_end.unwrap() as usize,
            );
            let window = text
                .chars()
                .skip(start)
                .take(end - start)
                .collect::<String>();
            assert_eq!(window, quote, "offsets must reproduce the quote");
        }
    }

    #[tokio::test]
    async fn two_portals_are_below_the_cross_jurisdiction_threshold_and_never_absent() {
        let repo = repository().await;
        for (index, portal) in ["a", "b"].iter().enumerate() {
            run(
                &repo,
                seed_document(portal, 1, "Agency", index as i64 + 1, "Acme Paving Inc"),
            )
            .await;
        }
        let answer = ask(&repo, "where does \"Acme Paving\" appear", 10).await;
        assert!(
            answer.summary.contains("below the threshold"),
            "{}",
            answer.summary
        );
        let none = ask(&repo, "where does \"Zeta Corp\" appear", 10).await;
        assert_eq!(none.status, IntentStatus::NoMatch);
        assert!(none.summary.contains("does not show"), "{}", none.summary);
    }

    #[tokio::test]
    async fn row_limit_is_exact_at_limit_minus_one_limit_and_limit_plus_one() {
        let repo = repository().await;
        for index in 1..=4_i64 {
            run(
                &repo,
                seed_document(&format!("p{index}"), 1, "Agency", index, "Acme Paving"),
            )
            .await;
        }
        for (limit, rows, truncated) in [(3, 3, true), (4, 4, false), (5, 4, false)] {
            let answer = ask(&repo, "where does \"Acme Paving\" appear", limit).await;
            assert_eq!(
                (answer.rows.len(), answer.truncated),
                (rows, truncated),
                "limit {limit}"
            );
        }
        for invalid in [0, MAX_ROW_LIMIT + 1] {
            assert!(
                answer_utterance(&repo, None, "show contradictions", invalid)
                    .await
                    .is_err()
            );
        }
    }

    #[tokio::test]
    async fn findings_row_cap_holds_at_the_boundaries_for_stored_findings() {
        let repo = repository().await;
        let findings = (1..=4)
            .map(|index| finding(FindingKind::ContradictoryStatement, "a", index, index))
            .collect::<Vec<_>>();
        for index in 1..=4 {
            run(&repo, seed_document("a", 1, "Agency", index, "text")).await;
        }
        repo.record_findings(findings).await.unwrap();
        for (limit, rows, truncated) in [(3, 3, true), (4, 4, false), (5, 4, false)] {
            let answer = ask(&repo, "show contradictions", limit).await;
            assert_eq!(
                (answer.rows.len(), answer.truncated),
                (rows, truncated),
                "limit {limit}"
            );
        }
    }

    #[tokio::test]
    async fn contradictions_are_scoped_to_the_agency_of_their_request() {
        let repo = repository().await;
        run(&repo, seed_document("a", 1, "Public Works", 1, "text one")).await;
        run(
            &repo,
            seed_document("a", 2, "Parks Department", 2, "text two"),
        )
        .await;
        repo.record_findings(vec![
            finding(FindingKind::ContradictoryStatement, "a", 1, 1),
            finding(FindingKind::ContradictoryStatement, "a", 2, 2),
        ])
        .await
        .unwrap();
        let works = ask(&repo, "show contradictions in agency Public Works", 10).await;
        assert_eq!(works.rows.len(), 1);
        assert_eq!(works.rows[0].agency_name.as_deref(), Some("Public Works"));
        assert_eq!(works.rows[0].basis, EvidenceBasis::StoredFinding);
        let all = ask(&repo, "show contradictions", 10).await;
        assert_eq!(all.rows.len(), 2);
        let none = ask(&repo, "show contradictions in agency Harbor", 10).await;
        assert_eq!(none.status, IntentStatus::NoMatch);
        assert!(none.summary.contains("does not show"), "{}", none.summary);
    }

    #[tokio::test]
    async fn an_empty_findings_table_is_unavailable_not_a_clean_bill() {
        let repo = repository().await;
        let answer = ask(&repo, "same denial language across agencies", 10).await;
        assert_eq!(answer.status, IntentStatus::Unavailable);
        assert!(answer.summary.contains("refresh"), "{}", answer.summary);
    }

    #[tokio::test]
    async fn shared_language_rows_report_the_portals_their_citations_span() {
        let repo = repository().await;
        for (index, portal) in ["a", "b", "c"].iter().enumerate() {
            run(
                &repo,
                seed_document(portal, 1, "Agency", index as i64 + 1, "denial"),
            )
            .await;
        }
        let mut shared = finding(FindingKind::CrossCorpusSharedLanguage, "a", 1, 1);
        shared.subject = FindingSubject {
            document_id: Some(1),
            ..Default::default()
        };
        shared.id = finding_id(shared.kind, "test", &shared.subject, &[]);
        shared.citations = ["a", "b", "c"]
            .iter()
            .enumerate()
            .map(|(index, portal)| FindingCitation {
                portal_key: (*portal).into(),
                document_id: index as i64 + 1,
                character_start: Some(0),
                character_end: Some(6),
                quote: Some("denial".into()),
                label: None,
            })
            .collect();
        repo.record_findings(vec![shared]).await.unwrap();
        let answer = ask(&repo, "same denial language across agencies", 10).await;
        assert_eq!(answer.rows.len(), 1);
        assert_eq!(answer.rows[0].portal_keys, vec!["a", "b", "c"]);
        assert!(answer.rows[0]
            .detail
            .as_deref()
            .unwrap()
            .contains("3 distinct portal(s)"));
    }

    #[tokio::test]
    async fn deadline_answers_state_which_determination_basis_they_rest_on() {
        let repo = repository().await;
        run(
            &repo,
            "INSERT INTO portals (subdomain, display_name, base_url) VALUES ('a', 'A', 'https://a.example.org');
             INSERT INTO requests (id, portal_subdomain, title, status, created_at, updated_at, determination_at, determination_source)
               VALUES (1, 'a', 'sourced ok', 'closed', '2026-01-01T00:00:00Z', '2026-01-03T00:00:00Z', '2026-01-04T00:00:00Z', 'notice letter');
             INSERT INTO requests (id, portal_subdomain, title, status, created_at, updated_at)
               VALUES (2, 'a', 'proxy late', 'closed', '2026-01-01T00:00:00Z', '2026-03-01T00:00:00Z');
             INSERT INTO requests (id, portal_subdomain, title, status, created_at, updated_at)
               VALUES (3, 'a', 'no activity', 'open', '2026-01-10T00:00:00Z', '2026-01-05T00:00:00Z');"
                .into(),
        )
        .await;
        let sourced = ask(&repo, "why did request 1 miss its deadline", 10).await;
        assert_eq!(
            sourced.rows[0].basis,
            EvidenceBasis::StatutorySourcedDetermination
        );
        assert!(
            sourced.summary.contains("did not miss"),
            "{}",
            sourced.summary
        );
        assert_eq!(sourced.rows[0].confidence_basis_points, None);

        let proxy = ask(&repo, "why did request 2 miss its deadline", 10).await;
        assert_eq!(proxy.rows[0].basis, EvidenceBasis::StatutoryActivityProxy);
        assert!(proxy.summary.contains("exceeded"), "{}", proxy.summary);
        assert!(
            proxy.summary.contains("stands in for the determination"),
            "{}",
            proxy.summary
        );

        let provisional = ask(&repo, "why did request 3 miss its deadline", 10).await;
        assert_eq!(
            provisional.rows[0].basis,
            EvidenceBasis::StatutoryProvisionalObservation
        );
        assert!(
            provisional.summary.contains("still running"),
            "{}",
            provisional.summary
        );

        let missing = ask(&repo, "why did request 99 miss its deadline", 10).await;
        assert_eq!(missing.status, IntentStatus::NoMatch);
    }

    #[tokio::test]
    async fn a_request_number_shared_by_two_portals_is_disambiguated_not_picked() {
        let repo = repository().await;
        run(
            &repo,
            "INSERT INTO portals (subdomain, display_name, base_url) VALUES
               ('a', 'A', 'https://a.example.org'), ('b', 'B', 'https://b.example.org');
             INSERT INTO requests (id, portal_subdomain, title, status, created_at, updated_at) VALUES
               (5, 'a', 'first', 'closed', '2026-01-01T00:00:00Z', '2026-01-02T00:00:00Z'),
               (5, 'b', 'second', 'closed', '2026-01-01T00:00:00Z', '2026-03-02T00:00:00Z');"
                .into(),
        )
        .await;
        let ambiguous = ask(&repo, "why did request 5 miss its deadline", 10).await;
        assert_eq!(ambiguous.status, IntentStatus::Ambiguous);
        assert!(ambiguous.rows.is_empty());
        assert_eq!(ambiguous.options.len(), 2);
        let follow_up = ask(&repo, &ambiguous.options[1].example, 10).await;
        assert_eq!(follow_up.status, IntentStatus::Answered);
        assert_eq!(follow_up.rows[0].portal_keys, vec!["b"]);
    }

    #[tokio::test]
    async fn unresolved_intent_lists_what_can_be_asked_and_reads_nothing() {
        let repo = repository().await;
        let answer = ask(&repo, "tell me a joke", 10).await;
        assert_eq!(answer.status, IntentStatus::NotUnderstood);
        assert!(answer.plan.is_none() && answer.rows.is_empty());
        assert_eq!(answer.options, intent::capabilities());
        let too_long = "x".repeat(MAX_UTTERANCE_CHARS + 1);
        assert!(answer_utterance(&repo, None, &too_long, 5).await.is_err());
    }

    #[tokio::test]
    async fn graph_intents_report_an_unavailable_corpus_instead_of_no_match() {
        let repo = repository().await;
        let missing = ask(&repo, "who signed for the City Attorney", 10).await;
        assert_eq!(missing.status, IntentStatus::Unavailable);
        let directory = tempdir().unwrap();
        let empty = CorpusIndex::open(directory.path().join("corpus.db")).unwrap();
        let answer = answer_utterance(&repo, Some(&empty), "who signed for the City Attorney", 10)
            .await
            .unwrap();
        assert_eq!(answer.status, IntentStatus::Unavailable);
        assert!(
            answer.summary.contains("no indexed documents"),
            "{}",
            answer.summary
        );
    }

    const SIGNED_LETTER: &str = "Sincerely,\nJane Q. Doe\nCity Attorney";

    /// A typed edge seeded directly. `entity_neighborhood` reduces each pair's edges with
    /// `MIN(relationship_kind)`, so a pair that also co-occurs reads as CO_OCCURRENCE; the
    /// extractor emits both for a real signature block. This pair therefore carries only the
    /// typed edge, which is the one shape the public read returns as SIGNED_BY.
    async fn corpus_with_signature(repo: &DuckDbRepository) -> (CorpusIndex, tempfile::TempDir) {
        let directory = tempdir().unwrap();
        let path = directory.path().join("corpus.db");
        drop(CorpusIndex::open(&path).unwrap());
        {
            let connection = duckdb::Connection::open(&path).unwrap();
            connection
                .execute_batch(
                    "INSERT INTO corpus_documents VALUES
                        ('a', 1, 100, repeat('a', 64), repeat('b', 64), 3, repeat('0', 16),
                         FALSE, 'deterministic-corpus-v3', '2026-09-29T00:00:00Z');
                     INSERT INTO corpus_entities VALUES
                        ('e-role', 'ROLE', 'city attorney', 'City Attorney', 'EXACT_NORMALIZED_MATCH'),
                        ('e-person', 'PERSON', 'jane q. doe', 'Jane Q. Doe', 'EXACT_NORMALIZED_MATCH');
                     INSERT INTO corpus_entity_document_counts VALUES ('e-role', 1), ('e-person', 1);
                     INSERT INTO corpus_entity_mentions VALUES
                        ('a', 100, 'e-role', 0, 0, 13, 5000, 'test', 1, 'FORMAT_VALIDATED'),
                        ('a', 100, 'e-person', 0, 14, 25, 5000, 'test', 1, 'FORMAT_VALIDATED');
                     INSERT INTO corpus_entity_relationships VALUES
                        ('a', 100, 'e-role', 'e-person', 'SIGNED_BY', 'OBSERVED',
                         0, 0, 25, 6500, NULL, NULL);",
                )
                .unwrap();
        }
        let corpus = CorpusIndex::open(&path).unwrap();
        run(
            repo,
            seed_document("a", 1, "Agency", 100, &SIGNED_LETTER.replace('\n', " ")),
        )
        .await;
        (corpus, directory)
    }

    #[tokio::test]
    async fn signatories_come_from_typed_edges_with_a_passage_citing_both_names() {
        let repo = repository().await;
        let (corpus, _keep) = corpus_with_signature(&repo).await;
        let answer = answer_utterance(&repo, Some(&corpus), "who signed for the City Attorney", 10)
            .await
            .unwrap();
        assert_rows_are_cited(&answer);
        assert_eq!(answer.status, IntentStatus::Answered, "{}", answer.summary);
        let row = &answer.rows[0];
        assert_eq!(row.basis, EvidenceBasis::TypedRelationship);
        assert_eq!(row.relationship_kind.as_deref(), Some("SIGNED_BY"));
        assert!(row.headline.contains("SIGNED_BY"), "{}", row.headline);
        assert!(
            row.headline.to_lowercase().contains("jane"),
            "{}",
            row.headline
        );
        let quote = row.citations[0].quote.as_deref().unwrap();
        assert!(
            quote.contains("Jane Q. Doe") && quote.contains("City Attorney"),
            "{quote}"
        );
    }

    #[tokio::test]
    async fn co_occurrence_is_never_phrased_as_a_relationship() {
        let repo = repository().await;
        let directory = tempdir().unwrap();
        let corpus = CorpusIndex::open(directory.path().join("corpus.db")).unwrap();
        let text = "The Records Officer and the Budget Analyst attended the meeting together.";
        corpus
            .index_document(CorpusDocumentInput {
                portal_key: "a".into(),
                request_id: 1,
                document_id: 1,
                raw_file_sha256: format!("{:064x}", 1),
                extracted_text_sha256: format!("{:064x}", 2),
                text: text.into(),
            })
            .await
            .unwrap();
        run(&repo, seed_document("a", 1, "Agency", 1, text)).await;
        let answer = answer_utterance(
            &repo,
            Some(&corpus),
            "how is \"Records Officer\" connected to \"Budget Analyst\"",
            10,
        )
        .await
        .unwrap();
        assert_rows_are_cited(&answer);
        for row in &answer.rows {
            assert_ne!(
                row.basis,
                EvidenceBasis::TypedRelationship,
                "{}",
                row.headline
            );
            assert!(row.headline.contains("co-occurrence"), "{}", row.headline);
            assert!(row.basis_statement.contains("not a stated relationship"));
        }
        let signatories = answer_utterance(
            &repo,
            Some(&corpus),
            "who signed for the Records Officer",
            10,
        )
        .await
        .unwrap();
        assert!(
            signatories.rows.is_empty(),
            "proximity must not read as a signature"
        );
        assert!(
            !signatories.summary.contains("Budget"),
            "{}",
            signatories.summary
        );
    }
}
