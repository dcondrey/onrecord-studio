use crate::error::AppError;
use crate::types::investigation::{
    RevokedSigningKey, SigningKeyRevocations, SigningKeyRotationResult,
};
use base64::Engine;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;
use std::sync::LazyLock;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

const HASH_BUFFER_BYTES: usize = 64 * 1024;

/// Serializes every read-modify-write of the local revocation list within this process.
/// `rotate_signing_key` and `mark_key_compromised` each read the current list, decide what to
/// append, and write the result back; without this, two genuinely concurrent calls (not the
/// normal case — the shipped UI's shared busy/request-gate state prevents a second click from
/// firing while one is in flight, but nothing stops two calls issued outside that UI) can both
/// read the same starting list and the last write silently discards the other call's entry —
/// including a COMPROMISED entry a concurrent rotation raced out. `tokio::sync::Mutex` (not
/// `std::sync::Mutex`) because the guard is held across the `.await` points of loading, signing
/// and durably writing the list. This lock is process-local only: it does not serialize two
/// separate app instances writing the same report directory.
static REVOCATION_WRITE_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));
const REVOCATION_LOCK_FILE: &str = ".onrecord-export-signing-key-revocations-v1.lock";

/// Acquires an OS-level advisory lock (`flock` on Unix, `LockFileEx` on Windows) on a dedicated
/// lock file, so two separate app instances — or processes — writing the same report directory
/// cannot lose one call's revocation entry to the other's write the way `REVOCATION_WRITE_LOCK`
/// alone only prevents within one process. A distinct lock file, not the revocations file itself,
/// because the revocations file is replaced via `rename` on every write, and a lock tied to an
/// inode that write replaces would stop protecting anything after the first write. `File::lock`
/// blocks the calling thread, so it runs on the blocking thread pool via `spawn_blocking` rather
/// than stalling the async executor; the returned `File` must be kept alive for as long as the
/// critical section it guards — its `Drop` releases the OS lock when the descriptor closes.
async fn acquire_cross_process_revocation_lock(
    report_directory: &Path,
) -> Result<std::fs::File, AppError> {
    let lock_path = report_directory.join(REVOCATION_LOCK_FILE);
    tokio::task::spawn_blocking(move || {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .map_err(|error| {
                AppError::Export(format!("Failed to open the revocation lock file: {error}"))
            })?;
        file.lock().map_err(|error| {
            AppError::Export(format!(
                "Failed to acquire the cross-process revocation lock: {error}"
            ))
        })?;
        Ok(file)
    })
    .await
    .map_err(|error| AppError::Export(format!("Revocation lock task panicked: {error}")))?
}
pub(crate) const SIGNATURE_ALGORITHM: &str = "Ed25519";
/// The honest label for a key this application generated and holds locally: it proves the
/// package was signed by whoever controlled this installation's key file, not by an externally
/// attested identity. A reader must not mistake this for a notarized or third-party-attested
/// signature; the confidence ladder in `types::c2pa` uses the same distinction on the ingest side.
pub(crate) const SELF_MANAGED_LOCAL_KEY: &str = "SELF_MANAGED_LOCAL_KEY_NOT_EXTERNALLY_ATTESTED";
pub(crate) const SIGNING_KEY_FILE: &str = ".onrecord-export-signing-key-v1";

/// Where private signing keys live: outside the reports directory, which users zip and send on,
/// so sharing exports can never carry the key that signed them.
pub struct SigningKeyDirectory(pub std::path::PathBuf);

/// Moves key files an older build wrote into the reports directory. Same volume, so each move is
/// an atomic rename; a destination that already exists is left alone rather than overwritten.
pub(crate) fn relocate_legacy_signing_keys(
    report_directory: &Path,
    key_directory: &Path,
) -> Result<(), AppError> {
    let entries = match std::fs::read_dir(report_directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(AppError::Export(format!(
                "Failed to scan for legacy signing keys: {error}"
            )))
        }
    };
    for entry in entries {
        let entry = entry.map_err(|error| {
            AppError::Export(format!("Failed to scan for signing keys: {error}"))
        })?;
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with(SIGNING_KEY_FILE) {
            continue;
        }
        let destination = key_directory.join(&name);
        if destination.exists() {
            continue;
        }
        std::fs::rename(entry.path(), &destination).map_err(|error| {
            AppError::Export(format!(
                "Failed to move signing key '{}' out of the reports directory: {error}",
                name.to_string_lossy()
            ))
        })?;
    }
    Ok(())
}

/// Loads this installation's local Ed25519 signing key, creating it on first use. Every signed
/// package this application produces — evidence exports, chain-of-custody exports, and portable
/// casework packets — is signed with this same key, so a recipient of two packets claiming
/// different contributor labels can tell whether they actually came from the same installation.
///
/// Draws directly from the OS CSPRNG (`OsRng`), not from hashing UUIDs and a timestamp: a UUIDv4
/// already derives from an RNG the OS or libc provides, so hashing it added an indirection without
/// adding entropy, and made the actual entropy source harder to audit.
pub(crate) fn generate_signing_key_seed() -> [u8; 32] {
    let mut seed = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut seed);
    seed
}

