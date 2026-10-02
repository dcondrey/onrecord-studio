use crate::{
    error::AppError,
    integrity::{
        check_revocation, load_or_create_signing_key, load_revocations, sign_package,
        verify_package_detailed, SignedPackage,
    },
    storage::sqlite::DuckDbRepository,
    types::casework::*,
    types::investigation::SigningKeyRevocations,
};
use std::path::{Path, PathBuf};
use tauri::State;

/// The signed envelope duplicates the packet (once as base64 canonical bytes, once as the plain
/// `manifest` tree) alongside the signature, so it runs a few times larger than the 2 MB inner
/// `MAX_PACKET` case-content limit the repository enforces. This is the limit the transport format
/// itself must respect, checked before an untrusted payload is ever parsed; it must match the
/// frontend's file-size and textarea limits in `CaseworkPanel.tsx` or a case that exports
/// successfully can become impossible to preview or import through the UI.
const MAXIMUM_SIGNED_PACKET_BYTES: usize = 8 * 1024 * 1024;

#[tauri::command]
pub async fn get_casework_state(
    repository: State<'_, DuckDbRepository>,
) -> Result<CaseworkState, AppError> {
    repository.casework_state().await
}
/// The documents, requests and findings one case covers, for scoping views to it.
#[tauri::command]
pub async fn get_case_scope(
    case_id: String,
    repository: State<'_, DuckDbRepository>,
) -> Result<crate::storage::case_scope::CaseScope, AppError> {
    repository.case_scope(case_id).await
}
#[tauri::command]
pub async fn create_casework_case(
    report_id: String,
    repository: State<'_, DuckDbRepository>,
) -> Result<CaseworkCase, AppError> {
    repository.create_casework_case(report_id).await
}
#[tauri::command]
pub async fn save_casework_case(
    case: CaseworkCase,
    expected_revision: u64,
    author: String,
    repository: State<'_, DuckDbRepository>,
) -> Result<CaseworkCase, AppError> {
    repository
        .save_casework_case(case, expected_revision, author)
        .await
}
#[tauri::command]
pub async fn export_casework_packet(
    case_id: String,
    contributor: String,
    repository: State<'_, DuckDbRepository>,
    key_directory: State<'_, crate::integrity::SigningKeyDirectory>,
) -> Result<String, AppError> {
    let payload = repository
        .export_casework_packet(case_id, contributor)
        .await?;
    let packet: CaseworkPacket = serde_json::from_str(&payload).map_err(|error| {
        AppError::DataParsing(format!("Failed to encode portable case packet: {error}"))
    })?;
    let signing_key = load_or_create_signing_key(&key_directory.0).await?;
    let signed = sign_package(packet, &signing_key).map_err(AppError::Export)?;
    let envelope = serde_json::to_string_pretty(&signed).map_err(|error| {
        AppError::DataParsing(format!("Failed to encode signed case packet: {error}"))
    })?;
    if envelope.len() > MAXIMUM_SIGNED_PACKET_BYTES {
        return Err(AppError::DataParsing(format!(
            "The signed export exceeds the {MAXIMUM_SIGNED_PACKET_BYTES}-byte transport limit. \
             Remove some case content and export again."
        )));
    }
    Ok(envelope)
}

