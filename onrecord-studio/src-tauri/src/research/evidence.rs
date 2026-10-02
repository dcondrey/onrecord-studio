use crate::{error::AppError, integrity::sha256_hex, storage::sqlite::DuckDbRepository, types::*};
use serde_json::{json, Value};
use std::collections::HashSet;

fn err(e: impl std::fmt::Display) -> AppError {
    AppError::Storage(e.to_string())
}

pub fn add_evidence(evidence: &mut Vec<ResearchEvidence>, incoming: Vec<ResearchEvidence>) {
    let mut seen = evidence
        .iter()
        .map(|e| e.id.clone())
        .collect::<HashSet<_>>();
    let mut size: usize = evidence.iter().map(|e| e.text.len()).sum();
    for item in incoming {
        if evidence.len() >= 320 {
            break;
        }
        if size + item.text.len() > 960_000 {
            continue;
        }
        if !item.text.is_empty() && seen.insert(item.id.clone()) {
            size += item.text.len();
            evidence.push(item);
        }
    }
}

/// Keep the retained evidence pool separate from each bounded model context. Fresh tests
/// must not disappear simply because initial retrieval filled the old 64-item context.
/// A critic always receives every source cited by the draft before newer material.
pub fn model_evidence<'a>(
    evidence: &'a [ResearchEvidence],
    pinned: &[String],
) -> Vec<&'a ResearchEvidence> {
    let mut selected = Vec::new();
    let mut seen = HashSet::new();
    let mut bytes: usize = 0;
    // Include every available cited source, even if it exceeds the usual context budget.
    // bounded_context must then preserve it or fail explicitly, never review a draft
    // against silently missing support or counterevidence.
    for item in evidence.iter().filter(|e| pinned.contains(&e.id)) {
        if seen.insert(item.id.as_str()) {
            bytes = bytes.saturating_add(
                serde_json::to_vec(item)
                    .map(|v| v.len())
                    .unwrap_or(usize::MAX),
            );
            selected.push(item);
        }
    }
    let budget = 135_000usize.max(bytes.saturating_add(32_000)).min(210_000);
    let mut push = |item: &'a ResearchEvidence| {
        let size = serde_json::to_vec(item)
            .map(|v| v.len())
            .unwrap_or(usize::MAX);
        if bytes.saturating_add(size) <= budget && seen.insert(item.id.as_str()) {
            bytes += size;
            selected.push(item);
        }
    };
    for item in evidence.iter().rev().take(24) {
        push(item);
    }
    // Then retain sources from the wider case, not only the latest single question.
    for item in evidence {
        push(item);
    }
    selected
}

pub fn schema_context(schema: &AnalyticalSchema) -> Value {
    Value::Array(schema.datasets.iter().take(40).map(|d| json!({
        "snapshot":d.snapshot_id,"table":d.table_name,"name":d.display_name,"url":d.source_uri,
        "sha256":d.source_sha256,"importedRows":d.imported_rows,"rejectedRows":d.rejected_rows,
        "partial":d.partial_dataset,"period":[d.observation_period_start,d.observation_period_end],
        "retrieved":d.retrieved_at,
        "columns":schema.columns.iter().filter(|c| c.table_name == d.table_name).take(100).map(|c| json!([c.column_name,c.logical_type])).collect::<Vec<_>>()
    })).collect())
}

