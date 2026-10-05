//! Email-thread-gap detector.
//!
//! Fires when a released `.msg` message names — through `In-Reply-To` or `References` — a
//! Message-ID that is not itself in the released set. Interpretation: the requester was given a
//! selection from a chain, not the whole chain. The missing message is not necessarily privileged
//! (some are legitimately outside scope); a missing internal message deep in a chain is a much
//! stronger signal than a missing initial ancestor that names a well-known listserv.
//!
//! Per-request scoping: gaps are computed against the set of `.msg` documents released against
//! the same request. A message referring to a Message-ID that is present in some *other*
//! request's release is still a gap for *this* request — the requester was not given it.
//!
//! Severity:
//! - `Notable`: one gap per request.
//! - `Serious`: three or more gaps in one request.
//! - `Critical`: a mid-thread gap — the missing Message-ID sits between two present messages
//!   (both a present message references it, and a present message names it as its parent's
//!   parent, or references it via a chain path). That's a hole cut out of the middle of the
//!   thread rather than a chain that opens outside the release.
//!
//! Confidence is a function of the missing id's shape: an obviously listserv-shaped id lowers
//! confidence (the sender may have joined a public list rather than routed a private mail); an
//! internal-looking id raises it.
//!
//! **Bounded**: reads at most `MAX_MSG_DOCUMENTS` per invocation.

use chrono::Utc;
use duckdb::params;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use crate::analysis::finding::{
    finding_id, Finding, FindingCitation, FindingConfidence, FindingKind, FindingSeverity,
    FindingSubject,
};
use crate::error::AppError;
use crate::parsers::email_headers::{looks_like_listserv, read_msg_header_chain, EmailHeaderChain};
use crate::storage::DuckDbRepository;

pub const SLUG: &str = "email_thread_gap";

const MAX_MSG_DOCUMENTS: i64 = 2_000;

struct MsgDocument {
    document_id: i64,
    portal_key: String,
    request_id: i64,
    file_name: Option<String>,
    local_path: String,
}

struct MsgObservation {
    document: MsgDocument,
    chain: EmailHeaderChain,
}

pub async fn run(repository: &DuckDbRepository) -> Result<Vec<Finding>, AppError> {
    let documents = load_msg_documents(repository).await?;
    let observations: Vec<MsgObservation> = documents
        .into_iter()
        .map(|document| {
            let chain =
                read_msg_header_chain(&PathBuf::from(&document.local_path)).unwrap_or_default();
            MsgObservation { document, chain }
        })
        .filter(|obs| !obs.chain.is_empty())
        .collect();

    // Group per (portal_key, request_id). A gap is only meaningful against the co-released set.
    let mut per_request: BTreeMap<(String, i64), Vec<MsgObservation>> = BTreeMap::new();
    for observation in observations {
        per_request
            .entry((
                observation.document.portal_key.clone(),
                observation.document.request_id,
            ))
            .or_default()
            .push(observation);
    }

    let mut findings = Vec::new();
    for ((portal_key, request_id), group) in per_request {
        findings.extend(scan_request(&portal_key, request_id, &group));
    }
    Ok(findings)
}

async fn load_msg_documents(repository: &DuckDbRepository) -> Result<Vec<MsgDocument>, AppError> {
    repository
        .query_connection("Email-thread-gap MSG document query", move |conn| {
            let mut statement = conn
                .prepare(
                    "SELECT d.id, d.portal_subdomain, d.request_id, d.file_name, d.local_path
                     FROM documents d
                     WHERE d.local_path IS NOT NULL
                       AND LOWER(TRIM(d.visibility)) = 'public'
                       AND d.portal_subdomain NOT IN (SELECT subdomain FROM user_supplied_sources)
                       AND (LOWER(COALESCE(d.content_type, '')) LIKE 'application/vnd.ms-outlook%'
                            OR LOWER(COALESCE(d.file_name, '')) LIKE '%.msg')
                     ORDER BY d.request_id
                     LIMIT ?1",
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to prepare MSG document query: {error}"))
                })?;
            let rows = statement
                .query_map(params![MAX_MSG_DOCUMENTS], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                })
                .map_err(|error| {
                    AppError::Storage(format!("Failed to run MSG document query: {error}"))
                })?;
            let mut docs = Vec::new();
            for row in rows {
                let (document_id, portal_key, request_id, file_name, local_path) =
                    row.map_err(|error| {
                        AppError::Storage(format!("Failed to decode MSG document row: {error}"))
                    })?;
                docs.push(MsgDocument {
                    document_id,
                    portal_key,
                    request_id,
                    file_name,
                    local_path,
                });
            }
            Ok(docs)
        })
        .await
}

