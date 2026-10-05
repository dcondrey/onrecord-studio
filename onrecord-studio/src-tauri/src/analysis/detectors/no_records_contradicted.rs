//! Denial-vs-release contradiction detector.
//!
//! For every response that plainly denies having responsive records ("no responsive records",
//! "no responsive documents", "unable to locate"), looks for public documents the same agency had
//! already released to OTHER requests on the subject the denied request asked about.
//!
//! The on-device embedding model only proposes candidates: each embedded document of the portal
//! is scored by the cosine of its closest chunk against the denied request's own text. A
//! candidate becomes evidence only through deterministic checks, all in [`admit`]:
//! - cosine at or above [`SIMILARITY_THRESHOLD`];
//! - public, from the same portal, from a different request;
//! - that request was assigned to the same agency (`requests.agency_id`, exact id equality; a
//!   department shared by a multi-department assignment is a different agency id and is not
//!   matched, and a request with no recorded agency matches nothing);
//! - released (`documents.created_at`) strictly before the denying request's `closed_at`, both
//!   parsed as RFC 3339 instants, so material the agency could not yet have had is never cited;
//! - not itself a denial.
//!
//! Word overlap is never used as a fallback: a records-request description is mostly boilerplate,
//! and on a live collection every token rule matched denials to unrelated releases. Without an
//! installed model or any embedded document the detector reports [`Outcome::NotRun`], which keeps
//! its earlier findings instead of reading the absence of a test as the absence of a finding.
//!
//! Cosine measures closeness of subject and wording, not responsiveness, so the finding has no
//! confidence score; its explanation states the observed cosine, threshold, and both dates.

use crate::{
    analysis::{
        assertion_index::extract_denial,
        finding::{
            finding_id, Finding, FindingCitation, FindingConfidence, FindingKind, FindingSeverity,
            FindingSubject,
        },
    },
    error::AppError,
    storage::{
        vector::{DocumentSimilarity, VectorIndex},
        DuckDbRepository,
    },
};
use chrono::{DateTime, FixedOffset, Utc};
use duckdb::params;

pub const SLUG: &str = "no_records_contradicted";

/// Max denial candidates inspected per invocation.
const MAX_CANDIDATES: i64 = 500;

/// Minimum cosine (bge-small-en-v1.5, unit vectors) between a release's closest chunk and the
/// denied request's text. A precision floor, not a calibrated cut: on the San Diego collection
/// the highest-scoring release judged not to contradict its denial scored 0.8531, and no
/// candidate judged a plausible contradiction scored above that. Recall is unmeasured. See
/// docs/ROADMAP.md, "Findings on real collections".
pub(crate) const SIMILARITY_THRESHOLD: f32 = 0.86;

/// Maximum contradicting-document rows to include in citations, to keep the inbox row compact.
const MAX_CITATION_ROWS: usize = 4;

pub enum Outcome {
    Ran(Vec<Finding>),
    /// The test could not be performed; the reason says why. Not a finding count of zero.
    NotRun(String),
}

pub(crate) struct Denial {
    portal_key: String,
    request_id: i64,
    rpid: Option<String>,
    agency_id: Option<i64>,
    agency_name: Option<String>,
    closed_at: Option<String>,
    document_id: i64,
    file_name: Option<String>,
    request_title: String,
    request_description: String,
    matched_phrase: String,
}

impl Denial {
    /// The text the request itself states. NextRequest titles are a truncated prefix of the
    /// description, so the description is used whole when present.
    fn query_text(&self) -> Option<&str> {
        [&self.request_description, &self.request_title]
            .into_iter()
            .map(|text| text.trim())
            .find(|text| !text.is_empty())
    }

    fn display_title(&self) -> String {
        if self.request_title.trim().is_empty() {
            format!("request {}", self.label())
        } else {
            self.request_title.trim().to_string()
        }
    }

    fn label(&self) -> String {
        self.rpid
            .clone()
            .unwrap_or_else(|| self.request_id.to_string())
    }
}