pub async fn query_evidence(
    repository: &DuckDbRepository,
    question: &str,
    sql: &str,
    assumptions: &str,
) -> Result<ResearchEvidence, AppError> {
    if sql.len() > 16_000 {
        return Err(AppError::DataParsing("Research SQL exceeds 16 KB".into()));
    }
    let result = repository.execute_restricted_sql(sql, 40).await?;
    // A dictionary once plus rows of values avoids repeating every field name for every row.
    let text = serde_json::to_string(&crate::storage::pii::mask_json_pii(
        &json!({"columns":result.columns,"rows":result.rows,"truncated":result.truncated}),
    )?)
    .map_err(err)?;
    if text.len() > 24_000 {
        return Err(AppError::DataParsing(
            "Query result is too large for research context. Aggregate or select fewer columns."
                .into(),
        ));
    }
    let source_references = repository
        .analytical_schema()
        .await?
        .datasets
        .into_iter()
        .filter(|dataset| {
            result
                .provenance
                .contains(&format!("{}@{}", dataset.table_name, dataset.snapshot_id))
        })
        .filter_map(|dataset| {
            Some(ResearchSourceReference {
                url: dataset.source_uri?,
                snapshot: dataset.snapshot_id,
                sha256: dataset.source_sha256,
            })
        })
        .collect();
    let digest = sha256_hex(format!("{sql}\n{text}\n{:?}", result.provenance).as_bytes());
    Ok(ResearchEvidence {
        id: format!("sql:{}", &digest[..16]), label: question.to_string(), source_url: None,
        portal_key: None, request_id: None, document_id: None, character_offset: None,
        source_sha256: digest, text, sql: Some(sql.to_string()),
        snapshot_provenance: result.provenance.clone(),
        source_references,
        limitations: format!("Read-only analytical query; {} result rows; truncated: {}. Assumptions to verify: {}. Snapshot provenance: {}", result.row_count, result.truncated, assumptions, result.provenance.join("; ")),
    })
}

fn normalized(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}
pub(crate) fn validate_citations(
    citations: &[ResearchCitation],
    evidence: &[ResearchEvidence],
) -> Result<(), AppError> {
    for cite in citations {
        let item = evidence
            .iter()
            .find(|e| e.id == cite.evidence_id)
            .ok_or_else(|| {
                AppError::DataParsing(
                    "Research cited evidence outside its retrieved context".into(),
                )
            })?;
        let quote = normalized(&cite.quote);
        let exact_row = item.sql.is_some()
            && serde_json::from_str::<Value>(&cite.quote)
                .ok()
                .filter(|v| v.is_array())
                .is_some_and(|quoted| {
                    serde_json::from_str::<Value>(&item.text)
                        .ok()
                        .is_some_and(|result| {
                            result["rows"]
                                .as_array()
                                .is_some_and(|rows| rows.contains(&quoted))
                        })
                });
        if quote.len() < 12
            || (if item.sql.is_some() {
                !exact_row
            } else {
                !normalized(&item.text).contains(&quote)
            })
        {
            return Err(AppError::DataParsing(format!(
                "Research quote did not match passage {}. Unverified quote preview: {:?}",
                cite.evidence_id,
                cite.quote.chars().take(180).collect::<String>()
            )));
        }
    }
    Ok(())
}

/// An overlapping excerpt from the same immutable original can contain the exact quote.
/// Correct the passage pointer only; never change quoted text or borrow another source.
pub fn align_citation_passages(
    report: &mut ResearchSynthesis,
    evidence: &[ResearchEvidence],
) -> Vec<String> {
    let mut notes = Vec::new();
    let citations = report
        .findings
        .iter_mut()
        .flat_map(|finding| {
            finding
                .citations
                .iter_mut()
                .chain(finding.counter_evidence.iter_mut())
        })
        .chain(
            report
                .meetings
                .iter_mut()
                .flat_map(|meeting| meeting.citations.iter_mut()),
        );
    for cite in citations {
        if validate_citations(std::slice::from_ref(cite), evidence).is_ok() {
            continue;
        }
        let Some(original) = evidence.iter().find(|item| item.id == cite.evidence_id) else {
            continue;
        };
        if original.sql.is_some()
            || original.document_id.is_none()
            || original.portal_key.is_none()
            || original.source_sha256.is_empty()
        {
            continue;
        }
        let quote = normalized(&cite.quote);
        if quote.len() < 12 {
            continue;
        }
        if let Some(target) = evidence.iter().find(|item| {
            item.sql.is_none()
                && item.document_id == original.document_id
                && item.portal_key == original.portal_key
                && item.source_sha256 == original.source_sha256
                && normalized(&item.text).contains(&quote)
        }) {
            notes.push(format!("Corrected overlapping excerpt reference {} to {} within the same original document and hash; quoted text unchanged.",cite.evidence_id,target.id));
            cite.evidence_id = target.id.clone();
        }
    }
    notes
}

