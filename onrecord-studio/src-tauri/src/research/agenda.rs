//! Deterministic agenda passages: a meeting heading and a whole relevant item,
//! never a date borrowed from approved minutes or a truncated lexical window.
use crate::{
    error::AppError,
    integrity::{is_sha256_hex, sha256_hex},
    storage::{pii::mask_pii, sqlite::DuckDbRepository},
    types::{ResearchCitation, ResearchEvidence, ResearchMeeting},
};
use chrono::NaiveDate;
use regex::Regex;
use serde::Serialize;
use std::sync::LazyLock;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgendaCandidate {
    pub body: String,
    pub date: String,
    pub agenda_item: String,
    pub title: String,
    pub source_url: String,
    pub source_sha256: String,
    pub date_citation: ResearchCitation,
    pub item_citation: ResearchCitation,
    pub upcoming: bool,
}

#[derive(Default)]
pub struct AgendaEvidence {
    pub evidence: Vec<ResearchEvidence>,
    pub candidates: Vec<AgendaCandidate>,
    pub limitations: Vec<String>,
}

/// A source-bound follow-through suggestion, not an inferred vote or model finding.
/// Callers must disclose when this deterministic fallback supplies an omitted action.
pub fn fallback_actions(agendas: &AgendaEvidence, today: NaiveDate) -> Vec<ResearchMeeting> {
    agendas.candidates.iter().filter_map(|candidate| {
        let date = NaiveDate::parse_from_str(&candidate.date,"%Y-%m-%d").ok()?;
        let amendment = ["amendment","agreement","contract","grant"].iter().any(|word|candidate.title.to_lowercase().contains(word));
        let comparison = if amendment {
            "Compare the prior and proposed funding, service scope, deliverables, reporting requirements and execution conditions; identify changes that need explanation."
        } else {
            "Read the complete staff report and attachments; compare the proposed use of funds and service commitments with the cited historical record, keeping the fiscal periods separate."
        };
        let action = if date >= today {
            format!("Review {} and its linked staff report and attachments for the {} meeting. {comparison} Prepare questions grounded in the documents and check the current official participation instructions. After the meeting, preserve the recorded vote and any executed agreement or amendment.",candidate.agenda_item,candidate.date)
        } else {
            format!("Follow up on {} from {} by locating the published minutes, recorded vote and executed agreement or amendment, if any. {comparison} Check whether the final action changed the proposal; do not assume the listed action was approved.",candidate.agenda_item,candidate.date)
        };
        Some(ResearchMeeting {
            body:candidate.body.clone(),date:candidate.date.clone(),agenda_item:candidate.agenda_item.clone(),
            relevance:format!("The official agenda lists: {}. This supports a specific follow-through question; it does not establish adoption, payment or execution.",candidate.title),
            action,citations:vec![candidate.date_citation.clone(),candidate.item_citation.clone()],
        })
    }).take(3).collect()
}

static AGENDA_IDENTITY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\bcouncil\s+agenda\b").unwrap());
static CITY_IDENTITY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\bcity\s+of\s+san\s+diego\b").unwrap());
pub(crate) static ITEMS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?P<label>ITEM\s*[-–]?\s*(?P<number>[0-9]{1,5}[A-Z]?))\s*:").unwrap()
});
static HEADER_DATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?P<weekday>Monday|Tuesday|Wednesday|Thursday|Friday|Saturday|Sunday),?\s+(?P<month>January|February|March|April|May|June|July|August|September|October|November|December)\s+(?P<day>[0-9]{1,2}),\s+(?P<year>20[0-9]{2})\b").unwrap()
});

/// Plain https on a City of San Diego host or one of its two agenda publishers.
pub(crate) fn official_url(url: &str) -> bool {
    reqwest::Url::parse(url).ok().is_some_and(|u| {
        u.scheme() == "https"
            && u.username().is_empty()
            && u.password().is_none()
            && u.port().is_none()
            && u.host_str().is_some_and(|host| {
                host == "sandiego.gov"
                    || host.ends_with(".sandiego.gov")
                    || matches!(host, "sandiego.hylandcloud.com" | "sandiego.granicus.com")
            })
    })
}

