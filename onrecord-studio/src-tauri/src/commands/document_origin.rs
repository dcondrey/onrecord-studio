//! Where a document came from, and what its file says about itself.
//!
//! A file the user added by hand was never fetched from an agency source, so this reports that
//! plainly and then says how the file relates to the documents that were: an exact copy of a
//! collected file, the same extracted text in a different file, or similar text. Each comparison
//! reports that it was not made when it could not be, and never as an empty result.
//!
//! The internal metadata is what the PDF and Office parsers read from the file. Whoever produced
//! the file wrote those values and could have written anything.

use crate::commands::investigation::AppInvestigationPaths;
use crate::commands::validation::{managed_evidence_file, portal_key as validate_portal_key};
use crate::error::AppError;
use crate::parsers::ooxml_docprops::{read_ooxml_docprops, MAX_PACKAGE_BYTES};
use crate::parsers::pdf_metadata_signals::{read_parsed_pdf_metadata_signals, MAX_PDF_BYTES};
use crate::storage::corpus::near_duplicates::NearDuplicateScan;
use crate::storage::corpus::CorpusIndex;
use crate::storage::document_origin::{DocumentDigests, MAXIMUM_LISTED_MATCHES};
use crate::storage::DuckDbRepository;
use crate::types::{
    DocumentInternalMetadata, DocumentOrigin, DocumentOriginReport, HeldComparison,
    HeldDocumentRelation, InternalMetadataField, InternalMetadataFormat,
    InternalMetadataUnreadReason, NearDuplicateMatch, NotComparedReason,
};
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::path::Path;
use tauri::State;

/// Fingerprint candidates read from the similarity index before those held under a collected
/// source are kept.
const NEAR_DUPLICATE_CANDIDATE_LIMIT: usize = 200;

async fn near_duplicates(
    repository: &DuckDbRepository,
    index: &CorpusIndex,
    document_id: u64,
    portal_key: &str,
    digests: &DocumentDigests,
) -> Result<HeldComparison<NearDuplicateMatch>, AppError> {
    let Some(text_digest) = digests.extracted_text_sha256.clone() else {
        return Ok(HeldComparison::NotCompared {
            reason: NotComparedReason::TextNotExtracted,
        });
    };
    let scan = index
        .near_duplicates_of(
            portal_key.to_string(),
            document_id,
            text_digest,
            NEAR_DUPLICATE_CANDIDATE_LIMIT,
        )
        .await?;
    let NearDuplicateScan::Scanned {
        candidates,
        other_indexed_documents,
    } = scan
    else {
        return Ok(HeldComparison::NotCompared {
            reason: NotComparedReason::NotInSimilarityIndex,
        });
    };
    let candidates_exhausted = candidates.len() < NEAR_DUPLICATE_CANDIDATE_LIMIT;
    let distances: HashMap<(String, i64), u32> = candidates
        .iter()
        .map(|candidate| {
            (
                (candidate.portal_key.clone(), candidate.document_id),
                candidate.simhash_distance,
            )
        })
        .collect();
    let keys = candidates
        .into_iter()
        .map(|candidate| (candidate.portal_key, candidate.document_id))
        .collect();
    let mut matches: Vec<NearDuplicateMatch> = repository
        .held_documents_by_key(keys)
        .await?
        .into_iter()
        .filter_map(|document| {
            let distance = distances.get(&(document.portal_key.clone(), document.document_id))?;
            Some(NearDuplicateMatch {
                simhash_distance: *distance,
                document,
            })
        })
        .collect();
    let truncated = matches.len() > MAXIMUM_LISTED_MATCHES || !candidates_exhausted;
    matches.truncate(MAXIMUM_LISTED_MATCHES);
    Ok(HeldComparison::Compared {
        matches,
        truncated,
        compared_count: other_indexed_documents,
    })
}