pub(crate) async fn load_or_create_signing_key(
    key_directory: &Path,
) -> Result<SigningKey, AppError> {
    let key_path = key_directory.join(SIGNING_KEY_FILE);
    match tokio::fs::read(&key_path).await {
        Ok(bytes) => return signing_key_from_bytes(&bytes, &key_path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(AppError::Export(format!(
                "Failed to read the local export signing key: {error}"
            )))
        }
    }

    let seed = generate_signing_key_seed();

    let mut options = tokio::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }
    match options.open(&key_path).await {
        Ok(mut file) => {
            if let Err(error) = file.write_all(&seed).await {
                drop(file);
                let _ = tokio::fs::remove_file(&key_path).await;
                return Err(AppError::Export(format!(
                    "Failed to write the local export signing key: {error}"
                )));
            }
            if let Err(error) = file.sync_all().await {
                drop(file);
                let _ = tokio::fs::remove_file(&key_path).await;
                return Err(AppError::Export(format!(
                    "Failed to sync the local export signing key: {error}"
                )));
            }
            Ok(SigningKey::from_bytes(&seed))
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let bytes = tokio::fs::read(&key_path).await.map_err(|read_error| {
                AppError::Export(format!(
                    "The export signing key was created concurrently but could not be read: {read_error}"
                ))
            })?;
            signing_key_from_bytes(&bytes, &key_path)
        }
        Err(error) => Err(AppError::Export(format!(
            "Failed to create the local export signing key: {error}"
        ))),
    }
}

fn signing_key_from_bytes(bytes: &[u8], path: &Path) -> Result<SigningKey, AppError> {
    let seed: [u8; 32] = bytes.try_into().map_err(|_| {
        AppError::Export(format!(
            "Export signing key '{}' must contain exactly 32 bytes",
            path.display()
        ))
    })?;
    Ok(SigningKey::from_bytes(&seed))
}

/// Whether `value` has the shape of a hex SHA-256 digest (either case).
pub(crate) fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(crate) fn sha256_hex(value: &[u8]) -> String {
    digest_hex(Sha256::digest(value))
}

/// Formats a digest without relying on the optional `LowerHex` implementation from a specific
/// digest crate version. Keeping this local avoids adding a second hex-encoding dependency.
pub(crate) fn digest_hex<D: AsRef<[u8]>>(digest: D) -> String {
    digest
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(crate) fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; HASH_BUFFER_BYTES];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            return Ok(digest_hex(digest.finalize()));
        }
        digest.update(&buffer[..count]);
    }
}

pub(crate) async fn sha256_file_async(path: &Path) -> std::io::Result<String> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut digest = Sha256::new();
    // Keep the async state machine compact; embedding 64 KiB directly would inflate every caller.
    let mut buffer = vec![0_u8; HASH_BUFFER_BYTES];
    loop {
        let count = file.read(&mut buffer).await?;
        if count == 0 {
            return Ok(digest_hex(digest.finalize()));
        }
        digest.update(&buffer[..count]);
    }
}

pub(crate) fn hex_decode(value: &str) -> Result<Vec<u8>, String> {
    if !value.len().is_multiple_of(2) || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("'{value}' is not a hex string"));
    }
    (0..value.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&value[i..i + 2], 16).map_err(|error| error.to_string()))
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PackageSignature {
    pub algorithm: String,
    pub public_key_hex: String,
    pub signer_key_id: String,
    pub signature_hex: String,
    pub identity_assurance: String,
}

/// A signed manifest of type `T`, self-contained enough for a verifier that has never seen this
/// application's source to check it. `manifest_canonical_base64` holds the exact bytes that were
/// hashed and signed; a verifier written in another language must never re-derive those bytes by
/// re-serializing `manifest` itself, because two JSON encoders are not guaranteed to produce
/// identical byte sequences (field order, whitespace, numeric formatting). `verify_package` checks
/// both: that the canonical bytes match their signature, and that `manifest` still round-trips to
/// those same bytes, so a caller that mutated one without the other is caught either way.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SignedPackage<T> {
    pub manifest_canonical_base64: String,
    pub manifest_sha256: String,
    pub manifest: T,
    pub signature: PackageSignature,
}

pub(crate) fn sign_package<T: Serialize>(
    manifest: T,
    signing_key: &SigningKey,
) -> Result<SignedPackage<T>, String> {
    let manifest_bytes = serde_json::to_vec(&manifest)
        .map_err(|error| format!("Failed to encode manifest: {error}"))?;
    let manifest_sha256 = sha256_hex(&manifest_bytes);
    let verifying_key = signing_key.verifying_key();
    let public_key = verifying_key.to_bytes();
    let signature = signing_key.sign(&manifest_bytes).to_bytes();
    Ok(SignedPackage {
        manifest_canonical_base64: base64::engine::general_purpose::STANDARD
            .encode(&manifest_bytes),
        manifest_sha256,
        manifest,
        signature: PackageSignature {
            algorithm: SIGNATURE_ALGORITHM.to_string(),
            public_key_hex: digest_hex(public_key),
            signer_key_id: sha256_hex(&public_key),
            signature_hex: digest_hex(signature),
            identity_assurance: SELF_MANAGED_LOCAL_KEY.to_string(),
        },
    })
}

/// The same three checks `scripts/verify-evidence-package.mjs` reports, computed independently in
/// Rust so the in-app verifier command and the standalone script never disagree on what "verified"
/// means. Each check is evaluated on its own rather than short-circuiting, so a caller (or a
/// reader) can see exactly which of the three failed rather than only "verification failed".
#[derive(Debug, Clone)]
pub(crate) struct PackageVerificationChecks {
    pub canonical_bytes_hash_matches: bool,
    pub signature_valid: bool,
    pub manifest_matches_canonical_bytes: bool,
    /// Set only when a structural problem (bad base64, wrong-length key/signature) prevented a
    /// check from running at all; that check is recorded as failed above regardless.
    pub detail: Option<String>,
}