pub(crate) struct Release {
    portal_key: String,
    request_id: i64,
    document_id: i64,
    file_name: Option<String>,
    public: bool,
    agency_id: Option<i64>,
    released_at: String,
    is_denial: bool,
    similarity: DocumentSimilarity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Rejection {
    BelowThreshold,
    NotPublic,
    OtherPortal,
    SameRequest,
    AgencyUnrecorded,
    OtherAgency,
    ClosureUndated,
    ReleaseUndated,
    NotBeforeClosure,
    ItselfADenial,
}

fn instant(value: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(value.trim()).ok()
}

/// The deterministic checks a semantically near document must pass to be cited as a release
/// that contradicts the denial.
pub(crate) fn admit(denial: &Denial, release: &Release, threshold: f32) -> Result<(), Rejection> {
    if release.similarity.cosine < threshold {
        return Err(Rejection::BelowThreshold);
    }
    if !release.public {
        return Err(Rejection::NotPublic);
    }
    if release.portal_key != denial.portal_key {
        return Err(Rejection::OtherPortal);
    }
    if release.request_id == denial.request_id {
        return Err(Rejection::SameRequest);
    }
    let Some(denying_agency) = denial.agency_id else {
        return Err(Rejection::AgencyUnrecorded);
    };
    if release.agency_id != Some(denying_agency) {
        return Err(Rejection::OtherAgency);
    }
    let Some(closed_at) = denial.closed_at.as_deref().and_then(instant) else {
        return Err(Rejection::ClosureUndated);
    };
    let Some(released_at) = instant(&release.released_at) else {
        return Err(Rejection::ReleaseUndated);
    };
    if released_at >= closed_at {
        return Err(Rejection::NotBeforeClosure);
    }
    if release.is_denial {
        return Err(Rejection::ItselfADenial);
    }
    Ok(())
}

pub async fn run(
    repository: &DuckDbRepository,
    semantic: Option<&VectorIndex>,
) -> Result<Outcome, AppError> {
    let denials = load_denials(repository).await?;
    if denials.is_empty() {
        return Ok(Outcome::Ran(Vec::new()));
    }
    let Some(index) = semantic else {
        return Ok(Outcome::NotRun(
            "the on-device embedding model is not available to this run".into(),
        ));
    };
    if !index.model_status().await?.installed {
        return Ok(Outcome::NotRun(
            "the on-device embedding model is not installed".into(),
        ));
    }
    let status = index.index_status(0).await?;
    if status.running {
        return Ok(Outcome::NotRun(
            "the semantic index is being rebuilt; releases not yet embedded cannot be tested"
                .into(),
        ));
    }
    if status.indexed_document_count == 0 {
        return Ok(Outcome::NotRun(
            "no document has been embedded; build the semantic index first".into(),
        ));
    }
    let mut findings = Vec::new();
    for denial in &denials {
        let Some(scored) = score_releases(repository, index, denial, None).await? else {
            return Ok(Outcome::NotRun(
                "the on-device embedding model is not installed".into(),
            ));
        };
        let admitted: Vec<Release> = scored
            .into_iter()
            .filter(|release| admit(denial, release, SIMILARITY_THRESHOLD).is_ok())
            .collect();
        if let Some(finding) = build_finding(denial, &admitted) {
            findings.push(finding);
        }
    }
    Ok(Outcome::Ran(findings))
}

/// Plain, unscoped denials with a stated request text. The SQL LIKE gate is coarse;
/// `extract_denial` re-verifies with the shared regex.
async fn load_denials(repository: &DuckDbRepository) -> Result<Vec<Denial>, AppError> {
    let rows = repository
        .query_connection("Denial-candidate query", move |conn| {
            let mut statement = conn
                .prepare(
                    "SELECT documents.portal_subdomain, documents.request_id, requests.rpid,
                            requests.agency_id, agencies.name, requests.closed_at,
                            documents.id, documents.file_name,
                            COALESCE(requests.title, ''), COALESCE(requests.description, ''),
                            documents_fts.extracted_text
                     FROM documents
                     INNER JOIN documents_fts
                        ON documents_fts.document_id = documents.id
                       AND documents_fts.portal_subdomain = documents.portal_subdomain
                     INNER JOIN requests
                        ON requests.id = documents.request_id
                       AND requests.portal_subdomain = documents.portal_subdomain
                     LEFT JOIN agencies
                        ON agencies.id = requests.agency_id
                       AND agencies.portal_subdomain = requests.portal_subdomain
                     WHERE LOWER(TRIM(documents.visibility)) = 'public'
                       AND documents.portal_subdomain NOT IN (SELECT subdomain FROM user_supplied_sources)
                       AND LENGTH(documents_fts.extracted_text) > 0
                       AND (
                           LOWER(documents_fts.extracted_text) LIKE '%no responsive records%' OR
                           LOWER(documents_fts.extracted_text) LIKE '%no responsive documents%' OR
                           LOWER(documents_fts.extracted_text) LIKE '%no records exist%' OR
                           LOWER(documents_fts.extracted_text) LIKE '%no records were found%' OR
                           LOWER(documents_fts.extracted_text) LIKE '%unable to locate%' OR
                           LOWER(documents_fts.extracted_text) LIKE '%cannot locate any records%'
                       )
                     ORDER BY documents.updated_at DESC, documents.id
                     LIMIT ?1",
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to prepare denial candidate query: {error}"))
                })?;
            let rows = statement
                .query_map(params![MAX_CANDIDATES], |row| {
                    Ok((
                        Denial {
                            portal_key: row.get(0)?,
                            request_id: row.get(1)?,
                            rpid: row.get(2)?,
                            agency_id: row.get(3)?,
                            agency_name: row.get(4)?,
                            closed_at: row.get(5)?,
                            document_id: row.get(6)?,
                            file_name: row.get(7)?,
                            request_title: row.get(8)?,
                            request_description: row.get(9)?,
                            matched_phrase: String::new(),
                        },
                        row.get::<_, String>(10)?,
                    ))
                })
                .map_err(|error| {
                    AppError::Storage(format!("Failed to run denial candidate query: {error}"))
                })?;
            rows.collect::<duckdb::Result<Vec<_>>>()
                .map_err(|error| AppError::Storage(format!("Failed to decode denial row: {error}")))
        })
        .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(mut denial, text)| {
            let fingerprint = extract_denial(&text)?;
            if fingerprint.scope_narrow || denial.query_text().is_none() {
                return None;
            }
            denial.matched_phrase = fingerprint.matched_phrase.trim().to_string();
            Some(denial)
        })
        .collect())
}

