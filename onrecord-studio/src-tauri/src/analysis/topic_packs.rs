//! Topic packs: named cue vocabularies that tag documents and requests with an investigation topic,
//! and the stored findings whose subject is exactly one of the tagged records.
//!
//! A tag says the record's own words contain a cue phrase. It is not a finding and is never
//! written to the findings table. A finding joins a topic only when its most specific subject (the
//! document if it names one, otherwise the request) is a tagged record in the same portal; agency,
//! entity, and citation-only links are not followed.

use crate::{
    analysis::{
        finding::{Finding, FindingKind},
        phrase_match::{has_phrase, words, Phrases, Word},
    },
    error::AppError,
    storage::{pii::mask_pii, DuckDbRepository},
};
use duckdb::params;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Characters of extracted text read per document. Longer documents (bulk CSV exports) are matched
/// on this opening only, and say so.
pub const TEXT_PREFIX_CHARACTERS: i64 = 500_000;
const DOCUMENT_BATCH: i64 = 64;
const MAX_DOCUMENTS: usize = 20_000;
const MAX_REQUESTS: i64 = 50_000;
/// Items listed per topic. Totals are reported separately so a capped list is not read as complete.
const MAX_ITEMS_PER_TOPIC: usize = 500;

pub struct TopicPack {
    pub id: &'static str,
    pub title: &'static str,
    pub cues: Phrases,
}

/// Single ambiguous words (sweep, abatement, overtime, notice, restroom) are deliberately absent:
/// street sweeping, weed abatement, fire overtime and building restrooms are not these topics.
pub const TOPIC_PACKS: [TopicPack; 4] = [
    TopicPack {
        id: "abatement",
        title: "Encampment abatement and sweeps",
        cues: &[
            &["encampment"],
            &["encampments"],
            &["homeless", "sweep"],
            &["homeless", "sweeps"],
            &["homeless", "abatement"],
            &["homeless", "camp"],
            &["homeless", "camps"],
            &["unsafe", "camping"],
            &["camping", "ban"],
            &["camping", "ordinance"],
            &["sidewalk", "camping"],
            &["illegal", "lodging"],
            &["tent", "removal"],
        ],
    },
    TopicPack {
        id: "belongings",
        title: "Belongings disposal and notice",
        cues: &[
            &["personal", "belongings"],
            &["personal", "property", "storage"],
            &["impounded", "property"],
            &["impounded", "belongings"],
            &["belongings", "discarded"],
            &["belongings", "were", "discarded"],
            &["discarded", "belongings"],
            &["belongings", "disposed"],
            &["disposed", "of", "belongings"],
            &["bag", "and", "tag"],
            &["bagged", "and", "tagged"],
            &["retrieve", "belongings"],
            &["retrieve", "their", "belongings"],
            &["retrieve", "their", "property"],
            &["cleanup", "notice"],
            &["cleanup", "notices"],
            &["notice", "of", "cleanup"],
        ],
    },
    TopicPack {
        id: "police",
        title: "Police overtime and budget",
        cues: &[
            &["police", "overtime"],
            &["sworn", "overtime"],
            &["police", "department", "overtime"],
            &["sdpd", "overtime"],
            &["law", "enforcement", "overtime"],
            &["police", "budget"],
            &["police", "department", "budget"],
            &["sdpd", "budget"],
            &["police", "spending"],
            &["police", "expenditures"],
            &["police", "department", "expenditures"],
        ],
    },
    TopicPack {
        id: "bathrooms",
        title: "Public restrooms",
        cues: &[
            &["public", "restroom"],
            &["public", "restrooms"],
            &["public", "toilet"],
            &["public", "toilets"],
            &["public", "bathroom"],
            &["public", "bathrooms"],
            &["comfort", "station"],
            &["comfort", "stations"],
            &["portable", "toilet"],
            &["portable", "toilets"],
            &["portable", "restroom"],
            &["portable", "restrooms"],
            &["handwashing", "station"],
            &["handwashing", "stations"],
            &["portland", "loo"],
            &["restroom", "access"],
            &["restroom", "closure"],
            &["restroom", "closures"],
        ],
    },
];