impl PackageVerificationChecks {
    pub fn all_passed(&self) -> bool {
        self.canonical_bytes_hash_matches
            && self.signature_valid
            && self.manifest_matches_canonical_bytes
    }
}

/// Re-verifies a signed package against its own embedded canonical bytes rather than trusting the
/// caller's `T: Serialize` implementation to reproduce them, so a court or a reader never has to
/// trust that this process's JSON encoder agrees with whatever produced the package.
pub(crate) fn verify_package_detailed<T: Serialize>(
    package: &SignedPackage<T>,
) -> PackageVerificationChecks {
    let mut detail = Vec::new();
    let bytes = match base64::engine::general_purpose::STANDARD
        .decode(&package.manifest_canonical_base64)
    {
        Ok(bytes) => Some(bytes),
        Err(error) => {
            detail.push(format!(
                "Canonical manifest bytes are not valid base64: {error}"
            ));
            None
        }
    };
    let hash_matches = bytes
        .as_ref()
        .is_some_and(|bytes| sha256_hex(bytes) == package.manifest_sha256);
    let manifest_matches = bytes.as_ref().is_some_and(|bytes| {
        serde_json::to_vec(&package.manifest)
            .map(|reencoded| &reencoded == bytes)
            .unwrap_or(false)
    });
    let signature_valid = bytes.as_ref().is_some_and(|bytes| {
        let Ok(public_key_bytes) = hex_decode(&package.signature.public_key_hex) else {
            detail.push("Public key is not valid hex.".to_string());
            return false;
        };
        let Ok(public_key_bytes): Result<[u8; 32], _> = public_key_bytes.try_into() else {
            detail.push("Public key is not 32 bytes.".to_string());
            return false;
        };
        // INVARIANT(signature): the signature block sits outside the signed bytes, so every field
        // a reader acts on must be bound here. Revocation and "signed by" both key off
        // `signer_key_id`; unbound, a package signed by a revoked key could claim another key's id.
        if sha256_hex(&public_key_bytes) != package.signature.signer_key_id.to_ascii_lowercase() {
            detail.push("Signer key id is not the SHA-256 of the embedded public key.".to_string());
            return false;
        }
        if package.signature.algorithm != SIGNATURE_ALGORITHM
            || package.signature.identity_assurance != SELF_MANAGED_LOCAL_KEY
        {
            detail.push(
                "Signature algorithm or identity assurance is not one this application produces."
                    .to_string(),
            );
            return false;
        }
        let Ok(verifying_key) = VerifyingKey::from_bytes(&public_key_bytes) else {
            detail.push("Public key bytes do not form a valid Ed25519 key.".to_string());
            return false;
        };
        let Ok(signature_bytes) = hex_decode(&package.signature.signature_hex) else {
            detail.push("Signature is not valid hex.".to_string());
            return false;
        };
        let Ok(signature_bytes): Result<[u8; 64], _> = signature_bytes.try_into() else {
            detail.push("Signature is not 64 bytes.".to_string());
            return false;
        };
        let signature = Signature::from_bytes(&signature_bytes);
        match verifying_key.verify(bytes, &signature) {
            Ok(()) => true,
            Err(error) => {
                detail.push(format!("Signature verification failed: {error}"));
                false
            }
        }
    });
    PackageVerificationChecks {
        canonical_bytes_hash_matches: hash_matches,
        signature_valid,
        manifest_matches_canonical_bytes: manifest_matches,
        detail: (!detail.is_empty()).then(|| detail.join(" ")),
    }
}

/// Convenience wrapper for a caller that only wants pass/fail, such as the self-check every export
/// runs on its own freshly signed package before writing it to disk.
pub(crate) fn verify_package<T: Serialize>(package: &SignedPackage<T>) -> Result<(), String> {
    let checks = verify_package_detailed(package);
    if checks.all_passed() {
        Ok(())
    } else {
        Err(checks
            .detail
            .unwrap_or_else(|| "Package verification failed.".to_string()))
    }
}

pub(crate) const SIGNING_KEY_REVOCATIONS_FILE: &str =
    ".onrecord-export-signing-key-revocations-v1.json";
const REVOCATION_LIST_SCHEMA: &str = "onrecord-signing-key-revocations-v1";

/// Loads this installation's local signing-key revocation list, self-verifying it the same way any
/// other signed package this application produces is verified. A missing file is not an error —
/// an installation that has never rotated its key has an empty list, not an absent one; callers
/// get `SigningKeyRevocations { entries: vec![] }` either way.
pub(crate) async fn load_revocations(
    report_directory: &Path,
) -> Result<SigningKeyRevocations, AppError> {
    let path = report_directory.join(SIGNING_KEY_REVOCATIONS_FILE);
    match tokio::fs::read(&path).await {
        Ok(bytes) => {
            let package: SignedPackage<SigningKeyRevocations> = serde_json::from_slice(&bytes)
                .map_err(|error| {
                    AppError::Export(format!(
                        "Revocation list at '{}' is corrupt: {error}",
                        path.display()
                    ))
                })?;
            verify_package(&package).map_err(|error| {
                AppError::Export(format!(
                    "Revocation list at '{}' failed self-verification: {error}",
                    path.display()
                ))
            })?;
            Ok(package.manifest)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(SigningKeyRevocations {
            schema_version: REVOCATION_LIST_SCHEMA.to_string(),
            entries: Vec::new(),
        }),
        Err(error) => Err(AppError::Export(format!(
            "Failed to read the signing key revocation list: {error}"
        ))),
    }
}

/// Looks up whether `key_id` (a `signer_key_id`, i.e. `sha256(public_key)`) appears in this
/// revocation list. Shared by every verification path — `verify_evidence_package`,
/// `verify_and_unwrap_packet`, and (given the distributed list file) the standalone `.mjs`
/// script's logic — so they can't independently drift on what "revoked" means.
/// A key can carry more than one entry (retired for hygiene, then later found compromised), and
/// the worst one always wins: this returns the COMPROMISED entry if one exists for `key_id`, the
/// RETIRED one otherwise. Returning the *first* entry instead would let `[RETIRED, COMPROMISED]`
/// for the same key read as merely retired — silently downgrading a real compromise record.
pub(crate) fn check_revocation<'a>(
    key_id: &str,
    revocations: &'a SigningKeyRevocations,
) -> Option<&'a RevokedSigningKey> {
    let mut matching = revocations
        .entries
        .iter()
        .filter(|entry| entry.key_id.eq_ignore_ascii_case(key_id));
    matching
        .clone()
        .find(|entry| entry.status != REVOCATION_STATUS_RETIRED)
        .or_else(|| matching.find(|entry| entry.status == REVOCATION_STATUS_RETIRED))
}

