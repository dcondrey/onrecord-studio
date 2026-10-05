use super::casework_operations::{
    apply_operations, chain, chain_is_intact, operations_between, partition_operations,
};
use super::sqlite::DuckDbRepository;
use crate::{
    analysis::request_compiler::{compile_cpra_request, plan_filing, RequestScope},
    error::AppError,
    integrity::{sha256_hex, sign_package, verify_package, SignedPackage},
    types::{casework::*, research::*},
};
use duckdb::{params, params_from_iter, Connection, OptionalExt, ToSql};
use ed25519_dalek::SigningKey;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
};

const MAX_PACKET: usize = 2_000_000;
/// Bumped from v1 to v2 alongside the ledger and redaction fields below; both landed in the same
/// bump per CLAUDE.md's schema-version guidance, since both change the signed manifest shape.
pub(crate) const CASE_EVIDENCE_PACKAGE_SCHEMA: &str = "onrecord-case-evidence-package-v2";
pub(crate) const REQUEST_FILING_RECORD_SCHEMA: &str = "onrecord-request-filing-record-v1";
/// `outcome` values for `casework_export_outcomes`. A reservation with no outcome row at all is a
/// third, implicit state ("reserved, not confirmed written") that `case_export_chain` reports
/// explicitly rather than folding into either of these.
pub(crate) const EXPORT_OUTCOME_WRITTEN: &str = "WRITTEN";
pub(crate) const EXPORT_OUTCOME_FAILED: &str = "FAILED";
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS casework_cases(id TEXT PRIMARY KEY,report_id TEXT NOT NULL,topic TEXT NOT NULL,disposition TEXT NOT NULL,revision BIGINT NOT NULL,payload_zstd BLOB NOT NULL,updated_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS casework_events(case_id TEXT NOT NULL,revision BIGINT NOT NULL,author TEXT NOT NULL,recorded_at TEXT NOT NULL,summary TEXT NOT NULL,payload_zstd BLOB NOT NULL,PRIMARY KEY(case_id,revision));
CREATE TABLE IF NOT EXISTS casework_sources(case_id TEXT NOT NULL,portal_key TEXT NOT NULL,document_id BIGINT NOT NULL,source_sha256 TEXT NOT NULL,PRIMARY KEY(case_id,portal_key,document_id));
CREATE TABLE IF NOT EXISTS casework_origins(case_id TEXT PRIMARY KEY,base_revision BIGINT NOT NULL,base_sha256 TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS casework_imports(case_id TEXT NOT NULL,revision BIGINT NOT NULL,packet_sha256 TEXT NOT NULL,payload_zstd BLOB NOT NULL,PRIMARY KEY(case_id,revision));
CREATE TABLE IF NOT EXISTS casework_export_ledger(case_id TEXT NOT NULL,sequence BIGINT NOT NULL,manifest_sha256 TEXT NOT NULL,previous_manifest_sha256 TEXT,signer_key_id TEXT NOT NULL,reserved_at TEXT NOT NULL,PRIMARY KEY(case_id,sequence),UNIQUE(manifest_sha256));
CREATE TABLE IF NOT EXISTS casework_operations(case_id TEXT NOT NULL,revision BIGINT NOT NULL,sequence BIGINT NOT NULL,author TEXT NOT NULL,recorded_at TEXT NOT NULL,kind TEXT NOT NULL,section TEXT NOT NULL,target TEXT NOT NULL,summary TEXT NOT NULL,content_sha256 TEXT,previous_hash TEXT,hash TEXT NOT NULL,PRIMARY KEY(case_id,revision,sequence));
CREATE TABLE IF NOT EXISTS casework_export_outcomes(case_id TEXT NOT NULL,sequence BIGINT NOT NULL,outcome TEXT NOT NULL,detail TEXT,recorded_at TEXT NOT NULL);
";
fn storage(e: impl std::fmt::Display) -> AppError {
    AppError::Storage(e.to_string())
}
fn invalid(message: impl Into<String>) -> AppError {
    AppError::DataParsing(message.into())
}
fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}
fn encode<T: serde::Serialize>(value: &T) -> Result<Vec<u8>, AppError> {
    let bytes = serde_json::to_vec(value).map_err(storage)?;
    if bytes.len() > MAX_PACKET {
        return Err(invalid(
            "Case packet exceeds the 2 MB structured-data limit.",
        ));
    }
    zstd::stream::encode_all(bytes.as_slice(), 9).map_err(storage)
}
fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, AppError> {
    let mut raw = Vec::new();
    zstd::stream::read::Decoder::new(bytes)
        .map_err(storage)?
        .take((MAX_PACKET + 1) as u64)
        .read_to_end(&mut raw)
        .map_err(storage)?;
    if raw.len() > MAX_PACKET {
        return Err(invalid("Stored case packet exceeds the read limit."));
    }
    serde_json::from_slice(&raw).map_err(storage)
}
fn read_case(conn: &Connection, id: &str) -> Result<Option<CaseworkCase>, AppError> {
    conn.query_row(
        "SELECT payload_zstd FROM casework_cases WHERE id=?1",
        params![id],
        |r| r.get::<_, Vec<u8>>(0),
    )
    .optional()
    .map_err(storage)?
    .map(|b| {
        let mut case = decode(&b)?;
        refresh_case(conn, &mut case)?;
        Ok(case)
    })
    .transpose()
}
fn read_report(conn: &Connection, id: &str) -> Result<Option<ResearchReport>, AppError> {
    conn.query_row(
        "SELECT report_zstd FROM research_reports WHERE id=?1",
        params![id],
        |r| r.get::<_, Vec<u8>>(0),
    )
    .optional()
    .map_err(storage)?
    .map(|b| decode(&b))
    .transpose()
}
fn reports(conn: &Connection) -> Result<Vec<ResearchReport>, AppError> {
    let rows = conn
        .prepare("SELECT report_zstd FROM research_reports ORDER BY created_at DESC LIMIT 60")
        .map_err(storage)?
        .query_map([], |r| r.get::<_, Vec<u8>>(0))
        .map_err(storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage)?;
    rows.iter().map(|b| decode(b)).collect()
}
fn cases(conn: &Connection) -> Result<Vec<CaseworkCase>, AppError> {
    let rows = conn
        .prepare("SELECT payload_zstd FROM casework_cases ORDER BY updated_at DESC LIMIT 200")
        .map_err(storage)?
        .query_map([], |r| r.get::<_, Vec<u8>>(0))
        .map_err(storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage)?;
    rows.iter()
        .map(|b| {
            let mut case = decode(b)?;
            refresh_case(conn, &mut case)?;
            Ok(case)
        })
        .collect()
}
fn events(conn: &Connection) -> Result<Vec<CaseworkEvent>, AppError> {
    conn.prepare("SELECT case_id,revision,author,recorded_at,summary FROM casework_events ORDER BY recorded_at DESC LIMIT 1000").map_err(storage)?.query_map([],|r|Ok(CaseworkEvent{case_id:r.get(0)?,revision:r.get(1)?,author:r.get(2)?,at:r.get(3)?,summary:r.get(4)?})).map_err(storage)?.collect::<Result<Vec<_>,_>>().map_err(storage)
}
fn portable_history(conn: &Connection, case_id: &str) -> Result<Vec<CaseworkEvent>, AppError> {
    // Scope before bounding: activity in other cases must never evict this case's provenance.
    let mut history = conn.prepare("SELECT case_id,revision,author,recorded_at,summary FROM casework_events WHERE case_id=?1 ORDER BY revision LIMIT 1001").map_err(storage)?.query_map(params![case_id], |row| Ok(CaseworkEvent { case_id: row.get(0)?, revision: row.get(1)?, author: row.get(2)?, at: row.get(3)?, summary: row.get(4)? })).map_err(storage)?.collect::<Result<Vec<_>,_>>().map_err(storage)?;
    if history.len() > 1000 {
        return Err(invalid("This case exceeds the 1,000-event portable history limit. No packet was created and the stored history remains intact."));
    }
    let import_count: u64 = conn
        .query_row(
            "SELECT COUNT(*) FROM casework_imports WHERE case_id=?1",
            params![case_id],
            |row| row.get(0),
        )
        .map_err(storage)?;
    if import_count > 40 {
        return Err(invalid("This case exceeds the 40 imported-packet export limit. No packet was created; all imported contribution histories remain stored locally."));
    }
    let packets=conn.prepare("SELECT packet_sha256,payload_zstd FROM casework_imports WHERE case_id=?1 ORDER BY revision DESC").map_err(storage)?.query_map(params![case_id],|row|Ok((row.get::<_,String>(0)?,row.get::<_,Vec<u8>>(1)?))).map_err(storage)?.collect::<Result<Vec<_>,_>>().map_err(storage)?;
    for (digest, blob) in packets {
        let packet: CaseworkPacket = decode(&blob)?;
        for mut event in packet.history {
            if !event.summary.starts_with("Portable history from packet ") {
                event.summary = format!(
                    "Portable history from packet {digest} (claimed contribution): {}",
                    event.summary
                );
            }
            history.push(event);
        }
    }
    let mut seen = BTreeSet::new();
    history.retain(|event| {
        seen.insert((
            event.case_id.clone(),
            event.revision,
            event.author.clone(),
            event.at.clone(),
            event.summary.clone(),
        ))
    });
    history.sort_by(|a, b| a.at.cmp(&b.at).then(a.revision.cmp(&b.revision)));
    if history.len() > 1000 {
        return Err(invalid("This case exceeds the 1,000-event portable history limit. No packet was created and the stored history remains intact."));
    }
    Ok(history)
}

fn canonical_citation(
    report: &ResearchReport,
    evidence: &ResearchEvidence,
    quote: &str,
) -> CaseworkCitation {
    CaseworkCitation {
        id: format!(
            "{}:{}:{}",
            report.id,
            evidence.id,
            &sha256_hex(quote.as_bytes())[..12]
        ),
        report_id: report.id.clone(),
        evidence_id: evidence.id.clone(),
        label: evidence.label.clone(),
        quote: quote.into(),
        source_url: evidence.source_url.clone(),
        source_sha256: evidence.source_sha256.clone(),
        source_references: evidence.source_references.clone(),
        portal_key: evidence.portal_key.clone(),
        document_id: evidence.document_id,
        character_offset: evidence.character_offset.and_then(|base| {
            evidence
                .text
                .find(quote)
                .map(|offset| base + evidence.text[..offset].chars().count())
        }),
        verification: "SAVED_REPORT".into(),
    }
}
fn citation_ids(
    report: &ResearchReport,
    citations: &[ResearchCitation],
    evidence: &mut Vec<CaseworkCitation>,
) -> Vec<String> {
    citations
        .iter()
        .filter_map(|citation| {
            report
                .evidence
                .iter()
                .find(|e| e.id == citation.evidence_id)
                .map(|record| {
                    let item = canonical_citation(report, record, &citation.quote);
                    let id = item.id.clone();
                    if !evidence.iter().any(|e| e.id == id) {
                        evidence.push(item);
                    }
                    id
                })
        })
        .collect()
}
fn references(
    report: &ResearchReport,
    ids: &[String],
    evidence: &mut Vec<CaseworkCitation>,
) -> Vec<String> {
    let citations = ids
        .iter()
        .filter_map(|id| {
            report
                .evidence
                .iter()
                .find(|e| &e.id == id)
                .map(|e| ResearchCitation {
                    evidence_id: id.clone(),
                    quote: e.text.chars().take(1600).collect(),
                })
        })
        .collect::<Vec<_>>();
    citation_ids(report, &citations, evidence)
}
fn quote_exists(source: &ResearchEvidence, quote: &str) -> bool {
    if source.sql.is_some() {
        if let (Ok(result), Ok(quoted)) = (
            serde_json::from_str::<serde_json::Value>(&source.text),
            serde_json::from_str::<serde_json::Value>(quote),
        ) {
            if quoted.is_array() {
                return result
                    .get("rows")
                    .and_then(|rows| rows.as_array())
                    .is_some_and(|rows| rows.contains(&quoted));
            }
        }
    }
    normalized(&source.text).contains(&normalized(quote))
}

fn normalized(value: &str) -> String {
    let mut text = String::with_capacity(value.len());
    for word in value.split_whitespace() {
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(word);
    }
    text
}
fn valid_date(value: &str) -> bool {
    chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").is_ok()
}
fn quoted_date(date: &str, items: &[&CaseworkCitation]) -> bool {
    let Ok(date) = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d") else {
        return false;
    };
    let forms = [
        date.format("%Y-%m-%d").to_string(),
        date.format("%B %-d, %Y").to_string(),
        date.format("%B %-d %Y").to_string(),
        date.format("%b %-d, %Y").to_string(),
        date.format("%b %-d %Y").to_string(),
        date.format("%-m/%-d/%Y").to_string(),
        date.format("%m/%d/%Y").to_string(),
    ];
    items.iter().any(|cite| {
        forms.iter().any(|d| {
            normalized(&cite.quote)
                .to_lowercase()
                .contains(&d.to_lowercase())
        })
    })
}
fn create_from_report(report: &ResearchReport, states: &BTreeMap<String, String>) -> CaseworkCase {
    let mut evidence = Vec::new();
    let mut questions = report
        .synthesis
        .findings
        .iter()
        .enumerate()
        .map(|(i, f)| CaseworkQuestion {
            id: format!("finding-{i}"),
            question: f.title.clone(),
            hypothesis: f.interpretation.clone(),
            next_test: f.next_test.clone(),
            disposition: match states
                .get(&format!("{}:finding:{i}", report.id))
                .map(String::as_str)
            {
                Some("DISMISSED") => "DISMISSED",
                _ => "OPEN",
            }
            .into(),
            owner: String::new(),
            citation_ids: citation_ids(report, &f.citations, &mut evidence),
            counter_citation_ids: citation_ids(report, &f.counter_evidence, &mut evidence),
        })
        .collect::<Vec<_>>();
    questions.extend(report.synthesis.questions.iter().enumerate().map(|(i, q)| {
        CaseworkQuestion {
            id: format!("question-{i}"),
            question: q.question.clone(),
            hypothesis: String::new(),
            next_test: q.why.clone(),
            disposition: "OPEN".into(),
            owner: String::new(),
            citation_ids: references(report, &q.evidence_ids, &mut evidence),
            counter_citation_ids: vec![],
        }
    }));
    let requests = report
        .synthesis
        .requests
        .iter()
        .enumerate()
        .map(|(i, r)| CaseworkRequest {
            id: format!("request-{i}"),
            title: r.title.clone(),
            custodian: r.custodian.clone(),
            gap: r.gap.clone(),
            resolves: r.resolves.clone(),
            draft: r.draft.clone(),
            state: "DRAFT".into(),
            expected_record_types: vec![r.title.clone()],
            expected_fields: vec![],
            expected_period: String::new(),
            citation_ids: references(report, &r.evidence_ids, &mut evidence),
            ..Default::default()
        })
        .collect();
    let decisions = report
        .synthesis
        .meetings
        .iter()
        .enumerate()
        .map(|(i, m)| CaseworkDecision {
            id: format!("meeting-{i}"),
            body: m.body.clone(),
            date: m.date.clone(),
            agenda_item: m.agenda_item.clone(),
            proposed_decision: m.relevance.clone(),
            action: m.action.clone(),
            state: "PROPOSED".into(),
            citation_ids: citation_ids(report, &m.citations, &mut evidence),
            ..Default::default()
        })
        .collect();
    CaseworkCase {
        id: format!("case-{}", uuid::Uuid::new_v4()),
        report_id: report.id.clone(),
        topic: report.topic.clone(),
        title: report.synthesis.title.clone(),
        owner: String::new(),
        disposition: "OPEN".into(),
        revision: 0,
        created_at: now(),
        updated_at: now(),
        questions,
        requests,
        decisions,
        commitments: vec![],
        evidence,
        notes: report.synthesis.summary.clone(),
    }
}
fn require_known(ids: &[String], evidence: &[CaseworkCitation]) -> Result<(), AppError> {
    if ids.iter().any(|id| !evidence.iter().any(|e| &e.id == id)) {
        return Err(invalid(
            "An item refers to a citation that is absent from this case.",
        ));
    }
    Ok(())
}
fn require_supported(ids: &[String], evidence: &[CaseworkCitation]) -> Result<(), AppError> {
    require_known(ids, evidence)?;
    if !ids.iter().any(|id| {
        evidence
            .iter()
            .any(|e| &e.id == id && e.verification == "SAVED_REPORT")
    }) {
        return Err(invalid("A recorded conclusion needs cited evidence from a locally saved report whose public sources still match. Historical evidence needs source review; portable evidence remains unverified."));
    }
    Ok(())
}
/// Batched counterpart of `read_report` for the distinct report ids a citation slice touches;
/// reports repeated across citations/leads (the common case) are decoded once instead of once
/// per citation.
fn read_reports(
    conn: &Connection,
    ids: &BTreeSet<String>,
) -> Result<BTreeMap<String, ResearchReport>, AppError> {
    if ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let sql = format!(
        "SELECT id, report_zstd FROM research_reports WHERE id IN ({})",
        vec!["?"; ids.len()].join(",")
    );
    let rows = conn
        .prepare(&sql)
        .map_err(storage)?
        .query_map(params_from_iter(ids.iter()), |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))
        })
        .map_err(storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage)?;
    rows.into_iter()
        .map(|(id, blob)| Ok((id, decode(&blob)?)))
        .collect()
}
/// Batched public/active status for a set of (portal, document id) pairs, replacing one
/// `query_row` per citation with a single `IN`-list lookup. A pair absent from the result never
/// matched a row (same as the per-citation query's `None`).
fn batch_public_status(
    conn: &Connection,
    pairs: &BTreeSet<(String, u64)>,
) -> Result<BTreeMap<(String, u64), bool>, AppError> {
    if pairs.is_empty() {
        return Ok(BTreeMap::new());
    }
    let sql = format!(
        "SELECT d.portal_subdomain, d.id, lower(trim(d.visibility))='public' AND p.lifecycle_state<>'REMOVED'
         FROM documents d JOIN portals p ON p.subdomain=d.portal_subdomain
         WHERE (d.portal_subdomain, d.id) IN ({})",
        vec!["(?,?)"; pairs.len()].join(",")
    );
    let values: Vec<Box<dyn ToSql>> = pairs
        .iter()
        .flat_map(|(portal, id)| -> [Box<dyn ToSql>; 2] {
            [Box::new(portal.clone()), Box::new(*id)]
        })
        .collect();
    conn.prepare(&sql)
        .map_err(storage)?
        .query_map(params_from_iter(values), |r| {
            Ok((
                (r.get::<_, String>(0)?, r.get::<_, u64>(1)?),
                r.get::<_, bool>(2)?,
            ))
        })
        .map_err(storage)?
        .collect::<Result<BTreeMap<_, _>, _>>()
        .map_err(storage)
}
/// Batched `research_report_sources` availability/staleness check, keyed by report id. A report
/// id absent from the result had no source rows at all, matching the per-citation aggregate
/// query's `(false, false)` for that case (the aggregate has no `GROUP BY`, so it always returns a
/// row even over zero matching source rows).
fn batch_report_sources(
    conn: &Connection,
    report_ids: &BTreeSet<String>,
) -> Result<BTreeMap<String, (bool, bool)>, AppError> {
    if report_ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let sql = format!(
        "SELECT s.report_id,
                COALESCE(bool_or(d.id IS NULL OR p.subdomain IS NULL OR lower(trim(d.visibility))<>'public' OR p.lifecycle_state='REMOVED'),false),
                COALESCE(bool_or(COALESCE(d.raw_file_sha256,'')<>s.source_sha256),false)
         FROM research_report_sources s
         LEFT JOIN documents d ON d.id=s.document_id AND d.portal_subdomain=s.portal_key
         LEFT JOIN portals p ON p.subdomain=s.portal_key
         WHERE s.report_id IN ({})
         GROUP BY s.report_id",
        vec!["?"; report_ids.len()].join(",")
    );
    conn.prepare(&sql)
        .map_err(storage)?
        .query_map(params_from_iter(report_ids.iter()), |r| {
            Ok((
                r.get::<_, String>(0)?,
                (r.get::<_, bool>(1)?, r.get::<_, bool>(2)?),
            ))
        })
        .map_err(storage)?
        .collect::<Result<BTreeMap<_, _>, _>>()
        .map_err(storage)
}
/// Recompute source status on every read and mutation; persisted verification is not trusted.
///
/// The per-citation document/portal and report-source checks below used to run as individual
/// `query_row` calls (up to ~4 per citation), which `casework_state` then multiplied by up to 40
/// leads plus 160 available-evidence items. The checks themselves are unchanged; only their data
/// is now prefetched in a few batched queries keyed by the same (portal, document id) pairs and
/// report ids the loop would otherwise look up one at a time.
fn refresh_evidence(
    conn: &Connection,
    evidence: &mut [CaseworkCitation],
    portable: bool,
) -> Result<Vec<String>, AppError> {
    let mut report_ids = BTreeSet::new();
    let mut public_pairs = BTreeSet::new();
    for citation in evidence.iter() {
        report_ids.insert(citation.report_id.clone());
        if let (Some(portal), Some(id)) = (&citation.portal_key, citation.document_id) {
            public_pairs.insert((portal.clone(), id));
        }
    }
    let reports_by_id = read_reports(conn, &report_ids)?;
    // The post-canonicalization check (below) looks up the *report's* evidence document/portal,
    // which may differ from the citation's own (unverified) fields checked above; both sets of
    // pairs are resolved by the one batched query.
    for citation in evidence.iter() {
        if let Some(report) = reports_by_id.get(&citation.report_id) {
            if let Some(source) = report
                .evidence
                .iter()
                .find(|e| e.id == citation.evidence_id)
            {
                if let (Some(portal), Some(id)) = (&source.portal_key, source.document_id) {
                    public_pairs.insert((portal.clone(), id));
                }
            }
        }
    }
    let public_status = batch_public_status(conn, &public_pairs)?;
    let source_status = batch_report_sources(conn, &report_ids)?;

    let mut warnings = Vec::new();
    let mut ids = BTreeSet::new();
    for citation in evidence {
        if !ids.insert(citation.id.clone())
            || citation.id.is_empty()
            || citation.quote.is_empty()
            || citation.quote.len() > 16000
        {
            return Err(invalid(
                "Citations need unique identities and bounded exact quotations.",
            ));
        }
        if let (Some(portal), Some(id)) = (&citation.portal_key, citation.document_id) {
            let public = public_status.get(&(portal.clone(), id)).copied();
            if public == Some(false) {
                return Err(invalid(
                    "A case citation refers to a private or removed local record.",
                ));
            }
        }
        if let Some(report) = reports_by_id.get(&citation.report_id) {
            let source = report
                .evidence
                .iter()
                .find(|e| e.id == citation.evidence_id)
                .ok_or_else(|| invalid("Citation evidence is absent from its saved report."))?;
            if !quote_exists(source, &citation.quote) {
                return Err(invalid(
                    "A case citation must quote retained report evidence exactly.",
                ));
            }
            let mut canonical = canonical_citation(report, source, &citation.quote);
            canonical.id = citation.id.clone();
            *citation = canonical;
        } else if portable || citation.verification == "UNVERIFIED_PORTABLE" {
            citation.verification = "UNVERIFIED_PORTABLE".into();
            warnings.push(format!(
                "{}: no local report verifies this portable source.",
                citation.label
            ));
        } else {
            return Err(invalid(
                "A source report was removed. Reload the case before saving.",
            ));
        }
        if citation.verification == "SAVED_REPORT" {
            let (unavailable, historical) = source_status
                .get(&citation.report_id)
                .copied()
                .unwrap_or((false, false));
            if unavailable {
                return Err(invalid(
                    "A case report source is unavailable, private or removed.",
                ));
            }
            if historical {
                citation.verification = "HISTORICAL_REPORT".into();
                warnings.push(format!("{}: the public source has a newer version. This retained quotation belongs to the saved historical report; review it before a new conclusion.", citation.label));
            }
            if let (Some(portal), Some(id)) = (&citation.portal_key, citation.document_id) {
                let public = public_status
                    .get(&(portal.clone(), id))
                    .copied()
                    .unwrap_or(false);
                if !public {
                    return Err(invalid(
                        "A saved report citation no longer points to an available public record.",
                    ));
                }
            }
        }
        for url in citation
            .source_url
            .iter()
            .chain(citation.source_references.iter().map(|s| &s.url))
        {
            let url =
                reqwest::Url::parse(url).map_err(|_| invalid("Case source URL is invalid."))?;
            if !["http", "https"].contains(&url.scheme())
                || !url.username().is_empty()
                || url.password().is_some()
            {
                return Err(invalid(
                    "Case sources require public HTTP links without embedded credentials.",
                ));
            }
        }
    }
    Ok(warnings)
}
fn historical_only(ids: &[String], evidence: &[CaseworkCitation]) -> bool {
    require_supported(ids, evidence).is_err()
        && evidence
            .iter()
            .any(|cite| ids.contains(&cite.id) && cite.verification == "HISTORICAL_REPORT")
}
/// Preserve the written assessment and evidence, but make source review explicit after replacement.
/// This is a derived status until the researcher saves the next immutable case revision.
fn mark_historical_assessments(case: &mut CaseworkCase) {
    for question in &mut case.questions {
        let ids = if question.disposition == "REFUTED" {
            &question.counter_citation_ids
        } else {
            &question.citation_ids
        };
        if ["SUPPORTED", "REFUTED"].contains(&question.disposition.as_str())
            && historical_only(ids, &case.evidence)
        {
            question.disposition = "NEEDS_REVIEW".into();
        }
    }
    for decision in &mut case.decisions {
        if decision.state == "DECIDED"
            && historical_only(&decision.outcome_citation_ids, &case.evidence)
        {
            decision.state = "NEEDS_REVIEW".into();
        }
    }
    for commitment in &mut case.commitments {
        if ["COMPLETE", "NOT_MET"].contains(&commitment.state.as_str())
            && historical_only(&commitment.completion_citation_ids, &case.evidence)
        {
            commitment.state = "NEEDS_REVIEW".into();
        }
    }
}
fn refresh_case(conn: &Connection, case: &mut CaseworkCase) -> Result<(), AppError> {
    refresh_evidence(conn, &mut case.evidence, false)?;
    mark_historical_assessments(case);
    Ok(())
}
fn validate(
    conn: &Connection,
    case: &mut CaseworkCase,
    portable: bool,
) -> Result<Vec<String>, AppError> {
    if case.id.is_empty()
        || case.id.len() > 120
        || case.report_id.len() > 150
        || case.title.trim().is_empty()
        || case.title.len() > 500
        || case.owner.len() > 120
        || !["homelessness", "police", "abatement", "bathrooms", "waste"]
            .contains(&case.topic.as_str())
        || !["OPEN", "ACTIVE", "ON_HOLD", "RESOLVED", "DISMISSED"]
            .contains(&case.disposition.as_str())
    {
        return Err(invalid(
            "Choose a valid case title, topic, disposition and owner.",
        ));
    }
    if case.questions.len() > 60
        || case.requests.len() > 40
        || case.decisions.len() > 40
        || case.commitments.len() > 60
        || case.evidence.len() > 160
    {
        return Err(invalid("Case item limits exceeded."));
    }
    let mut warnings = refresh_evidence(conn, &mut case.evidence, portable)?;
    mark_historical_assessments(case);
    let check_id = |id: &str| -> Result<(), AppError> {
        if id.is_empty() || id.len() > 150 {
            Err(invalid("Case item identity is invalid."))
        } else {
            Ok(())
        }
    };
    for q in &mut case.questions {
        check_id(&q.id)?;
        if q.question.trim().is_empty()
            || q.next_test.trim().is_empty()
            || ![
                "OPEN",
                "TESTING",
                "SUPPORTED",
                "REFUTED",
                "DISMISSED",
                "NEEDS_REVIEW",
            ]
            .contains(&q.disposition.as_str())
        {
            return Err(invalid("Invalid question disposition."));
        }
        require_known(&q.citation_ids, &case.evidence)?;
        require_known(&q.counter_citation_ids, &case.evidence)?;
        if ["SUPPORTED", "REFUTED"].contains(&q.disposition.as_str()) {
            if let Err(e) = require_supported(
                if q.disposition == "REFUTED" {
                    &q.counter_citation_ids
                } else {
                    &q.citation_ids
                },
                &case.evidence,
            ) {
                if portable {
                    q.disposition = "OPEN".into();
                    warnings.push("An imported hypothesis disposition lacks locally verified evidence; it remains open.".into());
                } else {
                    return Err(e);
                }
            }
        }
    }
    for r in &mut case.requests {
        check_id(&r.id)?;
        if ![
            "DRAFT",
            "SUBMITTED",
            "ACKNOWLEDGED",
            "PARTIAL",
            "PRODUCED",
            "CLOSED",
        ]
        .contains(&r.state.as_str())
        {
            return Err(invalid("Invalid records request state."));
        }
        require_known(&r.citation_ids, &case.evidence)?;
        for date in [&r.submitted_at, &r.response_due].into_iter().flatten() {
            if !valid_date(date) {
                return Err(invalid("Request dates must use YYYY-MM-DD."));
            }
        }
        if r.state != "DRAFT" && r.submitted_at.is_none() {
            return Err(invalid(
                "Record the actual submission date before tracking a submitted request.",
            ));
        }
        if r.productions.len() > 80 {
            return Err(invalid("Too many production records in one request."));
        }
        for p in &r.productions {
            check_id(&p.id)?;
            if !valid_date(&p.received_at) {
                return Err(invalid("A production requires a recorded received date."));
            }
            require_known(&p.citation_ids, &case.evidence)?;
        }
        if ["PARTIAL", "PRODUCED"].contains(&r.state.as_str()) && r.productions.is_empty() {
            return Err(invalid(
                "A production state requires a recorded production.",
            ));
        }
        if r.state == "PRODUCED" && !request_omissions(r).is_empty() {
            return Err(invalid("This request still has unresolved expected records, fields or periods. Keep it partial or resolve the omissions."));
        }
        r.follow_up_draft = follow_up(r);
        compile_request(r, &case.evidence, &now())?;
    }
    for d in &mut case.decisions {
        check_id(&d.id)?;
        require_known(&d.citation_ids, &case.evidence)?;
        require_known(&d.outcome_citation_ids, &case.evidence)?;
        if ![
            "PROPOSED",
            "MONITORING",
            "DECIDED",
            "UNKNOWN",
            "NEEDS_REVIEW",
        ]
        .contains(&d.state.as_str())
            || !valid_date(&d.date)
        {
            return Err(invalid("Decision windows require a valid date and state."));
        }
        let cites = case
            .evidence
            .iter()
            .filter(|e| d.citation_ids.contains(&e.id))
            .collect::<Vec<_>>();
        if !quoted_date(&d.date, &cites) {
            return Err(invalid(
                "A decision date must appear in its cited agenda evidence.",
            ));
        }
        if d.state == "DECIDED" {
            if d.outcome.trim().is_empty() {
                return Err(invalid(
                    "Record the decision outcome; an agenda alone is not a vote.",
                ));
            }
            if let Err(e) = require_supported(&d.outcome_citation_ids, &case.evidence) {
                if portable {
                    d.state = "UNKNOWN".into();
                    warnings.push("An imported decision outcome remains unverified.".into());
                } else {
                    return Err(e);
                }
            }
        }
    }
    for c in &mut case.commitments {
        check_id(&c.id)?;
        require_known(&c.citation_ids, &case.evidence)?;
        require_known(&c.completion_citation_ids, &case.evidence)?;
        if c.promise.trim().is_empty()
            || c.completion_test.trim().is_empty()
            || c.citation_ids.is_empty()
            || ![
                "UNKNOWN",
                "IN_PROGRESS",
                "COMPLETE",
                "NOT_MET",
                "NEEDS_REVIEW",
            ]
            .contains(&c.state.as_str())
            || c.due_date.as_deref().is_some_and(|d| !valid_date(d))
        {
            return Err(invalid(
                "A commitment requires a cited promise, a completion test and valid state/date.",
            ));
        }
        if ["COMPLETE", "NOT_MET"].contains(&c.state.as_str()) {
            let supported = require_supported(&c.completion_citation_ids, &case.evidence);
            if supported.is_err() || c.assessment.trim().is_empty() {
                if portable {
                    c.state = "UNKNOWN".into();
                    warnings.push("An imported commitment assessment lacks verified completion evidence; state is unknown.".into());
                } else {
                    return Err(invalid("Complete/not-met assessments require cited completion evidence and a written explanation; passing a deadline alone is insufficient."));
                }
            }
        }
    }
    encode(case)?;
    Ok(warnings)
}
pub fn request_omissions(request: &CaseworkRequest) -> Vec<String> {
    let mut missing = request.unresolved_omissions.clone();
    for (label, expected, observed) in [
        (
            "record",
            &request.expected_record_types,
            request
                .productions
                .iter()
                .flat_map(|p| p.record_types.iter())
                .collect::<Vec<_>>(),
        ),
        (
            "field",
            &request.expected_fields,
            request
                .productions
                .iter()
                .flat_map(|p| p.fields.iter())
                .collect::<Vec<_>>(),
        ),
    ] {
        for value in expected {
            if !observed
                .iter()
                .any(|actual| normalized(actual).eq_ignore_ascii_case(&normalized(value)))
            {
                missing.push(format!(
                    "Expected {label} not yet recorded as produced: {value}"
                ));
            }
        }
    }
    if !request.expected_period.trim().is_empty()
        && !request.productions.iter().any(|p| {
            normalized(&p.period).eq_ignore_ascii_case(&normalized(&request.expected_period))
        })
    {
        missing.push(format!(
            "Requested period coverage remains unconfirmed: {}",
            request.expected_period
        ));
    }
    missing.sort();
    missing.dedup();
    missing
}
/// Recomputes a request's CPRA draft and its statutory clock, so neither can be edited into
/// disagreeing with the request's own recorded scope, citations and submission date.
/// `observed_at` is the instant the clock is counted to: the save instant on a save, the record's
/// own generation instant on a signed filing record, so a record's bytes do not depend on when the
/// test or the reader happens to run.
fn compile_request(
    r: &mut CaseworkRequest,
    evidence: &[CaseworkCitation],
    observed_at: &str,
) -> Result<(), AppError> {
    let identified_by = r
        .citation_ids
        .iter()
        .filter_map(|id| evidence.iter().find(|e| &e.id == id))
        .map(|e| (e.label.clone(), e.source_sha256.clone()))
        .collect::<Vec<_>>();
    r.compiled_draft = compile_cpra_request(&RequestScope {
        title: &r.title,
        custodian: &r.custodian,
        record_types: &r.expected_record_types,
        fields: &r.expected_fields,
        period: &r.expected_period,
        resolves: &r.resolves,
        identified_by: &identified_by,
    });
    r.filing_plan = match &r.submitted_at {
        // The clock position labels the engine's evidence summary only; a casework request's id is
        // a string, and the engine counts in calendar days from the recorded filing date alone.
        Some(filed_on) => Some(plan_filing(
            0,
            filed_on,
            &r.productions
                .iter()
                .map(|p| p.received_at.clone())
                .collect::<Vec<_>>(),
            &request_omissions(r),
            observed_at,
        )?),
        None => None,
    };
    Ok(())
}
fn follow_up(r: &CaseworkRequest) -> String {
    let omissions = request_omissions(r);
    format!("To: {}\nSubject: Follow-up on {}{}\n\nPlease provide the remaining responsive records or clarify their disposition for the following unresolved items:\n{}\n\nThese records would help determine: {}\nPlease identify any withheld or unavailable portions and provide reasonably segregable responsive material.\n\nDraft for review; no message has been sent.",r.custodian,r.title,if r.external_reference.is_empty(){String::new()}else{format!(" ({})",r.external_reference)},if omissions.is_empty(){"- Confirm that the production covers the requested records, fields and period.".into()}else{omissions.iter().map(|m|format!("- {m}")).collect::<Vec<_>>().join("\n")},r.resolves)
}
fn operations_since(
    conn: &Connection,
    case_id: &str,
    after_revision: u64,
) -> Result<Vec<CaseOperation>, AppError> {
    let mut statement = conn
        .prepare("SELECT revision,sequence,author,recorded_at,kind,section,target,summary,content_sha256,previous_hash,hash FROM casework_operations WHERE case_id=?1 AND revision>?2 ORDER BY revision,sequence")
        .map_err(storage)?;
    let rows = statement
        .query_map(params![case_id, after_revision], |r| {
            Ok(CaseOperation {
                revision: r.get(0)?,
                sequence: r.get(1)?,
                author: r.get(2)?,
                at: r.get(3)?,
                kind: r.get(4)?,
                section: r.get(5)?,
                target: r.get(6)?,
                summary: r.get(7)?,
                content_sha256: r.get(8)?,
                previous_hash: r.get(9)?,
                hash: r.get(10)?,
            })
        })
        .map_err(storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage)?;
    Ok(rows)
}
/// Appends this revision's operations to the case's log, chained after the last recorded one.
fn record_operations(
    conn: &Connection,
    prior: Option<&CaseworkCase>,
    case: &CaseworkCase,
    author: &str,
) -> Result<(), AppError> {
    let previous: Option<String> = conn
        .query_row(
            "SELECT hash FROM casework_operations WHERE case_id=?1 ORDER BY revision DESC,sequence DESC LIMIT 1",
            params![case.id],
            |r| r.get(0),
        )
        .optional()
        .map_err(storage)?;
    let chained = chain(
        operations_between(prior, case),
        case.revision,
        author,
        &case.updated_at,
        previous,
    );
    for operation in chained {
        conn.execute(
            "INSERT INTO casework_operations VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![
                case.id,
                operation.revision,
                operation.sequence,
                operation.author,
                operation.at,
                operation.kind,
                operation.section,
                operation.target,
                operation.summary,
                operation.content_sha256,
                operation.previous_hash,
                operation.hash
            ],
        )
        .map_err(storage)?;
    }
    Ok(())
}
fn write_case(
    conn: &Connection,
    case: &mut CaseworkCase,
    author: &str,
    summary: &str,
) -> Result<(), AppError> {
    let prior = read_case(conn, &case.id)?;
    case.revision += 1;
    case.updated_at = now();
    let blob = encode(case)?;
    record_operations(conn, prior.as_ref(), case, author)?;
    conn.execute("INSERT INTO casework_cases VALUES (?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(id) DO UPDATE SET disposition=excluded.disposition,revision=excluded.revision,payload_zstd=excluded.payload_zstd,updated_at=excluded.updated_at",params![case.id,case.report_id,case.topic,case.disposition,case.revision,blob,case.updated_at]).map_err(storage)?;
    // `payload_zstd` used to carry a full compressed snapshot of the case on every revision,
    // forever — nothing in this application ever reads it back (grepped and confirmed); only
    // author/summary/recorded_at power the UI's contribution trail. A case edited 200 times wrote
    // 200 full copies for no reader. Going forward this writes an empty placeholder instead of
    // dropping the column: existing installs keep every historical snapshot they already have
    // (DROP COLUMN would destroy it irreversibly on upgrade, which was never authorized), and the
    // schema needs no upgrade migration that could fail and block startup.
    conn.execute(
        "INSERT INTO casework_events VALUES (?1,?2,?3,?4,?5,?6)",
        params![
            case.id,
            case.revision,
            author,
            case.updated_at,
            summary,
            Vec::<u8>::new()
        ],
    )
    .map_err(storage)?;
    conn.execute(
        "DELETE FROM casework_sources WHERE case_id=?1",
        params![case.id],
    )
    .map_err(storage)?;
    let report_ids: BTreeSet<_> = case
        .evidence
        .iter()
        .filter(|e| ["SAVED_REPORT", "HISTORICAL_REPORT"].contains(&e.verification.as_str()))
        .map(|e| &e.report_id)
        .collect();
    for report in report_ids {
        conn.execute("INSERT INTO casework_sources SELECT ?1,portal_key,document_id,source_sha256 FROM research_report_sources WHERE report_id=?2 ON CONFLICT DO NOTHING",params![case.id,report]).map_err(storage)?;
    }
    // An unverified portable quotation can still identify a local source exactly. Bind that
    // identity for purge without upgrading the quotation's verification status.
    for citation in &case.evidence {
        if let (Some(portal), Some(document)) = (&citation.portal_key, citation.document_id) {
            conn.execute("INSERT INTO casework_sources SELECT ?1,d.portal_subdomain,d.id,COALESCE(d.raw_file_sha256,'') FROM documents d JOIN portals p ON p.subdomain=d.portal_subdomain WHERE d.id=?2 AND d.portal_subdomain=?3 AND lower(trim(d.visibility))='public' AND p.lifecycle_state<>'REMOVED' AND COALESCE(d.raw_file_sha256,'')=?4 ON CONFLICT DO NOTHING",params![case.id,document,portal,citation.source_sha256]).map_err(storage)?;
        }
    }
    Ok(())
}
fn author_valid(author: &str) -> Result<(), AppError> {
    if author.trim().is_empty() || author.len() > 120 {
        Err(invalid(
            "Enter a contributor name or team label (up to 120 characters).",
        ))
    } else {
        Ok(())
    }
}
fn content_hash(case: &CaseworkCase) -> Result<String, AppError> {
    let mut copy = case.clone();
    copy.revision = 0;
    copy.created_at.clear();
    copy.updated_at.clear();
    Ok(sha256_hex(&serde_json::to_vec(&copy).map_err(storage)?))
}
/// Removes each requested citation from the case's evidence array and strips its id from every
/// citation-id list elsewhere in the case (questions, requests, productions, decisions,
/// commitments), so a chain-of-custody export never leaves a dangling reference to evidence it
/// omits. Each withheld citation is named by label and reason in the returned list instead — never
/// silently dropped. Every requested id must resolve to a real citation with a non-empty, distinct
/// reason, or the whole export is rejected rather than shipping a best-effort redaction.
fn redact_case_for_export(
    mut case: CaseworkCase,
    withheld: &[WithheldCitationRequest],
) -> Result<(CaseworkCase, Vec<WithheldCitation>), AppError> {
    if withheld.is_empty() {
        return Ok((case, Vec::new()));
    }
    let mut withheld_citations = Vec::with_capacity(withheld.len());
    let mut ids = BTreeSet::new();
    for request in withheld {
        let reason = request.reason.trim();
        if reason.is_empty() {
            return Err(invalid("Every withheld citation needs a non-empty reason."));
        }
        if !ids.insert(request.citation_id.clone()) {
            return Err(invalid("The same citation was withheld more than once."));
        }
        let citation = case
            .evidence
            .iter()
            .find(|c| c.id == request.citation_id)
            .ok_or_else(|| {
                invalid("A withheld citation id does not match this case's evidence.")
            })?;
        withheld_citations.push(WithheldCitation {
            label: citation.label.clone(),
            reason: reason.to_string(),
        });
    }
    case.evidence.retain(|c| !ids.contains(&c.id));
    let strip = |list: &mut Vec<String>| list.retain(|id| !ids.contains(id));
    for question in &mut case.questions {
        strip(&mut question.citation_ids);
        strip(&mut question.counter_citation_ids);
    }
    for request in &mut case.requests {
        strip(&mut request.citation_ids);
        for production in &mut request.productions {
            strip(&mut production.citation_ids);
        }
    }
    for decision in &mut case.decisions {
        strip(&mut decision.citation_ids);
        strip(&mut decision.outcome_citation_ids);
    }
    for commitment in &mut case.commitments {
        strip(&mut commitment.citation_ids);
        strip(&mut commitment.completion_citation_ids);
    }
    Ok((case, withheld_citations))
}
fn evidence_rollup(case: &CaseworkCase) -> (usize, Vec<String>) {
    let unverified_citation_labels: Vec<String> = case
        .evidence
        .iter()
        .filter(|citation| citation.verification != "SAVED_REPORT")
        .map(|citation| format!("{} ({})", citation.label, citation.verification))
        .collect();
    let verified_citation_count = case.evidence.len() - unverified_citation_labels.len();
    (verified_citation_count, unverified_citation_labels)
}
fn packet_preview(
    conn: &Connection,
    payload: &str,
) -> Result<(CaseworkPacket, CaseworkImportPreview), AppError> {
    if payload.len() > MAX_PACKET {
        return Err(invalid("Portable packet exceeds 2 MB."));
    }
    let mut packet: CaseworkPacket =
        serde_json::from_str(payload).map_err(|e| invalid(format!("Invalid case packet: {e}")))?;
    if packet.schema_version != 1 || packet.history.len() > 1000 || packet.operations.len() > 20_000
    {
        return Err(invalid("Unsupported case packet version or history size."));
    }
    author_valid(&packet.contributor)?;
    let mut warnings = validate(conn, &mut packet.case, true)?;
    warnings.push(
        "Contributor names and imported history are supplied labels, not authenticated identities."
            .into(),
    );
    let local = read_case(conn, &packet.case.id)?;
    let mode = if let Some(local) = &local {
        if local.report_id != packet.case.report_id || local.topic != packet.case.topic {
            warnings.push("The incoming packet changes the case's founding report or topic; merge into a separate case instead.".into());
            "CONFLICT"
        } else if content_hash(local)? == content_hash(&packet.case)? {
            "IDENTICAL"
        } else if local.revision == packet.base_revision
            && sha256_hex(&serde_json::to_vec(local).map_err(storage)?) == packet.base_sha256
        {
            "FAST_FORWARD"
        } else {
            "CONFLICT"
        }
    } else {
        "NEW"
    };
    let (commuting, contested) = match &local {
        Some(local)
            if mode == "CONFLICT"
                && local.report_id == packet.case.report_id
                && local.topic == packet.case.topic =>
        {
            if !packet.operations.is_empty() && !chain_is_intact(&packet.operations) {
                warnings.push("The packet's operation log does not chain: it was edited after it was written, so nothing in it is treated as commuting.".into());
            }
            // The last exchange of this case, in local numbering: the packet's base when this
            // desk is the origin of that base, else the revision our last import wrote.
            let last_import: Option<u64> = conn
                .query_row(
                    "SELECT MAX(revision) FROM casework_imports WHERE case_id=?1",
                    params![local.id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(storage)?
                .flatten();
            let is_origin: bool = conn
                .query_row(
                    "SELECT NOT EXISTS(SELECT 1 FROM casework_origins WHERE case_id=?1)",
                    params![local.id],
                    |r| r.get(0),
                )
                .map_err(storage)?;
            let sync = match (is_origin, last_import) {
                (true, Some(imported)) => packet.base_revision.max(imported),
                (true, None) => packet.base_revision,
                (false, imported) => imported.unwrap_or(0),
            };
            let local_since = operations_since(conn, &local.id, sync)?;
            partition_operations(&packet.operations, &packet.case, local, &local_since)
        }
        _ => (Vec::new(), Vec::new()),
    };
    let preview = CaseworkImportPreview {
        mode: mode.into(),
        local_revision: local.as_ref().map(|c| c.revision),
        packet_sha256: sha256_hex(payload.as_bytes()),
        warnings,
        case: packet.case.clone(),
        contributor: packet.contributor.clone(),
        signer_key_id: None,
        commuting,
        contested,
    };
    Ok((packet, preview))
}
/// DuckDB's `ALTER TABLE ADD COLUMN` refuses any constraint (CLAUDE.md), so this carries a type
/// only, and is idempotent so a fresh install (where `CREATE TABLE IF NOT EXISTS` already created
/// the column) and an upgraded one converge on the same shape.
fn ensure_casework_imports_signer_column(conn: &Connection) -> Result<(), AppError> {
    let has_column: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM information_schema.columns WHERE table_schema=current_schema() AND table_name='casework_imports' AND column_name='signer_key_id')",
            [],
            |r| r.get(0),
        )
        .map_err(storage)?;
    if has_column {
        return Ok(());
    }
    conn.execute_batch("ALTER TABLE casework_imports ADD COLUMN signer_key_id TEXT")
        .map_err(storage)
}
impl DuckDbRepository {
    pub async fn initialize_casework(&self) -> Result<(), AppError> {
        self.query_connection("Initialize durable casework", |conn| {
            conn.execute_batch(SCHEMA).map_err(storage)?;
            ensure_casework_imports_signer_column(conn)
        })
        .await
    }
    pub async fn casework_state(&self) -> Result<CaseworkState, AppError> {
        self.query_connection("Read durable casework", |conn| {
            let cases = cases(conn)?;
            let reports = reports(conn)?;
            let mut leads = rank_leads(&reports, &cases);
            for lead in &mut leads {
                refresh_evidence(conn, &mut lead.citations, false)?;
            }
            let mut available_evidence: Vec<CaseworkCitation> = reports
                .iter()
                .flat_map(|report| {
                    report.evidence.iter().map(move |e| {
                        canonical_citation(
                            report,
                            e,
                            &e.text.chars().take(1600).collect::<String>(),
                        )
                    })
                })
                .take(160)
                .collect();
            refresh_evidence(conn, &mut available_evidence, false)?;
            Ok(CaseworkState {
                available_evidence,
                cases,
                events: events(conn)?,
                leads,
            })
        })
        .await
    }
    pub async fn create_casework_case(&self, report_id: String) -> Result<CaseworkCase, AppError> {
        self.query_connection("Open investigation case", move |conn| {
            let existing: Option<Vec<u8>> = conn.query_row("SELECT payload_zstd FROM casework_cases WHERE report_id=?1 ORDER BY updated_at DESC LIMIT 1", params![report_id], |row| row.get(0)).optional().map_err(storage)?;
            if let Some(existing) = existing { let mut case = decode(&existing)?; refresh_case(conn, &mut case)?; return Ok(case); }
            let report = read_report(conn, &report_id)?
                .ok_or_else(|| invalid("The research report is no longer available."))?;
            let states = conn
                .prepare("SELECT action_key,state FROM research_actions")
                .map_err(storage)?
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .map_err(storage)?
                .collect::<Result<BTreeMap<_, _>, _>>()
                .map_err(storage)?;
            let mut case = create_from_report(&report, &states);
            validate(conn, &mut case, false)?;
            let tx = conn.transaction().map_err(storage)?;
            write_case(
                &tx,
                &mut case,
                "Research workspace",
                "Created from a saved research report; hypotheses require review.",
            )?;
            tx.commit().map_err(storage)?;
            Ok(case)
        })
        .await
    }
    pub async fn save_casework_case(
        &self,
        mut case: CaseworkCase,
        expected_revision: u64,
        author: String,
    ) -> Result<CaseworkCase, AppError> {
        author_valid(&author)?;
        self.query_connection("Save case revision",move|conn|{let prior=read_case(conn,&case.id)?.ok_or_else(||invalid("This case no longer exists."))?;if prior.revision!=expected_revision{return Err(invalid("This case changed since you opened it. Reload before saving; your changes were not applied."));}
if case.report_id!=prior.report_id||case.topic!=prior.topic{return Err(invalid("A case cannot change its source report or topic."));}case.revision=prior.revision;case.created_at=prior.created_at;validate(conn,&mut case,false)?;let tx=conn.transaction().map_err(storage)?;write_case(&tx,&mut case,&author,"Researcher updated casework.")?;tx.commit().map_err(storage)?;Ok(case)}).await
    }
    pub async fn export_casework_packet(
        &self,
        case_id: String,
        contributor: String,
    ) -> Result<String, AppError> {
        author_valid(&contributor)?;
        self.query_connection("Export portable case", move |conn| {
            let case = read_case(conn, &case_id)?.ok_or_else(|| invalid("Case not found."))?;
            let origin: Option<(u64, String)> = conn
                .query_row(
                    "SELECT base_revision,base_sha256 FROM casework_origins WHERE case_id=?1",
                    params![case_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(storage)?;
            let (base_revision, base_sha256) = origin.unwrap_or((
                case.revision,
                sha256_hex(&serde_json::to_vec(&case).map_err(storage)?),
            ));
            let history = portable_history(conn, &case_id)?;
            let operations = operations_since(conn, &case_id, 0)?;
            let packet = CaseworkPacket {
                schema_version: 1,
                operations,
                base_revision,
                base_sha256,
                exported_at: now(),
                contributor,
                case,
                history,
            };
            let payload = serde_json::to_string_pretty(&packet).map_err(storage)?;
            if payload.len() > MAX_PACKET {
                return Err(invalid("Export exceeds the portable packet size limit."));
            }
            Ok(payload)
        })
        .await
    }
    /// Signs a filing record for one records request, reading the case fresh so the record carries
    /// the request text, scope and clock the repository itself computed rather than whatever the
    /// caller's copy held.
    ///
    /// Refuses a request with no recorded submission date: a filing record for an unfiled request
    /// would state a filing that never happened.
    pub(crate) async fn sign_request_filing_record(
        &self,
        case_id: String,
        request_id: String,
        generated_at: String,
        signing_key: SigningKey,
    ) -> Result<SignedPackage<RequestFilingRecord>, AppError> {
        self.query_connection("Sign records-request filing record", move |conn| {
            let mut case = read_case(conn, &case_id)?.ok_or_else(|| invalid("Case not found."))?;
            refresh_case(conn, &mut case)?;
            let request = case
                .requests
                .iter()
                .find(|r| r.id == request_id)
                .ok_or_else(|| invalid("This case has no such records request."))?
                .clone();
            let Some(filed_at) = request.submitted_at.clone() else {
                return Err(invalid(
                    "Record the confirmed submission date before signing a filing record; this \
                     application does not send requests and cannot observe one.",
                ));
            };
            let mut request = request;
            compile_request(&mut request, &case.evidence, &generated_at)?;
            let filing_plan = request
                .filing_plan
                .clone()
                .ok_or_else(|| invalid("This request has no modeled determination clock."))?;
            let manifest = RequestFilingRecord {
                schema_version: REQUEST_FILING_RECORD_SCHEMA.to_string(),
                generated_at,
                case_id: case.id.clone(),
                case_revision: case.revision,
                request_id: request.id.clone(),
                title: request.title.clone(),
                custodian: request.custodian.clone(),
                external_reference: request.external_reference.clone(),
                filed_at,
                request_text_sha256: sha256_hex(request.compiled_draft.as_bytes()),
                request_text: request.compiled_draft.clone(),
                expected_record_types: request.expected_record_types.clone(),
                expected_fields: request.expected_fields.clone(),
                expected_period: request.expected_period.clone(),
                identifying_sources: request
                    .citation_ids
                    .iter()
                    .filter_map(|id| case.evidence.iter().find(|e| &e.id == id))
                    .map(|e| FilingIdentifyingSource {
                        label: e.label.clone(),
                        source_sha256: e.source_sha256.clone(),
                        verification: e.verification.clone(),
                    })
                    .collect(),
                filing_plan,
                remaining_omissions: request_omissions(&request),
            };
            let package = sign_package(manifest, &signing_key).map_err(invalid)?;
            verify_package(&package).map_err(invalid)?;
            Ok(package)
        })
        .await
    }
    /// Redacts, signs, and atomically reserves the next position in this case's local export
    /// ledger for a chain-of-custody package, all inside one transaction. Reading the ledger head,
    /// building the manifest (which embeds that head's hash), signing it, and inserting the
    /// reservation must happen together: splitting the head-read from the insert would let two
    /// concurrent exports both see the same head and both attempt the next sequence. The shared
    /// connection mutex behind `query_connection` is what actually prevents that fork here, not a
    /// database-level retry loop — see `PRIMARY KEY(case_id,sequence)` on the ledger table, which
    /// exists as a second, independent guard in case that assumption ever stops holding.
    pub(crate) async fn export_case_evidence_package_signed(
        &self,
        case_id: String,
        generated_at: String,
        withheld: Vec<WithheldCitationRequest>,
        signing_key: SigningKey,
    ) -> Result<SignedPackage<CaseEvidenceManifest>, AppError> {
        self.query_connection("Reserve and sign chain-of-custody export", move |conn| {
            let case = read_case(conn, &case_id)?.ok_or_else(|| invalid("Case not found."))?;
            let (redacted_case, withheld_citations) = redact_case_for_export(case, &withheld)?;
            let (verified_citation_count, unverified_citation_labels) =
                evidence_rollup(&redacted_case);
            let tx = conn.transaction().map_err(storage)?;
            let head: Option<(u64, String)> = tx
                .query_row(
                    "SELECT sequence,manifest_sha256 FROM casework_export_ledger WHERE case_id=?1 ORDER BY sequence DESC LIMIT 1",
                    params![case_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(storage)?;
            let sequence = head.as_ref().map(|(s, _)| s + 1).unwrap_or(1);
            let previous_export_sha256 = head.map(|(_, hash)| hash);
            let manifest = CaseEvidenceManifest {
                schema_version: CASE_EVIDENCE_PACKAGE_SCHEMA.to_string(),
                generated_at,
                case: redacted_case,
                verified_citation_count,
                unverified_citation_labels,
                withheld_citations,
                previous_export_sha256,
                ledger_sequence: sequence,
            };
            let package = sign_package(manifest, &signing_key).map_err(invalid)?;
            verify_package(&package).map_err(invalid)?;
            tx.execute(
                "INSERT INTO casework_export_ledger VALUES (?1,?2,?3,?4,?5,?6)",
                params![
                    case_id,
                    sequence,
                    package.manifest_sha256,
                    package.manifest.previous_export_sha256,
                    package.signature.signer_key_id,
                    now()
                ],
            )
            .map_err(storage)?;
            tx.commit().map_err(storage)?;
            Ok(package)
        })
        .await
    }
    /// Appends the outcome of a reserved export — never updates the reservation row — so a crash
    /// between signing and this call leaves a truthful "reserved, not confirmed written" gap
    /// (`case_export_chain` reports it as such) rather than a row silently rewritten in place.
    pub async fn record_case_export_outcome(
        &self,
        case_id: String,
        sequence: u64,
        outcome: &'static str,
        detail: Option<String>,
    ) -> Result<(), AppError> {
        self.query_connection("Record chain-of-custody export outcome", move |conn| {
            conn.execute(
                "INSERT INTO casework_export_outcomes VALUES (?1,?2,?3,?4,?5)",
                params![case_id, sequence, outcome, detail, now()],
            )
            .map_err(storage)?;
            Ok(())
        })
        .await
    }
    /// The case's local export ledger, oldest first, joined with each entry's most recently
    /// recorded outcome (if any). Chain continuity — every entry's `previousManifestSha256`
    /// matching the prior entry's `manifestSha256`, and only the first entry's being absent — is
    /// checkable by any reader of this list without trusting this installation, the same way
    /// `verify_package` needs no trust from the process that produced the package it checks.
    /// Every revision's recorded signer for this case's imported packets (`None` for a packet
    /// imported before item 5 added the column, or one that was never signed). The command layer
    /// cross-references this against the local revocation list, so an already-imported case can be
    /// flagged if its signer is later marked compromised — the reason this column exists at all.
    pub async fn case_import_signers(
        &self,
        case_id: String,
    ) -> Result<Vec<(u64, Option<String>)>, AppError> {
        self.query_connection("Read casework import signers", move |conn| {
            conn.prepare(
                "SELECT revision,signer_key_id FROM casework_imports WHERE case_id=?1 ORDER BY revision",
            )
            .map_err(storage)?
            .query_map(params![case_id], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)
        })
        .await
    }
    pub async fn case_export_chain(
        &self,
        case_id: String,
    ) -> Result<Vec<CaseExportLedgerEntry>, AppError> {
        self.query_connection("Read chain-of-custody export ledger", move |conn| {
            let mut statement = conn
                .prepare(
                    "SELECT sequence,manifest_sha256,previous_manifest_sha256,signer_key_id,reserved_at FROM casework_export_ledger WHERE case_id=?1 ORDER BY sequence",
                )
                .map_err(storage)?;
            let reservations = statement
                .query_map(params![case_id], |r| {
                    Ok((
                        r.get::<_, u64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                    ))
                })
                .map_err(storage)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(storage)?;
            drop(statement);
            let mut entries = Vec::with_capacity(reservations.len());
            let mut prior_manifest_sha256: Option<String> = None;
            for (sequence, manifest_sha256, previous_manifest_sha256, signer_key_id, reserved_at) in
                reservations
            {
                let outcome: Option<(String, Option<String>, String)> = conn
                    .query_row(
                        "SELECT outcome,detail,recorded_at FROM casework_export_outcomes WHERE case_id=?1 AND sequence=?2 ORDER BY recorded_at DESC LIMIT 1",
                        params![case_id, sequence],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )
                    .optional()
                    .map_err(storage)?;
                let (outcome_state, outcome_detail, outcome_recorded_at) = match outcome {
                    Some((outcome, detail, recorded_at)) => (outcome, detail, Some(recorded_at)),
                    None => ("RESERVED".to_string(), None, None),
                };
                // Sequence 1 has no predecessor (valid by definition); every later entry's link is
                // valid only if it names exactly the immediately preceding entry's manifest hash —
                // computed here so a reader gets a stated verdict, not raw hex strings to compare.
                let previous_link_valid = match &prior_manifest_sha256 {
                    None => previous_manifest_sha256.is_none(),
                    Some(prior) => previous_manifest_sha256.as_deref() == Some(prior.as_str()),
                };
                prior_manifest_sha256 = Some(manifest_sha256.clone());
                entries.push(CaseExportLedgerEntry {
                    sequence,
                    manifest_sha256,
                    previous_manifest_sha256,
                    signer_key_id,
                    reserved_at,
                    outcome: outcome_state,
                    outcome_detail,
                    outcome_recorded_at,
                    previous_link_valid,
                });
            }
            Ok(entries)
        })
        .await
    }
    pub async fn preview_casework_import(
        &self,
        payload: String,
    ) -> Result<CaseworkImportPreview, AppError> {
        self.query_connection("Preview portable case import", move |conn| {
            packet_preview(conn, &payload).map(|(_, p)| p)
        })
        .await
    }
    pub async fn import_casework_packet(
        &self,
        payload: String,
        expected_revision: Option<u64>,
        packet_sha256: String,
        author: String,
        signer_key_id: Option<String>,
    ) -> Result<CaseworkCase, AppError> {
        author_valid(&author)?;
        self.query_connection("Apply reviewed case import",move|conn|{let(packet,preview)=packet_preview(conn,&payload)?;if preview.packet_sha256!=packet_sha256||preview.local_revision!=expected_revision{return Err(invalid("The packet or local case changed after preview. Preview it again."));}
if preview.mode=="CONFLICT"{return Err(invalid("This import conflicts with local work. It was not applied; compare the versions and merge selected changes into the local case."));}
if preview.mode=="IDENTICAL"{return read_case(conn,&packet.case.id)?.ok_or_else(||invalid("Case disappeared."));}let mut case=packet.case.clone();case.revision=preview.local_revision.unwrap_or(0);let tx=conn.transaction().map_err(storage)?;let provenance=match &signer_key_id{Some(key_id)=>format!("Imported packet {} from claimed contributor {} (Ed25519-signed by local key {key_id}); {} supplied history entries are unverified.",preview.packet_sha256,packet.contributor,packet.history.len()),None=>format!("Imported packet {} from claimed contributor {}; {} supplied history entries are unverified.",preview.packet_sha256,packet.contributor,packet.history.len())};write_case(&tx,&mut case,&author,&provenance)?;tx.execute("INSERT INTO casework_origins VALUES (?1,?2,?3) ON CONFLICT DO NOTHING",params![case.id,packet.base_revision,packet.base_sha256]).map_err(storage)?;tx.execute("INSERT INTO casework_imports VALUES (?1,?2,?3,?4,?5)",params![case.id,case.revision,preview.packet_sha256,encode(&packet)?,signer_key_id]).map_err(storage)?;tx.commit().map_err(storage)?;Ok(case)}).await
    }
    /// Applies the incoming operations that commute with local work since the shared base, as
    /// one new local revision, and leaves every contested difference for the item picker. The
    /// commuting set is recomputed here from the packet and the local log, never taken from the
    /// caller, and the packet and local revision must be the ones the preview showed.
    pub async fn apply_commuting_case_operations(
        &self,
        payload: String,
        expected_revision: Option<u64>,
        packet_sha256: String,
        author: String,
        signer_key_id: Option<String>,
    ) -> Result<CaseworkCase, AppError> {
        author_valid(&author)?;
        self.query_connection("Apply commuting case operations", move |conn| {
            let (packet, preview) = packet_preview(conn, &payload)?;
            if preview.packet_sha256 != packet_sha256 || preview.local_revision != expected_revision {
                return Err(invalid("The packet or local case changed after preview. Preview it again."));
            }
            if preview.mode != "CONFLICT" {
                return Err(invalid("Only a conflicting import has commuting operations to apply; apply a fast-forward packet as a whole."));
            }
            if preview.commuting.is_empty() {
                return Err(invalid("No incoming operation commutes with local work since the shared base; merge selected items instead."));
            }
            let mut case = read_case(conn, &packet.case.id)?.ok_or_else(|| invalid("Case disappeared."))?;
            apply_operations(&mut case, &packet.case, &preview.commuting);
            validate(conn, &mut case, false)?;
            let signer = signer_key_id.as_deref().map_or(String::new(), |key| format!(", Ed25519-signed by local key {key}"));
            let provenance = format!(
                "Applied {} commuting operation(s) from packet {} (claimed contributor {}{signer}); {} contested operation(s) left for item-level merge.",
                preview.commuting.len(),
                preview.packet_sha256,
                packet.contributor,
                preview.contested.len()
            );
            let tx = conn.transaction().map_err(storage)?;
            write_case(&tx, &mut case, &author, &provenance)?;
            tx.commit().map_err(storage)?;
            Ok(case)
        })
        .await
    }
    pub async fn casework_research_context(
        &self,
        topic: &str,
    ) -> Result<serde_json::Value, AppError> {
        let topic = topic.to_string();
        self.query_connection("Load unresolved casework",move|conn|{conn.execute_batch(SCHEMA).map_err(storage)?;let mut output=Vec::new();for case in cases(conn)?.into_iter().filter(|c|c.topic==topic&&["OPEN","ACTIVE"].contains(&c.disposition.as_str())).take(6){let value=serde_json::json!({"caseId":case.id,"revision":case.revision,"title":case.title,"questions":case.questions.iter().filter(|q|["OPEN","TESTING","NEEDS_REVIEW"].contains(&q.disposition.as_str())).take(8).map(|q|serde_json::json!({"question":q.question,"hypothesis":q.hypothesis,"nextTest":q.next_test,"citationIds":q.citation_ids,"counterCitationIds":q.counter_citation_ids})).collect::<Vec<_>>(),"requests":case.requests.iter().filter(|r|r.state!="CLOSED").take(6).map(|r|serde_json::json!({"title":r.title,"gap":r.gap,"resolves":r.resolves,"expectedPeriod":r.expected_period,"missing":request_omissions(r),"state":r.state})).collect::<Vec<_>>(),"decisions":case.decisions,"commitments":case.commitments.iter().filter(|c|["UNKNOWN","IN_PROGRESS","NEEDS_REVIEW"].contains(&c.state.as_str())).take(8).collect::<Vec<_>>()});if serde_json::to_vec(&value).map_err(storage)?.len()<18000 {output.push(value);}
if serde_json::to_vec(&output).map_err(storage)?.len()>30000{output.pop();break;}}Ok(serde_json::json!({"instruction":"Revisit unresolved hypotheses, production omissions, decision outcomes and commitment tests using fresh sources. Workflow completion is not evidence of claim correctness. Do not repeat closed requests.","cases":output}))}).await
    }
}
/// Include in the autonomous scheduler fingerprint; no cases means a stable empty revision.
pub(crate) fn casework_revision(conn: &Connection) -> Result<String, AppError> {
    let exists:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM information_schema.tables WHERE table_name='casework_cases')",[],|r|r.get(0)).map_err(storage)?;
    if !exists {
        return Ok(String::new());
    }
    conn.query_row("SELECT COALESCE(md5(string_agg(id||':'||CAST(revision AS VARCHAR),'|' ORDER BY id)),'') FROM casework_cases WHERE disposition IN ('OPEN','ACTIVE')",[],|r|r.get(0)).map_err(storage)
}
pub(crate) fn purge_casework_references(
    conn: &Connection,
    portal: &str,
    document: Option<u64>,
) -> Result<(), AppError> {
    let exists:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM information_schema.tables WHERE table_name='casework_sources')",[],|r|r.get(0)).map_err(storage)?;
    if !exists {
        return Ok(());
    }
    let ids=conn.prepare("SELECT DISTINCT case_id FROM casework_sources WHERE portal_key=?1 AND (?2 IS NULL OR document_id=?2)").map_err(storage)?.query_map(params![portal,document],|r|r.get::<_,String>(0)).map_err(storage)?.collect::<Result<Vec<_>,_>>().map_err(storage)?;
    for id in ids {
        for table in [
            "casework_imports",
            "casework_origins",
            "casework_events",
            "casework_operations",
            "casework_sources",
        ] {
            conn.execute(
                &format!("DELETE FROM {table} WHERE case_id=?1"),
                params![id],
            )
            .map_err(storage)?;
        }
        conn.execute("DELETE FROM casework_cases WHERE id=?1", params![id])
            .map_err(storage)?;
    }
    Ok(())
}

fn rank_leads(reports: &[ResearchReport], cases: &[CaseworkCase]) -> Vec<CaseworkLead> {
    let today = chrono::Utc::now().date_naive();
    let mut leads = Vec::new();
    let mut topics = BTreeSet::new();
    for report in reports {
        if !topics.insert(&report.topic) {
            continue;
        }
        for (index, finding) in report.synthesis.findings.iter().enumerate() {
            let text = format!(
                "{} {} {}",
                finding.title, finding.interpretation, finding.why_it_matters
            )
            .to_lowercase();
            let consequence = if [
                "shelter",
                "bathroom",
                "restroom",
                "housing",
                "eviction",
                "abatement",
                "access",
                "oversight",
            ]
            .iter()
            .any(|term| text.contains(term))
            {
                3
            } else if text.contains('$') || text.contains("appropriat") || text.contains("contract")
            {
                2
            } else {
                1
            };
            let mut evidence = Vec::new();
            citation_ids(report, &finding.citations, &mut evidence);
            let identities = evidence
                .iter()
                .flat_map(|e| {
                    if e.source_references.is_empty() {
                        vec![format!(
                            "{}:{}",
                            e.source_url.as_deref().unwrap_or(&e.evidence_id),
                            e.source_sha256
                        )]
                    } else {
                        e.source_references
                            .iter()
                            .map(|s| format!("{}:{}", s.url, s.sha256))
                            .collect()
                    }
                })
                .collect::<BTreeSet<_>>()
                .len();
            let evidence_score = identities.min(3) as u32;
            let prior = reports
                .iter()
                .filter(|older| older.topic == report.topic && older.created_at < report.created_at)
                .any(|older| {
                    older.synthesis.findings.iter().any(|f| {
                        normalized(&f.title) == normalized(&finding.title)
                            && normalized(&f.interpretation) == normalized(&finding.interpretation)
                    })
                });
            let novelty = if prior { 0 } else { 2 };
            let urgency = report
                .synthesis
                .meetings
                .iter()
                .filter(|m| {
                    m.citations.iter().any(|c| {
                        finding
                            .citations
                            .iter()
                            .any(|f| f.evidence_id == c.evidence_id)
                    })
                })
                .filter_map(|m| chrono::NaiveDate::parse_from_str(&m.date, "%Y-%m-%d").ok())
                .filter(|date| *date >= today && *date <= today + chrono::Duration::days(14))
                .min();
            let factors=vec![CaseworkRankFactor{label:"Public consequence".into(),score:consequence,maximum:3,reason:if consequence==3{"Finding concerns access to essential services or oversight; assess local impact before prioritizing."}else if consequence==2{"Finding concerns an allocation or public contract; verify materiality."}else{"No essential-service or fiscal consequence was identified by the ranking rule."}.into()},CaseworkRankFactor{label:"Evidence available".into(),score:evidence_score,maximum:3,reason:format!("{identities} distinct cited source identities, capped at three. This counts available records, not independent corroboration or correctness.")},CaseworkRankFactor{label:"New lead".into(),score:novelty,maximum:2,reason:if prior{"Same title and interpretation appeared in an earlier dossier."}else{"No identical title and interpretation found in saved earlier dossiers; changed wording can affect this signal."}.into()},CaseworkRankFactor{label:"Decision urgency".into(),score:if urgency.is_some(){3}else{0},maximum:3,reason:urgency.map_or_else(||"No linked cited decision window in the next 14 days.".into(),|d|format!("A cited source also supports an agenda item dated {d}; verify its relevance, latest agenda and participation deadline."))}];
            leads.push(CaseworkLead {
                report_id: report.id.clone(),
                finding_index: index,
                title: finding.title.clone(),
                next_action: finding.next_test.clone(),
                score: factors.iter().map(|f| f.score).sum(),
                factors,
                citations: evidence,
                case_id: cases
                    .iter()
                    .find(|c| c.report_id == report.id)
                    .map(|c| c.id.clone()),
            });
        }
    }
    leads.sort_by(|a, b| b.score.cmp(&a.score).then(a.title.cmp(&b.title)));
    leads.truncate(40);
    leads
}

#[cfg(test)]
mod tests {
    use super::*;
    const TEXT:&str="Provider shall deliver monthly occupied bed-night reports for FY2025. September 28, 2026. Item 605: consider the amendment. The department will publish monthly performance reports by October 1, 2026.";
    fn fixture_report() -> ResearchReport {
        let citation = ResearchCitation {
            evidence_id: "e1".into(),
            quote: "Provider shall deliver monthly occupied bed-night reports for FY2025.".into(),
        };
        ResearchReport{id:"report-1".into(),topic:"homelessness".into(),created_at:"2026-09-26T12:00:00Z".into(),model:"fixture".into(),evidence:vec![ResearchEvidence{id:"e1".into(),label:"Shelter contract and agenda".into(),source_url:Some("https://www.sandiego.gov/fixture.pdf".into()),portal_key:Some("city".into()),request_id:Some(42),document_id:Some(1),character_offset:Some(0),source_sha256:"a".repeat(64),text:TEXT.into(),sql:None,snapshot_provenance:vec![],source_references:vec![],limitations:"Test evidence".into()}],synthesis:ResearchSynthesis{title:"Reconcile shelter reporting".into(),summary:"Obtain the service-volume denominator before comparing invoices.".into(),findings:vec![ResearchFinding{title:"Shelter reporting gap".into(),interpretation:"Delivered service volume is unresolved.".into(),why_it_matters:"Shelter spending needs a service denominator.".into(),citations:vec![citation],counter_evidence:vec![],alternative_explanations:vec!["Reports may be filed separately.".into()],next_test:"Obtain monthly occupied bed nights for FY2025.".into()}],questions:vec![],requests:vec![ResearchRequest{title:"Monthly performance reports".into(),custodian:"Housing Commission".into(),gap:"Bed nights".into(),resolves:"Delivered service volume".into(),draft:"Please provide monthly performance reports for FY2025, including occupied bed nights.".into(),evidence_ids:vec!["e1".into()]}],meetings:vec![ResearchMeeting{body:"City Council".into(),date:"2026-09-28".into(),agenda_item:"605".into(),relevance:"Consider the amendment".into(),action:"Obtain the adopted resolution after the meeting.".into(),citations:vec![ResearchCitation{evidence_id:"e1".into(),quote:"September 28, 2026. Item 605: consider the amendment.".into()}]}],limitations:vec!["Fixture only".into()]},investigation_log:vec![],input_tokens:0,output_tokens:0}
    }
    async fn setup(with_report: bool) -> DuckDbRepository {
        let db = DuckDbRepository::new(":memory:").unwrap();
        db.run_migrations(include_str!("../../migrations/0001_initial_schema.sql"))
            .await
            .unwrap();
        db.initialize_research().await.unwrap();
        db.initialize_casework().await.unwrap();
        let conn = db.analytics_connection().await.unwrap();
        conn.execute_batch("INSERT INTO portals(subdomain,display_name,base_url) VALUES ('city','City','https://www.sandiego.gov'); INSERT INTO requests(id,portal_subdomain,title,status,created_at,updated_at) VALUES (42,'city','Fixture','closed','2026-09-26','2026-09-26'); INSERT INTO documents(id,request_id,portal_subdomain,file_name,download_url,visibility,created_at,updated_at,raw_file_sha256,extraction_status) VALUES (1,42,'city','fixture.pdf','https://www.sandiego.gov/fixture.pdf','public','2026-09-26','2026-09-26',repeat('a',64),'SEARCHABLE');").unwrap();
        if with_report {
            db.store_research_report(fixture_report(), "fixture".into())
                .await
                .unwrap();
        }
        db
    }
    #[tokio::test]
    async fn casework_upgrades_pre_casework_database_and_preserves_real_dossiers() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("old.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("CREATE TABLE research_reports(id TEXT PRIMARY KEY,topic TEXT NOT NULL,created_at TEXT NOT NULL,fingerprint TEXT NOT NULL,report_zstd BLOB NOT NULL);").unwrap();
        }
        {
            let connection = Connection::open(&path).unwrap();
            connection.execute("INSERT INTO research_reports VALUES ('legacy','homelessness','2026-01-01','legacy',?1)",params![encode(&fixture_report()).unwrap()]).unwrap();
        }
        let db = DuckDbRepository::new(path).unwrap();
        db.initialize_casework().await.unwrap();
        db.initialize_casework().await.unwrap();
        let conn = db.analytics_connection().await.unwrap();
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM research_reports WHERE id='legacy'",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM casework_cases", [], |r| r
                .get::<_, u64>(0))
                .unwrap(),
            0
        );
        assert_eq!(casework_revision(&conn).unwrap(), "");
        assert_eq!(
            read_report(&conn, "legacy")
                .unwrap()
                .unwrap()
                .synthesis
                .title,
            "Reconcile shelter reporting"
        );
        assert!(
            conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM information_schema.columns WHERE table_name='casework_imports' AND column_name='signer_key_id')",
                [],
                |r| r.get::<_, bool>(0)
            )
            .unwrap(),
            "the upgrade must add the structured signer_key_id column item 5 relies on to flag \
             already-imported packets from a since-revoked key"
        );
    }
    /// CLAUDE.md: "every test here opens a fresh database, so nothing about an upgraded one is
    /// covered by default." Builds the pre-ledger casework schema directly (the five tables that
    /// shipped before item 3 of the chain-of-custody work) rather than stripping columns from a
    /// current database, and confirms `initialize_casework` adds the ledger tables cleanly and
    /// idempotently on top of real rows.
    #[tokio::test]
    async fn export_ledger_tables_are_added_to_a_database_that_predates_them() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("pre-ledger.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE casework_cases(id TEXT PRIMARY KEY,report_id TEXT NOT NULL,topic TEXT NOT NULL,disposition TEXT NOT NULL,revision BIGINT NOT NULL,payload_zstd BLOB NOT NULL,updated_at TEXT NOT NULL);
                 CREATE TABLE casework_events(case_id TEXT NOT NULL,revision BIGINT NOT NULL,author TEXT NOT NULL,recorded_at TEXT NOT NULL,summary TEXT NOT NULL,payload_zstd BLOB NOT NULL,PRIMARY KEY(case_id,revision));
                 CREATE TABLE casework_sources(case_id TEXT NOT NULL,portal_key TEXT NOT NULL,document_id BIGINT NOT NULL,source_sha256 TEXT NOT NULL,PRIMARY KEY(case_id,portal_key,document_id));
                 CREATE TABLE casework_origins(case_id TEXT PRIMARY KEY,base_revision BIGINT NOT NULL,base_sha256 TEXT NOT NULL);
                 CREATE TABLE casework_imports(case_id TEXT NOT NULL,revision BIGINT NOT NULL,packet_sha256 TEXT NOT NULL,payload_zstd BLOB NOT NULL,PRIMARY KEY(case_id,revision));",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO casework_cases VALUES ('legacy-case','','','OPEN',1,?1,'2026-01-01') ON CONFLICT DO NOTHING",
                params![encode(&CaseworkCase {
                    id: "legacy-case".into(),
                    disposition: "OPEN".into(),
                    revision: 1,
                    ..Default::default()
                })
                .unwrap()],
            )
            .unwrap();
            // A real pre-ledger installation has real event and import history with real
            // payload_zstd snapshots already written. The upgrade must not destroy them: nothing
            // here authorizes deleting existing evidence history, only stopping new growth.
            conn.execute(
                "INSERT INTO casework_events VALUES ('legacy-case',1,'Investigator','2026-01-01','Opened the case',?1)",
                params![b"legacy snapshot bytes".to_vec()],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO casework_imports VALUES ('legacy-case',1,'deadbeef',?1)",
                params![b"legacy packet bytes".to_vec()],
            )
            .unwrap();
        }
        let db = DuckDbRepository::new(&path).unwrap();
        db.initialize_casework().await.unwrap();
        db.initialize_casework().await.unwrap();
        let conn = db.analytics_connection().await.unwrap();
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM casework_export_ledger", [], |r| r
                .get::<_, u64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM casework_export_outcomes", [], |r| r
                .get::<_, u64>(
                0
            ))
            .unwrap(),
            0
        );
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM casework_cases WHERE id='legacy-case'",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
            1,
            "the upgrade must not disturb a real pre-ledger case row"
        );
        assert_eq!(
            conn.query_row(
                "SELECT author,summary,payload_zstd FROM casework_events WHERE case_id='legacy-case' AND revision=1",
                [],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Vec<u8>>(2)?))
            )
            .unwrap(),
            (
                "Investigator".to_string(),
                "Opened the case".to_string(),
                b"legacy snapshot bytes".to_vec()
            ),
            "the upgrade must not delete or alter a pre-existing event's snapshot"
        );
        assert_eq!(
            conn.query_row(
                "SELECT packet_sha256,payload_zstd FROM casework_imports WHERE case_id='legacy-case' AND revision=1",
                [],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))
            )
            .unwrap(),
            ("deadbeef".to_string(), b"legacy packet bytes".to_vec()),
            "the upgrade must not delete or alter a pre-existing imported packet"
        );
        assert_eq!(
            conn.query_row(
                "SELECT signer_key_id FROM casework_imports WHERE case_id='legacy-case' AND revision=1",
                [],
                |r| r.get::<_, Option<String>>(0)
            )
            .unwrap(),
            None,
            "a packet imported before item 5 has no recorded signer, which is absence, not an \
             empty string — the new column must read back as null for it"
        );
    }
    #[test]
    fn evidence_rollup_names_every_non_saved_report_citation_and_never_folds_it_into_verified() {
        let case = CaseworkCase {
            evidence: vec![
                CaseworkCitation {
                    label: "Confirmed budget line".into(),
                    verification: "SAVED_REPORT".into(),
                    ..Default::default()
                },
                CaseworkCitation {
                    label: "Superseded agenda quote".into(),
                    verification: "HISTORICAL_REPORT".into(),
                    ..Default::default()
                },
                CaseworkCitation {
                    label: "Imported from another install".into(),
                    verification: "UNVERIFIED_PORTABLE".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let (verified_citation_count, unverified_citation_labels) = evidence_rollup(&case);
        assert_eq!(verified_citation_count, 1);
        assert_eq!(
            unverified_citation_labels,
            vec![
                "Superseded agenda quote (HISTORICAL_REPORT)".to_string(),
                "Imported from another install (UNVERIFIED_PORTABLE)".to_string(),
            ]
        );
    }
    #[test]
    fn redact_case_for_export_removes_evidence_and_scrubs_every_citation_id_reference() {
        let case = CaseworkCase {
            evidence: vec![
                CaseworkCitation {
                    id: "keep".into(),
                    label: "Keep me".into(),
                    quote: "q1".into(),
                    verification: "SAVED_REPORT".into(),
                    ..Default::default()
                },
                CaseworkCitation {
                    id: "drop".into(),
                    label: "Sensitive source".into(),
                    quote: "q2".into(),
                    verification: "UNVERIFIED_PORTABLE".into(),
                    ..Default::default()
                },
            ],
            questions: vec![CaseworkQuestion {
                citation_ids: vec!["keep".into(), "drop".into()],
                counter_citation_ids: vec!["drop".into()],
                ..Default::default()
            }],
            requests: vec![CaseworkRequest {
                citation_ids: vec!["drop".into()],
                productions: vec![CaseworkProduction {
                    citation_ids: vec!["drop".into(), "keep".into()],
                    ..Default::default()
                }],
                ..Default::default()
            }],
            decisions: vec![CaseworkDecision {
                citation_ids: vec!["drop".into()],
                outcome_citation_ids: vec!["keep".into()],
                ..Default::default()
            }],
            commitments: vec![CaseworkCommitment {
                citation_ids: vec!["drop".into()],
                completion_citation_ids: vec!["keep".into()],
                ..Default::default()
            }],
            ..Default::default()
        };
        let withheld = vec![WithheldCitationRequest {
            citation_id: "drop".into(),
            reason: "Ongoing personnel investigation.".into(),
        }];
        let (redacted, withheld_citations) = redact_case_for_export(case, &withheld).unwrap();
        assert_eq!(redacted.evidence.len(), 1);
        assert_eq!(redacted.evidence[0].id, "keep");
        assert_eq!(
            withheld_citations,
            vec![WithheldCitation {
                label: "Sensitive source".into(),
                reason: "Ongoing personnel investigation.".into(),
            }]
        );
        assert_eq!(redacted.questions[0].citation_ids, vec!["keep".to_string()]);
        assert!(redacted.questions[0].counter_citation_ids.is_empty());
        assert!(redacted.requests[0].citation_ids.is_empty());
        assert_eq!(
            redacted.requests[0].productions[0].citation_ids,
            vec!["keep".to_string()]
        );
        assert!(redacted.decisions[0].citation_ids.is_empty());
        assert_eq!(
            redacted.decisions[0].outcome_citation_ids,
            vec!["keep".to_string()]
        );
        assert!(redacted.commitments[0].citation_ids.is_empty());
        assert_eq!(
            redacted.commitments[0].completion_citation_ids,
            vec!["keep".to_string()]
        );
    }
    #[test]
    fn redact_case_for_export_rejects_empty_reason_unknown_id_and_duplicate_withholding() {
        let case = CaseworkCase {
            evidence: vec![CaseworkCitation {
                id: "c1".into(),
                label: "L".into(),
                quote: "q".into(),
                verification: "SAVED_REPORT".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(redact_case_for_export(
            case.clone(),
            &[WithheldCitationRequest {
                citation_id: "c1".into(),
                reason: "  ".into(),
            }],
        )
        .is_err());
        assert!(redact_case_for_export(
            case.clone(),
            &[WithheldCitationRequest {
                citation_id: "missing".into(),
                reason: "Because.".into(),
            }],
        )
        .is_err());
        assert!(redact_case_for_export(
            case,
            &[
                WithheldCitationRequest {
                    citation_id: "c1".into(),
                    reason: "First.".into(),
                },
                WithheldCitationRequest {
                    citation_id: "c1".into(),
                    reason: "Second.".into(),
                },
            ],
        )
        .is_err());
    }
    /// Two exports fired concurrently against the same case must still serialize into a single,
    /// unforked chain: sequences 1 and 2, with #2's `previousManifestSha256` matching #1's
    /// `manifestSha256`. This is the scenario `PRIMARY KEY(case_id,sequence)` and the
    /// read-build-sign-insert-in-one-transaction design exist to guarantee.
    #[tokio::test]
    async fn concurrent_exports_serialize_into_a_correctly_linked_chain_without_forking() {
        let db = setup(false).await;
        let conn = db.analytics_connection().await.unwrap();
        let mut case = CaseworkCase {
            id: "case-concurrent".into(),
            title: "Concurrency fixture".into(),
            evidence: vec![CaseworkCitation {
                id: "c1".into(),
                label: "Portable claim".into(),
                quote: "A claim.".into(),
                verification: "UNVERIFIED_PORTABLE".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        write_case(&conn, &mut case, "Fixture author", "Created").unwrap();
        let (first, second) = tokio::join!(
            db.export_case_evidence_package_signed(
                "case-concurrent".into(),
                "2026-09-29T00:00:00Z".into(),
                Vec::new(),
                SigningKey::from_bytes(&[2_u8; 32]),
            ),
            db.export_case_evidence_package_signed(
                "case-concurrent".into(),
                "2026-09-29T00:00:01Z".into(),
                Vec::new(),
                SigningKey::from_bytes(&[2_u8; 32]),
            ),
        );
        let first = first.unwrap();
        let second = second.unwrap();
        let mut sequences = vec![
            first.manifest.ledger_sequence,
            second.manifest.ledger_sequence,
        ];
        sequences.sort_unstable();
        assert_eq!(
            sequences,
            vec![1, 2],
            "two concurrent exports must land on distinct, consecutive sequence numbers"
        );
        let chain = db
            .case_export_chain("case-concurrent".into())
            .await
            .unwrap();
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[0].sequence, 1);
        assert!(chain[0].previous_manifest_sha256.is_none());
        assert_eq!(chain[1].sequence, 2);
        assert_eq!(
            chain[1].previous_manifest_sha256.as_deref(),
            Some(chain[0].manifest_sha256.as_str()),
            "the later export must chain to the earlier one, whichever finished signing first"
        );
    }
    /// A ledger entry with no recorded outcome is neither "written" nor "failed" — it is "reserved,
    /// not confirmed written," the state a crash between signing and the outcome append leaves
    /// behind. `case_export_chain` must report that state explicitly rather than defaulting it to
    /// either extreme, and must report a genuinely failed write as failed, not silently as reserved.
    #[tokio::test]
    async fn case_export_chain_reports_reserved_written_and_failed_outcomes_truthfully() {
        let db = setup(false).await;
        let conn = db.analytics_connection().await.unwrap();
        let mut case = CaseworkCase {
            id: "case-outcomes".into(),
            title: "Outcome fixture".into(),
            evidence: vec![CaseworkCitation {
                id: "c1".into(),
                label: "L".into(),
                quote: "q".into(),
                verification: "UNVERIFIED_PORTABLE".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        write_case(&conn, &mut case, "Fixture author", "Created").unwrap();
        db.export_case_evidence_package_signed(
            "case-outcomes".into(),
            "t1".into(),
            Vec::new(),
            SigningKey::from_bytes(&[4_u8; 32]),
        )
        .await
        .unwrap();
        db.export_case_evidence_package_signed(
            "case-outcomes".into(),
            "t2".into(),
            Vec::new(),
            SigningKey::from_bytes(&[4_u8; 32]),
        )
        .await
        .unwrap();
        db.record_case_export_outcome(
            "case-outcomes".into(),
            2,
            EXPORT_OUTCOME_WRITTEN,
            Some("/tmp/out.json".into()),
        )
        .await
        .unwrap();
        db.export_case_evidence_package_signed(
            "case-outcomes".into(),
            "t3".into(),
            Vec::new(),
            SigningKey::from_bytes(&[4_u8; 32]),
        )
        .await
        .unwrap();
        db.record_case_export_outcome(
            "case-outcomes".into(),
            3,
            EXPORT_OUTCOME_FAILED,
            Some("disk full".into()),
        )
        .await
        .unwrap();

        let chain = db.case_export_chain("case-outcomes".into()).await.unwrap();
        assert_eq!(chain.len(), 3);
        assert_eq!(chain[0].outcome, "RESERVED");
        assert!(chain[0].outcome_detail.is_none());
        assert_eq!(chain[1].outcome, "WRITTEN");
        assert_eq!(chain[1].outcome_detail.as_deref(), Some("/tmp/out.json"));
        assert_eq!(chain[2].outcome, "FAILED");
        assert_eq!(chain[2].outcome_detail.as_deref(), Some("disk full"));
        assert!(chain[0].previous_manifest_sha256.is_none());
        assert_eq!(
            chain[1].previous_manifest_sha256.as_deref(),
            Some(chain[0].manifest_sha256.as_str())
        );
        assert_eq!(
            chain[2].previous_manifest_sha256.as_deref(),
            Some(chain[1].manifest_sha256.as_str())
        );
        assert!(chain[0].previous_link_valid);
        assert!(chain[1].previous_link_valid);
        assert!(chain[2].previous_link_valid);
    }
    /// `case_export_chain` must actually catch a broken link, not just report the raw hashes for a
    /// human to eyeball. Proven by directly corrupting a ledger row's `previous_manifest_sha256`
    /// with an `UPDATE` — the same tamper shape a compromised or buggy write would produce — and
    /// showing only the corrupted entry (not its neighbors) reads as invalid.
    #[tokio::test]
    async fn case_export_chain_flags_a_tampered_link() {
        let db = setup(false).await;
        let conn = db.analytics_connection().await.unwrap();
        let mut case = CaseworkCase {
            id: "case-tamper".into(),
            title: "Tamper fixture".into(),
            evidence: vec![CaseworkCitation {
                id: "c1".into(),
                label: "L".into(),
                quote: "q".into(),
                verification: "UNVERIFIED_PORTABLE".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        write_case(&conn, &mut case, "Fixture author", "Created").unwrap();
        for timestamp in ["t1", "t2", "t3"] {
            db.export_case_evidence_package_signed(
                "case-tamper".into(),
                timestamp.into(),
                Vec::new(),
                SigningKey::from_bytes(&[9_u8; 32]),
            )
            .await
            .unwrap();
        }
        conn.execute(
            "UPDATE casework_export_ledger SET previous_manifest_sha256='0'||previous_manifest_sha256 WHERE case_id='case-tamper' AND sequence=2",
            [],
        )
        .unwrap();
        let chain = db.case_export_chain("case-tamper".into()).await.unwrap();
        assert!(chain[0].previous_link_valid, "sequence 1 was not touched");
        assert!(
            !chain[1].previous_link_valid,
            "sequence 2's corrupted previous hash must be caught"
        );
        assert!(
            chain[2].previous_link_valid,
            "sequence 3's own link to sequence 2's real manifest hash was not touched by this \
             tamper and must still verify independently — the corruption is localized to the \
             specific broken pair, not smeared across the whole chain"
        );
    }
    /// Locks the cross-language contract the standalone Node verifier depends on: this exact case
    /// (persisted through the real repository, not hand-built in memory, so `refresh_evidence`'s
    /// validation is what produced these fields), signed with this exact key at this exact
    /// timestamp, must always serialize to these exact committed bytes. CLAUDE.md: hand-maintained
    /// cross-language contracts are "checked, not trusted." If this test fails after a deliberate
    /// schema change, regenerate the fixture from this same setup and update
    /// `scripts/verify-evidence-package.mjs` and its gate test together.
    #[tokio::test]
    async fn case_evidence_package_bytes_match_the_committed_cross_language_fixture() {
        let db = setup(true).await;
        let conn = db.analytics_connection().await.unwrap();
        let report = fixture_report();
        let saved = canonical_citation(
            &report,
            &report.evidence[0],
            "Provider shall deliver monthly occupied bed-night reports for FY2025.",
        );
        let portable = CaseworkCitation {
            id: "portable-1".into(),
            label: "Imported from another install".into(),
            quote: "A claim from a peer packet.".into(),
            verification: "UNVERIFIED_PORTABLE".into(),
            ..Default::default()
        };
        let mut case = CaseworkCase {
            id: "case-1".into(),
            title: "Fixture case".into(),
            evidence: vec![saved, portable],
            ..Default::default()
        };
        write_case(&conn, &mut case, "Fixture author", "Created fixture case").unwrap();
        // `write_case` stamps real wall-clock time into `updated_at`; pin it to a fixed value
        // directly in storage afterward so this byte-for-byte fixture stays reproducible across
        // every run, not just the one that happened to generate the committed file.
        case.updated_at = "2026-09-28T00:00:00Z".to_string();
        conn.execute(
            "UPDATE casework_cases SET updated_at=?1, payload_zstd=?2 WHERE id=?3",
            params![case.updated_at, encode(&case).unwrap(), case.id],
        )
        .unwrap();
        let key = SigningKey::from_bytes(&[3_u8; 32]);
        let package = db
            .export_case_evidence_package_signed(
                "case-1".into(),
                "2026-09-28T00:00:00Z".into(),
                Vec::new(),
                key,
            )
            .await
            .unwrap();
        verify_package(&package).expect("the generated fixture must self-verify");
        let bytes = serde_json::to_vec_pretty(&package).unwrap();
        let committed = include_bytes!("../../tests/fixtures/chain-of-custody-fixture.json");
        assert_eq!(
            String::from_utf8_lossy(&bytes),
            String::from_utf8_lossy(committed),
            "the signed package's byte layout has changed; see this test's doc comment"
        );
    }
    #[tokio::test]
    async fn creates_persistent_case_from_real_report_revisits_unresolved_work_and_rejects_stale_revisions(
    ) {
        let db = setup(true).await;
        let case = db.create_casework_case("report-1".into()).await.unwrap();
        assert_eq!(case.revision, 1);
        assert_eq!(
            case.questions[0].hypothesis,
            "Delivered service volume is unresolved."
        );
        assert!(case
            .evidence
            .iter()
            .all(|c| c.verification == "SAVED_REPORT"));
        assert_eq!(
            db.create_casework_case("report-1".into()).await.unwrap().id,
            case.id
        );
        let conn = db.analytics_connection().await.unwrap();
        let before = casework_revision(&conn).unwrap();
        let mut edited = case.clone();
        edited.owner = "Investigations desk".into();
        edited.questions[0].owner = "Private contributor label".into();
        edited.notes = "Private editorial notes".into();
        edited.questions[0].hypothesis = "Reports may exist in a separate contract file.".into();
        let saved = db
            .save_casework_case(edited, 1, "Editor".into())
            .await
            .unwrap();
        assert_eq!(saved.revision, 2);
        assert_ne!(casework_revision(&conn).unwrap(), before);
        assert!(db
            .save_casework_case(case, 1, "Second editor".into())
            .await
            .is_err());
        let context = db
            .casework_research_context("homelessness")
            .await
            .unwrap()
            .to_string();
        assert!(context.contains("separate contract file"));
        assert!(!context.contains("Private contributor label"));
        assert!(!context.contains("Private editorial notes"));
        assert!(context.contains("Expected record not yet recorded"));
        let state = db.casework_state().await.unwrap();
        assert_eq!(state.cases[0].owner, "Investigations desk");
        assert_eq!(state.events.len(), 2);
        assert_eq!(
            state.leads[0].score,
            state.leads[0].factors.iter().map(|f| f.score).sum::<u32>()
        );
        assert!(state.leads[0]
            .factors
            .iter()
            .any(|f| f.reason.contains("not independent corroboration")));
    }
    /// The filing record is the only artifact in this application that states a request was filed,
    /// and the application never observes a send. So it must refuse to exist for a request with no
    /// recorded submission date, and must carry the digest of the exact text it names as filed.
    /// The filing record is the only artifact in this application that states a request was filed,
    /// and the application never observes a send. So it must refuse to exist for a request with no
    /// recorded submission date, and must carry the digest of the exact text it names as filed.
    /// Same cross-language lock as the chain-of-custody fixture above, for the filing-record
    /// schema the browser verifier (`scripts/verify-evidence-package.html`) is tested against.
    #[tokio::test]
    async fn filing_record_bytes_match_the_committed_browser_verifier_fixture() {
        let db = setup(true).await;
        let mut case = db.create_casework_case("report-1".into()).await.unwrap();
        let request_id = case.requests[0].id.clone();
        case.requests[0].state = "SUBMITTED".into();
        case.requests[0].submitted_at = Some("2026-09-01".into());
        let saved = db
            .save_casework_case(case, 1, "Reporter".into())
            .await
            .unwrap();
        let package = db
            .sign_request_filing_record(
                saved.id.clone(),
                request_id,
                "2026-10-02T12:00:00Z".into(),
                SigningKey::from_bytes(&[11_u8; 32]),
            )
            .await
            .unwrap();
        verify_package(&package).expect("the generated record must self-verify");
        // The case id is minted at creation, so the repository-built manifest is re-signed under
        // a fixed id; everything else (compiled text, clock, omissions) is the repository's own.
        let mut manifest = package.manifest;
        manifest.case_id = "case-fixture".into();
        let package = sign_package(manifest, &SigningKey::from_bytes(&[11_u8; 32])).unwrap();
        assert_eq!(
            String::from_utf8_lossy(&serde_json::to_vec_pretty(&package).unwrap()),
            String::from_utf8_lossy(include_bytes!(
                "../../tests/fixtures/request-filing-record-v1-fixture.json"
            )),
            "the signed filing record's byte layout has changed; regenerate the fixture"
        );
    }

    #[tokio::test]
    async fn a_filing_record_requires_a_recorded_submission_and_binds_the_request_text_digest() {
        let db = setup(true).await;
        let mut case = db.create_casework_case("report-1".into()).await.unwrap();
        let request_id = case.requests[0].id.clone();
        let key = SigningKey::from_bytes(&[7_u8; 32]);

        let unfiled = db
            .sign_request_filing_record(case.id.clone(), request_id.clone(), now(), key.clone())
            .await
            .unwrap_err();
        assert!(unfiled.to_string().contains("confirmed submission date"));

        case.requests[0].state = "SUBMITTED".into();
        case.requests[0].submitted_at = Some("2026-09-01".into());
        case.requests[0].expected_fields = vec!["occupied bed nights".into()];
        let saved = db
            .save_casework_case(case, 1, "Reporter".into())
            .await
            .unwrap();

        let package = db
            .sign_request_filing_record(saved.id.clone(), request_id, now(), key)
            .await
            .unwrap();
        let manifest = &package.manifest;
        assert_eq!(
            manifest.request_text_sha256,
            sha256_hex(manifest.request_text.as_bytes()),
            "a filing record must bind the digest of the exact text it names as filed"
        );
        assert_eq!(manifest.filing_plan.filed_on, "2026-09-01");
        assert!(manifest
            .remaining_omissions
            .iter()
            .any(|omission| omission.contains("occupied bed nights")));
        assert!(manifest
            .identifying_sources
            .iter()
            .all(|source| !source.source_sha256.is_empty()));
    }
    #[tokio::test]
    async fn productions_reconcile_fields_and_periods_and_prepare_specific_unsent_followup() {
        let db = setup(true).await;
        let mut case = db.create_casework_case("report-1".into()).await.unwrap();
        let request = &mut case.requests[0];
        request.expected_fields = vec!["occupied bed nights".into(), "invoice month".into()];
        request.expected_period = "FY2025".into();
        request.state = "PARTIAL".into();
        request.submitted_at = Some("2026-09-01".into());
        request.external_reference = "PR-2026-42".into();
        request.productions.push(CaseworkProduction {
            id: "production-1".into(),
            received_at: "2026-09-15".into(),
            description: "Monthly report spreadsheet".into(),
            record_types: vec!["Monthly performance reports".into()],
            fields: vec!["invoice month".into()],
            period: "FY2024".into(),
            citation_ids: vec![],
            notes: String::new(),
        });
        let saved = db
            .save_casework_case(case, 1, "Reporter".into())
            .await
            .unwrap();
        let request = &saved.requests[0];
        assert!(request.follow_up_draft.contains("occupied bed nights"));
        assert!(request.follow_up_draft.contains("FY2025"));
        assert!(request.follow_up_draft.contains("PR-2026-42"));
        assert!(request.follow_up_draft.contains("no message has been sent"));
        assert!(!request_omissions(request)
            .iter()
            .any(|s| s.contains("invoice month")));
        let mut complete = saved.clone();
        complete.requests[0].state = "PRODUCED".into();
        assert!(db
            .save_casework_case(complete.clone(), 2, "Reporter".into())
            .await
            .is_err());
        complete.requests[0].productions[0]
            .fields
            .push("occupied bed nights".into());
        complete.requests[0].productions[0].period = "FY2025".into();
        assert!(db
            .save_casework_case(complete, 2, "Reporter".into())
            .await
            .is_ok());
    }
    #[tokio::test]
    async fn dated_decisions_and_commitments_require_actual_completion_evidence() {
        let db = setup(true).await;
        let mut case = db.create_casework_case("report-1".into()).await.unwrap();
        let cite = case
            .evidence
            .iter()
            .find(|c| c.quote.contains("department will publish"))
            .unwrap()
            .id
            .clone();
        case.commitments.push(CaseworkCommitment {
            id: "promise".into(),
            promise: "Publish monthly performance reports".into(),
            responsible_party: "Department".into(),
            due_date: Some("2020-01-01".into()),
            completion_test: "A dated public release contains all required reporting months."
                .into(),
            state: "UNKNOWN".into(),
            citation_ids: vec![cite.clone()],
            ..Default::default()
        });
        let saved = db
            .save_casework_case(case, 1, "Reporter".into())
            .await
            .unwrap();
        assert_eq!(saved.commitments[0].state, "UNKNOWN");
        let mut claimed = saved.clone();
        claimed.commitments[0].state = "NOT_MET".into();
        assert!(db
            .save_casework_case(claimed, 2, "Reporter".into())
            .await
            .is_err());
        let mut wrong_date = saved.clone();
        wrong_date.decisions[0].date = "2027-09-28".into();
        assert!(db
            .save_casework_case(wrong_date, 2, "Reporter".into())
            .await
            .is_err());
        let mut unsupported_vote = saved.clone();
        unsupported_vote.decisions[0].state = "DECIDED".into();
        unsupported_vote.decisions[0].outcome = "Approved".into();
        assert!(db
            .save_casework_case(unsupported_vote, 2, "Reporter".into())
            .await
            .is_err());
        let mut later = fixture_report();
        later.id = "report-2".into();
        later.evidence[0].text="On October 1, 2026 the department released the complete monthly performance reports for FY2025.".into();
        later.evidence[0].id = "completion".into();
        later.synthesis.meetings.clear();
        db.store_research_report(later, "later".into())
            .await
            .unwrap();
        let added = db
            .casework_state()
            .await
            .unwrap()
            .available_evidence
            .into_iter()
            .find(|e| e.report_id == "report-2")
            .unwrap();
        let mut complete = saved;
        complete.evidence.push(added.clone());
        complete.commitments[0].state = "COMPLETE".into();
        complete.commitments[0].assessment =
            "The dated release records completion of the required FY2025 reports.".into();
        complete.commitments[0].completion_citation_ids = vec![added.id];
        assert_eq!(
            db.save_casework_case(complete, 2, "Reporter".into())
                .await
                .unwrap()
                .commitments[0]
                .state,
            "COMPLETE"
        );
    }
    /// Round trip through the log: the partner adds a question and we edit the notes; the
    /// partner's addition commutes and is applied as one revision, our notes survive, and the
    /// partner's edit of an item we also changed is left contested.
    #[tokio::test]
    async fn commuting_operations_from_a_diverged_packet_apply_without_touching_local_work() {
        let db = setup(true).await;
        let case = db.create_casework_case("report-1".into()).await.unwrap();
        let question_id = case.questions[0].id.clone();
        let payload = db
            .export_casework_packet(case.id.clone(), "Origin".into())
            .await
            .unwrap();
        let mut packet: CaseworkPacket = serde_json::from_str(&payload).unwrap();
        assert!(
            !packet.operations.is_empty(),
            "a packet carries the log from the first revision"
        );

        let remote = setup(false).await;
        let preview = remote
            .preview_casework_import(payload.clone())
            .await
            .unwrap();
        let mut theirs = remote
            .import_casework_packet(
                payload,
                preview.local_revision,
                preview.packet_sha256,
                "Partner".into(),
                None,
            )
            .await
            .unwrap();
        theirs.questions.push(CaseworkQuestion {
            id: "partner-q".into(),
            question: "Did the contractor hold a licence?".into(),
            next_test: "Check the CSLB register.".into(),
            disposition: "OPEN".into(),
            ..CaseworkQuestion::default()
        });
        theirs.questions[0].question = "Partner's rewording".into();
        let revision = theirs.revision;
        remote
            .save_casework_case(theirs, revision, "Partner".into())
            .await
            .unwrap();
        let returned = remote
            .export_casework_packet(case.id.clone(), "Partner".into())
            .await
            .unwrap();

        let mut mine = case.clone();
        mine.notes = "Local notes".into();
        mine.questions[0].question = "My rewording".into();
        db.save_casework_case(mine, 1, "Me".into()).await.unwrap();

        let preview = db.preview_casework_import(returned.clone()).await.unwrap();
        assert_eq!(preview.mode, "CONFLICT");
        assert_eq!(
            preview
                .commuting
                .iter()
                .map(|o| o.target.as_str())
                .collect::<Vec<_>>(),
            vec!["partner-q"]
        );
        assert!(preview.contested.iter().any(|o| o.target == question_id));

        let merged = db
            .apply_commuting_case_operations(
                returned,
                preview.local_revision,
                preview.packet_sha256,
                "Me".into(),
                None,
            )
            .await
            .unwrap();
        assert_eq!(merged.notes, "Local notes");
        assert_eq!(merged.questions[0].question, "My rewording");
        assert!(merged.questions.iter().any(|q| q.id == "partner-q"));
        packet.case = merged;
        let log = db
            .export_casework_packet(case.id.clone(), "Me".into())
            .await
            .unwrap();
        let packet: CaseworkPacket = serde_json::from_str(&log).unwrap();
        assert!(super::super::casework_operations::chain_is_intact(
            &packet.operations
        ));
    }
    #[tokio::test]
    async fn portable_cases_require_preview_detect_conflicts_and_preserve_unverified_sources() {
        let db = setup(true).await;
        let case = db.create_casework_case("report-1".into()).await.unwrap();
        let original = db
            .export_casework_packet(case.id.clone(), "Origin desk".into())
            .await
            .unwrap();
        let mut packet: CaseworkPacket = serde_json::from_str(&original).unwrap();
        let mut unrelated = packet.clone();
        unrelated.case.topic = "police".into();
        assert_eq!(
            db.preview_casework_import(serde_json::to_string(&unrelated).unwrap())
                .await
                .unwrap()
                .mode,
            "CONFLICT",
            "a matching revision base cannot change the case identity"
        );
        packet.case.owner = "Partner desk".into();
        packet.contributor = "Partner".into();
        let payload = serde_json::to_string(&packet).unwrap();
        let preview = db.preview_casework_import(payload.clone()).await.unwrap();
        assert_eq!(preview.mode, "FAST_FORWARD");
        let mut local = case.clone();
        local.notes = "Local independent work".into();
        db.save_casework_case(local, 1, "Local editor".into())
            .await
            .unwrap();
        assert!(db
            .import_casework_packet(
                payload.clone(),
                preview.local_revision,
                preview.packet_sha256,
                "Local editor".into(),
                None
            )
            .await
            .is_err());
        assert_eq!(
            db.preview_casework_import(payload.clone())
                .await
                .unwrap()
                .mode,
            "CONFLICT"
        );
        assert_eq!(
            db.casework_state().await.unwrap().cases[0].notes,
            "Local independent work"
        );
        let remote = setup(false).await;
        let preview = remote
            .preview_casework_import(payload.clone())
            .await
            .unwrap();
        assert_eq!(preview.mode, "NEW");
        assert!(preview
            .case
            .evidence
            .iter()
            .all(|c| c.verification == "UNVERIFIED_PORTABLE"));
        let imported = remote
            .import_casework_packet(
                payload,
                preview.local_revision,
                preview.packet_sha256,
                "Receiving editor".into(),
                Some("fixture-signer-key".into()),
            )
            .await
            .unwrap();
        assert_eq!(imported.owner, "Partner desk");
        let forwarded: CaseworkPacket = serde_json::from_str(
            &remote
                .export_casework_packet(imported.id.clone(), "Receiving editor".into())
                .await
                .unwrap(),
        )
        .unwrap();
        assert!(
            forwarded
                .history
                .iter()
                .any(|event| event.author == "Research workspace"
                    && event.summary.contains("Portable history from packet")),
            "original contributor provenance must survive another handoff"
        );
        assert!(forwarded
            .history
            .iter()
            .any(|event| event.author == "Receiving editor"));

        assert!(remote.casework_state().await.unwrap().events[0]
            .summary
            .contains("claimed contributor Partner"));
        // Recording the verified signer key in the event history is what turns "contributor" from
        // a free-text label into something a later reader can cross-check against other packets.
        assert!(remote.casework_state().await.unwrap().events[0]
            .summary
            .contains("Ed25519-signed by local key fixture-signer-key"));
        let conn = remote.analytics_connection().await.unwrap();
        conn.execute("UPDATE documents SET visibility='private' WHERE id=1", [])
            .unwrap();
        assert!(remote.preview_casework_import(original).await.is_err());
        let operations = |conn: &Connection| -> i64 {
            conn.query_row(
                "SELECT COUNT(*) FROM casework_operations WHERE case_id=?1",
                params![imported.id],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert!(
            operations(&conn) > 0,
            "the import must have logged operations"
        );
        purge_casework_references(&conn, "city", Some(1)).unwrap();
        assert!(read_case(&conn,&imported.id).unwrap().is_none(),"portable cases with an exact local source identity participate in purge without becoming verified quotations");
        // The log's summaries quote the case's text, so a purge that left them would keep the
        // withdrawn content on disk and collide with a re-import of the same case.
        assert_eq!(operations(&conn), 0);
    }
    #[tokio::test]
    async fn portable_history_is_scoped_to_the_case_and_never_silently_truncated() {
        let db = setup(true).await;
        let case = db.create_casework_case("report-1".into()).await.unwrap();
        let conn = db.analytics_connection().await.unwrap();
        conn.execute("INSERT INTO casework_events SELECT 'other-case',range,'Other desk','2099-01-01','Unrelated activity',?1 FROM range(1,1002)", params![Vec::<u8>::new()]).unwrap();
        let packet: CaseworkPacket = serde_json::from_str(
            &db.export_casework_packet(case.id.clone(), "Editor".into())
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(packet.history.len(), 1);
        assert_eq!(packet.history[0].author, "Research workspace");
        conn.execute("INSERT INTO casework_events SELECT ?1,range,'Investigator','2026-09-27','Case activity',?2 FROM range(2,1002)", params![case.id, Vec::<u8>::new()]).unwrap();
        let error = db
            .export_casework_packet(case.id.clone(), "Editor".into())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("1,000-event"));
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM casework_events WHERE case_id=?1",
                params![case.id],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
            1001
        );
    }
    #[tokio::test]
    async fn historical_public_sources_remain_editable_but_assessments_require_review_and_privacy_stays_strict(
    ) {
        let db = setup(true).await;
        let mut case = db.create_casework_case("report-1".into()).await.unwrap();
        let mut invented = case.clone();
        invented.evidence[0].quote = "This invented quotation was never in the source.".into();
        assert!(db
            .save_casework_case(invented, 1, "Reporter".into())
            .await
            .is_err());
        let quote = case.evidence[0].quote.clone();
        let citation = case.questions[0].citation_ids[0].clone();
        case.questions[0].disposition = "SUPPORTED".into();
        case.decisions[0].state = "DECIDED".into();
        case.decisions[0].outcome = "Reviewer recorded the adopted action in this fixture.".into();
        case.decisions[0].outcome_citation_ids = vec![citation.clone()];
        case.commitments.push(CaseworkCommitment {
            id: "historical-completion".into(),
            promise: "Publish the monthly reports".into(),
            completion_test: "Compare the release to the required months".into(),
            state: "COMPLETE".into(),
            assessment: "Reviewer recorded the release in this fixture.".into(),
            citation_ids: vec![citation.clone()],
            completion_citation_ids: vec![citation],
            ..Default::default()
        });
        let prior = db
            .save_casework_case(case, 1, "Reporter".into())
            .await
            .unwrap();
        let conn = db.analytics_connection().await.unwrap();
        conn.execute(
            "UPDATE documents SET raw_file_sha256=repeat('b',64) WHERE id=1",
            [],
        )
        .unwrap();
        let loaded = db.casework_state().await.unwrap();
        let historical = &loaded.cases[0];
        assert!(historical
            .evidence
            .iter()
            .all(|c| c.verification == "HISTORICAL_REPORT"));
        assert_eq!(historical.evidence[0].quote, quote);
        assert_eq!(historical.evidence[0].source_sha256, "a".repeat(64));
        assert_eq!(historical.questions[0].disposition, "NEEDS_REVIEW");
        assert_eq!(historical.decisions[0].state, "NEEDS_REVIEW");
        assert_eq!(historical.commitments[0].state, "NEEDS_REVIEW");
        assert_eq!(
            historical.commitments[0].assessment,
            prior.commitments[0].assessment
        );
        assert!(loaded
            .available_evidence
            .iter()
            .all(|c| c.verification == "HISTORICAL_REPORT"));
        assert!(loaded
            .leads
            .iter()
            .flat_map(|l| &l.citations)
            .all(|c| c.verification == "HISTORICAL_REPORT"));
        let exported: CaseworkPacket = serde_json::from_str(
            &db.export_casework_packet(prior.id.clone(), "Editor".into())
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(exported.case.evidence[0].verification, "HISTORICAL_REPORT");
        assert_eq!(exported.case.commitments[0].state, "NEEDS_REVIEW");
        let mut edited = prior; // A stale open editor can still save notes after a publisher update.
        edited.notes = "Compare the newly published source with the preserved version.".into();
        edited.requests[0]
            .expected_fields
            .push("occupied bed nights".into());
        let saved = db
            .save_casework_case(edited, 2, "Reporter".into())
            .await
            .unwrap();
        assert_eq!(saved.revision, 3);
        assert_eq!(saved.questions[0].disposition, "NEEDS_REVIEW");
        assert_eq!(saved.commitments[0].state, "NEEDS_REVIEW");
        assert!(saved.notes.contains("newly published"));
        let context = db.casework_research_context("homelessness").await.unwrap();
        assert_eq!(
            context["cases"][0]["questions"].as_array().unwrap().len(),
            1
        );
        assert_eq!(
            context["cases"][0]["commitments"][0]["state"],
            "NEEDS_REVIEW"
        );
        // Attaching a later report whose original source matches can support a reviewed conclusion.
        let mut current = fixture_report();
        current.id = "report-current".into();
        current.evidence[0].source_sha256 = "b".repeat(64);
        db.store_research_report(current, "new-source-version".into())
            .await
            .unwrap();
        let current_citation = db
            .casework_state()
            .await
            .unwrap()
            .available_evidence
            .into_iter()
            .find(|c| c.report_id == "report-current")
            .unwrap();
        assert_eq!(current_citation.verification, "SAVED_REPORT");
        let mut reviewed = saved;
        reviewed.questions[0]
            .citation_ids
            .push(current_citation.id.clone());
        reviewed.questions[0].disposition = "SUPPORTED".into();
        reviewed.evidence.push(current_citation);
        let reviewed = db
            .save_casework_case(reviewed, 3, "Editor".into())
            .await
            .unwrap();
        assert_eq!(reviewed.questions[0].disposition, "SUPPORTED");
        conn.execute("UPDATE documents SET visibility='private' WHERE id=1", [])
            .unwrap();
        assert!(db
            .save_casework_case(reviewed.clone(), 4, "Reporter".into())
            .await
            .is_err());
        assert!(db
            .export_casework_packet(reviewed.id.clone(), "Editor".into())
            .await
            .is_err());
        assert!(db.casework_state().await.is_err());
        conn.execute("UPDATE documents SET visibility='public' WHERE id=1", [])
            .unwrap();
        conn.execute(
            "UPDATE portals SET lifecycle_state='REMOVED' WHERE subdomain='city'",
            [],
        )
        .unwrap();
        assert!(db
            .save_casework_case(reviewed.clone(), 4, "Reporter".into())
            .await
            .is_err());
        purge_casework_references(&conn, "city", Some(1)).unwrap();
        assert!(read_case(&conn, &reviewed.id).unwrap().is_none());
        assert!(events(&conn).unwrap().is_empty());
    }
    #[test]
    fn citation_validation_accepts_exact_sql_rows_with_formatting_differences_but_not_changed_values(
    ) {
        let mut evidence = fixture_report().evidence.remove(0);
        evidence.sql = Some("SELECT amount, revised_authority FROM analysis_fixture".into());
        evidence.text =
            r#"{"columns":["amount","revised_authority"],"rows":[["9007199254740993.01",null]]}"#
                .into();
        assert!(quote_exists(
            &evidence,
            r#"[ "9007199254740993.01", null ]"#
        ));
        assert!(!quote_exists(&evidence, r#"[ "9007199254740993.01", 0 ]"#));
        assert!(!quote_exists(
            &evidence,
            r#"[ "9007199254740992.00", null ]"#
        ));
    }
}
