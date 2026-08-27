//! C2PA manifest reading, validation, and the rule that turns a validation result into a
//! confidence level.
//!
//! # Verification reads local bytes only
//!
//! The `fetch_remote_manifests` feature is deliberately not enabled (see `Cargo.toml`). With it on,
//! validating an asset follows a URL embedded in that asset, which for an evidence tool means the
//! act of examining a document sends a request to a server the document's author chose. That is
//! both a disclosure of what an investigator is looking at and an input the subject of an
//! investigation controls. Everything here is derived from bytes already on disk.
//!
//! # What a validation result is worth
//!
//! [`classify_confidence`] is the whole trust rule and is deliberately pure, so the decisions it
//! encodes can be tested exhaustively without needing a signed fixture for every branch.
//! `Valid` and `Invalid` are additionally exercised end to end against real signed bytes by
//! `a_signed_asset_verifies_and_a_tampered_one_reports_an_invalid_manifest`, which mints a
//! throwaway certificate chain at test time rather than committing a fixture that would expire.
//! `Trusted` has no such test because this build cannot produce it: see
//! [`crate::types::c2pa::ProvenanceConfidence::TrustedChainOfCustody`].

use crate::error::AppError;
use crate::types::{ProvenanceConfidence, SignatureVerificationState};
use std::path::Path;

/// The strongest statement the C2PA SDK made about an asset's manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestValidation {
    /// A manifest is present but cannot be parsed, or its cryptographic integrity check failed.
    /// This is a finding, not an absence: something claimed provenance and the claim does not hold.
    Invalid,
    /// Well-formed, and the signature verifies over the asset's hard binding.
    Valid,
    /// Valid, and the signing certificate chains to a configured trust anchor.
    ///
    /// Unreachable without configuration. The SDK verifies against a trust list, but its default
    /// anchor set is empty, so validation tops out at [`Self::Valid`] until an operator supplies
    /// one. That is a deployment fact, not a property of a document.
    Trusted,
}

/// One assertion recorded in a manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetAssertion {
    pub assertion_type: String,
    /// Who asserted it. C2PA binds assertions to the manifest's signer rather than labelling each
    /// one individually, so this is the claim generator or signing issuer, not a per-assertion
    /// identity.
    pub actor_identifier: String,
    /// Whether tampering with this assertion would be detectable.
    ///
    /// C2PA hashes every assertion into the signed claim, so this is a property of the manifest's
    /// validation state rather than of the assertion: in a valid manifest every assertion is
    /// covered, and in an invalid one no assertion can be relied on.
    pub is_tamper_evident: bool,
}

/// What reading an asset's embedded provenance found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AssetProvenance {
    pub manifest_present: bool,
    pub validation: Option<ManifestValidation>,
    pub signature_issuer: Option<String>,
    pub signing_timestamp: Option<String>,
    pub claim_generator: Option<String>,
    pub assertions: Vec<AssetAssertion>,
    /// Human-readable findings to surface on the verification result.
    pub findings: Vec<String>,
}

impl AssetProvenance {
    /// Whether a signature was actually checked, and what the check said.
    pub fn signature_verification(&self) -> SignatureVerificationState {
        match self.validation {
            Some(ManifestValidation::Valid | ManifestValidation::Trusted) => {
                SignatureVerificationState::Valid
            }
            Some(ManifestValidation::Invalid) => SignatureVerificationState::Invalid,
            None => SignatureVerificationState::NotAttempted,
        }
    }

    /// Whether the asset carries a signature claim at all.
    pub fn claims_signature(&self) -> bool {
        self.signature_issuer.is_some()
            || matches!(
                self.validation,
                Some(ManifestValidation::Valid | ManifestValidation::Trusted)
            )
    }

    pub fn invalid_assertion_count(&self) -> usize {
        self.assertions
            .iter()
            .filter(|assertion| !assertion.is_tamper_evident)
            .count()
    }
}

/// Turns everything known about a document into the one level it has earned.
///
/// The ordering of the checks is the substance:
///
/// 1. Nothing readable to check outranks every other consideration.
/// 2. **A recorded digest that does not match dominates any C2PA result.** A validly signed
///    manifest proves the asset is intact *with respect to its own manifest*, which a substituted
///    but properly signed file also satisfies. Only the digest recorded at ingestion says the file
///    is the one this application downloaded, so a mismatch is a substitution regardless of how
///    good the manifest looks.
/// 3. Only then may a validated signature raise the level above local integrity.
///
/// An invalid manifest never lowers the level below what the digest earned -- a broken provenance
/// claim does not make an intact file less intact -- but it always produces a finding.
pub fn classify_confidence(
    evidence_file_present: bool,
    recorded_digest_present: bool,
    hash_matches: bool,
    manifest_present: bool,
    validation: Option<ManifestValidation>,
) -> ProvenanceConfidence {
    if !evidence_file_present {
        return ProvenanceConfidence::EvidenceFileUnavailable;
    }
    if recorded_digest_present && !hash_matches {
        return ProvenanceConfidence::DigestMismatch;
    }
    match validation {
        Some(ManifestValidation::Trusted) => ProvenanceConfidence::TrustedChainOfCustody,
        Some(ManifestValidation::Valid) => ProvenanceConfidence::VerifiedSignature,
        _ if hash_matches => ProvenanceConfidence::LocalDigestMatch,
        _ if manifest_present => ProvenanceConfidence::RecordedClaimOnly,
        _ => ProvenanceConfidence::NoRecordedDigest,
    }
}