/// Verifies a portable case packet's Ed25519 signature and unwraps it back to the plain
/// `CaseworkPacket` JSON the repository's preview/import logic already operates on and is already
/// tested against. Verification happens here, at the IPC boundary, not inside the repository: it is
/// what turns `contributor` from a typed label into a claim a later reader can cross-check against
/// the key that actually produced the bytes. A packet that does not verify is never handed to the
/// repository.
async fn verify_and_unwrap_packet(
    payload: &str,
    report_directory: &Path,
) -> Result<(String, String), AppError> {
    if payload.len() > MAXIMUM_SIGNED_PACKET_BYTES {
        return Err(AppError::DataParsing(format!(
            "This packet exceeds the {MAXIMUM_SIGNED_PACKET_BYTES}-byte transport limit."
        )));
    }
    let package: SignedPackage<CaseworkPacket> =
        serde_json::from_str(payload).map_err(|error| {
            AppError::DataParsing(format!(
            "This packet is not signed with a local export key ({error}). Ask the contributor to \
             re-export it from a version that signs portable case packets."
        ))
        })?;
    let checks = verify_package_detailed(&package);
    if !checks.all_passed() {
        return Err(AppError::DataParsing(format!(
            "This packet's signature does not verify; the bytes have changed since it was signed. \
             It was not previewed or imported. {}",
            checks.detail.unwrap_or_default()
        )));
    }
    let signer_key_id = package.signature.signer_key_id.clone();
    // A revoked key's signature is still mathematically valid over these bytes — that's not in
    // question. A routine, non-compromising rotation (RETIRED) does not block the import; only a
    // key this installation recorded as COMPROMISED does, since trusting a packet from a
    // compromised key is exactly the risk revocation exists to close off.
    let revocations = load_revocations(report_directory).await?;
    if let Some(revoked) = check_revocation(&signer_key_id, &revocations) {
        // Fail closed: only an exact "RETIRED" status is treated as informational. A typo, a
        // future status value, or corrupted data reads as compromised, never as silently trusted.
        if revoked.status != crate::integrity::REVOCATION_STATUS_RETIRED {
            return Err(AppError::DataParsing(format!(
                "This packet was signed by local export key {signer_key_id}, which this \
                 installation revoked as compromised on {} ({}). It was not previewed or \
                 imported.",
                revoked.revoked_at, revoked.reason
            )));
        }
    }
    let inner = serde_json::to_string(&package.manifest).map_err(|error| {
        AppError::DataParsing(format!("Failed to re-encode verified case packet: {error}"))
    })?;
    Ok((inner, signer_key_id))
}

/// Cross-references this case's already-imported revisions against this installation's local
/// revocation list, so an import accepted before a signer was ever marked compromised can still
/// be flagged after the fact — the reason `casework_imports.signer_key_id` was made a structured
/// column instead of living only in a free-text event summary.
/// Pure over an already-fetched signer list so it is testable without a `DuckDbRepository`: a
/// revision with no recorded signer (a legacy import predating `signer_key_id`) is silently
/// skipped, never treated as either compromised or as evidence of a trusted signer.
fn compromised_case_imports_inner(
    signers: Vec<(u64, Option<String>)>,
    revocations: &SigningKeyRevocations,
) -> Vec<CompromisedImportWarning> {
    signers
        .into_iter()
        .filter_map(|(revision, signer_key_id)| {
            let signer_key_id = signer_key_id?;
            let revoked = check_revocation(&signer_key_id, revocations)?;
            (revoked.status != crate::integrity::REVOCATION_STATUS_RETIRED).then(|| {
                CompromisedImportWarning {
                    revision,
                    signer_key_id: signer_key_id.clone(),
                    revoked_at: revoked.revoked_at.clone(),
                    reason: revoked.reason.clone(),
                }
            })
        })
        .collect()
}

#[tauri::command]
pub async fn compromised_case_imports(
    case_id: String,
    repository: State<'_, DuckDbRepository>,
    report_directory: State<'_, PathBuf>,
) -> Result<Vec<CompromisedImportWarning>, AppError> {
    let signers = repository.case_import_signers(case_id).await?;
    let revocations = load_revocations(&report_directory).await?;
    Ok(compromised_case_imports_inner(signers, &revocations))
}

#[tauri::command]
pub async fn preview_casework_import(
    payload: String,
    repository: State<'_, DuckDbRepository>,
    report_directory: State<'_, PathBuf>,
) -> Result<CaseworkImportPreview, AppError> {
    let (inner, signer_key_id) = verify_and_unwrap_packet(&payload, &report_directory).await?;
    let mut preview = repository.preview_casework_import(inner).await?;
    preview.warnings.push(format!(
        "Signature verified: this packet has not changed since it was signed by local export key \
         {signer_key_id}. That proves origin and integrity of the file, not the contributor's \
         real-world identity."
    ));
    preview.signer_key_id = Some(signer_key_id);
    Ok(preview)
}
#[tauri::command]
pub async fn import_casework_packet(
    payload: String,
    expected_revision: Option<u64>,
    packet_sha256: String,
    author: String,
    repository: State<'_, DuckDbRepository>,
    report_directory: State<'_, PathBuf>,
) -> Result<CaseworkCase, AppError> {
    let (inner, signer_key_id) = verify_and_unwrap_packet(&payload, &report_directory).await?;
    repository
        .import_casework_packet(
            inner,
            expected_revision,
            packet_sha256,
            author,
            Some(signer_key_id),
        )
        .await
}