pub(crate) const REVOCATION_STATUS_RETIRED: &str = "RETIRED";
pub(crate) const REVOCATION_STATUS_COMPROMISED: &str = "COMPROMISED";

/// Signs `revocations` with `signing_key` and durably replaces the on-disk list: temp file,
/// `sync_all`, then `rename`, so a reader never observes a partially written list, and a crash
/// before the rename leaves the previous (still valid) list in place. Shared by every writer of
/// this file so none of them can drift on what "durably written" actually means.
async fn write_revocations_durably(
    report_directory: &Path,
    revocations: SigningKeyRevocations,
    signing_key: &SigningKey,
) -> Result<(), AppError> {
    let signed = sign_package(revocations, signing_key).map_err(AppError::Export)?;
    verify_package(&signed).map_err(|error| {
        AppError::Export(format!(
            "Freshly signed revocation list failed self-verification: {error}"
        ))
    })?;
    let bytes = serde_json::to_vec_pretty(&signed).map_err(|error| {
        AppError::Export(format!("Failed to encode the revocation list: {error}"))
    })?;
    let revocations_path = report_directory.join(SIGNING_KEY_REVOCATIONS_FILE);
    let temp_revocations_path = report_directory.join(format!(
        "{SIGNING_KEY_REVOCATIONS_FILE}.tmp-{}",
        Uuid::new_v4()
    ));
    {
        let mut file = tokio::fs::File::create(&temp_revocations_path)
            .await
            .map_err(|error| {
                AppError::Export(format!(
                    "Failed to create the signing key revocation list: {error}"
                ))
            })?;
        if let Err(error) = file.write_all(&bytes).await {
            drop(file);
            let _ = tokio::fs::remove_file(&temp_revocations_path).await;
            return Err(AppError::Export(format!(
                "Failed to write the signing key revocation list: {error}"
            )));
        }
        if let Err(error) = file.sync_all().await {
            drop(file);
            let _ = tokio::fs::remove_file(&temp_revocations_path).await;
            return Err(AppError::Export(format!(
                "Failed to sync the signing key revocation list: {error}"
            )));
        }
    }
    tokio::fs::rename(&temp_revocations_path, &revocations_path)
        .await
        .map_err(|error| {
            AppError::Export(format!(
                "Failed to finalize the signing key revocation list: {error}"
            ))
        })
}

/// Escalates a key this installation already knows about (it must already carry at least a
/// RETIRED entry) to COMPROMISED, without rotating the currently active key. The escalation is
/// signed by the *current* active key, matching every other write to this list. This is the path
/// for the ordinary case item 5 exists for: a key rotated for routine hygiene turns out, later, to
/// have actually been compromised. Idempotent: escalating an already-COMPROMISED key is a no-op.
pub(crate) async fn mark_key_compromised(
    report_directory: &Path,
    key_directory: &Path,
    key_id: String,
    reason: String,
) -> Result<(), AppError> {
    let reason = reason.trim().to_string();
    if reason.is_empty() {
        return Err(AppError::Export(
            "Marking a key compromised needs a non-empty reason.".to_string(),
        ));
    }
    let _lock = REVOCATION_WRITE_LOCK.lock().await;
    let _cross_process_lock = acquire_cross_process_revocation_lock(report_directory).await?;
    let mut revocations = load_revocations(report_directory).await?;
    if !revocations
        .entries
        .iter()
        .any(|entry| entry.key_id == key_id)
    {
        return Err(AppError::Export(format!(
            "No revocation entry exists for key {key_id}; only a key this installation has \
             already retired or otherwise recorded can be escalated to compromised."
        )));
    }
    if check_revocation(&key_id, &revocations)
        .is_some_and(|existing| existing.status != REVOCATION_STATUS_RETIRED)
    {
        return Ok(());
    }
    revocations.entries.push(RevokedSigningKey {
        key_id,
        revoked_at: chrono::Utc::now().to_rfc3339(),
        reason,
        status: REVOCATION_STATUS_COMPROMISED.to_string(),
    });
    let signing_key = load_or_create_signing_key(key_directory).await?;
    write_revocations_durably(report_directory, revocations, &signing_key).await
}