/// A missing id is a genuine hole cut from the middle of the thread — not simply the point where
/// the release happens to start — only when the *same* message's own ordered `References` header
/// shows a present ancestor older than it. `References` accumulates oldest-first with the
/// immediate parent last (RFC 5322 §3.6.4), so "the missing id has a present ancestor" means the
/// entry immediately *before* it in that list is itself in the released set. Checking only "does
/// some other message also name this id" (the prior implementation) is wrong: every descendant of
/// an unreleased root necessarily repeats that root's id in its own accumulated `References`, so
/// that test fires on the ordinary "the released chain opens outside the release" case as soon as
/// the release has more than one descendant of the missing root — which is the common case, not a
/// deletion from the middle. The message itself being present already establishes a present
/// descendant, so only the ancestor side needs checking here.
fn is_interior_gap(
    missing_id: &str,
    chain: &EmailHeaderChain,
    present: &std::collections::BTreeSet<&str>,
) -> bool {
    match chain.references.iter().position(|r| r == missing_id) {
        Some(position) if position > 0 => present.contains(chain.references[position - 1].as_str()),
        _ => false,
    }
}

fn scan_request(portal_key: &str, request_id: i64, group: &[MsgObservation]) -> Vec<Finding> {
    // The set of Message-IDs present in this release. Only case-folded lowered ids, matching
    // the parser's own normalization.
    let present: BTreeSet<&str> = group
        .iter()
        .filter_map(|obs| obs.chain.message_id.as_deref())
        .collect();

    // For each observation, collect the referenced ids that aren't present. Track per-missing-id
    // the first observation that pointed at it, and note whether it sits mid-thread.
    struct Gap<'a> {
        missing_id: String,
        via: &'a MsgObservation,
        mid_thread: bool,
    }
    let mut gaps: Vec<Gap<'_>> = Vec::new();
    for observation in group {
        let referenced = observation.chain.all_referenced_ids();
        for id in referenced {
            if present.contains(id.as_str()) {
                continue;
            }
            let mid_thread = is_interior_gap(&id, &observation.chain, &present);
            gaps.push(Gap {
                missing_id: id,
                via: observation,
                mid_thread,
            });
        }
    }

    if gaps.is_empty() {
        return Vec::new();
    }

    // Deduplicate gaps by missing_id: one finding per missing id, not one per pointer.
    let mut by_missing: BTreeMap<String, Vec<&Gap<'_>>> = BTreeMap::new();
    for gap in &gaps {
        by_missing
            .entry(gap.missing_id.clone())
            .or_default()
            .push(gap);
    }
    let distinct_gap_count = by_missing.len();

    let mut findings = Vec::new();
    for (missing_id, pointers) in by_missing {
        let listserv = looks_like_listserv(&missing_id);
        let mid_thread = pointers.iter().any(|g| g.mid_thread);

        let severity = if mid_thread {
            FindingSeverity::Critical
        } else if distinct_gap_count >= 3 {
            FindingSeverity::Serious
        } else {
            FindingSeverity::Notable
        };

        // Confidence: mid-thread + internal-looking pushes toward the top; a single tail-end
        // listserv gap sits toward the bottom. Numbers are bounded interpretive weights, not
        // measurements — labeled accordingly through the Finding's confidence field.
        let mut basis_points: u16 = 5000;
        if mid_thread {
            basis_points = basis_points.saturating_add(2000);
        }
        if !listserv {
            basis_points = basis_points.saturating_add(1500);
        }
        if listserv {
            basis_points = basis_points.saturating_sub(3000);
        }
        let confidence = FindingConfidence::measured(basis_points.min(10_000));

        // Subject anchors at the referencing message (there may be several; we pick the first
        // as the anchor for the row's subject fields, and cite the rest as additional citations).
        let anchor = pointers[0].via;
        let subject = FindingSubject {
            portal_key: Some(anchor.document.portal_key.clone()),
            request_id: Some(anchor.document.request_id),
            document_id: Some(anchor.document.document_id),
            ..Default::default()
        };
        let salient = format!(
            "missing_id={missing_id};mid={mid_thread};gaps_in_request={distinct_gap_count}"
        );
        let id = finding_id(FindingKind::EmailThreadGap, SLUG, &subject, &[&salient]);

        let anchor_name = anchor
            .document
            .file_name
            .clone()
            .unwrap_or_else(|| format!("message {}", anchor.document.document_id));
        let headline = if mid_thread {
            format!("Mid-thread email missing from release ({missing_id})")
        } else {
            format!("Released email references unreleased Message-ID {missing_id}")
        };
        let listserv_note = if listserv {
            " The missing id resembles a public list-server address; the gap may be a subscription artifact rather than a withheld internal message."
        } else {
            ""
        };
        let mid_thread_note = if mid_thread {
            format!(
                " {} additional released message{plural} in this request also reference {missing_id}, so the gap sits between messages the requester was given.",
                pointers.iter().map(|g| if g.mid_thread { 1 } else { 0 }).sum::<usize>().max(1),
                plural = if pointers.len() == 1 { "" } else { "s" }
            )
        } else {
            String::new()
        };
        let explanation = format!(
            "\"{anchor_name}\" (request {request_id}, portal {portal_key}) names {missing_id} \
             through its In-Reply-To or References header, but no released document in this \
             request carries that Message-ID.{mid_thread_note}{listserv_note}"
        );

        let mut citations = Vec::with_capacity(pointers.len().min(5));
        for pointer in pointers.iter().take(5) {
            let via_name = pointer
                .via
                .document
                .file_name
                .clone()
                .unwrap_or_else(|| format!("message {}", pointer.via.document.document_id));
            citations.push(FindingCitation {
                portal_key: pointer.via.document.portal_key.clone(),
                document_id: pointer.via.document.document_id,
                character_start: None,
                character_end: None,
                quote: None,
                label: Some(format!("{via_name} references {missing_id}")),
            });
        }
        let now = Utc::now();
        findings.push(Finding {
            id,
            kind: FindingKind::EmailThreadGap,
            severity,
            confidence,
            subject,
            headline,
            explanation,
            citations,
            discovered_at: now,
            last_seen_at: now,
            dismissed_at: None,
            source_detector: SLUG.into(),
        });
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(
        id: i64,
        message_id: Option<&str>,
        in_reply_to: Option<&str>,
        refs: &[&str],
    ) -> MsgObservation {
        MsgObservation {
            document: MsgDocument {
                document_id: id,
                portal_key: "test".into(),
                request_id: 1,
                file_name: Some(format!("msg{id}.msg")),
                local_path: format!("/tmp/msg{id}.msg"),
            },
            chain: EmailHeaderChain {
                message_id: message_id.map(|s| s.to_string()),
                in_reply_to: in_reply_to.map(|s| s.to_string()),
                references: refs.iter().map(|s| s.to_string()).collect(),
            },
        }
    }

    #[test]
    fn no_findings_when_every_referenced_id_is_present_in_the_release() {
        let group = vec![
            observation(1, Some("a@x"), None, &[]),
            observation(2, Some("b@x"), Some("a@x"), &["a@x"]),
            observation(3, Some("c@x"), Some("b@x"), &["a@x", "b@x"]),
        ];
        let findings = scan_request("test", 1, &group);
        assert!(findings.is_empty(), "chain is closed, no gaps");
    }

    #[test]
    fn notable_finding_when_one_message_references_a_missing_ancestor() {
        let group = vec![observation(2, Some("b@x"), Some("a@x"), &["a@x"])];
        let findings = scan_request("test", 1, &group);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].kind, FindingKind::EmailThreadGap);
        assert_eq!(findings[0].severity, FindingSeverity::Notable);
    }

    #[test]
    fn serious_finding_when_three_or_more_distinct_gaps_exist_in_one_request() {
        let group = vec![
            observation(2, Some("b@x"), Some("a@x"), &["a@x"]),
            observation(3, Some("c@x"), Some("missing2@x"), &["missing2@x"]),
            observation(4, Some("d@x"), Some("missing3@x"), &["missing3@x"]),
        ];
        let findings = scan_request("test", 1, &group);
        assert_eq!(findings.len(), 3);
        for finding in &findings {
            assert_eq!(finding.severity, FindingSeverity::Serious);
        }
    }

    #[test]
    fn critical_finding_when_the_missing_id_sits_between_two_present_messages() {
        // Chain a -> b -> c -> d -> e; b and d are missing, a/c/e are present. Both are genuine
        // interior holes: c's own References shows a (present) immediately before b, and e's own
        // References shows c (present) immediately before d — each missing id has a present
        // ancestor on one side and a present descendant (the referencing message itself) on the
        // other.
        let group = vec![
            observation(1, Some("a@x"), None, &[]),
            observation(3, Some("c@x"), Some("b@x"), &["a@x", "b@x"]),
            observation(5, Some("e@x"), Some("d@x"), &["a@x", "b@x", "c@x", "d@x"]),
        ];
        let findings = scan_request("test", 1, &group);
        assert_eq!(findings.len(), 2);
        let critical_count = findings
            .iter()
            .filter(|f| f.severity == FindingSeverity::Critical)
            .count();
        assert_eq!(
            critical_count, 2,
            "both b@x and d@x sit between a present ancestor and a present descendant"
        );
    }

    /// Locks the actual bug fix: every descendant of an unreleased *root* necessarily repeats
    /// that root's id in its own accumulated References header (RFC 5322 References accumulates
    /// the whole ancestor chain, not just the immediate parent), so "some other message also
    /// names this id" is satisfied by the ordinary "the release opens after this point" case as
    /// soon as more than one descendant is released — it must never read as Critical on its own.
    #[test]
    fn not_critical_when_the_missing_id_is_only_the_unreleased_root_the_release_opens_after() {
        let group = vec![
            observation(1, Some("a@x"), Some("x@x"), &["x@x"]),
            observation(2, Some("b@x"), Some("a@x"), &["x@x", "a@x"]),
            observation(3, Some("c@x"), Some("b@x"), &["x@x", "a@x", "b@x"]),
        ];
        let findings = scan_request("test", 1, &group);
        assert_eq!(
            findings.len(),
            1,
            "one missing id (x@x) -> one finding, deduplicated"
        );
        assert_eq!(
            findings[0].severity,
            FindingSeverity::Notable,
            "x@x is the release's unreleased root, not a hole cut from the middle, no matter how \
             many released descendants repeat it in their own References headers"
        );
    }

    #[test]
    fn listserv_gap_gets_reduced_confidence() {
        let group_listserv = vec![observation(
            1,
            Some("a@x"),
            Some("mailman.99.list@lists.example.org"),
            &[],
        )];
        let group_internal = vec![observation(
            2,
            Some("b@x"),
            Some("internal-thread@department.city.gov"),
            &[],
        )];
        let listserv_finding = scan_request("test", 1, &group_listserv).pop().unwrap();
        let internal_finding = scan_request("test", 1, &group_internal).pop().unwrap();
        match (listserv_finding.confidence, internal_finding.confidence) {
            (
                FindingConfidence::Measured {
                    basis_points: bp_list,
                },
                FindingConfidence::Measured {
                    basis_points: bp_int,
                },
            ) => assert!(
                bp_int > bp_list,
                "internal gap ({bp_int}) must outrank listserv gap ({bp_list})"
            ),
            _ => panic!("both must be measured"),
        }
    }

    #[test]
    fn finding_id_stable_across_re_runs() {
        let group = vec![observation(2, Some("b@x"), Some("a@x"), &["a@x"])];
        let a = scan_request("test", 1, &group);
        let b = scan_request("test", 1, &group);
        assert_eq!(a[0].id, b[0].id);
    }

    #[test]
    fn deduplicates_gaps_by_missing_id_so_multiple_pointers_produce_one_finding() {
        let group = vec![
            observation(2, Some("b@x"), Some("missing@x"), &["missing@x"]),
            observation(3, Some("c@x"), Some("missing@x"), &["missing@x"]),
        ];
        let findings = scan_request("test", 1, &group);
        assert_eq!(findings.len(), 1, "one missing id -> one finding");
    }
}
