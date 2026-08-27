//! Provenance evidence types, and the confidence ladder that separates what this application has
//! *recorded* from what it has *verified*.
//!
//! Every structure in this module other than [`ProvenanceConfidence`] describes claims read out of
//! local storage. A manifest row saying an asset is signed is a statement someone made, not a
//! signature this application checked. The distinction is the whole point of the module: an
//! investigator citing a document in a filing needs to know which of the two they are holding.

use serde::{Deserialize, Serialize};

/// How much a provenance result is actually worth as evidence.
///
/// The ladder is ordered by the strength of the evidence behind each state, and every state names
/// the check that produced it. The top state cannot be produced by this build: it needs a
/// configured trust anchor, and nothing here supplies one or offers anywhere to configure one.
/// That is a deployment fact rather than a property of any document.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProvenanceConfidence {
    /// There is no readable evidence file to check: none was ever recorded, the recorded path no
    /// longer resolves to a regular file, or the path failed the managed-evidence-store gate.
    ///
    /// Weakest state, and deliberately not an error. "Checked at this time, no readable evidence"
    /// is exactly the fact an investigator needs on the record, so it is verified and logged like
    /// any other outcome. The specific reason is in `verification_errors`.
    EvidenceFileUnavailable,
    /// The bytes on disk do not hash to the digest recorded for them. Either the file changed
    /// after ingestion or the wrong file is stored under this identifier.
    DigestMismatch,
    /// No digest was recorded for this document, so nothing about the stored bytes can be
    /// confirmed.
    NoRecordedDigest,
    /// The asset carries an embedded manifest, but nothing about the stored bytes was confirmed:
    /// the manifest failed validation, and no ingestion digest was recorded to fall back on.
    ///
    /// Ranked above [`Self::NoRecordedDigest`] because metadata on file is more to work with than
    /// none, and below [`Self::LocalDigestMatch`] because none of it has been checked. The
    /// accompanying `verification_errors` carry the SDK's validation statuses, which are the part
    /// worth reading.
    RecordedClaimOnly,
    /// The stored bytes hash to the recorded digest.
    ///
    /// This is local integrity: it shows the file has not changed since this application wrote it
    /// down. It is not provenance. It says nothing about who produced the file, whether the agency
    /// released it unaltered, or whether any manifest attached to it is genuine.
    LocalDigestMatch,
    /// A cryptographic signature over the asset was verified against the signer's certificate.
    ///
    /// Produced by [`crate::provenance::classify_confidence`] when the C2PA SDK validates an
    /// embedded manifest over the asset's hard binding. It is evidence that the asset matches the
    /// manifest signed over it, and nothing more: it does not establish who the signer is, because
    /// the certificate chains to no independently trusted anchor at this rung.
    VerifiedSignature,
    /// A verified signature chains to an independently trusted anchor, making the result
    /// third-party evidence of custody rather than a local assertion.
    ///
    /// **This build cannot produce it.** The SDK evaluates the signer against a trust list, but
    /// its default anchor set is empty and this application neither supplies anchors nor exposes
    /// anywhere to configure them, so validation tops out at [`Self::VerifiedSignature`]. The
    /// variant is kept deliberately: trust anchors are planned, the work is a settings surface
    /// feeding `Settings.trust` through the `Context` built in
    /// [`crate::provenance::read_asset_provenance`], and this is the only rung that is independent
    /// third-party evidence rather than this application's own bookkeeping. It is also still read
    /// back out of the verification log, which is append-only.
    TrustedChainOfCustody,
}

impl ProvenanceConfidence {
    /// Whether the state rests on a cryptographic check this application actually performed.
    pub const fn is_cryptographically_verified(self) -> bool {
        matches!(self, Self::VerifiedSignature | Self::TrustedChainOfCustody)
    }

    /// Whether the state may be presented as independent evidence of chain of custody.
    pub const fn is_independent_custody_evidence(self) -> bool {
        matches!(self, Self::TrustedChainOfCustody)
    }

    /// Parses the form written to the verification log.
    ///
    /// Returns `None` for anything else, including the `UNCLASSIFIED_LEGACY_RECORD` sentinel that
    /// rows written before classification existed carry. A caller must surface that as "not
    /// recorded" rather than substituting a level the check never produced.
    pub fn from_stored(value: &str) -> Option<Self> {
        [
            Self::EvidenceFileUnavailable,
            Self::DigestMismatch,
            Self::NoRecordedDigest,
            Self::RecordedClaimOnly,
            Self::LocalDigestMatch,
            Self::VerifiedSignature,
            Self::TrustedChainOfCustody,
        ]
        .into_iter()
        .find(|candidate| candidate.as_str() == value)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::EvidenceFileUnavailable => "EVIDENCE_FILE_UNAVAILABLE",
            Self::DigestMismatch => "DIGEST_MISMATCH",
            Self::NoRecordedDigest => "NO_RECORDED_DIGEST",
            Self::RecordedClaimOnly => "RECORDED_CLAIM_ONLY",
            Self::LocalDigestMatch => "LOCAL_DIGEST_MATCH",
            Self::VerifiedSignature => "VERIFIED_SIGNATURE",
            Self::TrustedChainOfCustody => "TRUSTED_CHAIN_OF_CUSTODY",
        }
    }
}

/// Whether a signature was checked, and if not, why not.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SignatureVerificationState {
    /// No signature validation was performed, because the asset carries no manifest the SDK could
    /// read. Not a statement that a signature is absent from the original file.
    NotAttempted,
    /// A signature was checked and verifies over the asset.
    Valid,
    /// A signature was checked and does not hold: the manifest is unparseable, or its
    /// cryptographic integrity check failed. A finding, not an absence.
    Invalid,
}