/// Every embedded document of the denial's portal scored against the request text, with the
/// metadata [`admit`] checks. Only documents at or above the threshold are returned unless
/// `top` asks for the closest N regardless (the measurement harness). `None` when no model is
/// installed.
async fn score_releases(
    repository: &DuckDbRepository,
    index: &VectorIndex,
    denial: &Denial,
    top: Option<usize>,
) -> Result<Option<Vec<Release>>, AppError> {
    let Some(query) = denial.query_text() else {
        return Ok(Some(Vec::new()));
    };
    let Some(mut nearest) = index
        .nearest_documents(query, &denial.portal_key, usize::MAX)
        .await?
    else {
        return Ok(None);
    };
    match top {
        Some(count) => nearest.truncate(count),
        None => nearest.retain(|near| near.cosine >= SIMILARITY_THRESHOLD),
    }
    if nearest.is_empty() {
        return Ok(Some(Vec::new()));
    }
    let portal_key = denial.portal_key.clone();
    let ids: Vec<i64> = nearest
        .iter()
        .filter_map(|near| i64::try_from(near.document_id).ok())
        .collect();
    let metadata = repository
        .query_connection("Release metadata", move |conn| {
            let placeholders = (2..2 + ids.len())
                .map(|slot| format!("?{slot}"))
                .collect::<Vec<_>>()
                .join(", ");
            let mut statement = conn
                .prepare(&format!(
                    "SELECT documents.id, documents.request_id, documents.file_name,
                            LOWER(TRIM(documents.visibility)) = 'public', requests.agency_id,
                            documents.created_at, COALESCE(documents_fts.extracted_text, '')
                     FROM documents
                     INNER JOIN requests
                        ON requests.id = documents.request_id
                       AND requests.portal_subdomain = documents.portal_subdomain
                     LEFT JOIN documents_fts
                        ON documents_fts.document_id = documents.id
                       AND documents_fts.portal_subdomain = documents.portal_subdomain
                     WHERE documents.portal_subdomain = ?1 AND documents.id IN ({placeholders})"
                ))
                .map_err(|error| {
                    AppError::Storage(format!("Failed to prepare release metadata: {error}"))
                })?;
            let mut bound: Vec<&dyn duckdb::ToSql> = Vec::with_capacity(ids.len() + 1);
            bound.push(&portal_key);
            for id in &ids {
                bound.push(id);
            }
            let rows = statement
                .query_map(bound.as_slice(), |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, bool>(3)?,
                        row.get::<_, Option<i64>>(4)?,
                        row.get::<_, String>(5)?,
                        extract_denial(&row.get::<_, String>(6)?).is_some(),
                    ))
                })
                .map_err(|error| {
                    AppError::Storage(format!("Failed to run release metadata: {error}"))
                })?;
            rows.collect::<duckdb::Result<Vec<_>>>().map_err(|error| {
                AppError::Storage(format!("Failed to decode release metadata: {error}"))
            })
        })
        .await?;
    let mut by_id: std::collections::HashMap<i64, _> =
        metadata.into_iter().map(|row| (row.0, row)).collect();
    Ok(
        Some(
            nearest
                .into_iter()
                .filter_map(|similarity| {
                    let id = i64::try_from(similarity.document_id).ok()?;
                    let (
                        document_id,
                        request_id,
                        file_name,
                        public,
                        agency_id,
                        released_at,
                        is_denial,
                    ) = by_id.remove(&id)?;
                    Some(Release {
                        portal_key: denial.portal_key.clone(),
                        request_id,
                        document_id,
                        file_name,
                        public,
                        agency_id,
                        released_at,
                        is_denial,
                        similarity,
                    })
                })
                .collect(),
        ),
    )
}