/// Rotates this installation's local Ed25519 signing key: archives the current key file under a
/// timestamped name (never overwritten, never deleted — a court or auditor may need to recover a
/// specific historical key later), writes a freshly generated key as the new active signing key,
/// and appends the retired key's id to the local revocation list, re-signed with the new key. Every
/// package this application signs from this point on uses the new key.
///
/// `compromised` decides what every verification path does with packages the old key already
/// signed: `false` (routine hygiene rotation) leaves them trusted — the entry is informational
/// only. `true` marks the key itself untrustworthy, and every package it ever signed, past or
/// future, must read as failing verification everywhere this revocation list is checked.
pub(crate) async fn rotate_signing_key(
    report_directory: &Path,
    key_directory: &Path,
    reason: String,
    compromised: bool,
) -> Result<SigningKeyRotationResult, AppError> {
    let reason = reason.trim().to_string();
    if reason.is_empty() {
        return Err(AppError::Export(
            "Key rotation needs a non-empty reason.".to_string(),
        ));
    }
    let _lock = REVOCATION_WRITE_LOCK.lock().await;
    let _cross_process_lock = acquire_cross_process_revocation_lock(report_directory).await?;
    let old_key = load_or_create_signing_key(key_directory).await?;
    let old_key_id = sha256_hex(&old_key.verifying_key().to_bytes());
    let key_path = key_directory.join(SIGNING_KEY_FILE);

    // The new key is generated and used to build and durably write the revocation list *before*
    // anything about the active key file changes. If any of that fails, the old key is still
    // fully active and nothing on disk has moved — there is no window where a failure could
    // silently drop the compromise record or leave signing capability in a half-rotated state.
    let new_key = SigningKey::from_bytes(&generate_signing_key_seed());
    let new_key_id = sha256_hex(&new_key.verifying_key().to_bytes());

    let mut revocations = load_revocations(report_directory).await?;
    // Idempotent under retry, but never downgrades severity: if a prior attempt already recorded
    // this key at least as severely as this call is asking for (a prior COMPROMISED covers a
    // retry of either kind; a prior RETIRED covers only a retry of a routine rotation), skip the
    // append. A retry that asks for COMPROMISED when only a RETIRED entry exists still appends —
    // otherwise a routine rotation that failed mid-activation would permanently block the user
    // from ever escalating that same key to compromised on retry.
    let already_at_least_as_severe = check_revocation(&old_key_id, &revocations)
        .is_some_and(|existing| existing.status != REVOCATION_STATUS_RETIRED || !compromised);
    if !already_at_least_as_severe {
        revocations.entries.push(RevokedSigningKey {
            key_id: old_key_id.clone(),
            revoked_at: chrono::Utc::now().to_rfc3339(),
            reason,
            status: if compromised {
                REVOCATION_STATUS_COMPROMISED
            } else {
                REVOCATION_STATUS_RETIRED
            }
            .to_string(),
        });
    }
    write_revocations_durably(report_directory, revocations, &new_key).await?;

    // The revocation is already durably recorded at this point, so nothing below can lose it —
    // only fail to finish activating the new key, in which case the old key is still exactly
    // where it always was (never moved, never removed) and remains fully usable.
    //
    // The new key is staged at a private temp path with the same create_new + 0o600 + sync_all
    // guarantees `load_or_create_signing_key` uses for a fresh key, never with default (world-
    // readable) permissions. `key_path` itself is archived via `hard_link`, not `rename`: a hard
    // link leaves the original name still pointing at the old key's bytes throughout, so a
    // concurrent `load_or_create_signing_key` call can never observe `key_path` as missing and
    // race-create an unrelated key that would end up silently overwritten. Activation is a single
    // `rename` of the staged file over `key_path`, which POSIX guarantees is atomic: the file at
    // `key_path` is never briefly absent.
    let staged_key_path =
        key_directory.join(format!("{SIGNING_KEY_FILE}.staged-{}", Uuid::new_v4()));
    let mut options = tokio::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }
    {
        let mut file = options.open(&staged_key_path).await.map_err(|error| {
            AppError::Export(format!(
                "The revocation was recorded, but failed to stage the new signing key: {error}"
            ))
        })?;
        if let Err(error) = file.write_all(&new_key.to_bytes()).await {
            drop(file);
            let _ = tokio::fs::remove_file(&staged_key_path).await;
            return Err(AppError::Export(format!(
                "The revocation was recorded, but failed to write the staged signing key: {error}"
            )));
        }
        if let Err(error) = file.sync_all().await {
            drop(file);
            let _ = tokio::fs::remove_file(&staged_key_path).await;
            return Err(AppError::Export(format!(
                "The revocation was recorded, but failed to sync the staged signing key: {error}"
            )));
        }
    }

    let archived_key_path = key_directory.join(format!(
        "{SIGNING_KEY_FILE}.retired-{}-{}",
        chrono::Utc::now().format("%Y%m%dT%H%M%SZ"),
        &old_key_id[..12]
    ));
    if let Err(error) = tokio::fs::hard_link(&key_path, &archived_key_path).await {
        let _ = tokio::fs::remove_file(&staged_key_path).await;
        return Err(AppError::Export(format!(
            "The revocation was recorded, but failed to archive the retired signing key: {error}"
        )));
    }
    if let Err(error) = tokio::fs::rename(&staged_key_path, &key_path).await {
        // The failed rename left a private key sitting at a predictable, world-discoverable
        // filename (0600 permissions protect it from other users, but it should not linger).
        let _ = tokio::fs::remove_file(&staged_key_path).await;
        return Err(AppError::Export(format!(
            "The revocation was recorded and the old key archived to '{}', but failed to \
             activate the new signing key: {error}. The old key at '{}' is untouched and still \
             active; retry the rotation.",
            archived_key_path.display(),
            key_path.display()
        )));
    }

    Ok(SigningKeyRotationResult {
        retired_key_id: old_key_id,
        new_key_id,
        archived_key_path: archived_key_path.to_string_lossy().into_owned(),
    })
}