/// What a stored finding rests on, derived from its kind and never chosen by the caller.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TopicFindingBasis {
    /// A counted comparison against a rule or commitment; the only legal-claim basis.
    Counted,
    /// The record itself states the fact the finding reports.
    Stated,
    /// A statistical or pattern signal for review, not a legal claim.
    Pattern,
}

/// Exhaustive on purpose: a new `FindingKind` must be assigned a basis here before it compiles.
pub fn finding_basis(kind: FindingKind) -> TopicFindingBasis {
    use FindingKind as K;
    match kind {
        K::PolicyViolationDetected
        | K::PracticeDepartureCounted
        | K::ContractCeilingExceeded
        | K::PaymentOutsideContractTerm
        | K::AppropriationExceeded
        | K::RestrictedFundOffProgram
        | K::ApprovalThresholdExceeded
        | K::SplitPurchaseProhibitedByPolicy
        | K::EncampmentNoticeShortfall => TopicFindingBasis::Counted,
        // The document states departures from its own rule without a count.
        K::PracticeDepartureStated | K::EncampmentNoticeStatedShortfall => {
            TopicFindingBasis::Stated
        }
        K::HeldBackResponse
        | K::BoilerplateDenial
        | K::JustInTimeCompliance
        | K::ExtensionAbuse
        | K::AbsenceOfRecords
        | K::DifferentialTreatment
        | K::MetadataContradictsContent
        | K::RegeneratedResponse
        | K::EmailThreadGap
        | K::ContradictoryStatement
        | K::NoRecordsContradictedByRelease
        | K::EvasionParaphrasedDenials
        | K::EvasionRoundNumberThresholds
        | K::EvasionRedactionAtLimit
        | K::EvasionLateArrivingRecords
        | K::RedactionCandidateInferred
        | K::CrossCorpusSharedLanguage
        | K::CrossCorpusTemporalShift
        | K::CrossCorpusOrphanTopic
        | K::CrossCorpusReusedDenialGrounds
        | K::ClaimContradictsData
        | K::CorrelatedFindings
        | K::PaymentWithoutContractRecord
        | K::ApprovalRecordAbsent
        | K::SplitPurchasePattern
        | K::DepartmentSpendingYearOverYear
        | K::OvertimeAboveAdoptedLine
        | K::FundActualAboveAdopted
        | K::RedactionInconsistentAcrossReleases
        | K::RollingProduction
        | K::ServiceReportLead => TopicFindingBasis::Pattern,
    }
}

