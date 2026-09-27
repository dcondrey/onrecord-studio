use super::sqlite::DuckDbRepository;
use crate::{
    error::AppError,
    integrity::sha256_hex,
    types::{casework::*, research::*},
};
use duckdb::{params, Connection, OptionalExt};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
};

const MAX_PACKET: usize = 2_000_000;
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS casework_cases(id TEXT PRIMARY KEY,report_id TEXT NOT NULL,topic TEXT NOT NULL,disposition TEXT NOT NULL,revision BIGINT NOT NULL,payload_zstd BLOB NOT NULL,updated_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS casework_events(case_id TEXT NOT NULL,revision BIGINT NOT NULL,author TEXT NOT NULL,recorded_at TEXT NOT NULL,summary TEXT NOT NULL,payload_zstd BLOB NOT NULL,PRIMARY KEY(case_id,revision));
CREATE TABLE IF NOT EXISTS casework_sources(case_id TEXT NOT NULL,portal_key TEXT NOT NULL,document_id BIGINT NOT NULL,source_sha256 TEXT NOT NULL,PRIMARY KEY(case_id,portal_key,document_id));
CREATE TABLE IF NOT EXISTS casework_origins(case_id TEXT PRIMARY KEY,base_revision BIGINT NOT NULL,base_sha256 TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS casework_imports(case_id TEXT NOT NULL,revision BIGINT NOT NULL,packet_sha256 TEXT NOT NULL,payload_zstd BLOB NOT NULL,PRIMARY KEY(case_id,revision));
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
    value.split_whitespace().collect::<Vec<_>>().join(" ")
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
/// Recompute source status on every read and mutation; persisted verification is not trusted.
fn refresh_evidence(
    conn: &Connection,
    evidence: &mut [CaseworkCitation],
    portable: bool,
) -> Result<Vec<String>, AppError> {
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
            let public:Option<bool>=conn.query_row("SELECT lower(trim(d.visibility))='public' AND p.lifecycle_state<>'REMOVED' FROM documents d JOIN portals p ON p.subdomain=d.portal_subdomain WHERE d.id=?1 AND d.portal_subdomain=?2",params![id,portal],|r|r.get(0)).optional().map_err(storage)?;
            if public == Some(false) {
                return Err(invalid(
                    "A case citation refers to a private or removed local record.",
                ));
            }
        }
        if let Some(report) = read_report(conn, &citation.report_id)? {
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
            let mut canonical = canonical_citation(&report, source, &citation.quote);
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
            let (unavailable, historical): (bool, bool) = conn.query_row(
                "SELECT COALESCE(bool_or(d.id IS NULL OR p.subdomain IS NULL OR lower(trim(d.visibility))<>'public' OR p.lifecycle_state='REMOVED'),false), COALESCE(bool_or(COALESCE(d.raw_file_sha256,'')<>s.source_sha256),false) FROM research_report_sources s LEFT JOIN documents d ON d.id=s.document_id AND d.portal_subdomain=s.portal_key LEFT JOIN portals p ON p.subdomain=s.portal_key WHERE s.report_id=?1",
                params![citation.report_id], |r| Ok((r.get(0)?,r.get(1)?)),
            ).map_err(storage)?;
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
                let public:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM documents d JOIN portals p ON p.subdomain=d.portal_subdomain WHERE d.id=?1 AND d.portal_subdomain=?2 AND lower(trim(d.visibility))='public' AND p.lifecycle_state<>'REMOVED')",params![id,portal],|r|r.get(0)).map_err(storage)?;
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
fn follow_up(r: &CaseworkRequest) -> String {
    let omissions = request_omissions(r);
    format!("To: {}\nSubject: Follow-up on {}{}\n\nPlease provide the remaining responsive records or clarify their disposition for the following unresolved items:\n{}\n\nThese records would help determine: {}\nPlease identify any withheld or unavailable portions and provide reasonably segregable responsive material.\n\nDraft for review; no message has been sent.",r.custodian,r.title,if r.external_reference.is_empty(){String::new()}else{format!(" ({})",r.external_reference)},if omissions.is_empty(){"- Confirm that the production covers the requested records, fields and period.".into()}else{omissions.iter().map(|m|format!("- {m}")).collect::<Vec<_>>().join("\n")},r.resolves)
}
fn write_case(
    conn: &Connection,
    case: &mut CaseworkCase,
    author: &str,
    summary: &str,
) -> Result<(), AppError> {
    case.revision += 1;
    case.updated_at = now();
    let blob = encode(case)?;
    conn.execute("INSERT INTO casework_cases VALUES (?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(id) DO UPDATE SET disposition=excluded.disposition,revision=excluded.revision,payload_zstd=excluded.payload_zstd,updated_at=excluded.updated_at",params![case.id,case.report_id,case.topic,case.disposition,case.revision,blob,case.updated_at]).map_err(storage)?;
    conn.execute(
        "INSERT INTO casework_events VALUES (?1,?2,?3,?4,?5,?6)",
        params![
            case.id,
            case.revision,
            author,
            case.updated_at,
            summary,
            blob
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
fn packet_preview(
    conn: &Connection,
    payload: &str,
) -> Result<(CaseworkPacket, CaseworkImportPreview), AppError> {
    if payload.len() > MAX_PACKET {
        return Err(invalid("Portable packet exceeds 2 MB."));
    }
    let mut packet: CaseworkPacket =
        serde_json::from_str(payload).map_err(|e| invalid(format!("Invalid case packet: {e}")))?;
    if packet.schema_version != 1 || packet.history.len() > 1000 {
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
    let preview = CaseworkImportPreview {
        mode: mode.into(),
        local_revision: local.as_ref().map(|c| c.revision),
        packet_sha256: sha256_hex(payload.as_bytes()),
        warnings,
        case: packet.case.clone(),
        contributor: packet.contributor.clone(),
    };
    Ok((packet, preview))
}
impl DuckDbRepository {
    pub async fn initialize_casework(&self) -> Result<(), AppError> {
        self.query_connection("Initialize durable casework", |conn| {
            conn.execute_batch(SCHEMA).map_err(storage)
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
            let packet = CaseworkPacket {
                schema_version: 1,
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
    ) -> Result<CaseworkCase, AppError> {
        author_valid(&author)?;
        self.query_connection("Apply reviewed case import",move|conn|{let(packet,preview)=packet_preview(conn,&payload)?;if preview.packet_sha256!=packet_sha256||preview.local_revision!=expected_revision{return Err(invalid("The packet or local case changed after preview. Preview it again."));}
if preview.mode=="CONFLICT"{return Err(invalid("This import conflicts with local work. It was not applied; compare the versions and merge selected changes into the local case."));}
if preview.mode=="IDENTICAL"{return read_case(conn,&packet.case.id)?.ok_or_else(||invalid("Case disappeared."));}let mut case=packet.case.clone();case.revision=preview.local_revision.unwrap_or(0);let tx=conn.transaction().map_err(storage)?;write_case(&tx,&mut case,&author,&format!("Imported packet {} from claimed contributor {}; {} supplied history entries are unverified.",preview.packet_sha256,packet.contributor,packet.history.len()))?;tx.execute("INSERT INTO casework_origins VALUES (?1,?2,?3) ON CONFLICT DO NOTHING",params![case.id,packet.base_revision,packet.base_sha256]).map_err(storage)?;tx.execute("INSERT INTO casework_imports VALUES (?1,?2,?3,?4)",params![case.id,case.revision,preview.packet_sha256,encode(&packet)?]).map_err(storage)?;tx.commit().map_err(storage)?;Ok(case)}).await
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
                "Local editor".into()
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
        let conn = remote.analytics_connection().await.unwrap();
        conn.execute("UPDATE documents SET visibility='private' WHERE id=1", [])
            .unwrap();
        assert!(remote.preview_casework_import(original).await.is_err());
        purge_casework_references(&conn, "city", Some(1)).unwrap();
        assert!(read_case(&conn,&imported.id).unwrap().is_none(),"portable cases with an exact local source identity participate in purge without becoming verified quotations");
    }
    #[tokio::test]
    async fn portable_history_is_scoped_to_the_case_and_never_silently_truncated() {
        let db = setup(true).await;
        let case = db.create_casework_case("report-1".into()).await.unwrap();
        let conn = db.analytics_connection().await.unwrap();
        conn.execute("INSERT INTO casework_events SELECT 'other-case',range,'Other desk','2099-01-01','Unrelated activity',?1 FROM range(1,1002)", params![encode(&case).unwrap()]).unwrap();
        let packet: CaseworkPacket = serde_json::from_str(
            &db.export_casework_packet(case.id.clone(), "Editor".into())
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(packet.history.len(), 1);
        assert_eq!(packet.history[0].author, "Research workspace");
        conn.execute("INSERT INTO casework_events SELECT ?1,range,'Investigator','2026-09-27','Case activity',?2 FROM range(2,1002)", params![case.id,encode(&case).unwrap()]).unwrap();
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