pub async fn describe_origin(
    repository: &DuckDbRepository,
    index: &CorpusIndex,
    document_id: u64,
    portal_key: &str,
) -> Result<DocumentOriginReport, AppError> {
    let digests = repository
        .document_digests(document_id, portal_key.to_string())
        .await?;
    let source_kind = digests.source_kind;
    let held_relation = if source_kind.is_user_supplied() {
        let near_duplicates =
            near_duplicates(repository, index, document_id, portal_key, &digests).await?;
        let (exact_copies, same_text) = repository.held_documents_sharing_digests(digests).await?;
        Some(HeldDocumentRelation {
            exact_copies,
            same_text,
            near_duplicates,
        })
    } else {
        None
    };
    Ok(DocumentOriginReport {
        document_id,
        portal_key: portal_key.to_string(),
        origin: if source_kind.is_user_supplied() {
            DocumentOrigin::AddedByUser
        } else {
            DocumentOrigin::CollectedFromSource
        },
        source_kind,
        held_relation,
    })
}

fn metadata_format(
    file_name: Option<&str>,
    content_type: Option<&str>,
) -> Option<InternalMetadataFormat> {
    let name = file_name.unwrap_or_default().to_ascii_lowercase();
    let content_type = content_type.unwrap_or_default().trim().to_ascii_lowercase();
    if name.ends_with(".pdf") || content_type.starts_with("application/pdf") {
        Some(InternalMetadataFormat::Pdf)
    } else if [".docx", ".xlsx", ".pptx"]
        .iter()
        .any(|extension| name.ends_with(extension))
        || content_type.starts_with("application/vnd.openxmlformats-officedocument")
    {
        Some(InternalMetadataFormat::Ooxml)
    } else {
        None
    }
}

fn field(name: &str, value: Option<String>) -> InternalMetadataField {
    InternalMetadataField {
        name: name.to_string(),
        value: value
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
    }
}

fn instant(value: Option<DateTime<Utc>>) -> Option<String> {
    value.map(|value| value.to_rfc3339())
}

/// `None` when the file could not be parsed as the named format, which is not the same as a file
/// that parsed and records nothing.
fn read_fields(
    path: &Path,
    format: InternalMetadataFormat,
) -> Result<Option<Vec<InternalMetadataField>>, AppError> {
    match format {
        InternalMetadataFormat::Pdf => Ok(read_parsed_pdf_metadata_signals(path)?.map(|signals| {
            vec![
                field("Info Creator", signals.info_creator),
                field("Info Producer", signals.info_producer),
                field("XMP CreatorTool", signals.xmp_creator_tool),
                field("XMP Producer", signals.xmp_pdf_producer),
                field("Info CreationDate", instant(signals.info_creation_date)),
                field("Info ModDate", instant(signals.info_mod_date)),
                field("XMP CreateDate", instant(signals.xmp_create_date)),
                field("XMP ModifyDate", instant(signals.xmp_modify_date)),
                field("XMP MetadataDate", instant(signals.xmp_metadata_date)),
            ]
        })),
        InternalMetadataFormat::Ooxml => {
            // The reader returns empty properties for a file that is not a package at all.
            let is_package = std::fs::File::open(path)
                .ok()
                .is_some_and(|file| zip::ZipArchive::new(file).is_ok());
            if !is_package {
                return Ok(None);
            }
            let properties = read_ooxml_docprops(path)?;
            Ok(Some(vec![
                field("Last modified by", properties.last_modified_by),
                field("Application", properties.application),
                field("Company", properties.company),
                field("Created", instant(properties.created)),
                field("Modified", instant(properties.modified)),
                field("Last printed", instant(properties.last_printed)),
            ]))
        }
    }
}

pub async fn read_internal_metadata(
    repository: &DuckDbRepository,
    document_root: &Path,
    document_id: u64,
    portal_key: &str,
) -> Result<DocumentInternalMetadata, AppError> {
    let not_read = |reason| Ok(DocumentInternalMetadata::NotRead { reason });
    let digests = repository
        .document_digests(document_id, portal_key.to_string())
        .await?;
    let Some(format) = metadata_format(
        digests.file_name.as_deref(),
        digests.content_type.as_deref(),
    ) else {
        return not_read(InternalMetadataUnreadReason::UnsupportedFormat);
    };
    let maximum_bytes = match format {
        InternalMetadataFormat::Pdf => MAX_PDF_BYTES,
        InternalMetadataFormat::Ooxml => MAX_PACKAGE_BYTES,
    };
    let Ok(stored_path) = repository
        .resolve_public_document_path(document_id, portal_key)
        .await
    else {
        return not_read(InternalMetadataUnreadReason::FileUnavailable);
    };
    let Ok(path) = managed_evidence_file(
        stored_path,
        document_root,
        maximum_bytes,
        "The internal metadata reader",
    )
    .await
    else {
        return not_read(InternalMetadataUnreadReason::FileUnavailable);
    };
    let fields = crate::blocking::spawn("Internal metadata read", move || {
        Ok(read_fields(&path, format).ok().flatten())
    })
    .await??;
    match fields {
        Some(fields) => Ok(DocumentInternalMetadata::Read { format, fields }),
        None => not_read(InternalMetadataUnreadReason::Unreadable),
    }
}