impl SignatureVerificationState {
    /// Parses the form written to the verification log. See
    /// [`ProvenanceConfidence::from_stored`] for why unknown values are `None`.
    pub fn from_stored(value: &str) -> Option<Self> {
        [Self::NotAttempted, Self::Valid, Self::Invalid]
            .into_iter()
            .find(|candidate| candidate.as_str() == value)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotAttempted => "NOT_ATTEMPTED",
            Self::Valid => "VALID",
            Self::Invalid => "INVALID",
        }
    }
}

/// Which recorded digest a verification compared the stored bytes against.
///
/// The two are not equivalent evidence. An ingestion digest is this application's own record of
/// what it downloaded; a manifest digest is a third party's statement about the asset. Matching
/// the former proves only that local storage is intact.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RecordedDigestSource {
    /// No digest is recorded for the document.
    None,
    /// The digest recorded in a provenance manifest.
    ProvenanceManifest,
    /// The digest this application computed when it ingested the file.
    IngestionRecord,
}

impl RecordedDigestSource {
    /// Parses the form written to the verification log. See
    /// [`ProvenanceConfidence::from_stored`] for why unknown values are `None`.
    pub fn from_stored(value: &str) -> Option<Self> {
        [Self::None, Self::ProvenanceManifest, Self::IngestionRecord]
            .into_iter()
            .find(|candidate| candidate.as_str() == value)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "NONE",
            Self::ProvenanceManifest => "PROVENANCE_MANIFEST",
            Self::IngestionRecord => "INGESTION_RECORD",
        }
    }
}

/// Manifest metadata as recorded locally. Every field is a stored claim, not a checked result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvenanceManifest {
    pub document_id: u64,
    pub file_hash_sha256: String,
    /// Whether the manifest row *claims* a cryptographic signature. This is a stored claim, so a
    /// true value here is not evidence a signature verified; that lives on the verification
    /// result's `signature_verification`, which comes from validating the bytes on disk.
    pub is_cryptographically_signed: bool,
    /// Issuer named by the manifest row. Not authenticated against any certificate or trust list.
    pub signature_issuer: Option<String>,
    /// Signing time named by the manifest. Not a trusted timestamp.
    pub signing_timestamp: Option<String>,
    pub assertions: Vec<C2pAssertion>,
}

/// A single recorded assertion. As with the manifest, the fields are claims held locally.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct C2pAssertion {
    pub assertion_type: String, // e.g., "c2pa.actions", "c2pa.metadata"
    pub actor_identifier: String,
    /// Whether the assertion was recorded as tamper-evident. Not re-derived from the asset.
    pub is_tamper_evident: bool,
    pub validation_errors: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_trusted_anchor_earns_the_independent_custody_claim() {
        // The states below a verified signature must never read as cryptographic evidence, and a
        // verified signature must never read as independent custody evidence: the certificate
        // behind it chains to no anchor this build knows.
        for reachable in [
            ProvenanceConfidence::EvidenceFileUnavailable,
            ProvenanceConfidence::DigestMismatch,
            ProvenanceConfidence::NoRecordedDigest,
            ProvenanceConfidence::RecordedClaimOnly,
            ProvenanceConfidence::LocalDigestMatch,
        ] {
            assert!(!reachable.is_cryptographically_verified());
            assert!(!reachable.is_independent_custody_evidence());
        }
        assert!(ProvenanceConfidence::VerifiedSignature.is_cryptographically_verified());
        assert!(!ProvenanceConfidence::VerifiedSignature.is_independent_custody_evidence());
        assert!(ProvenanceConfidence::TrustedChainOfCustody.is_independent_custody_evidence());
    }

    #[test]
    fn stored_classifications_round_trip_and_reject_the_legacy_sentinel() {
        for confidence in [
            ProvenanceConfidence::EvidenceFileUnavailable,
            ProvenanceConfidence::DigestMismatch,
            ProvenanceConfidence::NoRecordedDigest,
            ProvenanceConfidence::RecordedClaimOnly,
            ProvenanceConfidence::LocalDigestMatch,
            ProvenanceConfidence::VerifiedSignature,
            ProvenanceConfidence::TrustedChainOfCustody,
        ] {
            assert_eq!(
                ProvenanceConfidence::from_stored(confidence.as_str()),
                Some(confidence)
            );
        }
        // A row written before classification existed must read back as "not recorded", never as
        // a level the check never produced.
        assert_eq!(
            ProvenanceConfidence::from_stored("UNCLASSIFIED_LEGACY_RECORD"),
            None
        );
        assert_eq!(
            SignatureVerificationState::from_stored("NOT_ATTEMPTED"),
            Some(SignatureVerificationState::NotAttempted)
        );
        assert_eq!(
            RecordedDigestSource::from_stored("INGESTION_RECORD"),
            Some(RecordedDigestSource::IngestionRecord)
        );
        assert_eq!(
            RecordedDigestSource::from_stored("UNCLASSIFIED_LEGACY_RECORD"),
            None
        );
    }

    #[test]
    fn confidence_ladder_orders_local_integrity_below_cryptographic_evidence() {
        assert!(
            ProvenanceConfidence::DigestMismatch > ProvenanceConfidence::EvidenceFileUnavailable
        );
        assert!(ProvenanceConfidence::LocalDigestMatch > ProvenanceConfidence::RecordedClaimOnly);
        assert!(ProvenanceConfidence::VerifiedSignature > ProvenanceConfidence::LocalDigestMatch);
        assert!(
            ProvenanceConfidence::TrustedChainOfCustody > ProvenanceConfidence::VerifiedSignature
        );
    }
}