pub fn bound_synthesis(report: &mut ResearchSynthesis) {
    let before = (
        report.findings.len(),
        report.questions.len(),
        report.requests.len(),
        report.meetings.len(),
    );
    report.findings.truncate(8);
    report.questions.truncate(8);
    report.requests.truncate(6);
    report.meetings.truncate(6);
    let after = (
        report.findings.len(),
        report.questions.len(),
        report.requests.len(),
        report.meetings.len(),
    );
    if before != after {
        report.limitations.push("The provider returned more items than the case limit. Only the first eight findings/questions and six requests/meetings are retained; retained citations still undergo full validation.".into());
    }
}

fn validate_finding(
    finding: &ResearchFinding,
    evidence: &[ResearchEvidence],
) -> Result<(), AppError> {
    if finding.citations.is_empty()
        || finding.title.trim().is_empty()
        || finding.interpretation.trim().is_empty()
        || finding.next_test.trim().is_empty()
        || finding.alternative_explanations.is_empty()
        || finding
            .alternative_explanations
            .iter()
            .any(|s| s.trim().is_empty())
    {
        return Err(AppError::DataParsing(
            "A finding needs citations, an interpretation, a competing explanation, and a falsifiable next test".into(),
        ));
    }
    validate_citations(&finding.citations, evidence)?;
    validate_citations(&finding.counter_evidence, evidence)?;
    // Relative effective-year qualifiers change whether a service change has happened.
    // Do not accept an interpretation that silently drops a quoted before/after/until.
    static CONDITION: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?i)\b(after|before|until)\s+(?:FY\s*|fiscal\s+year\s*)(20\d{2})\b")
            .expect("fiscal effective-date qualifier")
    });
    let interpreted = CONDITION
        .captures_iter(&finding.interpretation)
        .map(|m| format!("{}:{}", m[1].to_lowercase(), &m[2]))
        .collect::<HashSet<_>>();
    for cite in &finding.citations {
        for qualifier in CONDITION.captures_iter(&cite.quote) {
            let key = format!("{}:{}", qualifier[1].to_lowercase(), &qualifier[2]);
            if !interpreted.contains(&key) {
                return Err(AppError::DataParsing(format!("Interpretation must preserve the cited effective-date condition '{}' and explicitly resolve any conflicting period, rather than treating a future change as current.",&qualifier[0])));
            }
        }
    }
    Ok(())
}

fn validate_followup(ids: &[String], evidence: &[ResearchEvidence]) -> Result<(), AppError> {
    if ids.is_empty() || ids.iter().any(|id| !evidence.iter().any(|e| &e.id == id)) {
        return Err(AppError::DataParsing(
            "Research follow-up lacks a traceable evidence basis".into(),
        ));
    }
    Ok(())
}

fn same_record(a: &ResearchEvidence, b: &ResearchEvidence) -> bool {
    a.id == b.id
        || (!a.source_sha256.is_empty()
            && a.source_sha256 == b.source_sha256
            && a.source_url.is_some()
            && a.source_url == b.source_url)
}

fn official_agenda_source(item: &ResearchEvidence, evidence: &[ResearchEvidence]) -> bool {
    let official = item
        .source_url
        .as_ref()
        .and_then(|s| reqwest::Url::parse(s).ok())
        .is_some_and(|url| {
            url.scheme() == "https"
                && url.username().is_empty()
                && url.password().is_none()
                && url.host_str().is_some_and(|host| {
                    host == "sandiego.gov"
                        || host.ends_with(".sandiego.gov")
                        || host == "sandiego.hylandcloud.com"
                        || host == "sandiego.granicus.com"
                })
        });
    official
        && evidence.iter().filter(|e| same_record(item, e)).any(|e| {
            let context = format!("{} {}", e.label, e.text).to_lowercase();
            context.contains("agenda") || context.contains("meeting")
        })
}

fn item_markers(text: &str) -> Vec<(String, String)> {
    static ITEM: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?i)\bitem\s*[-:–—]?\s*([0-9]{1,5}(?:\.[0-9]+)?[a-z]?)\b").unwrap()
    });
    ITEM.captures_iter(text)
        .map(|c| (c[1].to_ascii_lowercase(), c[0].to_string()))
        .collect()
}