fn topic_score(text: &str, topic: &str) -> usize {
    let terms: &[&str] = match topic {
        "homelessness" => &["homeless", "shelter", "encampment", "housing", "riverbed"],
        "abatement" => &["encampment", "abatement", "riverbed", "outreach", "impound"],
        "bathrooms" => &[
            "restroom",
            "bathroom",
            "toilet",
            "shower",
            "sanitation",
            "comfort station",
            "handwash",
        ],
        "police" => &["police", "overtime", "law enforcement", "surveillance"],
        "waste" => &[
            "audit",
            "overpayment",
            "reimbursement",
            "contract",
            "amendment",
            "expenditure",
        ],
        _ => return 0,
    };
    let lower = text.to_lowercase();
    terms.iter().filter(|term| lower.contains(**term)).count()
}

fn dated_heading(text: &str, first_item: usize) -> Option<(NaiveDate, std::ops::Range<usize>)> {
    let header = &text[..first_item];
    if header.chars().count() > 12_000
        || !AGENDA_IDENTITY.is_match(header)
        || !CITY_IDENTITY.is_match(header)
    {
        return None;
    }
    match heading_date(header) {
        HeadingDate::Recorded(date, range) => Some((date, range)),
        HeadingDate::Conflicting | HeadingDate::Absent => None,
    }
}

/// A meeting date read from a heading. Every weekday/full-date phrase in the heading must name
/// the same, internally consistent date; anything else is `Conflicting`, never a best guess.
pub(crate) enum HeadingDate {
    Recorded(NaiveDate, std::ops::Range<usize>),
    Conflicting,
    Absent,
}

pub(crate) fn heading_date(header: &str) -> HeadingDate {
    const MONTHS: [&str; 12] = [
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
    ];
    let mut found: Option<(NaiveDate, std::ops::Range<usize>)> = None;
    for captures in HEADER_DATE.captures_iter(header) {
        let date = MONTHS
            .iter()
            .position(|m| captures["month"].eq_ignore_ascii_case(m))
            .and_then(|month| {
                NaiveDate::from_ymd_opt(
                    captures["year"].parse().ok()?,
                    month as u32 + 1,
                    captures["day"].parse().ok()?,
                )
            });
        let Some(date) = date.filter(|date| {
            date.format("%A")
                .to_string()
                .eq_ignore_ascii_case(&captures["weekday"])
        }) else {
            return HeadingDate::Conflicting;
        };
        if found.as_ref().is_some_and(|(prior, _)| *prior != date) {
            return HeadingDate::Conflicting;
        }
        if found.is_none() {
            found = Some((date, captures.get(0).expect("whole match").range()));
        }
    }
    found.map_or(HeadingDate::Absent, |(date, range)| {
        HeadingDate::Recorded(date, range)
    })
}

fn passage(
    source: &ResearchEvidence,
    text: &str,
    range: std::ops::Range<usize>,
    label: &str,
) -> Result<ResearchEvidence, AppError> {
    let offset = source.character_offset.unwrap_or(0) + text[..range.start].chars().count();
    let masked = mask_pii(&text[range])?.masked_text;
    let mut result = source.clone();
    result.id = format!(
        "doc:{}:{}:{offset}:{}",
        source.portal_key.as_deref().unwrap_or_default(),
        source.document_id.unwrap_or_default(),
        &sha256_hex(masked.as_bytes())[..10]
    );
    result.text = masked;
    result.character_offset = Some(offset);
    result.label = format!("{} — {label}", source.label);
    result.limitations = "Exact retained City Council agenda passage. A listed proposed action is not an adopted decision. Item text retains page headings and original extraction order; publication changes, current participation instructions and subsequent votes must be checked at the source.".into();
    Ok(result)
}