pub(crate) fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

#[cfg(test)]
mod tests {
    use super::{
        check_revocation, constant_time_equal, load_or_create_signing_key, load_revocations,
        mark_key_compromised, rotate_signing_key, sha256_hex, sign_package, verify_package,
        verify_package_detailed, write_revocations_durably, REVOCATION_LIST_SCHEMA,
        REVOCATION_STATUS_COMPROMISED, REVOCATION_STATUS_RETIRED, SIGNING_KEY_FILE,
    };
    use crate::types::investigation::{RevokedSigningKey, SigningKeyRevocations};
    use ed25519_dalek::SigningKey;
    use serde::{Deserialize, Serialize};

    #[test]
    fn hashes_and_compares_without_early_byte_exit() {
        let digest = sha256_hex(b"forensic evidence");
        assert_eq!(digest.len(), 64);
        assert!(constant_time_equal(digest.as_bytes(), digest.as_bytes()));
        assert!(!constant_time_equal(digest.as_bytes(), b"different"));
    }

    /// The move must carry an older build's active and retired keys out of the shared reports
    /// directory and must never replace a key already at the destination.
    #[tokio::test]
    async fn legacy_keys_leave_the_reports_directory_without_overwriting() {
        let reports = tempfile::tempdir().unwrap();
        let keys = tempfile::tempdir().unwrap();
        let active = load_or_create_signing_key(reports.path()).await.unwrap();
        let retired_name = format!("{SIGNING_KEY_FILE}.retired-1-abc");
        std::fs::write(reports.path().join(&retired_name), [9_u8; 32]).unwrap();
        std::fs::write(reports.path().join("report.md"), b"kept").unwrap();

        super::relocate_legacy_signing_keys(reports.path(), keys.path()).unwrap();
        assert!(!reports.path().join(SIGNING_KEY_FILE).exists());
        assert!(!reports.path().join(&retired_name).exists());
        assert!(reports.path().join("report.md").exists());
        let moved = load_or_create_signing_key(keys.path()).await.unwrap();
        assert_eq!(moved.to_bytes(), active.to_bytes());

        std::fs::write(reports.path().join(SIGNING_KEY_FILE), [1_u8; 32]).unwrap();
        super::relocate_legacy_signing_keys(reports.path(), keys.path()).unwrap();
        let kept = load_or_create_signing_key(keys.path()).await.unwrap();
        assert_eq!(kept.to_bytes(), active.to_bytes());
    }

    #[tokio::test]
    async fn export_signing_key_is_created_once_and_reused() {
        let directory = tempfile::tempdir().unwrap();
        let first = load_or_create_signing_key(directory.path()).await.unwrap();
        let second = load_or_create_signing_key(directory.path()).await.unwrap();
        assert_eq!(first.verifying_key(), second.verifying_key());
        assert_eq!(
            tokio::fs::read(directory.path().join(SIGNING_KEY_FILE))
                .await
                .unwrap()
                .len(),
            32
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn export_signing_key_file_is_created_owner_only_readable() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        load_or_create_signing_key(directory.path()).await.unwrap();
        let permissions = tokio::fs::metadata(directory.path().join(SIGNING_KEY_FILE))
            .await
            .unwrap()
            .permissions();
        assert_eq!(
            permissions.mode() & 0o777,
            0o600,
            "the signing key must never be group- or world-readable"
        );
    }

    /// Proves the atomic-activation design directly, not just its outcome: the archived copy
    /// (hard-linked, not renamed) holds exactly the old key's bytes, the active file now holds a
    /// *different* key that signs correctly, and — the case `rename`-based archival could get
    /// wrong — `key_path` was never left pointing at nothing in between.
    #[cfg(unix)]
    #[tokio::test]
    async fn rotate_signing_key_activates_atomically_and_archives_the_exact_old_key() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let old_key = load_or_create_signing_key(directory.path()).await.unwrap();
        let old_key_bytes = tokio::fs::read(directory.path().join(SIGNING_KEY_FILE))
            .await
            .unwrap();

        let rotated = rotate_signing_key(
            directory.path(),
            directory.path(),
            "Routine rotation.".to_string(),
            false,
        )
        .await
        .unwrap();

        let archived_bytes = tokio::fs::read(&rotated.archived_key_path).await.unwrap();
        assert_eq!(
            archived_bytes, old_key_bytes,
            "the archived file must hold exactly the retired key's bytes"
        );

        let active_key = load_or_create_signing_key(directory.path()).await.unwrap();
        assert_ne!(
            active_key.verifying_key(),
            old_key.verifying_key(),
            "the active key must now be a different key from the retired one"
        );
        assert_eq!(
            sha256_hex(&active_key.verifying_key().to_bytes()),
            rotated.new_key_id
        );

        let active_permissions = tokio::fs::metadata(directory.path().join(SIGNING_KEY_FILE))
            .await
            .unwrap()
            .permissions();
        assert_eq!(
            active_permissions.mode() & 0o777,
            0o600,
            "the newly activated key must be owner-only readable, not default (world-readable) permissions"
        );
    }