#[tauri::command]
pub async fn describe_document_origin(
    document_id: u64,
    portal_key: String,
    repository: State<'_, DuckDbRepository>,
    index: State<'_, CorpusIndex>,
) -> Result<DocumentOriginReport, AppError> {
    let portal_key = validate_portal_key(&portal_key)?;
    describe_origin(&repository, &index, document_id, portal_key).await
}

#[tauri::command]
pub async fn read_document_internal_metadata(
    document_id: u64,
    portal_key: String,
    repository: State<'_, DuckDbRepository>,
    paths: State<'_, AppInvestigationPaths>,
) -> Result<DocumentInternalMetadata, AppError> {
    let portal_key = validate_portal_key(&portal_key)?;
    read_internal_metadata(&repository, &paths.document_root, document_id, portal_key).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrity::sha256_hex;
    use crate::storage::corpus::CorpusDocumentInput;
    use crate::types::HeldDocumentMatch;
    use std::io::Write;

    const UPLOAD: &str = "source-upload";
    const OTHER_UPLOAD: &str = "source-other";
    const RELEASE: &str = "release";

    struct Fixture {
        _directory: tempfile::TempDir,
        root: std::path::PathBuf,
        repository: DuckDbRepository,
        index: CorpusIndex,
    }

    async fn fixture() -> Fixture {
        let directory = tempfile::tempdir().unwrap();
        let repository = DuckDbRepository::new(directory.path().join("core.db")).unwrap();
        repository
            .run_migrations(include_str!("../../migrations/0001_initial_schema.sql"))
            .await
            .unwrap();
        let index = CorpusIndex::open(directory.path().join("corpus.db")).unwrap();
        let root = directory.path().join("evidence");
        std::fs::create_dir(&root).unwrap();
        let conn = repository.analytics_connection().await.unwrap();
        for (portal, name, kind) in [
            (RELEASE, "City records portal", "NEXT_REQUEST"),
            (UPLOAD, "letter.pdf", "LOCAL_FILE"),
            (OTHER_UPLOAD, "copy.pdf", "LOCAL_FILE"),
        ] {
            conn.execute(
                "INSERT INTO portals (subdomain, display_name, base_url, source_kind)
                 VALUES (?1, ?2, 'https://example.test', ?3)",
                duckdb::params![portal, name, kind],
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO requests (id, portal_subdomain, rpid, title, status, created_at, updated_at)
             VALUES (7, ?1, '24-100', 'Contracts', 'closed', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            duckdb::params![RELEASE],
        )
        .unwrap();
        Fixture {
            _directory: directory,
            root,
            repository,
            index,
        }
    }

    /// 400 distinct pronounceable tokens, so one changed token moves the fingerprint very little.
    fn long_text(changed: Option<usize>) -> String {
        let onsets = [
            "b", "d", "f", "g", "k", "l", "m", "n", "p", "r", "s", "t", "v", "z", "br", "dr", "fl",
            "gr", "pl", "st",
        ];
        let rimes = [
            "ane", "ebo", "ilu", "omar", "unet", "arin", "elos", "idar", "oben", "ulim", "astor",
            "endal", "ivor", "ostan", "urek", "amil", "erot", "inas", "olund", "ubar",
        ];
        let mut words = Vec::new();
        for onset in onsets {
            for rime in rimes {
                words.push(format!("{onset}{rime}"));
            }
        }
        if let Some(changed) = changed {
            words[changed] = "quixotry".to_string();
        }
        words.join(" ")
    }

    async fn add(
        fixture: &Fixture,
        portal: &str,
        id: u64,
        raw: Option<&str>,
        text: Option<&str>,
        indexed: bool,
    ) {
        let raw_digest = raw.map(|raw| sha256_hex(raw.as_bytes()));
        let text_digest = text.map(|text| sha256_hex(text.as_bytes()));
        let conn = fixture.repository.analytics_connection().await.unwrap();
        conn.execute(
            "INSERT INTO documents (id, request_id, portal_subdomain, file_name, visibility,
                created_at, updated_at, raw_file_sha256, extracted_text_sha256)
             VALUES (?1, 7, ?2, ?3, 'public', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', ?4, ?5)",
            duckdb::params![id, portal, format!("file-{id}.pdf"), raw_digest, text_digest],
        )
        .unwrap();
        if indexed {
            fixture
                .index
                .index_document(CorpusDocumentInput {
                    portal_key: portal.to_string(),
                    request_id: 7,
                    document_id: id,
                    raw_file_sha256: raw_digest.unwrap(),
                    extracted_text_sha256: text_digest.unwrap(),
                    text: text.unwrap().to_string(),
                })
                .await
                .unwrap();
        }
    }

    fn not_compared<T>(reason: NotComparedReason) -> HeldComparison<T> {
        HeldComparison::NotCompared { reason }
    }

    fn listed<T>(comparison: &HeldComparison<T>, id: impl Fn(&T) -> i64) -> Vec<i64> {
        match comparison {
            HeldComparison::Compared { matches, .. } => matches.iter().map(id).collect(),
            HeldComparison::NotCompared { reason } => panic!("not compared: {reason:?}"),
        }
    }

    #[tokio::test]
    async fn a_hand_added_file_is_matched_only_against_files_a_source_served() {
        let fixture = fixture().await;
        let text = long_text(None);
        let similar = long_text(Some(17));
        add(&fixture, UPLOAD, 1, Some("bytes"), Some(&text), true).await;
        add(&fixture, RELEASE, 10, Some("bytes"), Some(&text), true).await;
        add(&fixture, RELEASE, 11, Some("rescanned"), Some(&text), true).await;
        add(&fixture, RELEASE, 12, Some("draft"), Some(&similar), true).await;
        add(
            &fixture,
            RELEASE,
            13,
            Some("unrelated"),
            Some("A notice about street sweeping schedules."),
            true,
        )
        .await;
        // Another hand-added copy of the same bytes is not something a source served.
        add(&fixture, OTHER_UPLOAD, 20, Some("bytes"), Some(&text), true).await;

        let report = describe_origin(&fixture.repository, &fixture.index, 1, UPLOAD)
            .await
            .unwrap();
        assert_eq!(report.origin, DocumentOrigin::AddedByUser);
        let relation = report.held_relation.expect("a hand-added file is compared");
        let by_id = |held: &HeldDocumentMatch| held.document_id;
        assert_eq!(listed(&relation.exact_copies, by_id), vec![10]);
        assert_eq!(listed(&relation.same_text, by_id), vec![11]);
        assert_eq!(
            listed(&relation.near_duplicates, |near| near.document.document_id),
            vec![12]
        );
        let HeldComparison::Compared { matches, .. } = &relation.exact_copies else {
            unreachable!()
        };
        assert_eq!(
            (
                matches[0].source_name.as_str(),
                matches[0].request_label.as_deref(),
                matches[0].file_name.as_deref()
            ),
            ("City records portal", Some("24-100"), Some("file-10.pdf"))
        );

        let collected = describe_origin(&fixture.repository, &fixture.index, 10, RELEASE)
            .await
            .unwrap();
        assert_eq!(collected.origin, DocumentOrigin::CollectedFromSource);
        assert_eq!(collected.held_relation, None);
    }

    #[tokio::test]
    async fn a_comparison_that_could_not_run_is_not_reported_as_no_match() {
        let fixture = fixture().await;
        add(
            &fixture,
            RELEASE,
            10,
            Some("held"),
            Some("Held text."),
            true,
        )
        .await;
        add(&fixture, UPLOAD, 1, None, None, false).await;
        add(
            &fixture,
            OTHER_UPLOAD,
            2,
            Some("mine"),
            Some("Text nobody else holds."),
            false,
        )
        .await;

        let unprocessed = describe_origin(&fixture.repository, &fixture.index, 1, UPLOAD)
            .await
            .unwrap()
            .held_relation
            .unwrap();
        assert_eq!(
            unprocessed.exact_copies,
            not_compared(NotComparedReason::FileDigestNotRecorded)
        );
        assert_eq!(
            unprocessed.same_text,
            not_compared(NotComparedReason::TextNotExtracted)
        );
        assert_eq!(
            unprocessed.near_duplicates,
            not_compared(NotComparedReason::TextNotExtracted)
        );

        let describe = || describe_origin(&fixture.repository, &fixture.index, 2, OTHER_UPLOAD);
        let unindexed = describe().await.unwrap().held_relation.unwrap();
        let nothing_among_one = HeldComparison::Compared {
            matches: Vec::new(),
            truncated: false,
            compared_count: 1,
        };
        assert_eq!(unindexed.exact_copies, nothing_among_one);
        assert_eq!(unindexed.same_text, nothing_among_one);
        assert_eq!(
            unindexed.near_duplicates,
            not_compared(NotComparedReason::NotInSimilarityIndex)
        );

        fixture
            .index
            .index_document(CorpusDocumentInput {
                portal_key: OTHER_UPLOAD.to_string(),
                request_id: 7,
                document_id: 2,
                raw_file_sha256: sha256_hex(b"mine"),
                extracted_text_sha256: sha256_hex(b"Text nobody else holds."),
                text: "Text nobody else holds.".to_string(),
            })
            .await
            .unwrap();
        assert_eq!(
            describe()
                .await
                .unwrap()
                .held_relation
                .unwrap()
                .near_duplicates,
            HeldComparison::Compared {
                matches: Vec::new(),
                truncated: false,
                compared_count: 1,
            }
        );
    }

    #[tokio::test]
    async fn a_file_that_cannot_be_parsed_is_not_reported_as_recording_nothing() {
        let fixture = fixture().await;
        let conn = fixture.repository.analytics_connection().await.unwrap();
        let store = |id: u64, name: &str, bytes: &[u8]| {
            let path = fixture.root.join(name);
            std::fs::write(&path, bytes).unwrap();
            conn.execute(
                "INSERT INTO documents (id, request_id, portal_subdomain, file_name, visibility,
                    created_at, updated_at, local_path)
                 VALUES (?1, 7, ?2, ?3, 'public', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', ?4)",
                duckdb::params![id, UPLOAD, name, path.to_string_lossy()],
            )
            .unwrap();
        };
        let mut package = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut package));
            let options: zip::write::FileOptions<()> = zip::write::FileOptions::default();
            zip.start_file("docProps/core.xml", options).unwrap();
            zip.write_all(b"<cp:lastModifiedBy>J. Clerk</cp:lastModifiedBy>")
                .unwrap();
            zip.finish().unwrap();
        }
        store(1, "memo.docx", &package);
        store(2, "broken.pdf", b"not a pdf at all");
        store(3, "broken.docx", b"not a package at all");
        store(4, "notes.txt", b"plain text");

        let read = |id| read_internal_metadata(&fixture.repository, &fixture.root, id, UPLOAD);
        let DocumentInternalMetadata::Read { format, fields } = read(1).await.unwrap() else {
            panic!("a package with docProps must be read");
        };
        assert_eq!(format, InternalMetadataFormat::Ooxml);
        let value = |name: &str| {
            fields
                .iter()
                .find(|field| field.name == name)
                .unwrap()
                .value
                .clone()
        };
        assert_eq!(value("Last modified by").as_deref(), Some("J. Clerk"));
        assert_eq!(value("Created"), None);

        let not_read = |reason| DocumentInternalMetadata::NotRead { reason };
        assert_eq!(
            read(2).await.unwrap(),
            not_read(InternalMetadataUnreadReason::Unreadable)
        );
        assert_eq!(
            read(3).await.unwrap(),
            not_read(InternalMetadataUnreadReason::Unreadable)
        );
        assert_eq!(
            read(4).await.unwrap(),
            not_read(InternalMetadataUnreadReason::UnsupportedFormat)
        );
        std::fs::remove_file(fixture.root.join("memo.docx")).unwrap();
        assert_eq!(
            read(1).await.unwrap(),
            not_read(InternalMetadataUnreadReason::FileUnavailable)
        );
    }
}