fn extract(
    source: &ResearchEvidence,
    topic: &str,
    today: NaiveDate,
) -> Result<AgendaEvidence, AppError> {
    let mut result = AgendaEvidence::default();
    let Some(url) = source.source_url.as_deref().filter(|url| official_url(url)) else {
        return Ok(result);
    };
    if !is_sha256_hex(&source.source_sha256)
        || source.document_id.is_none()
        || source.portal_key.is_none()
    {
        return Ok(result);
    }
    let text = &source.text;
    let items = ITEMS.captures_iter(text).collect::<Vec<_>>();
    let Some(first) = items.first().and_then(|c| c.get(0)) else {
        return Ok(result);
    };
    let Some((date, date_range)) = dated_heading(text, first.start()) else {
        return Ok(result);
    };
    let heading = passage(source, text, date_range, "City Council agenda meeting date")?;
    let date_citation = ResearchCitation {
        evidence_id: heading.id.clone(),
        quote: heading.text.clone(),
    };
    let mut relevant = Vec::new();
    // A following explicit item heading proves the end. Do not silently call a
    // truncated source tail or appended participation instructions a complete item.
    for pair in items.windows(2) {
        let first = pair[0].get(0).unwrap();
        let next = pair[1].get(0).unwrap();
        let range = first.start()..next.start();
        let body = &text[range.clone()];
        let score = topic_score(body, topic);
        if score == 0 {
            continue;
        }
        if body.chars().count() > 12_000 {
            result.limitations.push(format!(
                "{} item {} exceeds the complete-item context bound; inspect its original source.",
                source.label, &pair[0]["number"]
            ));
            continue;
        }
        let evidence = passage(source, text, range, "City Council agenda complete item")?;
        let title_end = ["Council District", "Proposed Actions"]
            .iter()
            .filter_map(|marker| text[first.end()..next.start()].find(marker))
            .min()
            .unwrap_or(next.start() - first.end());
        let title = text[first.end()..first.end() + title_end]
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let candidate = AgendaCandidate {
            body: "San Diego City Council".into(),
            date: date.format("%Y-%m-%d").to_string(),
            agenda_item: pair[0]["label"].trim().to_string(),
            title: title.chars().take(600).collect(),
            source_url: url.to_string(),
            source_sha256: source.source_sha256.clone(),
            date_citation: date_citation.clone(),
            item_citation: ResearchCitation {
                evidence_id: evidence.id.clone(),
                quote: evidence.text.clone(),
            },
            upcoming: date >= today,
        };
        relevant.push((score, evidence, candidate));
    }
    relevant.sort_by_key(|item| std::cmp::Reverse(item.0));
    for (_, evidence, candidate) in relevant.into_iter().take(6) {
        result.evidence.push(evidence);
        result.candidates.push(candidate);
    }
    if !result.candidates.is_empty() {
        result.evidence.push(heading);
    }
    Ok(result)
}

