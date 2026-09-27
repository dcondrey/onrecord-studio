//! Reproducible editorial checks and time-aware, explicitly reviewed public relationships.
use crate::{
    error::AppError,
    integrity::sha256_hex,
    research::evidence::validate_citations,
    types::{review::*, *},
};
use chrono::NaiveDate;
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};

fn bad(message: &str) -> AppError {
    AppError::DataParsing(message.into())
}
fn text(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max
}
fn normalize(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

pub fn validate_review(
    input: &FindingReviewInput,
    report: &ResearchReport,
) -> Result<(), AppError> {
    if input.report_id != report.id
        || input.finding_index >= report.synthesis.findings.len()
        || !text(&input.reviewer, 120)
        || input.assigned_to.len() > 120
        || !text(&input.reasoning, 8000)
        || !text(&input.falsification_test, 4000)
        || !text(&input.counter_search, 4000)
        || input.counter_evidence.len() > 12
        || input
            .counter_evidence
            .iter()
            .any(|c| c.quote.len() > 24000 || c.evidence_id.len() > 300)
        || !matches!(
            input.verdict.as_str(),
            "OPEN" | "SUPPORTED" | "CONTESTED" | "REJECTED"
        )
    {
        return Err(bad("Provide a reviewer, reasoning, a falsification test and the counterevidence search performed for an existing finding."));
    }
    validate_citations(&input.counter_evidence, &report.evidence)
}

pub fn challenges(report: &ResearchReport, replays: &[EvidenceReplay]) -> Vec<FindingChallenge> {
    report.synthesis.findings.iter().enumerate().map(|(index, finding)| {
        let mut blockers = Vec::new();
        if finding.citations.is_empty() { blockers.push("No supporting citation.".into()); }
        if let Err(e) = validate_citations(&finding.citations, &report.evidence) { blockers.push(e.to_string()); }
        if let Err(e) = validate_citations(&finding.counter_evidence, &report.evidence) { blockers.push(e.to_string()); }
        let mut sources = BTreeSet::new();
        let mut checks = Vec::new();
        for cite in &finding.citations {
            if let Some(evidence) = report.evidence.iter().find(|e| e.id == cite.evidence_id) {
                if let Some(url) = &evidence.source_url { sources.insert(url.clone()); }
                for source in &evidence.source_references { sources.insert(source.url.clone()); }
                if evidence.sql.is_some() {
                    if !sql_result_complete(evidence) {
                        blockers.push(format!("{}: saved SQL result is truncated or completeness is unknown. Re-run a complete, narrower comparison before supporting the conclusion.", evidence.label));
                    }
                    match replays.iter().find(|r| r.evidence_id == evidence.id) {
                        Some(r) if r.state == "MATCH" => checks.push(format!("Recomputed {}: complete result rows match.", evidence.label)),
                        Some(r) => blockers.push(format!("{}: {} — {}", evidence.label, r.state, r.detail)),
                        None => blockers.push(format!("Recompute the saved query: {}.", evidence.label)),
                    }
                } else if let Some(r) = replays.iter().find(|r| r.evidence_id == evidence.id) {
                    if r.state != "MATCH" { blockers.push(format!("{}: {}", evidence.label, r.detail)); }
                } else { blockers.push(format!("Verify retained source identity: {}.", evidence.label)); }
            }
        }
        if finding.alternative_explanations.is_empty() { blockers.push("No alternative explanation recorded.".into()); }
        if finding.next_test.trim().is_empty() { blockers.push("No falsifiable next test recorded.".into()); }
        if finding.counter_evidence.is_empty() { checks.push("No counterevidence cited. Record the search performed; absence here does not establish absence in the public record.".into()); }
        checks.push(format!("{} distinct source URLs; syndicated or dependent sources may still share one origin.", sources.len()));
        checks.push("Citation matching checks the quotation, not whether the interpretation is justified. An accountable reviewer must assess the claim.".into());
        FindingChallenge { finding_index: index, title: finding.title.clone(), independent_sources: sources.len(), checks, blockers, next_test: finding.next_test.clone() }
    }).collect()
}

pub fn sql_result_complete(evidence: &ResearchEvidence) -> bool {
    let parsed = serde_json::from_str::<serde_json::Value>(&evidence.text).ok();
    if let Some(truncated) = parsed.as_ref().and_then(|v| v.get("truncated")) {
        return truncated.as_bool() == Some(false);
    }
    // Upgrade compatibility: earlier native packets recorded this field in their fixed prefix.
    // Text in the analyst's assumptions cannot override the prefix.
    evidence
        .limitations
        .starts_with("Read-only analytical query; ")
        && evidence
            .limitations
            .split(". Assumptions to verify:")
            .next()
            .is_some_and(|prefix| prefix.ends_with("truncated: false"))
}

pub fn validate_relationship(
    value: &PublicRelationship,
    report: &ResearchReport,
) -> Result<(), AppError> {
    if value.report_id != report.id
        || value.id.len() > 100
        || value.citations.is_empty()
        || value.citations.len() > 12
        || value
            .citations
            .iter()
            .any(|c| c.quote.len() > 24000 || c.evidence_id.len() > 300)
        || !text(&value.note, 4000)
        || !text(&value.reviewer, 120)
        || !matches!(
            value.status.as_str(),
            "CANDIDATE" | "CONFIRMED" | "REJECTED"
        )
        || !matches!(
            value.predicate.as_str(),
            "HOLDS_PUBLIC_ROLE"
                | "AWARDED_CONTRACT"
                | "VENDOR_FOR"
                | "DISCLOSED_INTEREST"
                | "ALIAS_OF"
                | "FUNDS"
                | "DELIVERS"
        )
    {
        return Err(bad("A public relationship needs evidence, a reviewer, an explanation and a supported relationship type."));
    }
    for entity in [&value.subject, &value.object] {
        if !text(&entity.key, 200)
            || !text(&entity.name, 200)
            || !text(&entity.jurisdiction, 200)
            || !matches!(
                entity.kind.as_str(),
                "PUBLIC_OFFICIAL"
                    | "PUBLIC_ROLE"
                    | "AGENCY"
                    | "VENDOR"
                    | "CONTRACT"
                    | "PROGRAM"
                    | "DISCLOSURE"
            )
        {
            return Err(bad("Use a stable public record identifier and a public entity type; a shared name alone is not an identity."));
        }
    }
    if value.subject.key == value.object.key {
        return Err(bad(
            "A relationship must identify two distinct public entities.",
        ));
    }
    for date in [&value.valid_from, &value.valid_to].into_iter().flatten() {
        if date.len() != 10 || NaiveDate::parse_from_str(date, "%Y-%m-%d").is_err() {
            return Err(bad("Effective dates must be YYYY-MM-DD, or unknown."));
        }
    }
    if let (Some(start), Some(end)) = (&value.valid_from, &value.valid_to) {
        if end < start {
            return Err(bad("Effective end cannot precede start."));
        }
    }
    validate_citations(&value.citations, &report.evidence)?;
    let quoted = normalize(
        &value
            .citations
            .iter()
            .map(|c| c.quote.as_str())
            .collect::<Vec<_>>()
            .join(" "),
    );
    if !quoted.contains(&normalize(&value.subject.name))
        || !quoted.contains(&normalize(&value.object.name))
    {
        return Err(bad("Both entity names must occur in the cited passages. Use the source spelling and preserve identity uncertainty."));
    }
    // Dates are assertions too. A date absent from the cited passages cannot be silently supplied.
    for date in [&value.valid_from, &value.valid_to].into_iter().flatten() {
        let parsed = NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap();
        if ![
            date.clone(),
            parsed.format("%B %-d, %Y").to_string().to_lowercase(),
            parsed.format("%-m/%-d/%Y").to_string(),
        ]
        .iter()
        .any(|d| quoted.contains(d))
        {
            return Err(bad(
                "Effective dates must appear in the citations. Leave unknown tenure unfilled.",
            ));
        }
    }
    Ok(())
}

/// Proposals use only explicit labelled fields. They remain candidates and are not identity merges.
pub fn relationship_candidates(report: &ResearchReport) -> Vec<PublicRelationship> {
    let vendor = Regex::new(r"(?im)^\s*(?:vendor|contractor)\s*:\s*([^\r\n;|]{3,100})").unwrap();
    let contract = Regex::new(r"(?im)^\s*(?:contract|agreement)(?:\s+(?:id|number|no\.?))?\s*[:#]\s*([a-zA-Z0-9][a-zA-Z0-9._/-]{3,70})").unwrap();
    let mut result = Vec::new();
    for evidence in &report.evidence {
        if evidence.sql.is_some() {
            continue;
        }
        if let (Some(v), Some(c)) = (
            vendor.captures(&evidence.text),
            contract.captures(&evidence.text),
        ) {
            let vendor_name = v[1].trim().to_string();
            let contract_name = c[1].to_string();
            let id = format!(
                "candidate-{}",
                &sha256_hex(format!("{}:{}:{}", report.id, evidence.id, vendor_name).as_bytes())
                    [..24]
            );
            result.push(PublicRelationship { id, report_id: report.id.clone(), revision: 0,
                subject: PublicEntityRef { key: format!("unresolved:{}:vendor", evidence.id), name: vendor_name, kind: "VENDOR".into(), jurisdiction: "Identity and jurisdiction require review".into() },
                predicate: "AWARDED_CONTRACT".into(), object: PublicEntityRef { key: format!("unresolved:{}:contract", evidence.id), name: contract_name, kind: "CONTRACT".into(), jurisdiction: "Identity and jurisdiction require review".into() },
                valid_from: None, valid_to: None, citations: vec![ResearchCitation { evidence_id: evidence.id.clone(), quote: evidence.text.clone() }], status: "CANDIDATE".into(), reviewer: "labelled-field extraction".into(), note: "Vendor and contract labels occur in the same excerpt. Verify the relationship, issuing agency and stable identifiers before confirmation.".into() });
        }
    }
    result.truncate(24);
    result
}

pub fn connections(relationships: &[PublicRelationship]) -> (Vec<PublicConnection>, Vec<String>) {
    let confirmed: Vec<_> = relationships
        .iter()
        .filter(|r| r.status == "CONFIRMED")
        .collect();
    let mut identities: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut definitions: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for r in &confirmed {
        for e in [&r.subject, &r.object] {
            identities
                .entry(normalize(&e.name))
                .or_default()
                .insert(e.key.clone());
            definitions
                .entry(e.key.clone())
                .or_default()
                .insert(format!(
                    "{}|{}|{}",
                    normalize(&e.name),
                    e.kind,
                    normalize(&e.jurisdiction)
                ));
        }
    }
    let mut warnings = Vec::new();
    for (name, ids) in &identities {
        if ids.len() > 1 {
            warnings.push(format!(
                "'{name}' has {} distinct identifiers; they have not been merged.",
                ids.len()
            ));
        }
    }
    let conflicts: BTreeSet<_> = definitions
        .iter()
        .filter(|(_, values)| values.len() > 1)
        .map(|(k, _)| k.clone())
        .collect();
    for key in &conflicts {
        warnings.push(format!("Identifier '{key}' has conflicting entity definitions. Connections using it are withheld."));
    }
    let mut paths = Vec::new();
    for a in &confirmed {
        for b in &confirmed {
            if a.id == b.id
                || a.object.key != b.subject.key
                || a.subject.key == b.object.key
                || a.report_id == b.report_id
            {
                continue;
            }
            if [&a.subject.key, &a.object.key, &b.object.key]
                .iter()
                .any(|k| conflicts.contains(*k))
            {
                continue;
            }
            let mut limits = vec!["A documented connection is not evidence of improper influence or a conflict of interest.".into()];
            if let (Some(a_end), Some(b_start)) = (&a.valid_to, &b.valid_from) {
                if a_end < b_start {
                    continue;
                }
            }
            if let (Some(b_end), Some(a_start)) = (&b.valid_to, &a.valid_from) {
                if b_end < a_start {
                    continue;
                }
            }
            if [&a.valid_from, &a.valid_to, &b.valid_from, &b.valid_to]
                .iter()
                .any(|d| d.is_none())
            {
                limits.push("At least one effective period is unknown; contemporaneous involvement is not established.".into());
            }
            paths.push(PublicConnection { subject: a.subject.name.clone(), via: a.object.name.clone(), object: b.object.name.clone(), relationship_ids: vec![a.id.clone(),b.id.clone()], report_ids: vec![a.report_id.clone(),b.report_id.clone()], explanation: format!("{} → {} across independently reviewed assertions. Follow the source quotations for both links.", a.predicate, b.predicate), limitations: limits });
            if paths.len() == 100 {
                return (paths, warnings);
            }
        }
    }
    (paths, warnings)
}

pub fn evaluate_report(report: &ResearchReport) -> InvestigationEvaluation {
    let checks = report.synthesis.findings.iter().enumerate().map(|(i,f)| {
        let valid = !f.citations.is_empty() && validate_citations(&f.citations,&report.evidence).is_ok() && validate_citations(&f.counter_evidence,&report.evidence).is_ok() && !f.alternative_explanations.is_empty() && !f.next_test.trim().is_empty();
        EvaluationCheck { name: format!("Finding {}: {}", i+1, f.title), passed: valid, detail: "Exact source quotation / complete SQL row, alternatives, and a next test. This structural check does not measure truth or human usefulness.".into() }
    }).collect();
    InvestigationEvaluation { id: report.id.clone(), title: report.synthesis.title.clone(), checked_at: chrono::Utc::now().to_rfc3339(), checks, assessments: vec![], limitations: vec!["Reference replay validates the retained evidence packet; it does not refetch historical originals or re-run the model.".into(), "Supported discoveries, false leads and usefulness require human assessment. Unassessed values are not zero.".into()] }
}

/// Replay archived executed comparisons with integer decimal arithmetic. This is independent
/// of the model's prose and does not need the released CSV downloads or a paid model call.
pub fn reference_finance_arithmetic() -> Result<Vec<EvaluationCheck>, AppError> {
    let sources: Vec<ResearchEvidence> = serde_json::from_str(include_str!(
        "../../../docs/validation/finance-comparisons.json"
    ))
    .map_err(|error| bad(&error.to_string()))?;
    let mut checks = Vec::new();
    for source in sources {
        let result: serde_json::Value =
            serde_json::from_str(&source.text).map_err(|error| bad(&error.to_string()))?;
        let columns = result["columns"]
            .as_array()
            .ok_or_else(|| bad("Reference columns missing"))?;
        let index = |name: &str| {
            columns
                .iter()
                .position(|column| column.as_str() == Some(name))
        };
        let formula = [
            ("actual_spending", "adopted_budget", "actual_minus_adopted"),
            ("actual_overtime", "adopted_overtime", "overtime_variance"),
            ("current_adopted", "prior_adopted", "allocation_change"),
        ]
        .into_iter()
        .find_map(|(a, b, c)| Some((index(a)?, index(b)?, index(c)?)));
        let Some((a, b, c)) = formula else {
            continue;
        };
        let rows = result["rows"]
            .as_array()
            .ok_or_else(|| bad("Reference rows missing"))?;
        let decimal = |value: &serde_json::Value| match value {
            serde_json::Value::String(value) => super::financial::decimal(value),
            serde_json::Value::Number(value) => super::financial::decimal(&value.to_string()),
            _ => None,
        };
        let passed = !rows.is_empty()
            && rows.iter().all(|row| {
                let calculated = decimal(&row[a])
                    .zip(decimal(&row[b]))
                    .and_then(|(a, b)| a.checked_sub(b));
                match calculated {
                    Some(value) => decimal(&row[c]) == Some(value),
                    None => row[c].is_null(),
                }
            });
        checks.push(EvaluationCheck { name: format!("Arithmetic replay: {}", source.label), passed,
            detail: format!("{} complete saved SQL rows independently recomputed with exact decimal subtraction. Missing operands remain unknown. Original source URLs, hashes and SQL are retained in the reference packet.", rows.len()) });
    }
    Ok(checks)
}

#[cfg(test)]
mod tests {
    use super::*;
    pub fn reference() -> ResearchReport {
        serde_json::from_str(include_str!(
            "../../../docs/validation/live-finance-dossier.json"
        ))
        .unwrap()
    }
    #[test]
    fn archived_finance_findings_retain_exact_rows_and_missing_denominator_limits() {
        let mut r = reference();
        let evaluation = evaluate_report(&r);
        assert_eq!(evaluation.checks.len(), 6);
        assert!(evaluation.checks.iter().all(|c| c.passed));
        r.synthesis.findings[0].citations[0].quote = "[\"invented\",10656234]".into();
        assert!(!evaluate_report(&r).checks[0].passed);
    }

    #[test]
    fn archived_finance_arithmetic_replays_all_saved_comparisons_without_a_model() {
        let checks = reference_finance_arithmetic().unwrap();
        assert_eq!(checks.len(), 4);
        assert!(checks.iter().all(|check| check.passed), "{checks:?}");
    }
    #[test]
    fn source_and_sql_replay_are_required_before_supporting_a_finding() {
        let r = reference();
        assert!(challenges(&r, &[]).iter().all(|c| !c.blockers.is_empty()));
        let replays = r
            .evidence
            .iter()
            .map(|e| EvidenceReplay {
                evidence_id: e.id.clone(),
                checked_at: "today".into(),
                state: "MATCH".into(),
                detail: "test".into(),
            })
            .collect::<Vec<_>>();
        let checked = challenges(&r, &replays);
        assert!(checked[0].blockers.is_empty());
        assert!(checked[1].blockers.iter().any(|b| b.contains("truncated")));
    }
    #[test]
    fn sql_completeness_never_comes_from_matching_prefixes_or_analyst_assumptions() {
        let mut evidence = reference()
            .evidence
            .into_iter()
            .find(|e| e.sql.is_some())
            .unwrap();
        assert!(sql_result_complete(&evidence));
        evidence.limitations = "Read-only analytical query; 40 result rows; truncated: true. Assumptions to verify: truncated: false".into();
        assert!(!sql_result_complete(&evidence));
        evidence.text = r#"{"columns":["amount"],"rows":[["100.25"]],"truncated":true}"#.into();
        assert!(!sql_result_complete(&evidence));
        evidence.text = r#"{"columns":["amount"],"rows":[["100.25"]],"truncated":false}"#.into();
        assert!(sql_result_complete(&evidence));
    }
    #[test]
    fn relationship_paths_require_shared_identifiers_and_compatible_tenure() {
        let entity = |key: &str| PublicEntityRef {
            key: key.into(),
            name: key.into(),
            kind: "AGENCY".into(),
            jurisdiction: "San Diego".into(),
        };
        let edge = |id: &str, a: &str, b: &str| PublicRelationship {
            id: id.into(),
            report_id: id.into(),
            revision: 1,
            subject: entity(a),
            predicate: "FUNDS".into(),
            object: entity(b),
            valid_from: Some("2025-01-01".into()),
            valid_to: Some("2025-12-31".into()),
            citations: vec![],
            status: "CONFIRMED".into(),
            reviewer: "reviewer".into(),
            note: "test".into(),
        };
        let a = edge("one", "city", "program");
        let mut b = edge("two", "program", "vendor");
        assert_eq!(connections(&[a.clone(), b.clone()]).0.len(), 1);
        b.valid_from = Some("2026-01-01".into());
        b.valid_to = None;
        assert!(connections(&[a.clone(), b.clone()]).0.is_empty());
        b.valid_from = None;
        b.subject.key = "other-program".into();
        let (paths, warnings) = connections(&[a, b]);
        assert!(paths.is_empty());
        assert_eq!(warnings.len(), 1);
    }
    #[test]
    fn public_relation_validation_rejects_invented_names_dates_and_private_entity_types() {
        let mut r = reference();
        r.id = "fixture".into();
        r.evidence[0].sql = None;
        r.evidence[0].text="Vendor: Public Housing Services\nContract: CITY-1200\nFrom 2025-01-01 through 2025-12-31".into();
        let mut e = relationship_candidates(&r).remove(0);
        e.reviewer = "reviewer".into();
        validate_relationship(&e, &r).unwrap();
        e.valid_from = Some("2026-01-01".into());
        assert!(validate_relationship(&e, &r).is_err());
        e.valid_from = None;
        e.subject.name = "Invented Company".into();
        assert!(validate_relationship(&e, &r).is_err());
        e.subject.name = "Public Housing Services".into();
        e.subject.kind = "PRIVATE_PERSON".into();
        assert!(validate_relationship(&e, &r).is_err());
    }
}