/// How much of a document's extracted text the match saw.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TopicTextCoverage {
    Full,
    /// Only the first `TEXT_PREFIX_CHARACTERS` were read.
    Prefix,
    /// No extracted text is stored; only the file name was read.
    FileNameOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TopicDocumentMatch {
    pub portal_key: String,
    pub document_id: i64,
    pub request_id: i64,
    pub file_name: Option<String>,
    pub matched_cues: Vec<String>,
    pub text_coverage: TopicTextCoverage,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TopicRequestMatch {
    pub portal_key: String,
    pub request_id: i64,
    pub title: Option<String>,
    pub matched_cues: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TopicFindingMatch {
    pub basis: TopicFindingBasis,
    /// The request the matched subject belongs to: the subject's own request, or the tagged
    /// document's request.
    pub anchor_request_id: i64,
    pub finding: Finding,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TopicPackView {
    pub id: String,
    pub title: String,
    pub cues: Vec<String>,
    pub document_total: usize,
    pub documents: Vec<TopicDocumentMatch>,
    pub request_total: usize,
    pub requests: Vec<TopicRequestMatch>,
    pub finding_total: usize,
    /// Ranked as the findings inbox ranks them.
    pub findings: Vec<TopicFindingMatch>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TopicPacksReport {
    pub topics: Vec<TopicPackView>,
    pub documents_scanned: usize,
    pub documents_prefix_only: usize,
    pub documents_file_name_only: usize,
    pub document_limit_reached: bool,
    pub requests_scanned: usize,
    pub request_limit_reached: bool,
    pub text_prefix_characters: i64,
}

fn cue_label(cue: &[&str]) -> String {
    cue.join(" ")
}

/// The distinct cues of `pack` that occur in `words`, in pack order.
pub(crate) fn matched_cues(words: &[Word], pack: &TopicPack) -> Vec<String> {
    pack.cues
        .iter()
        .filter(|cue| has_phrase(words, std::slice::from_ref(*cue)))
        .map(|cue| cue_label(cue))
        .collect()
}

struct DocumentRow {
    portal_key: String,
    document_id: i64,
    request_id: i64,
    file_name: Option<String>,
    text: Option<String>,
    prefix_only: bool,
}

async fn load_document_batch(
    repository: &DuckDbRepository,
    after: Option<(String, i64)>,
) -> Result<Vec<DocumentRow>, AppError> {
    repository
        .query_connection("Topic pack document sweep", move |conn| {
            let (after_portal, after_id) = after.unwrap_or_else(|| (String::new(), i64::MIN));
            let mut statement = conn
                .prepare(
                    "SELECT d.portal_subdomain, d.id, d.request_id, d.file_name,
                            substr(f.extracted_text, 1, ?3),
                            COALESCE(length(f.extracted_text) > ?3, FALSE)
                     FROM documents d
                     LEFT JOIN documents_fts f
                       ON f.document_id = d.id AND f.portal_subdomain = d.portal_subdomain
                     WHERE d.portal_subdomain > ?1
                        OR (d.portal_subdomain = ?1 AND d.id > ?2)
                     ORDER BY d.portal_subdomain, d.id
                     LIMIT ?4",
                )
                .map_err(|error| AppError::Storage(format!("Topic document sweep: {error}")))?;
            let rows = statement
                .query_map(
                    params![
                        after_portal,
                        after_id,
                        TEXT_PREFIX_CHARACTERS,
                        DOCUMENT_BATCH
                    ],
                    |row| {
                        Ok(DocumentRow {
                            portal_key: row.get(0)?,
                            document_id: row.get(1)?,
                            request_id: row.get(2)?,
                            file_name: row.get(3)?,
                            text: row.get(4)?,
                            prefix_only: row.get(5)?,
                        })
                    },
                )
                .map_err(|error| AppError::Storage(format!("Topic document sweep: {error}")))?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|error| AppError::Storage(format!("Topic document row: {error}")))
        })
        .await
}

struct RequestRow {
    portal_key: String,
    request_id: i64,
    title: Option<String>,
    description: Option<String>,
}

async fn load_requests(repository: &DuckDbRepository) -> Result<Vec<RequestRow>, AppError> {
    repository
        .query_connection("Topic pack request sweep", move |conn| {
            let mut statement = conn
                .prepare(
                    "SELECT portal_subdomain, id, title, description
                     FROM requests
                     ORDER BY portal_subdomain, id
                     LIMIT ?1",
                )
                .map_err(|error| AppError::Storage(format!("Topic request sweep: {error}")))?;
            let rows = statement
                .query_map(params![MAX_REQUESTS + 1], |row| {
                    Ok(RequestRow {
                        portal_key: row.get(0)?,
                        request_id: row.get(1)?,
                        title: row.get(2)?,
                        description: row.get(3)?,
                    })
                })
                .map_err(|error| AppError::Storage(format!("Topic request sweep: {error}")))?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|error| AppError::Storage(format!("Topic request row: {error}")))
        })
        .await
}

struct FindingAnchor {
    id: String,
    portal_key: String,
    request_id: Option<i64>,
    document_id: Option<i64>,
}

/// Active findings with a portal and a document or request subject, ranked as `list_findings`.
async fn load_finding_anchors(
    repository: &DuckDbRepository,
) -> Result<Vec<FindingAnchor>, AppError> {
    repository
        .query_connection("Topic pack finding anchors", move |conn| {
            let mut statement = conn
                .prepare(
                    "SELECT id, subject_portal_key, subject_request_id, subject_document_id
                     FROM findings
                     WHERE dismissed_at IS NULL
                       AND subject_portal_key IS NOT NULL
                       AND (subject_document_id IS NOT NULL OR subject_request_id IS NOT NULL)
                     ORDER BY CASE severity
                         WHEN 'CRITICAL' THEN 4
                         WHEN 'SERIOUS' THEN 3
                         WHEN 'NOTABLE' THEN 2
                         WHEN 'INFO' THEN 1
                         ELSE 0 END DESC,
                       COALESCE(confidence_basis_points, -1) DESC,
                       last_seen_at DESC,
                       id",
                )
                .map_err(|error| AppError::Storage(format!("Topic finding anchors: {error}")))?;
            let rows = statement
                .query_map([], |row| {
                    Ok(FindingAnchor {
                        id: row.get(0)?,
                        portal_key: row.get(1)?,
                        request_id: row.get(2)?,
                        document_id: row.get(3)?,
                    })
                })
                .map_err(|error| AppError::Storage(format!("Topic finding anchors: {error}")))?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|error| AppError::Storage(format!("Topic finding anchor row: {error}")))
        })
        .await
}

fn masked(value: Option<String>) -> Result<Option<String>, AppError> {
    value
        .map(|text| mask_pii(&text).map(|result| result.masked_text))
        .transpose()
}

#[derive(Default)]
struct PackAccumulator {
    documents: Vec<TopicDocumentMatch>,
    requests: Vec<TopicRequestMatch>,
    /// (portal, document) -> request
    document_keys: HashMap<(String, i64), i64>,
    request_keys: HashSet<(String, i64)>,
}

/// The request a finding's most specific subject resolves to within this pack's tagged records,
/// or `None` when the subject is not tagged. The document decides when the subject names one.
fn anchor_in_pack(anchor: &FindingAnchor, pack: &PackAccumulator) -> Option<i64> {
    let portal = anchor.portal_key.clone();
    match (anchor.document_id, anchor.request_id) {
        (Some(document_id), _) => pack.document_keys.get(&(portal, document_id)).copied(),
        (None, Some(request_id)) => pack
            .request_keys
            .contains(&(portal, request_id))
            .then_some(request_id),
        (None, None) => None,
    }
}

pub async fn build_topic_packs(
    repository: &DuckDbRepository,
) -> Result<TopicPacksReport, AppError> {
    let mut packs: Vec<PackAccumulator> = TOPIC_PACKS
        .iter()
        .map(|_| PackAccumulator::default())
        .collect();

    let mut documents_scanned = 0_usize;
    let mut documents_prefix_only = 0_usize;
    let mut documents_file_name_only = 0_usize;
    let mut document_limit_reached = false;
    let mut after = None;
    loop {
        let batch = load_document_batch(repository, after.clone()).await?;
        let Some(last) = batch.last() else { break };
        after = Some((last.portal_key.clone(), last.document_id));
        let exhausted = (batch.len() as i64) < DOCUMENT_BATCH;
        for row in batch {
            if documents_scanned >= MAX_DOCUMENTS {
                document_limit_reached = true;
                break;
            }
            documents_scanned += 1;
            let coverage = match (&row.text, row.prefix_only) {
                (None, _) => TopicTextCoverage::FileNameOnly,
                (Some(_), true) => TopicTextCoverage::Prefix,
                (Some(_), false) => TopicTextCoverage::Full,
            };
            match coverage {
                TopicTextCoverage::Prefix => documents_prefix_only += 1,
                TopicTextCoverage::FileNameOnly => documents_file_name_only += 1,
                TopicTextCoverage::Full => {}
            }
            // File name and text are tokenized apart so no phrase spans the seam between them.
            let name_words = words(row.file_name.as_deref().unwrap_or(""));
            let text_words = words(row.text.as_deref().unwrap_or(""));
            let mut masked_name: Option<Option<String>> = None;
            for (pack, accumulator) in TOPIC_PACKS.iter().zip(packs.iter_mut()) {
                let mut cues = matched_cues(&name_words, pack);
                for cue in matched_cues(&text_words, pack) {
                    if !cues.contains(&cue) {
                        cues.push(cue);
                    }
                }
                if cues.is_empty() {
                    continue;
                }
                accumulator
                    .document_keys
                    .insert((row.portal_key.clone(), row.document_id), row.request_id);
                if masked_name.is_none() {
                    masked_name = Some(masked(row.file_name.clone())?);
                }
                accumulator.documents.push(TopicDocumentMatch {
                    portal_key: row.portal_key.clone(),
                    document_id: row.document_id,
                    request_id: row.request_id,
                    file_name: masked_name.clone().flatten(),
                    matched_cues: cues,
                    text_coverage: coverage,
                });
            }
        }
        if exhausted || document_limit_reached {
            break;
        }
    }

    let mut requests = load_requests(repository).await?;
    let request_limit_reached = requests.len() as i64 > MAX_REQUESTS;
    requests.truncate(MAX_REQUESTS as usize);
    let requests_scanned = requests.len();
    for row in requests {
        let title_words = words(row.title.as_deref().unwrap_or(""));
        let description_words = words(row.description.as_deref().unwrap_or(""));
        let mut masked_title: Option<Option<String>> = None;
        for (pack, accumulator) in TOPIC_PACKS.iter().zip(packs.iter_mut()) {
            let mut cues = matched_cues(&title_words, pack);
            for cue in matched_cues(&description_words, pack) {
                if !cues.contains(&cue) {
                    cues.push(cue);
                }
            }
            if cues.is_empty() {
                continue;
            }
            accumulator
                .request_keys
                .insert((row.portal_key.clone(), row.request_id));
            if masked_title.is_none() {
                masked_title = Some(masked(row.title.clone())?);
            }
            accumulator.requests.push(TopicRequestMatch {
                portal_key: row.portal_key.clone(),
                request_id: row.request_id,
                title: masked_title.clone().flatten(),
                matched_cues: cues,
            });
        }
    }

    let anchors = load_finding_anchors(repository).await?;
    let mut fetched: HashMap<String, Option<Finding>> = HashMap::new();
    let mut topics = Vec::with_capacity(TOPIC_PACKS.len());
    for (pack, accumulator) in TOPIC_PACKS.iter().zip(packs) {
        let mut finding_total = 0_usize;
        let mut findings = Vec::new();
        for anchor in &anchors {
            let Some(anchor_request_id) = anchor_in_pack(anchor, &accumulator) else {
                continue;
            };
            finding_total += 1;
            if findings.len() >= MAX_ITEMS_PER_TOPIC {
                continue;
            }
            if !fetched.contains_key(&anchor.id) {
                let finding = repository.get_finding_by_id(anchor.id.clone()).await?;
                fetched.insert(anchor.id.clone(), finding);
            }
            // Dismissed between the two reads: leave it out, as the inbox would.
            let Some(finding) = fetched[&anchor.id]
                .as_ref()
                .filter(|finding| finding.dismissed_at.is_none())
            else {
                finding_total -= 1;
                continue;
            };
            // Masked as the verifiable report masks them; detector prose can carry source text.
            let mut finding = finding.clone();
            finding.headline = mask_pii(&finding.headline)?.masked_text;
            finding.explanation = mask_pii(&finding.explanation)?.masked_text;
            for citation in &mut finding.citations {
                citation.label = masked(citation.label.take())?;
            }
            findings.push(TopicFindingMatch {
                basis: finding_basis(finding.kind),
                anchor_request_id,
                finding,
            });
        }
        let document_total = accumulator.documents.len();
        let request_total = accumulator.requests.len();
        let mut documents = accumulator.documents;
        documents.truncate(MAX_ITEMS_PER_TOPIC);
        let mut requests = accumulator.requests;
        requests.truncate(MAX_ITEMS_PER_TOPIC);
        topics.push(TopicPackView {
            id: pack.id.to_string(),
            title: pack.title.to_string(),
            cues: pack.cues.iter().map(|cue| cue_label(cue)).collect(),
            document_total,
            documents,
            request_total,
            requests,
            finding_total,
            findings,
        });
    }

    Ok(TopicPacksReport {
        topics,
        documents_scanned,
        documents_prefix_only,
        documents_file_name_only,
        document_limit_reached,
        requests_scanned,
        request_limit_reached,
        text_prefix_characters: TEXT_PREFIX_CHARACTERS,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::finding::{
        FindingCitation, FindingConfidence, FindingSeverity, FindingSubject,
    };
    use chrono::Utc;

    fn tags(text: &str) -> Vec<&'static str> {
        let tokens = words(text);
        TOPIC_PACKS
            .iter()
            .filter(|pack| !matched_cues(&tokens, pack).is_empty())
            .map(|pack| pack.id)
            .collect()
    }

    #[test]
    fn each_pack_matches_its_topic_and_not_its_homonyms() {
        assert_eq!(
            tags("Crews cleared a homeless encampment on 17th St."),
            ["abatement"]
        );
        assert_eq!(tags("Staff bagged and tagged the items."), ["belongings"]);
        assert_eq!(tags("FY25 police overtime exceeded plan."), ["police"]);
        assert_eq!(
            tags("Downtown public restrooms close at 6 pm."),
            ["bathrooms"]
        );
        assert!(tags("Street sweeping schedule; weed abatement; fire department overtime; restrooms are on floor 2; notice of hearing.").is_empty());
    }

    #[test]
    fn counted_basis_is_exactly_the_legal_claim_kinds() {
        for kind in [
            FindingKind::PolicyViolationDetected,
            FindingKind::HeldBackResponse,
            FindingKind::ClaimContradictsData,
            FindingKind::ContradictoryStatement,
            FindingKind::DifferentialTreatment,
            FindingKind::PracticeDepartureCounted,
            FindingKind::PracticeDepartureStated,
            FindingKind::ContractCeilingExceeded,
            FindingKind::SplitPurchasePattern,
            FindingKind::SplitPurchaseProhibitedByPolicy,
            FindingKind::EncampmentNoticeShortfall,
            FindingKind::EncampmentNoticeStatedShortfall,
            FindingKind::FundActualAboveAdopted,
        ] {
            assert_eq!(
                finding_basis(kind) == TopicFindingBasis::Counted,
                kind.is_legal_claim(),
                "{kind:?}"
            );
        }
    }

    fn finding(id: &str, subject: FindingSubject) -> Finding {
        let now = Utc::now();
        Finding {
            id: id.into(),
            kind: FindingKind::HeldBackResponse,
            severity: FindingSeverity::Notable,
            confidence: FindingConfidence::Unassessed,
            subject,
            headline: id.into(),
            explanation: String::new(),
            citations: vec![],
            discovered_at: now,
            last_seen_at: now,
            dismissed_at: None,
            source_detector: "test".into(),
        }
    }

    #[tokio::test]
    async fn findings_join_a_topic_only_by_exact_tagged_subject() {
        let temp = tempfile::tempdir().unwrap();
        let repository = DuckDbRepository::new(temp.path().join("core.db")).unwrap();
        repository
            .run_migrations(include_str!("../../migrations/0001_initial_schema.sql"))
            .await
            .unwrap();
        {
            let conn = repository.analytics_connection().await.unwrap();
            for portal in ["city", "county"] {
                conn.execute(
                    "INSERT INTO portals(subdomain,display_name,base_url) VALUES (?1,?1,'https://example.test')",
                    [portal],
                )
                .unwrap();
            }
            conn.execute_batch(
                "INSERT INTO requests(id,portal_subdomain,title,description,status,created_at,updated_at) VALUES
                   (1,'city','Encampment sweep logs',NULL,'closed','2026-01-01','2026-01-01'),
                   (2,'city','Parking citations',NULL,'closed','2026-01-01','2026-01-01'),
                   (1,'county','Parking citations',NULL,'closed','2026-01-01','2026-01-01');
                 INSERT INTO documents(id,request_id,portal_subdomain,file_name,visibility,created_at,updated_at) VALUES
                   (10,2,'city','memo.pdf','public','2026-01-01','2026-01-01'),
                   (11,2,'city','ledger.pdf','public','2026-01-01','2026-01-01'),
                   (12,2,'city','public-restrooms-map.pdf','public','2026-01-01','2026-01-01'),
                   (10,1,'county','memo.pdf','public','2026-01-01','2026-01-01');
                 INSERT INTO documents_fts(document_id,portal_subdomain,file_name,extracted_text) VALUES
                   (10,'city','memo.pdf','The encampment on C Street was cleared.'),
                   (11,'city','ledger.pdf','Vendor payments.'),
                   (10,'county','memo.pdf','Vendor payments.');",
            )
            .unwrap();
        }
        let citation = FindingCitation {
            portal_key: "city".into(),
            document_id: 10,
            character_start: None,
            character_end: None,
            quote: None,
            label: None,
        };
        let subject =
            |portal: Option<&str>, request: Option<i64>, document: Option<i64>| FindingSubject {
                portal_key: portal.map(str::to_string),
                request_id: request,
                document_id: document,
                ..Default::default()
            };
        let mut cited_only = finding("cited-only", subject(Some("city"), Some(2), Some(11)));
        cited_only.citations.push(citation);
        let mut dismissed = finding("dismissed", subject(Some("city"), None, Some(10)));
        dismissed.dismissed_at = Some(Utc::now());
        repository
            .record_findings(vec![
                finding("tagged-document", subject(Some("city"), Some(2), Some(10))),
                finding("tagged-request", subject(Some("city"), Some(1), None)),
                // Cites the tagged document, but its subject is an untagged one.
                cited_only,
                // Same document id in another portal.
                finding("other-portal", subject(Some("county"), None, Some(10))),
                finding("no-portal", subject(None, None, Some(10))),
                finding(
                    "agency-only",
                    FindingSubject {
                        portal_key: Some("city".into()),
                        agency_id: Some(1),
                        ..Default::default()
                    },
                ),
                // Request 2 is untagged; its tagged document does not lift the request.
                finding("untagged-request", subject(Some("city"), Some(2), None)),
                dismissed,
            ])
            .await
            .unwrap();

        let report = build_topic_packs(&repository).await.unwrap();
        let abatement = report
            .topics
            .iter()
            .find(|topic| topic.id == "abatement")
            .unwrap();
        let mut ids: Vec<&str> = abatement
            .findings
            .iter()
            .map(|m| m.finding.id.as_str())
            .collect();
        ids.sort_unstable();
        assert_eq!(ids, ["tagged-document", "tagged-request"]);
        assert_eq!(abatement.finding_total, 2);
        assert_eq!(abatement.documents.len(), 1);
        assert_eq!(abatement.requests.len(), 1);
        assert!(abatement
            .findings
            .iter()
            .all(|m| m.basis == TopicFindingBasis::Pattern));

        let bathrooms = report
            .topics
            .iter()
            .find(|topic| topic.id == "bathrooms")
            .unwrap();
        assert_eq!(bathrooms.documents.len(), 1);
        assert_eq!(
            bathrooms.documents[0].text_coverage,
            TopicTextCoverage::FileNameOnly
        );
        assert_eq!(report.documents_scanned, 4);
        assert_eq!(report.documents_file_name_only, 1);
    }
}