fn item_number(label: &str) -> Option<String> {
    let markers = item_markers(label);
    if markers.len() == 1 {
        return Some(markers[0].0.clone());
    }
    let label = label.trim();
    if !label.is_empty() && label.len() <= 5 && label.chars().all(|c| c.is_ascii_digit()) {
        return Some(label.to_string());
    }
    None
}

fn label_in_quote(label: &str, quote: &str) -> bool {
    let normalized_label = normalized(label).to_lowercase();
    if normalized_label.is_empty() {
        return false;
    }
    // A bare item number cannot be satisfied by an amount, year, or another item number.
    if normalized_label.chars().all(|c| c.is_ascii_digit()) {
        return item_markers(quote)
            .iter()
            .any(|(number, _)| number == &normalized_label);
    }
    // Match the whole label at a word boundary, not merely as a substring, so "Item 4" does not
    // match inside "Item 40" (or "Subitem 4").
    let normalized_quote = normalized(quote).to_lowercase();
    normalized_quote
        .match_indices(normalized_label.as_str())
        .any(|(i, _)| {
            let before_ok = normalized_quote[..i]
                .chars()
                .next_back()
                .is_none_or(|c| !c.is_alphanumeric());
            let after_ok = normalized_quote[i + normalized_label.len()..]
                .chars()
                .next()
                .is_none_or(|c| !c.is_alphanumeric());
            before_ok && after_ok
        })
}

fn date_in_quote(date: chrono::NaiveDate, quote: &str) -> bool {
    let quote = normalized(quote).to_lowercase();
    [
        "%Y-%m-%d",
        "%B %-d, %Y",
        "%b %-d, %Y",
        "%m/%d/%Y",
        "%-m/%-d/%Y",
    ]
    .iter()
    .any(|format| quote.contains(&date.format(format).to_string().to_lowercase()))
}

fn validate_meeting(
    meeting: &ResearchMeeting,
    evidence: &[ResearchEvidence],
) -> Result<(), AppError> {
    validate_citations(&meeting.citations, evidence)?;
    let date = chrono::NaiveDate::parse_from_str(&meeting.date, "%Y-%m-%d").map_err(|_| {
        AppError::DataParsing("Meeting date must be established by a dated agenda".into())
    })?;
    let grounded = meeting.citations.iter().any(|item_cite| {
        let Some(item) = evidence.iter().find(|e| e.id == item_cite.evidence_id) else {
            return false;
        };
        official_agenda_source(item, evidence)
            && label_in_quote(&meeting.agenda_item, &item_cite.quote)
            && meeting.citations.iter().any(|date_cite| {
                evidence
                    .iter()
                    .any(|e| e.id == date_cite.evidence_id && same_record(item, e))
                    && date_in_quote(date, &date_cite.quote)
            })
    });
    if !grounded {
        return Err(AppError::DataParsing("Meeting requires an exact item label and full date quoted from the same official agenda.".into()));
    }
    Ok(())
}

/// Feed machine-verifiable failures into the existing editorial pass, without an extra model call.
/// These diagnostics contain positions and reasons, never untrusted source instructions.
pub fn validation_issues(report: &ResearchSynthesis, evidence: &[ResearchEvidence]) -> Vec<String> {
    let mut issues = Vec::new();
    for (i, finding) in report.findings.iter().enumerate() {
        if let Err(error) = validate_finding(finding, evidence) {
            issues.push(format!("findings[{i}]: {error}"));
        }
    }
    for (kind, ids) in [
        (
            "questions",
            report
                .questions
                .iter()
                .map(|q| &q.evidence_ids)
                .collect::<Vec<_>>(),
        ),
        (
            "requests",
            report
                .requests
                .iter()
                .map(|r| &r.evidence_ids)
                .collect::<Vec<_>>(),
        ),
    ] {
        for (i, ids) in ids.into_iter().enumerate() {
            if let Err(error) = validate_followup(ids, evidence) {
                issues.push(format!("{kind}[{i}]: {error}"));
            }
        }
    }
    for (i, meeting) in report.meetings.iter().enumerate() {
        if let Err(error) = validate_meeting(meeting, evidence) {
            issues.push(format!("meetings[{i}]: {error}"));
        }
    }
    issues
}