/// Applies only the incoming operations that commute with local work; see the repository method.
#[tauri::command]
pub async fn apply_commuting_case_operations(
    payload: String,
    expected_revision: Option<u64>,
    packet_sha256: String,
    author: String,
    repository: State<'_, DuckDbRepository>,
    report_directory: State<'_, PathBuf>,
) -> Result<CaseworkCase, AppError> {
    let (inner, signer_key_id) = verify_and_unwrap_packet(&payload, &report_directory).await?;
    repository
        .apply_commuting_case_operations(
            inner,
            expected_revision,
            packet_sha256,
            author,
            Some(signer_key_id),
        )
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;

    fn packet() -> CaseworkPacket {
        CaseworkPacket {
            schema_version: 1,
            operations: Vec::new(),
            base_revision: 0,
            base_sha256: "a".repeat(64),
            exported_at: "2026-09-28T00:00:00Z".into(),
            contributor: "Origin desk".into(),
            case: CaseworkCase::default(),
            history: vec![],
        }
    }

    #[tokio::test]
    async fn verify_and_unwrap_packet_accepts_a_signed_round_trip_and_reports_the_signer() {
        let directory = tempfile::tempdir().unwrap();
        let key = SigningKey::from_bytes(&[9_u8; 32]);
        let signed = sign_package(packet(), &key).unwrap();
        let payload = serde_json::to_string(&signed).unwrap();
        let (inner, signer_key_id) = verify_and_unwrap_packet(&payload, directory.path())
            .await
            .unwrap();
        assert_eq!(signer_key_id, signed.signature.signer_key_id);
        let round_tripped: CaseworkPacket = serde_json::from_str(&inner).unwrap();
        assert_eq!(round_tripped.contributor, "Origin desk");
    }

    #[tokio::test]
    async fn verify_and_unwrap_packet_enforces_the_transport_limit_at_the_exact_boundary() {
        let directory = tempfile::tempdir().unwrap();
        let over = "a".repeat(MAXIMUM_SIGNED_PACKET_BYTES + 1);
        assert!(verify_and_unwrap_packet(&over, directory.path())
            .await
            .unwrap_err()
            .to_string()
            .contains("transport limit"));

        // At and under the limit the payload must reach the parser (and fail there, since it is
        // not valid JSON), never be rejected for its size.
        for payload in [
            "a".repeat(MAXIMUM_SIGNED_PACKET_BYTES),
            "a".repeat(MAXIMUM_SIGNED_PACKET_BYTES - 1),
        ] {
            assert!(!verify_and_unwrap_packet(&payload, directory.path())
                .await
                .unwrap_err()
                .to_string()
                .contains("transport limit"));
        }
    }

    #[test]
    fn a_near_maximum_inner_packet_still_fits_the_signed_transport_limit() {
        let mut large = packet();
        large.case.notes = "x".repeat(1_900_000);
        let key = SigningKey::from_bytes(&[4_u8; 32]);
        let signed = sign_package(large, &key).unwrap();
        let envelope = serde_json::to_string_pretty(&signed).unwrap();
        assert!(
            envelope.len() <= MAXIMUM_SIGNED_PACKET_BYTES,
            "a signed envelope of {} bytes exceeds the {MAXIMUM_SIGNED_PACKET_BYTES}-byte transport limit",
            envelope.len()
        );
    }

    #[tokio::test]
    async fn verify_and_unwrap_packet_rejects_a_packet_that_was_never_signed() {
        let directory = tempfile::tempdir().unwrap();
        let payload = serde_json::to_string(&packet()).unwrap();
        let error = verify_and_unwrap_packet(&payload, directory.path())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("not signed"));
    }

    #[tokio::test]
    async fn verify_and_unwrap_packet_rejects_a_packet_edited_after_signing() {
        let directory = tempfile::tempdir().unwrap();
        let key = SigningKey::from_bytes(&[9_u8; 32]);
        let mut signed = sign_package(packet(), &key).unwrap();
        signed.manifest.contributor = "Attacker-edited contributor".into();
        let payload = serde_json::to_string(&signed).unwrap();
        let error = verify_and_unwrap_packet(&payload, directory.path())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("does not verify"));
    }

    #[tokio::test]
    async fn verify_and_unwrap_packet_rejects_a_packet_signed_by_a_different_key() {
        let directory = tempfile::tempdir().unwrap();
        let signed = sign_package(packet(), &SigningKey::from_bytes(&[1_u8; 32])).unwrap();
        let mut forged = signed.clone();
        forged.signature.public_key_hex =
            sign_package(packet(), &SigningKey::from_bytes(&[2_u8; 32]))
                .unwrap()
                .signature
                .public_key_hex;
        let payload = serde_json::to_string(&forged).unwrap();
        assert!(verify_and_unwrap_packet(&payload, directory.path())
            .await
            .is_err());
    }

    #[tokio::test]
    async fn verify_and_unwrap_packet_rejects_a_packet_from_a_key_this_installation_revoked() {
        let directory = tempfile::tempdir().unwrap();
        let retired_key = SigningKey::from_bytes(&[9_u8; 32]);
        // Seed the active signing key file with exactly the key that will sign the packet, so
        // rotating it below retires *that* key rather than some unrelated freshly generated one.
        tokio::fs::write(
            directory.path().join(crate::integrity::SIGNING_KEY_FILE),
            retired_key.to_bytes(),
        )
        .await
        .unwrap();
        let signed = sign_package(packet(), &retired_key).unwrap();
        let payload = serde_json::to_string(&signed).unwrap();

        crate::integrity::rotate_signing_key(
            directory.path(),
            directory.path(),
            "Key left the organization.".into(),
            true,
        )
        .await
        .unwrap();

        let error = verify_and_unwrap_packet(&payload, directory.path())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("revoked"));
        assert!(error.to_string().contains("Key left the organization."));
    }

    #[tokio::test]
    async fn verify_and_unwrap_packet_accepts_a_packet_from_a_routinely_retired_key() {
        let directory = tempfile::tempdir().unwrap();
        let retiring_key = SigningKey::from_bytes(&[3_u8; 32]);
        tokio::fs::write(
            directory.path().join(crate::integrity::SIGNING_KEY_FILE),
            retiring_key.to_bytes(),
        )
        .await
        .unwrap();
        let signed = sign_package(packet(), &retiring_key).unwrap();
        let payload = serde_json::to_string(&signed).unwrap();

        crate::integrity::rotate_signing_key(
            directory.path(),
            directory.path(),
            "Routine hygiene rotation.".into(),
            false,
        )
        .await
        .unwrap();

        let (inner, signer_key_id) = verify_and_unwrap_packet(&payload, directory.path())
            .await
            .expect("a packet from a routinely retired (non-compromising) key must still import");
        assert_eq!(signer_key_id, signed.signature.signer_key_id);
        let round_tripped: CaseworkPacket = serde_json::from_str(&inner).unwrap();
        assert_eq!(round_tripped.contributor, "Origin desk");
    }

    fn revoked_entry(key_id: &str, status: &str) -> crate::types::investigation::RevokedSigningKey {
        crate::types::investigation::RevokedSigningKey {
            key_id: key_id.to_string(),
            revoked_at: "2026-09-29T00:00:00Z".to_string(),
            reason: "test".to_string(),
            status: status.to_string(),
        }
    }

    #[test]
    fn compromised_case_imports_inner_flags_only_the_compromised_signer() {
        let revocations = SigningKeyRevocations {
            schema_version: "onrecord-signing-key-revocations-v1".to_string(),
            entries: vec![
                revoked_entry("compromised-key", "COMPROMISED"),
                revoked_entry("retired-key", "RETIRED"),
            ],
        };
        let signers = vec![
            (1_u64, Some("compromised-key".to_string())),
            (2_u64, Some("retired-key".to_string())),
            (3_u64, None),
        ];

        let warnings = compromised_case_imports_inner(signers, &revocations);

        assert_eq!(
            warnings.len(),
            1,
            "a retired (non-compromising) signer and a legacy import with no recorded signer \
             must never be flagged, only the compromised one"
        );
        assert_eq!(warnings[0].revision, 1);
        assert_eq!(warnings[0].signer_key_id, "compromised-key");
    }

    #[test]
    fn compromised_case_imports_inner_ignores_a_signer_with_no_revocation_entry_at_all() {
        let revocations = SigningKeyRevocations {
            schema_version: "onrecord-signing-key-revocations-v1".to_string(),
            entries: vec![],
        };
        let signers = vec![(1_u64, Some("never-revoked-key".to_string()))];

        assert!(compromised_case_imports_inner(signers, &revocations).is_empty());
    }
}