    #[tokio::test]
    async fn rotate_signing_key_records_retired_or_compromised_status_correctly() {
        let directory = tempfile::tempdir().unwrap();
        load_or_create_signing_key(directory.path()).await.unwrap();
        let retired = rotate_signing_key(
            directory.path(),
            directory.path(),
            "Routine.".to_string(),
            false,
        )
        .await
        .unwrap();
        let revocations = load_revocations(directory.path()).await.unwrap();
        assert_eq!(
            check_revocation(&retired.retired_key_id, &revocations)
                .unwrap()
                .status,
            REVOCATION_STATUS_RETIRED
        );

        let compromised = rotate_signing_key(
            directory.path(),
            directory.path(),
            "Compromise.".to_string(),
            true,
        )
        .await
        .unwrap();
        let revocations = load_revocations(directory.path()).await.unwrap();
        assert_eq!(
            check_revocation(&compromised.retired_key_id, &revocations)
                .unwrap()
                .status,
            REVOCATION_STATUS_COMPROMISED
        );
        // The first (retired) entry must still be present and unchanged — rotation appends, it
        // never rewrites prior history.
        assert_eq!(
            check_revocation(&retired.retired_key_id, &revocations)
                .unwrap()
                .status,
            REVOCATION_STATUS_RETIRED
        );
    }

    fn revoked(key_id: &str, status: &str) -> RevokedSigningKey {
        RevokedSigningKey {
            key_id: key_id.to_string(),
            revoked_at: chrono::Utc::now().to_rfc3339(),
            reason: "test".to_string(),
            status: status.to_string(),
        }
    }

    /// The signature block sits outside the signed bytes, so a packet can carry its signer id in
    /// any case and still verify. A revocation must match all of them.
    #[test]
    fn check_revocation_matches_the_signer_id_in_any_case() {
        let revocations = SigningKeyRevocations {
            schema_version: REVOCATION_LIST_SCHEMA.to_string(),
            entries: vec![revoked("abcdef0123", REVOCATION_STATUS_COMPROMISED)],
        };
        for spelling in ["abcdef0123", "ABCDEF0123", "AbCdEf0123"] {
            assert!(
                check_revocation(spelling, &revocations).is_some(),
                "{spelling}"
            );
        }
        assert!(check_revocation("abcdef0124", &revocations).is_none());
    }

    /// Returning the first matching entry instead of the worst would let `[RETIRED, COMPROMISED]`
    /// read as merely retired. Checked in both orders since a naive `.find()` is order-sensitive.
    #[test]
    fn check_revocation_returns_the_worst_severity_entry_regardless_of_order() {
        for statuses in [
            [REVOCATION_STATUS_RETIRED, REVOCATION_STATUS_COMPROMISED],
            [REVOCATION_STATUS_COMPROMISED, REVOCATION_STATUS_RETIRED],
        ] {
            let revocations = SigningKeyRevocations {
                schema_version: REVOCATION_LIST_SCHEMA.to_string(),
                entries: statuses
                    .iter()
                    .map(|status| revoked("key-a", status))
                    .collect(),
            };
            assert_eq!(
                check_revocation("key-a", &revocations).unwrap().status,
                REVOCATION_STATUS_COMPROMISED,
                "{statuses:?}"
            );
        }
    }

    #[tokio::test]
    async fn revocation_writes_refuse_an_empty_reason_or_an_unrecorded_key() {
        let directory = tempfile::tempdir().unwrap();
        load_or_create_signing_key(directory.path()).await.unwrap();
        assert!(
            rotate_signing_key(directory.path(), directory.path(), "   ".to_string(), false)
                .await
                .is_err()
        );
        let retired = rotate_signing_key(
            directory.path(),
            directory.path(),
            "Routine.".to_string(),
            false,
        )
        .await
        .unwrap();
        assert!(mark_key_compromised(
            directory.path(),
            directory.path(),
            retired.retired_key_id,
            "   ".to_string()
        )
        .await
        .is_err());
        assert!(
            mark_key_compromised(
                directory.path(),
                directory.path(),
                "never-recorded-key-id".to_string(),
                "Reason.".to_string(),
            )
            .await
            .is_err(),
            "escalating a key this installation never recorded must fail, not invent an entry"
        );
    }

    #[tokio::test]
    async fn mark_key_compromised_escalates_a_retired_key_once() {
        let directory = tempfile::tempdir().unwrap();
        load_or_create_signing_key(directory.path()).await.unwrap();
        let retired = rotate_signing_key(
            directory.path(),
            directory.path(),
            "Routine.".to_string(),
            false,
        )
        .await
        .unwrap();

        mark_key_compromised(
            directory.path(),
            directory.path(),
            retired.retired_key_id.clone(),
            "First report.".to_string(),
        )
        .await
        .unwrap();
        mark_key_compromised(
            directory.path(),
            directory.path(),
            retired.retired_key_id.clone(),
            "Second report of the same compromise.".to_string(),
        )
        .await
        .unwrap();

        let revocations = load_revocations(directory.path()).await.unwrap();
        let matching: Vec<_> = revocations
            .entries
            .iter()
            .filter(|entry| entry.key_id == retired.retired_key_id)
            .collect();
        // `rotate_signing_key(compromised: false)` writes the original RETIRED entry, and the
        // first `mark_key_compromised` call appends a COMPROMISED entry on top of it (history is
        // preserved by design, never overwritten) — so 2 is correct after one escalation. The
        // invariant under test is that a *second*, repeated escalation call does not push a THIRD
        // entry for the same key.
        assert_eq!(
            matching.len(),
            2,
            "a repeated escalation of the same key must not append a second COMPROMISED entry \
             on top of the first"
        );
        assert_eq!(
            matching
                .iter()
                .filter(|entry| entry.status == REVOCATION_STATUS_COMPROMISED)
                .count(),
            1
        );
        assert_eq!(
            check_revocation(&retired.retired_key_id, &revocations)
                .unwrap()
                .status,
            REVOCATION_STATUS_COMPROMISED,
            "escalation must turn an already-retired key into a compromised one"
        );
    }