/// A rejected component must not erase independent supported work or leak via the overview.
/// Repairs are limited to copying the exact agenda marker for the SAME explicit item number;
/// quotes, dates, source identities, and interpretations are never guessed or rewritten.
pub fn reconcile_synthesis(
    report: &mut ResearchSynthesis,
    evidence: &[ResearchEvidence],
) -> Vec<String> {
    let mut notes = align_citation_passages(report, evidence);
    for (i, meeting) in report.meetings.iter_mut().enumerate() {
        if validate_meeting(meeting, evidence).is_ok() {
            continue;
        }
        let Some(number) = item_number(&meeting.agenda_item) else {
            continue;
        };
        let marker = meeting
            .citations
            .iter()
            .flat_map(|cite| item_markers(&cite.quote))
            .find(|(candidate, _)| candidate == &number)
            .map(|(_, marker)| marker);
        if let Some(marker) = marker {
            let mut corrected = meeting.clone();
            corrected.agenda_item = marker;
            if validate_meeting(&corrected, evidence).is_ok() {
                *meeting = corrected;
                notes.push(format!("meetings[{i}]: normalized the explicit item number to its exact quoted agenda label."));
            }
        }
    }
    let rejected = validation_issues(report, evidence);
    if !rejected.is_empty() {
        report
            .findings
            .retain(|f| validate_finding(f, evidence).is_ok());
        report
            .questions
            .retain(|q| validate_followup(&q.evidence_ids, evidence).is_ok());
        report
            .requests
            .retain(|r| validate_followup(&r.evidence_ids, evidence).is_ok());
        report
            .meetings
            .retain(|m| validate_meeting(m, evidence).is_ok());
        // The model's overview could repeat a removed claim. Replace it, rather than presenting
        // an unsupported headline above otherwise correctly quarantined components.
        report.title = "San Diego investigation: validated evidence and follow-up".into();
        report.summary = if report.findings.is_empty() {
            "Source-validated follow-up questions and actions remain below. Unsupported components were withheld; no finding is established by this report.".into()
        } else {
            format!("Retained investigative leads: {}. Unsupported components were withheld; review each lead's evidence, alternative explanations, and next test.", report.findings.iter().take(3).map(|f| f.title.as_str()).collect::<Vec<_>>().join("; "))
        };
        report.limitations.push(format!("Source validation withheld {} unsupported components. Their claims are not published in this dossier. Exact quotations establish source fidelity; interpretations still require researcher review.", rejected.len()));
        notes.extend(
            rejected
                .into_iter()
                .map(|issue| format!("Withheld {issue}")),
        );
    }
    notes
}