fn build_finding(denial: &Denial, admitted: &[Release]) -> Option<Finding> {
    let closest = admitted.first()?;
    let subject = FindingSubject {
        portal_key: Some(denial.portal_key.clone()),
        agency_id: denial.agency_id,
        request_id: Some(denial.request_id),
        document_id: Some(denial.document_id),
        ..Default::default()
    };
    let id = finding_id(
        FindingKind::NoRecordsContradictedByRelease,
        SLUG,
        &subject,
        &[],
    );
    let display_title = denial.display_title();
    let agency = denial
        .agency_name
        .clone()
        .unwrap_or_else(|| "the same agency".into());
    let closed_at = denial.closed_at.clone().unwrap_or_default();
    let headline = format!(
        "Denial for '{display_title}' follows {n} earlier release(s) by {agency} on its subject",
        n = admitted.len()
    );
    let explanation = format!(
        "Request {label} ('{display_title}') was closed {closed_at} with \"{phrase}\". Before that \
         closure, {agency} had released {n} public document(s) to other requests whose text is \
         near this request's own: the closest, {closest_name} (request {closest_request}, \
         released {released}), has cosine {cosine:.3} against the request text, at or above the \
         {threshold:.2} threshold of the on-device bge-small-en-v1.5 model. Cosine measures \
         closeness of subject and wording, not that the release answers this request; read the \
         cited passages to judge whether the denial's factual basis holds.",
        label = denial.label(),
        phrase = denial.matched_phrase,
        n = admitted.len(),
        closest_name = document_label(closest.file_name.as_deref(), closest.document_id),
        closest_request = closest.request_id,
        released = closest.released_at,
        cosine = closest.similarity.cosine,
        threshold = SIMILARITY_THRESHOLD,
    );

    let mut citations = Vec::with_capacity(1 + MAX_CITATION_ROWS);
    citations.push(FindingCitation {
        portal_key: denial.portal_key.clone(),
        document_id: denial.document_id,
        character_start: None,
        character_end: None,
        quote: Some(denial.matched_phrase.clone()),
        label: Some(format!(
            "Denial: {}",
            document_label(denial.file_name.as_deref(), denial.document_id)
        )),
    });
    for release in admitted.iter().take(MAX_CITATION_ROWS) {
        citations.push(FindingCitation {
            portal_key: release.portal_key.clone(),
            document_id: release.document_id,
            character_start: Some(release.similarity.character_start),
            character_end: Some(release.similarity.character_end),
            quote: Some(release.similarity.excerpt.clone()),
            label: Some(format!(
                "Released {} to request {} (cosine {:.3}): {}",
                release.released_at,
                release.request_id,
                release.similarity.cosine,
                document_label(release.file_name.as_deref(), release.document_id)
            )),
        });
    }

    let now = Utc::now();
    Some(Finding {
        id,
        kind: FindingKind::NoRecordsContradictedByRelease,
        severity: FindingSeverity::Notable,
        confidence: FindingConfidence::Unassessed,
        subject,
        headline,
        explanation,
        citations,
        discovered_at: now,
        last_seen_at: now,
        dismissed_at: None,
        source_detector: SLUG.into(),
    })
}