    /// The idempotent-retry guard in `rotate_signing_key` must never let a retry that asks for
    /// `compromised: true` be silently absorbed by an existing `RETIRED` entry for the same key —
    /// that would downgrade a real compromise report to a no-op. Simulates the retry directly by
    /// seeding a `RETIRED` entry for the still-active key before rotating it with `compromised:
    /// true`, mirroring a prior attempt that recorded the retirement but failed before swapping
    /// the key file.
    #[tokio::test]
    async fn rotating_a_key_with_an_existing_retired_entry_upgrades_it_to_compromised() {
        let directory = tempfile::tempdir().unwrap();
        let old_key = load_or_create_signing_key(directory.path()).await.unwrap();
        let old_key_id = sha256_hex(&old_key.verifying_key().to_bytes());

        let seeded = SigningKeyRevocations {
            schema_version: REVOCATION_LIST_SCHEMA.to_string(),
            entries: vec![revoked(&old_key_id, REVOCATION_STATUS_RETIRED)],
        };
        write_revocations_durably(directory.path(), seeded, &old_key)
            .await
            .unwrap();

        let rotated = rotate_signing_key(
            directory.path(),
            directory.path(),
            "Escalating to compromised.".to_string(),
            true,
        )
        .await
        .unwrap();
        assert_eq!(rotated.retired_key_id, old_key_id);

        let revocations = load_revocations(directory.path()).await.unwrap();
        assert_eq!(
            check_revocation(&old_key_id, &revocations).unwrap().status,
            REVOCATION_STATUS_COMPROMISED,
            "a rotation that escalates severity for an already-retired key must not be skipped \
             by the idempotent-retry guard"
        );
    }

    /// `mark_key_compromised` and `rotate_signing_key` each read the revocation list, decide what
    /// to append, and write the whole list back. Run genuinely concurrently on a multi-thread
    /// runtime (not the normal case — the shipped UI's shared busy/request-gate state serializes
    /// these — but nothing at this layer enforces that), the loser's read is missing the winner's
    /// append and its write silently erases it. Repeated across several fresh directories to make
    /// an unlucky interleaving unlikely to hide the race.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_mark_compromised_and_rotate_do_not_lose_either_entrys_write() {
        for _ in 0..20 {
            let directory = tempfile::tempdir().unwrap();
            load_or_create_signing_key(directory.path()).await.unwrap();
            let first_rotation = rotate_signing_key(
                directory.path(),
                directory.path(),
                "Routine.".to_string(),
                false,
            )
            .await
            .unwrap();

            let (mark_result, second_rotation_result) = tokio::join!(
                mark_key_compromised(
                    directory.path(),
                    directory.path(),
                    first_rotation.retired_key_id.clone(),
                    "Found on a leaked backup.".to_string(),
                ),
                rotate_signing_key(
                    directory.path(),
                    directory.path(),
                    "Second rotation.".to_string(),
                    false
                ),
            );
            mark_result.unwrap();
            let second_rotation = second_rotation_result.unwrap();

            let revocations = load_revocations(directory.path()).await.unwrap();
            assert_eq!(
                check_revocation(&first_rotation.retired_key_id, &revocations)
                    .unwrap()
                    .status,
                REVOCATION_STATUS_COMPROMISED,
                "a concurrent rotation must never silently drop a compromise another call recorded"
            );
            assert_eq!(
                check_revocation(&second_rotation.retired_key_id, &revocations)
                    .unwrap()
                    .status,
                REVOCATION_STATUS_RETIRED,
                "a concurrent mark_key_compromised must never silently drop a rotation's own entry"
            );
        }
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    struct Manifest {
        claim: String,
    }

    /// Each tampering fails exactly its own check, and the pass/fail wrapper rejects all of them.
    #[test]
    fn each_of_the_three_checks_fails_independently_of_the_others() {
        let key = SigningKey::from_bytes(&[11_u8; 32]);
        let signed = |claim: &str, key: &SigningKey| {
            sign_package(
                Manifest {
                    claim: claim.into(),
                },
                key,
            )
            .unwrap()
        };
        let package = signed("unaltered", &key);
        assert!(verify_package_detailed(&package).all_passed());
        verify_package(&package).expect("an untouched package must verify");

        let mut manifest_edited = package.clone();
        manifest_edited.manifest.claim = "edited".into();
        let mut hash_corrupted = package.clone();
        hash_corrupted.manifest_sha256 = "0".repeat(64);
        let mut signature_corrupted = package.clone();
        signature_corrupted.signature.signature_hex = "00".repeat(64);
        let mut foreign_bytes = package;
        foreign_bytes.manifest_canonical_base64 = signed(
            "altered after signing",
            &SigningKey::from_bytes(&[9_u8; 32]),
        )
        .manifest_canonical_base64;

        // Expected: (hash matches, signature valid, manifest matches).
        for (name, tampered, expected) in [
            ("manifest field", manifest_edited, (true, true, false)),
            ("recorded hash", hash_corrupted, (false, true, true)),
            ("signature", signature_corrupted, (true, false, true)),
            ("canonical bytes", foreign_bytes, (false, false, false)),
        ] {
            let checks = verify_package_detailed(&tampered);
            assert_eq!(
                (
                    checks.canonical_bytes_hash_matches,
                    checks.signature_valid,
                    checks.manifest_matches_canonical_bytes,
                ),
                expected,
                "{name}"
            );
            assert!(verify_package(&tampered).is_err(), "{name}");
        }
    }
}