pub fn validate_synthesis(
    report: &ResearchSynthesis,
    evidence: &[ResearchEvidence],
) -> Result<(), AppError> {
    if report.findings.len() > 8
        || report.requests.len() > 6
        || report.meetings.len() > 6
        || report.questions.len() > 8
    {
        return Err(AppError::DataParsing(
            "Research exceeded its findings budget".into(),
        ));
    }
    if let Some(issue) = validation_issues(report, evidence).first() {
        return Err(AppError::DataParsing(issue.clone()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn evidence() -> ResearchEvidence {
        ResearchEvidence {
            id: "e1".into(),
            label: "Agenda".into(),
            source_url: Some("https://www.sandiego.gov/agenda.pdf".into()),
            portal_key: None,
            request_id: None,
            document_id: None,
            character_offset: None,
            source_sha256: "hash".into(),
            text:
                "Council meeting September 30, 2026. Item 4: Shelter contract cost is $2 million."
                    .into(),
            sql: None,
            snapshot_provenance: vec![],
            source_references: vec![],
            limitations: String::new(),
        }
    }
    fn report() -> ResearchSynthesis {
        ResearchSynthesis {
            title: "Review".into(),
            summary: String::new(),
            findings: vec![],
            questions: vec![],
            requests: vec![],
            meetings: vec![],
            limitations: vec![],
        }
    }
    #[test]
    fn overlong_provider_output_is_bounded_without_bypassing_citation_validation() {
        let mut synthesis = report();
        synthesis.questions = (0..10)
            .map(|i| ResearchQuestion {
                question: format!("Question {i}"),
                why: "A traceable gap".into(),
                evidence_ids: vec!["e1".into()],
            })
            .collect();
        bound_synthesis(&mut synthesis);
        assert_eq!(synthesis.questions.len(), 8);
        assert!(synthesis.limitations[0].contains("case limit"));
        assert!(validate_synthesis(&synthesis, &[evidence()]).is_ok());
        synthesis.questions[0].evidence_ids = vec!["invented".into()];
        assert!(validate_synthesis(&synthesis, &[evidence()]).is_err());
    }

    #[test]
    fn label_in_quote_rejects_item_number_prefix_collision() {
        assert!(!label_in_quote(
            "Item 4",
            "Council approved Item 40 unanimously."
        ));
        assert!(label_in_quote(
            "Item 4",
            "Council approved Item 4 unanimously."
        ));
        assert!(!label_in_quote(
            "Item 4: Shelter",
            "Item 4: Library funding was approved."
        ));
    }

    #[test]
    fn retrieves_later_substantive_passages_with_original_unicode_offsets() {
        let text = format!("Homelessness introduction {} Homelessness contract amendment adopted: $2 million; outcomes and placement expenditure recommendation. {} Homelessness later finding", "é".repeat(4000), "x".repeat(4000));
        let passages = crate::research::retrieval::ranked_passages(&text, "homelessness");
        assert_eq!(passages.len(), 2);
        assert!(passages[0].0 > 3000);
        assert!(passages[0].1.contains("$2 million"));
        for (offset, passage) in passages {
            assert_eq!(
                text.chars()
                    .skip(offset)
                    .take(passage.chars().count())
                    .collect::<String>(),
                passage
            );
        }
    }

    #[test]
    fn late_disproof_evidence_enters_model_context_without_losing_pinned_sources() {
        let mut pool = Vec::new();
        for index in 0..70 {
            let mut item = evidence();
            item.id = format!("source-{index}");
            item.text = "Retained exact source passage. ".repeat(80);
            add_evidence(&mut pool, vec![item]);
        }
        let mut disproof = evidence();
        disproof.id = "new-disproof".into();
        disproof.text =
            "The earlier difference was an accounting transfer, not unspent funding.".into();
        add_evidence(&mut pool, vec![disproof]);
        assert_eq!(
            pool.len(),
            71,
            "The former 64-item ceiling blocked later investigation"
        );
        let selected = model_evidence(&pool, &["source-0".into()]);
        assert!(selected.iter().any(|e| e.id == "new-disproof"));
        assert!(selected.iter().any(|e| e.id == "source-0"));
        assert!(serde_json::to_vec(&selected).unwrap().len() < 136_000);
        assert_eq!(pool[0].text, "Retained exact source passage. ".repeat(80));
    }

    #[test]
    fn rejects_invented_quotes_and_unknown_sources() {
        let e = vec![evidence()];
        assert!(validate_citations(
            &[ResearchCitation {
                evidence_id: "e1".into(),
                quote: "Shelter contract cost is $2 million.".into()
            }],
            &e
        )
        .is_ok());
        assert!(validate_citations(
            &[ResearchCitation {
                evidence_id: "e1".into(),
                quote: "Shelter contract cost is $20 million.".into()
            }],
            &e
        )
        .is_err());
        assert!(validate_citations(
            &[ResearchCitation {
                evidence_id: "unknown".into(),
                quote: e[0].text.clone()
            }],
            &e
        )
        .is_err());
    }
    #[test]
    fn quote_alignment_preserves_text_and_cannot_cross_original_identity() {
        let mut first = evidence();
        first.portal_key = Some("city".into());
        first.document_id = Some(1);
        first.source_sha256 = "a".repeat(64);
        let mut later = first.clone();
        later.id = "later".into();
        later.character_offset = Some(4000);
        later.text = "The later passage reports $507,141 in overbilling credits.".into();
        let mut other = later.clone();
        other.id = "other".into();
        other.document_id = Some(2);
        let quote = later.text.clone();
        let mut synthesis = report();
        synthesis.findings = vec![finding(&quote)];
        assert!(align_citation_passages(&mut synthesis, &[first.clone(), other]).is_empty());
        assert_eq!(synthesis.findings[0].citations[0].evidence_id, "e1");
        let mut changed = later.clone();
        changed.source_sha256 = "b".repeat(64);
        assert!(align_citation_passages(&mut synthesis, &[first.clone(), changed]).is_empty());
        let notes = align_citation_passages(&mut synthesis, &[first, later]);
        assert_eq!(notes.len(), 1);
        assert_eq!(synthesis.findings[0].citations[0].evidence_id, "later");
        assert_eq!(synthesis.findings[0].citations[0].quote, quote);
    }
    #[test]
    fn current_service_claim_cannot_drop_a_quoted_future_effective_year() {
        let archived: crate::types::ResearchReport = serde_json::from_str(include_str!(
            "../../../docs/validation/investigation-depth-first-attempt-2026-09-27.json"
        ))
        .unwrap();
        let mut finding = archived
            .synthesis
            .findings
            .iter()
            .find(|f| f.title.contains("Day Center"))
            .unwrap()
            .clone();
        assert!(validate_finding(&finding, &archived.evidence)
            .unwrap_err()
            .to_string()
            .contains("effective-date condition"));
        finding.interpretation="The IBA describes continuation after FY2027, while planning text and proposed reductions concern different periods. The actual transition date and present service capacity remain unresolved.".into();
        assert!(validate_finding(&finding, &archived.evidence).is_ok());
    }
    #[test]
    fn sql_citations_allow_json_spacing_but_not_changed_values_or_types() {
        let mut e = evidence();
        e.sql = Some("SELECT actual,adopted".into());
        e.text = r#"{"columns":["actual","adopted"],"rows":[["50818708","40162474"]]}"#.into();
        let cite = |quote: &str| ResearchCitation {
            evidence_id: "e1".into(),
            quote: quote.into(),
        };
        assert!(validate_citations(&[cite(r#"["50818708", "40162474"]"#)], &[e.clone()]).is_ok());
        assert!(validate_citations(&[cite(r#"["50818709", "40162474"]"#)], &[e.clone()]).is_err());
        assert!(validate_citations(&[cite("[50818708, 40162474]")], &[e]).is_err());
    }
    #[test]
    fn meeting_requires_the_actual_date_in_official_source() {
        let e = vec![evidence()];
        let mut report = report();
        report.meetings.push(ResearchMeeting {
            body: "Council".into(),
            date: "2026-09-30".into(),
            agenda_item: "4".into(),
            relevance: "Shelter".into(),
            action: "Read contract".into(),
            citations: vec![ResearchCitation {
                evidence_id: "e1".into(),
                quote: e[0].text.clone(),
            }],
        });
        assert!(validate_synthesis(&report, &e).is_ok());
        report.meetings[0].agenda_item = "Item 99: Invented discussion".into();
        assert!(validate_synthesis(&report, &e).is_err());
        report.meetings[0].agenda_item = "4".into();
        report.meetings[0].date = "2027-09-30".into();
        assert!(validate_synthesis(&report, &e).is_err());
        report.meetings[0].date = "2026-09-30".into();
        let mut fake = e;
        fake[0].source_url = Some("https://example.test/agenda".into());
        assert!(validate_synthesis(&report, &fake).is_err());
    }
    fn finding(quote: &str) -> ResearchFinding {
        ResearchFinding {
            title: "Check shelter contract performance".into(),
            interpretation: "The agenda identifies a $2 million shelter contract; delivered services still need verification.".into(),
            why_it_matters: "Connect authority to actual deliverables".into(),
            citations: vec![ResearchCitation { evidence_id: "e1".into(), quote: quote.into() }],
            counter_evidence: vec![],
            alternative_explanations: vec!["The award may differ from actual spending".into()],
            next_test: "Obtain the executed contract and payment ledger".into(),
        }
    }

    #[test]
    fn independent_findings_survive_bad_output_without_leaking_it_in_overview() {
        let mut report = report();
        report.title = "Invented scandal".into();
        report.summary = "Unverified $20 million scandal".into();
        report.findings = vec![
            finding("Shelter contract cost is $2 million."),
            finding("Shelter contract cost is $20 million."),
        ];
        report.questions.push(ResearchQuestion {
            question: "Is the contract executed?".into(),
            why: "Agenda authority differs from expenditure".into(),
            evidence_ids: vec!["e1".into()],
        });
        report.requests.push(ResearchRequest {
            title: "Invented request".into(),
            custodian: "City".into(),
            gap: "Unknown source".into(),
            resolves: "Unknown".into(),
            draft: "Existing ledger".into(),
            evidence_ids: vec!["made-up".into()],
        });
        let evidence = vec![evidence()];
        assert_eq!(validation_issues(&report, &evidence).len(), 2);
        let notes = reconcile_synthesis(&mut report, &evidence);
        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.questions.len(), 1);
        assert!(report.requests.is_empty());
        assert_eq!(notes.len(), 2);
        assert!(!report.title.contains("scandal"));
        assert!(!report.summary.contains("scandal"));
        assert!(report.summary.contains("shelter contract performance"));
        assert!(report.limitations[0].contains("2 unsupported components"));
        assert!(validate_synthesis(&report, &evidence).is_ok());
    }

    #[test]
    fn agenda_header_and_item_can_be_separate_excerpts_but_never_separate_records() {
        let mut header = evidence();
        header.text = "City Council Special Meeting Agenda Monday, September 28, 2026.".into();
        let mut item = header.clone();
        item.id = "e2".into();
        item.label = "9-28-26-special.pdf".into();
        item.text = "ITEM-605: First Amendment to the San Diego Riverbed Encampment Resolution Project. Authorize $3,666,572.".into();
        let mut report = report();
        report.meetings.push(ResearchMeeting {
            body: "Council".into(),
            date: "2026-09-28".into(),
            agenda_item: "Item 605: Riverbed grant extension".into(),
            relevance: "Grant authority and deliverables".into(),
            action: "Review amendment before comment".into(),
            citations: vec![
                ResearchCitation {
                    evidence_id: "e1".into(),
                    quote: header.text.clone(),
                },
                ResearchCitation {
                    evidence_id: "e2".into(),
                    quote: item.text.clone(),
                },
            ],
        });
        let mut records = vec![header, item];
        assert!(validate_synthesis(&report, &records).is_err());
        let notes = reconcile_synthesis(&mut report, &records);
        assert_eq!(report.meetings.len(), 1);
        assert_eq!(report.meetings[0].agenda_item, "ITEM-605");
        assert_eq!(notes.len(), 1);
        assert!(validate_synthesis(&report, &records).is_ok());
        records[1].source_url = Some("https://www.sandiego.gov/a-different-agenda.pdf".into());
        assert!(
            validate_synthesis(&report, &records).is_err(),
            "A date in another agenda cannot date this item"
        );
        records[1].source_url = records[0].source_url.clone();
        records[1].source_sha256 = "different-publication-at-same-url".into();
        assert!(
            validate_synthesis(&report, &records).is_err(),
            "Reused URLs cannot connect different content versions"
        );
    }

    #[test]
    fn agenda_normalization_never_guesses_a_different_item_or_date() {
        let e = evidence();
        let mut report = report();
        report.meetings.push(ResearchMeeting {
            body: "Council".into(),
            date: "2026-09-30".into(),
            agenda_item: "Item 40: Shelter".into(),
            relevance: "Shelter".into(),
            action: "Review".into(),
            citations: vec![ResearchCitation {
                evidence_id: "e1".into(),
                quote: e.text.clone(),
            }],
        });
        reconcile_synthesis(&mut report, &[e]);
        assert!(report.meetings.is_empty());
        assert!(!label_in_quote("4", "ITEM-40: Shelter contract $4 million"));
        assert!(!label_in_quote("4", "Contract expenditure: $4 million"));
    }

    #[test]
    fn sql_citation_requires_the_entire_row_even_when_a_fragment_occurs_verbatim() {
        let mut e = evidence();
        e.sql = Some("SELECT department,actual,adopted".into());
        e.text = r#"{"columns":["department","actual","adopted"],"rows":[["Police","50818708","40162474"]]}"#.into();
        let partial = ResearchCitation {
            evidence_id: "e1".into(),
            quote: "\"50818708\",\"40162474\"".into(),
        };
        assert!(validate_citations(&[partial], &[e]).is_err());
    }
}