fn document_label(file_name: Option<&str>, document_id: i64) -> String {
    file_name
        .map(str::to_string)
        .unwrap_or_else(|| format!("document {document_id}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn denial() -> Denial {
        Denial {
            portal_key: "sd".into(),
            request_id: 1,
            rpid: Some("25-1".into()),
            agency_id: Some(7),
            agency_name: Some("Purchasing & Contracting".into()),
            closed_at: Some("2026-01-15T15:56:00-08:00".into()),
            document_id: 100,
            file_name: Some("closure-notice-25-1.txt".into()),
            request_title: "Maintenance contracts with Caltrans".into(),
            request_description: "Maintenance contracts with Caltrans for the bike path".into(),
            matched_phrase: "no responsive documents".into(),
        }
    }

    fn release(released_at: &str, agency_id: Option<i64>) -> Release {
        Release {
            portal_key: "sd".into(),
            request_id: 2,
            document_id: 200,
            file_name: Some("caltrans-maintenance-agreement.pdf".into()),
            public: true,
            agency_id,
            released_at: released_at.into(),
            is_denial: false,
            similarity: DocumentSimilarity {
                request_id: 2,
                document_id: 200,
                cosine: SIMILARITY_THRESHOLD + 0.05,
                character_start: 0,
                character_end: 40,
                excerpt: "Maintenance agreement between the City and Caltrans".into(),
            },
        }
    }

    #[test]
    fn a_near_release_after_the_closure_is_not_a_contradiction() {
        // 14:57 -10:00 is 61 minutes after the 15:56 -08:00 closure, though it sorts before it
        // as a string.
        assert_eq!(
            admit(
                &denial(),
                &release("2026-01-15T14:57:00-10:00", Some(7)),
                SIMILARITY_THRESHOLD
            ),
            Err(Rejection::NotBeforeClosure)
        );
    }

    #[test]
    fn a_near_release_before_the_closure_from_the_same_agency_is() {
        let denial = denial();
        let earlier = release("2025-11-01T09:00:00-07:00", Some(7));
        assert_eq!(admit(&denial, &earlier, SIMILARITY_THRESHOLD), Ok(()));
        let finding = build_finding(&denial, &[earlier]).expect("an admitted release fires");
        assert_eq!(finding.confidence, FindingConfidence::Unassessed);
        assert!(finding.explanation.contains("2025-11-01T09:00:00-07:00"));
        assert!(finding.explanation.contains("2026-01-15T15:56:00-08:00"));
    }

    #[test]
    fn a_near_release_from_another_agency_is_not_a_contradiction() {
        assert_eq!(
            admit(
                &denial(),
                &release("2025-11-01T09:00:00-07:00", Some(8)),
                SIMILARITY_THRESHOLD
            ),
            Err(Rejection::OtherAgency)
        );
    }

    async fn seeded_with_a_denial() -> DuckDbRepository {
        let repository = DuckDbRepository::new(":memory:").expect("open");
        repository
            .run_migrations(include_str!("../../../migrations/0001_initial_schema.sql"))
            .await
            .expect("migrate");
        repository
            .query_connection("no-records fixture", |connection| {
                connection
                    .execute_batch(
                        "INSERT INTO portals (subdomain, display_name, base_url)
                           VALUES ('sd', 'SD', 'https://sd.example');
                         INSERT INTO agencies (id, portal_subdomain, name) VALUES (7, 'sd', 'Purchasing');
                         INSERT INTO requests (id, portal_subdomain, title, description, status,
                                               created_at, updated_at, closed_at, agency_id)
                           VALUES (1, 'sd', 'Caltrans maintenance contracts',
                                   'Maintenance contracts with Caltrans for the bike path', 'closed',
                                   '2026-01-01T00:00:00Z', '2026-01-15T00:00:00Z',
                                   '2026-01-15T00:00:00Z', 7);
                         INSERT INTO documents (id, request_id, portal_subdomain, file_name,
                                                visibility, created_at, updated_at)
                           VALUES (100, 1, 'sd', 'closure-notice.txt', 'public',
                                   '2026-01-15T00:00:00Z', '2026-01-15T00:00:00Z');
                         INSERT INTO documents_fts (document_id, portal_subdomain, file_name, extracted_text)
                           VALUES (100, 'sd', 'closure-notice.txt',
                                   'The City of San Diego has no responsive documents.');",
                    )
                    .map_err(|error| AppError::Storage(format!("fixture: {error}")))
            })
            .await
            .expect("seed");
        repository
    }

    #[tokio::test]
    async fn without_the_model_the_detector_is_not_run_and_keeps_its_findings() {
        let repository = seeded_with_a_denial().await;
        let earlier = build_finding(&denial(), &[release("2025-11-01T09:00:00-07:00", Some(7))])
            .expect("finding");
        let earlier_id = earlier.id.clone();
        repository
            .record_findings(vec![earlier])
            .await
            .expect("record");

        let temp = tempfile::tempdir().expect("temp");
        let index = VectorIndex::open(&temp.path().join("v.db"), temp.path().join("cache"))
            .expect("vector index");
        for semantic in [None, Some(&index)] {
            match run(&repository, semantic).await.expect("run") {
                Outcome::NotRun(reason) => assert!(reason.contains("model"), "{reason}"),
                Outcome::Ran(findings) => {
                    panic!("ran without a model: {} findings", findings.len())
                }
            }
        }

        let summary = crate::analysis::detectors::refresh_all_findings_with_embeddings(
            &repository,
            Some(&index),
        )
        .await
        .expect("refresh");
        assert!(summary
            .not_run
            .iter()
            .any(|not_run| not_run.detector == SLUG));
        let kept = repository
            .list_findings(Default::default(), 50)
            .await
            .expect("list");
        assert!(kept.iter().any(|finding| finding.id == earlier_id));
    }

    /// Scores every denial in a collected live database against its nearest releases and prints
    /// the evidence the threshold is pinned from. `ONRECORD_EVAL_DIR` holds the `core.db` a
    /// `live_evaluation` run left; the semantic index is built beside it as `vectors.db` from
    /// the installed model in `ONRECORD_EMBEDDING_CACHE` (never downloaded).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "needs a collected live database and an installed embedding model"]
    async fn measure_on_a_live_collection() {
        let root = std::path::PathBuf::from(std::env::var("ONRECORD_EVAL_DIR").expect("dir"));
        let cache = std::path::PathBuf::from(
            std::env::var("ONRECORD_EMBEDDING_CACHE").expect("ONRECORD_EMBEDDING_CACHE"),
        );
        let repository = DuckDbRepository::new(root.join("core.db")).unwrap();
        let index = VectorIndex::open(&root.join("vectors.db"), cache).unwrap();
        let report = index.index_documents(&repository, false).await.unwrap();
        eprintln!("index: {report:?}");
        let denials = load_denials(&repository).await.unwrap();
        eprintln!("{} denials tested", denials.len());
        for denial in &denials {
            eprintln!(
                "\n{} [{}] closed {:?}: {}",
                denial.label(),
                denial.agency_name.as_deref().unwrap_or("-"),
                denial.closed_at,
                denial
                    .query_text()
                    .unwrap_or("")
                    .chars()
                    .take(160)
                    .collect::<String>()
            );
            let scored = score_releases(&repository, &index, denial, Some(8))
                .await
                .unwrap()
                .expect("model installed");
            for release in &scored {
                eprintln!(
                    "  {:.4} req {} doc {} {:?} released {} -> {:?}\n      {}",
                    release.similarity.cosine,
                    release.request_id,
                    release.document_id,
                    release.file_name,
                    release.released_at,
                    admit(denial, release, 0.0),
                    release
                        .similarity
                        .excerpt
                        .chars()
                        .take(200)
                        .collect::<String>()
                        .replace('\n', " ")
                );
            }
        }
        match run(&repository, Some(&index)).await.unwrap() {
            Outcome::NotRun(reason) => eprintln!("not run: {reason}"),
            Outcome::Ran(findings) => {
                eprintln!("\n{} findings at {SIMILARITY_THRESHOLD}", findings.len());
                for finding in findings {
                    eprintln!("  {}", finding.headline);
                }
            }
        }
    }
}
