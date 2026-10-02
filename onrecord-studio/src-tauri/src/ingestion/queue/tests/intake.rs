use super::super::{extraction::archive_entry_id, intake::stable_source_identifier};
use super::*;

// LIMIT(protocol): the two synthetic identifier generators are NOT disjoint. Both set
// SYNTHETIC_DOCUMENT_NAMESPACE as the high bit, but `stable_source_identifier` masks the
// remainder to 51 bits while `archive_entry_id` masks to 52. The crawled-source span is a
// strict subset of the archive-entry span, so a crawled page and an archive entry can be
// handed the same `documents.id`.
//
// Pinned rather than fixed, deliberately. `documents(id, portal_subdomain)` is a primary key
// that `provenance_manifests` and the corpus tables carry as a foreign key, so re-masking
// either generator orphans every already-ingested document and its provenance manifest: a
// migration, not a one-line mask change. It would also buy very little. Both values are
// truncated SHA-256, so collisions *within* either span already arrive at roughly 2^26
// documents by the birthday bound -- setting a partition bit buys one bit against a limit
// that is two dozen bits lower. Widening the identifier past 53 bits, which means giving up
// exact representability in JavaScript, is the fix that would actually matter.
#[test]
fn synthetic_identifier_spans_overlap_rather_than_partition() {
    let source_span_end = SYNTHETIC_DOCUMENT_NAMESPACE | SYNTHETIC_SOURCE_IDENTIFIER_MASK;
    let digest = "b".repeat(64);

    // Crawled sources occupy only the lower half of the namespace: [2^52, 2^52 + 2^51).
    for value in [
        "page|https://records.example.gov/a",
        "local|/evidence/one.pdf",
        "pasted|abc",
    ] {
        let identifier = stable_source_identifier(value);
        assert!(identifier >= SYNTHETIC_DOCUMENT_NAMESPACE);
        assert!(identifier <= source_span_end);
    }

    // Archive entries occupy the whole namespace, so about half of them land inside the span
    // above. Finding one is the collision surface, stated as an assertion rather than prose.
    let overlapping = (0..64_u64)
        .map(|index| archive_entry_id(index, &digest, 1, Path::new("evidence.txt")))
        .find(|identifier| *identifier <= source_span_end);
    assert!(
        overlapping.is_some(),
        "archive entry identifiers reach into the crawled-source span",
    );
    assert!(overlapping.unwrap() >= SYNTHETIC_DOCUMENT_NAMESPACE);
    assert!(overlapping.unwrap() < (1_u64 << 53));
}