/// Reads and validates the C2PA manifest embedded in an asset.
///
/// Blocking: the SDK reads the file synchronously. Callers must run this on a blocking worker.
pub fn read_asset_provenance(path: &Path) -> Result<AssetProvenance, AppError> {
    // An explicit `Context` rather than the deprecated `Reader::from_file`, which resolves its
    // settings from thread-local state. This runs on a blocking worker pool, so thread-local
    // settings would make a verification's behavior depend on which worker happened to pick it up.
    // The context is also where a trust anchor list would be supplied to make `Trusted` reachable.
    // None is supplied and there is nowhere to configure one, so this build tops out at `Valid`.
    match c2pa::Reader::from_context(c2pa::Context::new()).with_file(path) {
        Ok(reader) => Ok(from_reader(&reader)),
        // Most released records carry no provenance at all, and many arrive in containers the SDK
        // does not parse. Neither is a defect in the document or a failure of the check.
        Err(
            c2pa::Error::JumbfNotFound
            | c2pa::Error::JumbfBoxNotFound
            | c2pa::Error::UnsupportedType
            | c2pa::Error::NotFound,
        ) => Ok(AssetProvenance::default()),
        // The SDK returns an error both for an asset carrying no provenance and for one whose
        // provenance cannot be parsed, and does not reliably distinguish the two. Defaulting to
        // "manifest present but invalid" would flag every ordinary released document as carrying a
        // broken provenance claim, so absence is the default and the reason is surfaced as a
        // finding. A manifest is only ever reported as invalid when the SDK actually parsed one.
        Err(error) => Ok(AssetProvenance {
            findings: vec![format!("C2PA provenance could not be read: {error}")],
            ..AssetProvenance::default()
        }),
    }
}