/// This retrieval bypasses lexical excerpt ranking for meeting structure, while
/// retaining the same public visibility, lifecycle and immutable source identity.
pub async fn evidence(
    repository: &DuckDbRepository,
    topic: &str,
) -> Result<AgendaEvidence, AppError> {
    let topic = topic.to_string();
    repository.query_connection("Retrieve exact Council agenda items",move |conn| {
        let _deadline = crate::analysis::sql::QueryDeadline::start(conn,std::time::Duration::from_secs(15))?;
        let mut statement = conn.prepare("SELECT d.portal_subdomain,d.request_id,d.id,COALESCE(d.file_name,'Agenda'),d.download_url,COALESCE(d.raw_file_sha256,''),substr(f.extracted_text,1,500000) FROM documents d JOIN documents_fts f ON f.document_id=d.id AND f.portal_subdomain=d.portal_subdomain JOIN portals p ON p.subdomain=d.portal_subdomain WHERE lower(trim(d.visibility))='public' AND p.lifecycle_state<>'REMOVED' AND d.extraction_status IN ('SEARCHABLE','SALVAGED') AND regexp_matches(substr(f.extracted_text,1,12000),'council\\s+agenda','i') ORDER BY d.updated_at DESC,d.id LIMIT 24").map_err(|e|AppError::Storage(e.to_string()))?;
        let rows = statement.query_map([],|row|Ok(ResearchEvidence {
            id: String::new(), portal_key: Some(row.get(0)?), request_id: Some(row.get(1)?), document_id:Some(row.get(2)?),label:row.get(3)?,source_url:row.get(4)?,source_sha256:row.get(5)?,text:row.get(6)?,character_offset:Some(0),sql:None,snapshot_provenance:vec![],source_references:vec![],limitations:String::new(),
        })).map_err(|e|AppError::Storage(e.to_string()))?.collect::<Result<Vec<_>,_>>().map_err(|e|AppError::Storage(e.to_string()))?;
        let today = chrono::Local::now().date_naive();
        let mut result = AgendaEvidence::default();
        for source in rows {
            let found = extract(&source,&topic,today)?;
            result.evidence.extend(found.evidence); result.candidates.extend(found.candidates); result.limitations.extend(found.limitations);
        }
        result.candidates.sort_by(|a,b|b.upcoming.cmp(&a.upcoming).then_with(||if a.upcoming {a.date.cmp(&b.date)} else {b.date.cmp(&a.date)}));
        let mut seen = std::collections::HashSet::new();
        result.candidates.retain(|candidate| seen.insert((candidate.source_sha256.clone(),candidate.date.clone(),candidate.agenda_item.to_lowercase())));
        result.candidates.truncate(6);
        let ids = result.candidates.iter().flat_map(|c|[c.date_citation.evidence_id.as_str(),c.item_citation.evidence_id.as_str()]).collect::<std::collections::HashSet<_>>();
        result.evidence.retain(|e|ids.contains(e.id.as_str()));
        result.limitations.push("Deterministic agenda lookup covers at most 24 retained City Council agendas and 500,000 characters per document. It requires an unambiguous weekday/full-date heading and explicit item boundaries; other bodies and unrecognized layouts are not covered. No meeting deadline, vote or adoption is inferred.".into());
        Ok(result)
    }).await
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source() -> ResearchEvidence {
        ResearchEvidence {
            id: String::new(),
            label: "9-28-26-special.pdf".into(),
            source_url: Some(
                "https://www.sandiego.gov/sites/default/files/2026-09/9-28-26-special.pdf".into(),
            ),
            portal_key: Some("city".into()),
            request_id: Some(1),
            document_id: Some(2),
            character_offset: Some(0),
            source_sha256: "2fcf7ff0011c1047bf58cc50120ab00d970b07a135982b63a67dbe0a5433dc9f"
                .into(),
            text: include_str!("../../tests/fixtures/council-agenda-2026-09-28.txt").into(),
            sql: None,
            snapshot_provenance: vec![],
            source_references: vec![],
            limitations: String::new(),
        }
    }
    #[tokio::test]
    async fn retained_agenda_query_and_fallback_pass_existing_citation_validation() {
        let temp = tempfile::tempdir().unwrap();
        let repository = DuckDbRepository::new(temp.path().join("core.db")).unwrap();
        repository
            .run_migrations(include_str!("../../migrations/0001_initial_schema.sql"))
            .await
            .unwrap();
        let record = source();
        let conn = repository.analytics_connection().await.unwrap();
        conn.execute("INSERT INTO portals(subdomain,display_name,base_url) VALUES ('city','City','https://www.sandiego.gov')",[]).unwrap();
        conn.execute("INSERT INTO requests(id,portal_subdomain,title,status,created_at,updated_at) VALUES (1,'city','Council agenda','closed','2026-09-27','2026-09-27')",[]).unwrap();
        conn.execute("INSERT INTO documents(id,request_id,portal_subdomain,file_name,download_url,visibility,created_at,updated_at,raw_file_sha256,extraction_status) VALUES (2,1,'city','9-28-26-special.pdf',?1,'public','2026-09-27','2026-09-27',?2,'SEARCHABLE')",duckdb::params![record.source_url,record.source_sha256]).unwrap();
        conn.execute("INSERT INTO documents_fts(document_id,portal_subdomain,file_name,extracted_text) VALUES (2,'city','9-28-26-special.pdf',?1)",[record.text]).unwrap();
        drop(conn);
        let found = evidence(&repository, "homelessness").await.unwrap();
        let meetings = fallback_actions(&found, NaiveDate::from_ymd_opt(2026, 9, 27).unwrap());
        assert!(meetings
            .iter()
            .any(|meeting| meeting.agenda_item == "Item 605" && meeting.date == "2026-09-28"));
        assert!(meetings.len() <= 3);
        let report = crate::types::ResearchSynthesis {
            title: "Published agenda follow-through".into(),
            summary: "Exact agenda items, without inferred outcomes".into(),
            findings: vec![],
            questions: vec![],
            requests: vec![],
            meetings,
            limitations: vec![],
        };
        super::super::evidence::validate_synthesis(&report, &found.evidence).unwrap();
        let past = fallback_actions(&found, NaiveDate::from_ymd_opt(2026, 9, 29).unwrap());
        assert!(past
            .iter()
            .all(|meeting| meeting.action.contains("published minutes")
                && !meeting.action.contains("participation instructions")));
    }
    #[test]
    fn actual_agenda_preserves_complete_riverbed_item_and_heading_date() {
        let source = source();
        let found = extract(
            &source,
            "homelessness",
            NaiveDate::from_ymd_opt(2026, 9, 27).unwrap(),
        )
        .unwrap();
        let item = found
            .candidates
            .iter()
            .find(|c| c.agenda_item == "Item 605")
            .unwrap();
        assert_eq!(item.date, "2026-09-28");
        assert!(item.upcoming);
        assert!(
            item.item_citation.quote.contains("$3,666,572")
                && item.item_citation.quote.contains("June 14, 2027")
        );
        assert!(!item.item_citation.quote.contains("Item 606"));
        assert!(item.title.contains("Riverbed"));
        for e in &found.evidence {
            let original = source
                .text
                .chars()
                .skip(e.character_offset.unwrap())
                .take(e.text.chars().count())
                .collect::<String>();
            assert_eq!(
                original, e.text,
                "Agenda passages must preserve the retained character offsets"
            );
        }
        let historical = extract(
            &source,
            "abatement",
            NaiveDate::from_ymd_opt(2026, 9, 29).unwrap(),
        )
        .unwrap();
        assert!(historical.candidates.iter().all(|c| !c.upcoming));
    }
    #[test]
    fn actual_native_retained_item_does_not_stop_at_a_page_header() {
        let retained: ResearchEvidence = serde_json::from_str(include_str!(
            "../../tests/fixtures/council-agenda-native-item-span-2026-09-28.json"
        ))
        .unwrap();
        let mut source = source();
        source.text = format!(
            "CITY OF SAN DIEGO COUNCIL AGENDA MONDAY, SEPTEMBER 28, 2026. {}",
            retained.text
        );
        let found = extract(
            &source,
            "abatement",
            NaiveDate::from_ymd_opt(2026, 9, 27).unwrap(),
        )
        .unwrap();
        let item = found
            .candidates
            .iter()
            .find(|c| c.agenda_item == "Item 605")
            .unwrap();
        assert!(item.item_citation.quote.contains("Page 8 Proposed Actions"));
        assert!(
            item.item_citation.quote.contains("$3,666,572")
                && item.item_citation.quote.contains("June 14, 2027")
        );
        let evidence = found
            .evidence
            .iter()
            .find(|e| e.id == item.item_citation.evidence_id)
            .unwrap();
        assert_eq!(
            source
                .text
                .chars()
                .skip(evidence.character_offset.unwrap())
                .take(evidence.text.chars().count())
                .collect::<String>(),
            evidence.text
        );
    }
    #[test]
    fn rejects_minutes_dates_ambiguous_headings_and_unofficial_agenda_copies() {
        let mut record = source();
        record.text = record
            .text
            .replace("MONDAY, SEPTEMBER 28, 2026", "MEETING DATE OMITTED")
            .replace("Monday, September 28, 2026", "MEETING DATE OMITTED");
        assert!(extract(
            &record,
            "homelessness",
            NaiveDate::from_ymd_opt(2026, 9, 27).unwrap()
        )
        .unwrap()
        .candidates
        .is_empty());
        record = source();
        record.source_url = Some("https://example.com/agenda.pdf".into());
        assert!(extract(
            &record,
            "homelessness",
            NaiveDate::from_ymd_opt(2026, 9, 27).unwrap()
        )
        .unwrap()
        .candidates
        .is_empty());
        record = source();
        record.text = record.text.replacen(
            "COUNCIL AGENDA",
            "COUNCIL AGENDA Tuesday, September 29, 2026",
            1,
        );
        assert!(extract(
            &record,
            "homelessness",
            NaiveDate::from_ymd_opt(2026, 9, 27).unwrap()
        )
        .unwrap()
        .candidates
        .is_empty());
        record = source();
        record.text = record.text.replace("COUNCIL AGENDA", "BUDGET REPORT");
        assert!(extract(
            &record,
            "homelessness",
            NaiveDate::from_ymd_opt(2026, 9, 27).unwrap()
        )
        .unwrap()
        .candidates
        .is_empty());
    }
}