fn from_reader(reader: &c2pa::Reader) -> AssetProvenance {
    let validation = match reader.validation_state() {
        c2pa::ValidationState::Invalid => ManifestValidation::Invalid,
        c2pa::ValidationState::Valid => ManifestValidation::Valid,
        c2pa::ValidationState::Trusted => ManifestValidation::Trusted,
    };
    let is_tamper_evident = matches!(
        validation,
        ManifestValidation::Valid | ManifestValidation::Trusted
    );
    let active = reader.active_manifest();
    let claim_generator = active.and_then(|manifest| manifest.claim_generator().map(String::from));
    let signature_issuer = active.and_then(|manifest| manifest.issuer());
    let actor_identifier = claim_generator
        .clone()
        .or_else(|| signature_issuer.clone())
        .unwrap_or_else(|| "Unattributed manifest signer".to_string());
    let assertions = active
        .map(|manifest| {
            manifest
                .assertions()
                .iter()
                .map(|assertion| AssetAssertion {
                    assertion_type: assertion.label().to_string(),
                    actor_identifier: actor_identifier.clone(),
                    is_tamper_evident,
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let mut findings = Vec::new();
    if validation == ManifestValidation::Invalid {
        findings.push(
            "The embedded C2PA manifest failed validation, so its provenance claims cannot be \
             relied on"
                .to_string(),
        );
        for status in reader.validation_status().unwrap_or_default() {
            findings.push(format!(
                "C2PA validation status '{}'{}",
                status.code(),
                status
                    .explanation()
                    .map(|explanation| format!(": {explanation}"))
                    .unwrap_or_default()
            ));
        }
    } else if validation == ManifestValidation::Valid {
        // Valid means the signature verifies; it does not mean the signer is anyone in particular.
        findings.push(
            "The C2PA signature is valid, but the signing certificate was not checked against a \
             trust list, so the signer's identity is unverified"
                .to_string(),
        );
    }

    AssetProvenance {
        manifest_present: true,
        validation: Some(validation),
        signature_issuer,
        signing_timestamp: active.and_then(|manifest| manifest.time()),
        claim_generator,
        assertions,
        findings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The smallest asset the SDK will embed a manifest into that is also plain text, so the
    /// tampering step below is a legible one-byte edit rather than a blind poke at binary.
    const ONE_PIXEL_SVG: &[u8] =
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><rect width="1" height="1" fill="#000000"/></svg>"##;

    /// The whole point of reading a manifest is the signature, and until this test existed nothing
    /// exercised one: `Valid` and `Invalid` were reached only through `classify_confidence`, which
    /// is pure and takes the validation state as an argument. This signs a real asset and reads it
    /// back through the same path a document verification takes.
    ///
    /// The fixture is generated rather than committed. `EphemeralSigner` mints a throwaway
    /// CA + end-entity Ed25519 chain, so nothing binary lives in the tree and nothing expires.
    /// Its certificate chains to no trust anchor, which is exactly the state every document in a
    /// default deployment reaches: `Valid`, never `Trusted`.
    #[test]
    fn a_signed_asset_verifies_and_a_tampered_one_reports_an_invalid_manifest() {
        let directory = tempfile::tempdir().expect("temp dir must be created");
        let source = directory.path().join("source.svg");
        std::fs::write(&source, ONE_PIXEL_SVG).expect("source asset must be written");
        let signed = directory.path().join("signed.svg");

        let signer = c2pa::EphemeralSigner::new("onrecord-studio.test")
            .expect("an ephemeral signing chain must be generated");
        // A manifest whose first action is not `created` or `opened` is rejected by the spec's
        // action rules, which would make the asset read as Invalid for a reason that has nothing
        // to do with its signature. The declaration is the minimum a well-formed manifest needs.
        // An explicit `Context`, for the same reason `read_asset_provenance` uses one: the
        // deprecated constructors resolve their settings from thread-local state.
        let mut builder = c2pa::Builder::from_context(c2pa::Context::new())
            .with_definition(
            r#"{
                "claim_generator_info": [{ "name": "onrecord-studio", "version": "0.0.0-test" }],
                "assertions": [
                    {
                        "label": "c2pa.actions.v2",
                        "data": {
                            "actions": [
                                {
                                    "action": "c2pa.created",
                                    "digitalSourceType": "http://cv.iptc.org/newscodes/digitalsourcetype/digitalCapture"
                                }
                            ]
                        }
                    }
                ]
            }"#,
            )
            .expect("the manifest definition must parse");
        builder
            .sign_file(&signer, &source, &signed)
            .expect("the asset must be signed");

        let intact = read_asset_provenance(&signed).expect("reading a signed asset must not fail");
        assert!(intact.manifest_present);
        assert_eq!(
            intact.validation,
            Some(ManifestValidation::Valid),
            "a signature that verifies over the asset must read as Valid; findings: {:?}",
            intact.findings
        );
        assert_eq!(
            intact.signature_verification(),
            SignatureVerificationState::Valid
        );
        assert!(intact.claims_signature());
        // No trust anchor is configured, so the rung stops one below chain of custody.
        assert_eq!(
            classify_confidence(true, true, true, intact.manifest_present, intact.validation),
            ProvenanceConfidence::VerifiedSignature
        );
        assert_ne!(
            intact.validation,
            Some(ManifestValidation::Trusted),
            "no trust anchor is configured in this build, so Trusted must be unreachable"
        );

        // Change one byte of the asset the manifest is bound to. The manifest itself is untouched,
        // so this is the substitution case: a provenance claim that no longer holds.
        let mut bytes = std::fs::read(&signed).expect("the signed asset must be readable");
        let fill = bytes
            .windows(7)
            .position(|window| window == b"#000000")
            .expect("the signed asset must retain the original fill attribute");
        bytes[fill + 1] = b'f';
        let tampered = directory.path().join("tampered.svg");
        std::fs::write(&tampered, &bytes).expect("the tampered asset must be written");

        let broken =
            read_asset_provenance(&tampered).expect("reading a tampered asset must not fail");
        assert!(broken.manifest_present);
        assert_eq!(
            broken.validation,
            Some(ManifestValidation::Invalid),
            "a manifest whose hard binding no longer matches must read as Invalid; findings: {:?}",
            broken.findings
        );
        assert_eq!(
            broken.signature_verification(),
            SignatureVerificationState::Invalid
        );
        assert!(
            !broken.findings.is_empty(),
            "an invalid manifest must explain itself"
        );
    }

    /// A file with no embedded provenance is not a failure, and must not be reported as a manifest.
    #[test]
    fn an_asset_without_provenance_reports_no_manifest() {
        let directory = tempfile::tempdir().expect("temp dir must be created");
        let path = directory.path().join("release.txt");
        std::fs::write(&path, b"An ordinary released record.\n").expect("file must be written");
        let provenance = read_asset_provenance(&path).expect("reading must not fail");
        assert!(!provenance.manifest_present);
        // The common case must not generate noise: almost every released record carries no
        // provenance, and saying so on each one would bury the documents that do.
        assert!(
            provenance.findings.is_empty(),
            "an asset without provenance produced findings: {:?}",
            provenance.findings
        );
        assert_eq!(provenance.validation, None);
        assert_eq!(
            provenance.signature_verification(),
            SignatureVerificationState::NotAttempted
        );
        assert!(!provenance.claims_signature());
    }

    /// An asset the reader cannot parse is reported as unread, never as a broken provenance claim.
    ///
    /// The SDK returns an error for both "no provenance" and "provenance that will not parse", so
    /// treating an unrecognised error as a manifest would put a broken-claim finding on every
    /// corrupt or unusual document in a release.
    #[test]
    fn an_unparseable_asset_is_reported_without_inventing_a_manifest() {
        let directory = tempfile::tempdir().expect("temp dir must be created");
        let path = directory.path().join("truncated.pdf");
        std::fs::write(&path, b"%PDF-1.7\ntruncated before the xref table\n")
            .expect("file must be written");
        let provenance = read_asset_provenance(&path).expect("reading must not fail");
        assert!(!provenance.manifest_present);
        assert_eq!(provenance.validation, None);
        assert_eq!(
            provenance.signature_verification(),
            SignatureVerificationState::NotAttempted
        );
        assert!(
            provenance
                .findings
                .iter()
                .any(|finding| finding.contains("could not be read")),
            "the reason must be surfaced, got {:?}",
            provenance.findings
        );
    }

    /// A recorded digest that does not match must dominate any manifest verdict.
    ///
    /// A validly signed manifest says the asset matches *its own* manifest, which a substituted
    /// file carrying its own valid manifest also satisfies. Letting a good signature outrank a
    /// digest mismatch would report a swapped document as cryptographically verified.
    #[test]
    fn a_digest_mismatch_outranks_even_a_trusted_manifest() {
        for validation in [
            None,
            Some(ManifestValidation::Invalid),
            Some(ManifestValidation::Valid),
            Some(ManifestValidation::Trusted),
        ] {
            assert_eq!(
                classify_confidence(true, true, false, true, validation),
                ProvenanceConfidence::DigestMismatch,
                "validation {validation:?} must not outrank a digest mismatch"
            );
        }
    }

    #[test]
    fn validation_raises_confidence_only_above_local_integrity() {
        assert_eq!(
            classify_confidence(true, true, true, true, Some(ManifestValidation::Trusted)),
            ProvenanceConfidence::TrustedChainOfCustody
        );
        assert_eq!(
            classify_confidence(true, true, true, true, Some(ManifestValidation::Valid)),
            ProvenanceConfidence::VerifiedSignature
        );
        // An invalid manifest produces a finding, but does not make an intact file less intact.
        assert_eq!(
            classify_confidence(true, true, true, true, Some(ManifestValidation::Invalid)),
            ProvenanceConfidence::LocalDigestMatch
        );
        assert_eq!(
            classify_confidence(true, true, true, false, None),
            ProvenanceConfidence::LocalDigestMatch
        );
    }

    #[test]
    fn absent_evidence_and_absent_records_are_distinguished() {
        assert_eq!(
            classify_confidence(false, true, false, true, Some(ManifestValidation::Valid)),
            ProvenanceConfidence::EvidenceFileUnavailable,
            "an unreadable file cannot be validated no matter what was recorded for it"
        );
        // No digest recorded, but a manifest is on file: claims exist, nothing was confirmed.
        assert_eq!(
            classify_confidence(true, false, false, true, Some(ManifestValidation::Invalid)),
            ProvenanceConfidence::RecordedClaimOnly
        );
        assert_eq!(
            classify_confidence(true, false, false, false, None),
            ProvenanceConfidence::NoRecordedDigest
        );
    }

    #[test]
    fn assertion_tamper_evidence_follows_the_manifest_verdict() {
        let signed = AssetProvenance {
            manifest_present: true,
            validation: Some(ManifestValidation::Valid),
            assertions: vec![AssetAssertion {
                assertion_type: "c2pa.actions".to_string(),
                actor_identifier: "Example Generator".to_string(),
                is_tamper_evident: true,
            }],
            ..AssetProvenance::default()
        };
        assert_eq!(signed.invalid_assertion_count(), 0);
        assert_eq!(
            signed.signature_verification(),
            SignatureVerificationState::Valid
        );

        let broken = AssetProvenance {
            validation: Some(ManifestValidation::Invalid),
            assertions: vec![AssetAssertion {
                assertion_type: "c2pa.actions".to_string(),
                actor_identifier: "Example Generator".to_string(),
                is_tamper_evident: false,
            }],
            ..signed
        };
        assert_eq!(broken.invalid_assertion_count(), 1);
        assert_eq!(
            broken.signature_verification(),
            SignatureVerificationState::Invalid
        );
    }
}
