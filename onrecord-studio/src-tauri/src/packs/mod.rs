//! Data packs: City datasets retrieved and compiled by the pack builder, signed with the
//! publisher key, and imported by the app from their original files (`docs/data-packs.md`).
//!
//! A pack is a directory holding `manifest.json` (a [`SignedPackage`] of [`PackManifest`]) and the
//! files it lists. Nothing in a pack is trusted until [`verify_manifest`] has checked the
//! signature and that the key is one this app pins, and no file is used until
//! [`check_pack_file`] has checked its size and SHA-256 against the manifest.

use crate::error::AppError;
use crate::integrity::{sha256_file, verify_package_detailed, SignedPackage};
use ed25519_dalek::VerifyingKey;
use serde::{Deserialize, Serialize};
use std::path::{Component, Path};

#[cfg(feature = "pack-builder")]
pub mod builder;
pub(crate) mod catalog;
#[cfg(feature = "pack-builder")]
mod changes;
pub mod conformed;
pub(crate) mod import;
pub(crate) mod published;
#[cfg(feature = "pack-builder")]
pub mod snapshot;

/// The manifest's file name inside a pack directory.
pub(crate) const PACK_MANIFEST_FILE: &str = "manifest.json";

/// Which packs a pinned publisher key may sign. A key signs only its own kind of pack, so the CI
/// key, held where the scheduled job runs, cannot stand in for the owner's curated releases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PackKeyScope {
    /// Every pack but the snapshot pack.
    Curated,
    /// Only [`catalog::SNAPSHOT_PACK`].
    Snapshot,
}

impl PackKeyScope {
    pub(crate) fn signs(self, pack: &str) -> bool {
        (pack == catalog::SNAPSHOT_PACK) == (self == Self::Snapshot)
    }
}

/// Pinned pack publisher keys as `(kid, hex of the 32-byte Ed25519 key, the packs it signs)`.
/// Separate from the per-installation export key and the entitlement keys; rotation adds an entry.
/// There is no runtime route to add a key: tests pass their own keys to [`verify_pack`].
/// `pack-2026-1` signs the curated releases from the owner's Keychain; `pack-ci-2026-1` signs the
/// daily snapshot pack on the CI runner.
pub(crate) const PACK_PUBLIC_KEYS: &[(&str, &str, PackKeyScope)] = &[
    (
        "pack-2026-1",
        "b42202850cd9ecda4ab2b96548891d3a2a8275f74dde76bba2153826089ca8ff",
        PackKeyScope::Curated,
    ),
    (
        "pack-ci-2026-1",
        "3049662bc87c61409d8b8a515d60adf1420a32504bd4f6215ac0c121550a318f",
        PackKeyScope::Snapshot,
    ),
];

/// A publisher key verification accepts, with its pinned name and the packs it signs.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PinnedPackKey {
    pub key_id: &'static str,
    pub key: VerifyingKey,
    pub scope: PackKeyScope,
}

#[cfg(test)]
impl PinnedPackKey {
    /// A test key pinned as `pack-test`.
    pub(crate) fn test(key: VerifyingKey, scope: PackKeyScope) -> Self {
        Self {
            key_id: "pack-test",
            key,
            scope,
        }
    }
}

/// The licence a pack's files are published under, with the publisher's required disclaimer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PackLicence {
    pub id: String,
    pub url: String,
    pub disclaimer: String,
}

/// One file in a pack: an original as the publisher served it, or a Parquet copy of one.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PackFile {
    /// Relative path inside the pack directory.
    pub path: String,
    /// The conformed table the file belongs to, such as `police_calls`.
    pub table: String,
    pub format: PackFileFormat,
    pub source_url: String,
    /// When the pack builder retrieved the original, RFC 3339.
    pub retrieved_at: String,
    pub sha256: String,
    pub bytes: u64,
    pub rows: u64,
    /// For a Parquet copy, the original's rows the import rejected as unreadable, which the copy
    /// lacks. Absent on an original: nothing was excluded from it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excluded_rows: Option<u64>,
    /// The publisher's year for a table split by year.
    pub year: Option<u16>,
    /// The file covers a period not yet over, so later rows may still be published.
    pub partial: bool,
    /// The publisher's freshness markers as served, used to fetch only what changed later.
    pub last_modified: Option<String>,
    pub etag: Option<String>,
    /// For a change record, what it compares and how many rows of each kind it holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changes: Option<ChangeSummary>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum PackFileFormat {
    Original,
    Parquet,
    /// A change record between two snapshots of one publisher file. Never imported.
    Changes,
}

/// What a change record compares and how many of its rows are of each kind.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChangeSummary {
    /// The earlier snapshot's original, as the manifest lists it.
    pub previous: String,
    /// The later snapshot's original.
    pub current: String,
    pub added: u64,
    pub removed: u64,
    /// Rows naming one changed field of a record present in both snapshots.
    pub changed: u64,
    /// More than half the records changed, so changed records are listed as removed and added
    /// rather than field by field.
    pub whole_records: bool,
}

/// A retrieval whose bytes equalled the latest snapshot of its file. It says the file had not
/// changed when checked, nothing about the file between checks.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PackCheck {
    pub table: String,
    pub source_url: String,
    /// When the file was retrieved for the check, RFC 3339.
    pub checked_at: String,
    pub sha256: String,
    pub bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_modified: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
    /// The snapshot original whose bytes the retrieval matched.
    pub matches: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PackManifest {
    /// The pack's name, such as `san-diego-money`.
    pub pack: String,
    pub pack_version: String,
    /// The conformed schema's version; a new column bumps the minor version.
    pub schema_version: String,
    /// When the pack builder finished, RFC 3339. Rows are the builder's retrieval as of then.
    pub built_at: String,
    pub licence: PackLicence,
    pub files: Vec<PackFile>,
    /// The `PACK_PUBLIC_KEYS` name of the signing key, as the signer states it; the signature's
    /// `signerKeyId` is the key's SHA-256, which verification binds to the key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
    /// A snapshot pack's checks that found a file unchanged, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<PackCheck>,
}

/// A manifest whose signature and publisher key have been checked, and whose every listed path
/// stays inside the pack. Its files are checked only as [`verify_pack`] or a caller of
/// [`check_pack_file`] checks them.
#[derive(Debug)]
pub(crate) struct SignedManifest {
    pub manifest: PackManifest,
    pub signer_key_id: String,
}

/// The app's pinned publisher keys. A malformed entry is skipped, which can only reduce what
/// verifies.
pub(crate) fn pinned_pack_keys() -> Vec<PinnedPackKey> {
    PACK_PUBLIC_KEYS
        .iter()
        .filter_map(|(key_id, hex, scope)| {
            let bytes: [u8; 32] = decode_hex(hex)?.try_into().ok()?;
            Some(PinnedPackKey {
                key_id,
                key: VerifyingKey::from_bytes(&bytes).ok()?,
                scope: *scope,
            })
        })
        .collect()
}

/// The pinned entry for the key whose hex is `public_key_hex`, refused unless it may sign `pack`.
/// The one rule for who signs what: verification applies it to every manifest, and the builder to
/// its key before a run.
pub(crate) fn pinned_signer<'a>(
    pinned: &'a [PinnedPackKey],
    public_key_hex: &str,
    pack: &str,
) -> Result<&'a PinnedPackKey, AppError> {
    let signer = pinned
        .iter()
        .find(|pin| {
            crate::integrity::digest_hex(pin.key.to_bytes()).eq_ignore_ascii_case(public_key_hex)
        })
        .ok_or_else(|| refused("it is not signed by a pack publisher this app trusts"))?;
    if !signer.scope.signs(pack) {
        return Err(refused(format!(
            "it is signed by {}, a key that does not sign {pack}",
            signer.key_id
        )));
    }
    Ok(signer)
}

/// Of several retrievals, the latest; one whose time does not parse counts as earliest.
pub(crate) fn latest_retrieved<'a>(
    files: impl IntoIterator<Item = &'a PackFile>,
) -> Option<&'a PackFile> {
    files
        .into_iter()
        .max_by_key(|file| chrono::DateTime::parse_from_rfc3339(&file.retrieved_at).ok())
}

fn decode_hex(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok())
        .collect()
}

fn refused(reason: impl Into<String>) -> AppError {
    AppError::Provenance(format!("Data pack refused: {}", reason.into()))
}

/// A listed path that stays inside the pack: relative, with no `..`, root or prefix part.
pub(crate) fn pack_relative(path: &str) -> Option<&Path> {
    let relative = Path::new(path);
    (!path.is_empty()
        && relative
            .components()
            .all(|part| matches!(part, Component::Normal(_))))
    .then_some(relative)
}

/// Checks a signed manifest: its signature over its own canonical bytes, that the signing key is
/// one of `pinned` and may sign the manifest's pack ([`pinned_signer`]), that a key name the
/// manifest states is that key's, and that every listed path stays inside the pack. Reads no
/// listed file.
pub(crate) fn verify_manifest_bytes(
    bytes: &[u8],
    pinned: &[PinnedPackKey],
) -> Result<SignedManifest, AppError> {
    let package: SignedPackage<PackManifest> = serde_json::from_slice(bytes)
        .map_err(|error| refused(format!("its manifest is not a pack manifest: {error}")))?;
    let checks = verify_package_detailed(&package);
    if !checks.all_passed() {
        return Err(refused(checks.detail.unwrap_or_else(|| {
            "its manifest does not match its signature".into()
        })));
    }
    let signer = pinned_signer(
        pinned,
        &package.signature.public_key_hex,
        &package.manifest.pack,
    )?;
    if let Some(named) = package
        .manifest
        .key_id
        .as_deref()
        .filter(|named| *named != signer.key_id)
    {
        return Err(refused(format!(
            "it names {named} as its key but is signed by {}",
            signer.key_id
        )));
    }
    if let Some(file) = package
        .manifest
        .files
        .iter()
        .find(|file| pack_relative(&file.path).is_none())
    {
        return Err(refused(format!(
            "it lists a path outside the pack: {}",
            file.path
        )));
    }
    let manifest = &package.manifest;
    if let Some(check) = manifest.checks.iter().find(|check| {
        !manifest.files.iter().any(|file| {
            file.format == PackFileFormat::Original
                && file.path == check.matches
                && file.bytes == check.bytes
                && file.sha256.eq_ignore_ascii_case(&check.sha256)
        })
    }) {
        return Err(refused(format!(
            "it records a check matching no original it lists: {}",
            check.matches
        )));
    }
    Ok(SignedManifest {
        signer_key_id: package.signature.signer_key_id.clone(),
        manifest: package.manifest,
    })
}

/// [`verify_manifest_bytes`] over the manifest in `directory`.
pub(crate) fn verify_manifest(
    directory: &Path,
    pinned: &[PinnedPackKey],
) -> Result<SignedManifest, AppError> {
    let bytes = std::fs::read(directory.join(PACK_MANIFEST_FILE))
        .map_err(|error| refused(format!("its manifest could not be read: {error}")))?;
    verify_manifest_bytes(&bytes, pinned)
}

/// Checks that `path` holds `file` as its manifest lists it: a regular file with the listed size
/// and SHA-256.
pub(crate) fn check_pack_file(path: &Path, file: &PackFile) -> Result<(), AppError> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| refused(format!("{} is missing: {error}", file.path)))?;
    if !metadata.is_file() {
        return Err(refused(format!("{} is not a regular file", file.path)));
    }
    if metadata.len() != file.bytes {
        return Err(refused(format!(
            "{} is not the size its manifest lists",
            file.path
        )));
    }
    let digest = sha256_file(path)
        .map_err(|error| refused(format!("{} could not be read: {error}", file.path)))?;
    if !digest.eq_ignore_ascii_case(&file.sha256) {
        return Err(refused(format!(
            "{} does not match its manifest's SHA-256",
            file.path
        )));
    }
    Ok(())
}

/// Checks a pack before anything in it is used: its manifest ([`verify_manifest`]) and every
/// listed file ([`check_pack_file`]).
pub(crate) fn verify_pack(
    directory: &Path,
    pinned: &[PinnedPackKey],
) -> Result<SignedManifest, AppError> {
    let signed = verify_manifest(directory, pinned)?;
    for file in &signed.manifest.files {
        check_pack_file(&directory.join(&file.path), file)?;
    }
    Ok(signed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrity::sign_package;
    use ed25519_dalek::SigningKey;

    const ORIGINAL: &[u8] = b"incident_num,date_time\nE1,2025-01-01 00:00:00\n";

    fn manifest(sha256: String, bytes: u64) -> PackManifest {
        PackManifest {
            pack: "san-diego-reports".into(),
            pack_version: "2026.10.6".into(),
            schema_version: "1.0".into(),
            built_at: "2026-10-06T00:00:00Z".into(),
            licence: PackLicence {
                id: "ODC-PDDL-1.0".into(),
                url: "https://opendefinition.org/licenses/odc-pddl/".into(),
                disclaimer: "Informational only.".into(),
            },
            files: vec![PackFile {
                path: "originals/police_calls/2025.csv".into(),
                table: "police_calls".into(),
                format: PackFileFormat::Original,
                source_url: "https://seshat.datasd.org/police_calls_for_service/pd_calls_for_service_2025_datasd.csv".into(),
                retrieved_at: "2026-10-06T00:00:00Z".into(),
                sha256,
                bytes,
                rows: 1,
                excluded_rows: None,
                year: Some(2025),
                partial: false,
                last_modified: None,
                etag: None,
                changes: None,
            }],
            key_id: None,
            checks: Vec::new(),
        }
    }

    /// Writes a signed pack and returns its directory, the key, and the manifest's path.
    fn signed_pack() -> (tempfile::TempDir, SigningKey) {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("originals/police_calls/2025.csv");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, ORIGINAL).unwrap();
        let key = SigningKey::from_bytes(&[5_u8; 32]);
        let package = sign_package(
            manifest(
                crate::integrity::sha256_hex(ORIGINAL),
                ORIGINAL.len() as u64,
            ),
            &key,
        )
        .unwrap();
        std::fs::write(
            directory.path().join(PACK_MANIFEST_FILE),
            serde_json::to_vec_pretty(&package).unwrap(),
        )
        .unwrap();
        (directory, key)
    }

    fn flip_last_byte(path: &Path) {
        let mut bytes = std::fs::read(path).unwrap();
        let last = bytes.len() - 2;
        bytes[last] ^= 1;
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn a_pack_verifies_only_whole_and_signed_by_a_pinned_key() {
        let (directory, key) = signed_pack();
        let pinned = [PinnedPackKey::test(
            key.verifying_key(),
            PackKeyScope::Curated,
        )];
        let pack = verify_pack(directory.path(), &pinned).unwrap();
        assert_eq!(pack.manifest.files.len(), 1);

        let stranger = [PinnedPackKey::test(
            SigningKey::from_bytes(&[6_u8; 32]).verifying_key(),
            PackKeyScope::Curated,
        )];
        assert!(verify_pack(directory.path(), &stranger).is_err());

        // One byte of one file changed.
        let (directory, key) = signed_pack();
        flip_last_byte(&directory.path().join("originals/police_calls/2025.csv"));
        let refused = verify_pack(
            directory.path(),
            &[PinnedPackKey::test(
                key.verifying_key(),
                PackKeyScope::Curated,
            )],
        )
        .unwrap_err();
        assert!(refused.to_string().contains("SHA-256"), "{refused}");

        // One byte of the signed manifest changed: its listed row count.
        let (directory, key) = signed_pack();
        let manifest_path = directory.path().join(PACK_MANIFEST_FILE);
        let text = std::fs::read_to_string(&manifest_path).unwrap();
        std::fs::write(
            &manifest_path,
            text.replacen("\"rows\": 1", "\"rows\": 2", 1),
        )
        .unwrap();
        assert!(verify_pack(
            directory.path(),
            &[PinnedPackKey::test(
                key.verifying_key(),
                PackKeyScope::Curated
            )]
        )
        .is_err());
    }

    /// Checks a pack the builder produced (`ONRECORD_PACK_DIR`) against the key this app pins.
    #[test]
    #[ignore = "needs ONRECORD_PACK_DIR"]
    fn a_built_pack_verifies_against_the_pinned_key() {
        let directory = std::env::var("ONRECORD_PACK_DIR").unwrap();
        let pack = verify_pack(Path::new(&directory), &pinned_pack_keys()).unwrap();
        assert!(!pack.manifest.files.is_empty());
        assert!(pack
            .manifest
            .files
            .iter()
            .all(|file| Path::new(&directory).join(&file.path).is_file()));
    }

    /// A manifest is checked with none of its files present, as before a download: tampered, or
    /// signed by a key this app does not pin, or listing a path outside the pack, it is refused.
    #[test]
    fn a_manifest_alone_verifies_only_untampered_pinned_and_inside_the_pack() {
        let key = SigningKey::from_bytes(&[5_u8; 32]);
        let pinned = [PinnedPackKey::test(
            key.verifying_key(),
            PackKeyScope::Curated,
        )];
        let signed = |manifest: PackManifest| {
            serde_json::to_vec_pretty(&sign_package(manifest, &key).unwrap()).unwrap()
        };
        let bytes = signed(manifest("ab".repeat(32), 7));
        let verified = verify_manifest_bytes(&bytes, &pinned).unwrap();
        assert_eq!(verified.manifest.files.len(), 1);

        let stranger = [PinnedPackKey::test(
            SigningKey::from_bytes(&[6_u8; 32]).verifying_key(),
            PackKeyScope::Curated,
        )];
        let refused = verify_manifest_bytes(&bytes, &stranger).unwrap_err();
        assert!(refused.to_string().contains("trusts"), "{refused}");

        let tampered =
            String::from_utf8(bytes)
                .unwrap()
                .replacen("\"bytes\": 7", "\"bytes\": 8", 1);
        assert!(verify_manifest_bytes(tampered.as_bytes(), &pinned).is_err());

        let mut escaping = manifest("ab".repeat(32), 7);
        escaping.files[0].path = "originals/../../escape.csv".into();
        let refused = verify_manifest_bytes(&signed(escaping), &pinned).unwrap_err();
        assert!(
            refused.to_string().contains("outside the pack"),
            "{refused}"
        );
    }

    /// A pinned key signs only its own kind of pack, and a key name the manifest states must be
    /// the signing key's: a leaked CI key cannot pass as a curated release, nor name the owner's.
    #[test]
    fn a_key_verifies_only_the_packs_it_signs_under_its_own_name() {
        let (curated, ci) = (
            SigningKey::from_bytes(&[5_u8; 32]),
            SigningKey::from_bytes(&[6_u8; 32]),
        );
        let pinned = [
            PinnedPackKey {
                key_id: "pack-owner",
                key: curated.verifying_key(),
                scope: PackKeyScope::Curated,
            },
            PinnedPackKey {
                key_id: "pack-ci",
                key: ci.verifying_key(),
                scope: PackKeyScope::Snapshot,
            },
        ];
        let verify = |manifest: PackManifest, key: &SigningKey| {
            verify_manifest_bytes(
                &serde_json::to_vec(&sign_package(manifest, key).unwrap()).unwrap(),
                &pinned,
            )
        };
        let reports = manifest("ab".repeat(32), 7);
        let mut snapshots = reports.clone();
        snapshots.pack = catalog::SNAPSHOT_PACK.into();
        snapshots.key_id = Some("pack-ci".into());

        assert!(verify(reports.clone(), &curated).is_ok());
        assert!(verify(snapshots.clone(), &ci).is_ok());
        for (manifest, key) in [(reports, &ci), (snapshots.clone(), &curated)] {
            let refused = verify(manifest, key).unwrap_err();
            assert!(refused.to_string().contains("does not sign"), "{refused}");
        }
        snapshots.key_id = Some("pack-owner".into());
        let refused = verify(snapshots, &ci).unwrap_err();
        assert!(
            refused.to_string().contains("names pack-owner"),
            "{refused}"
        );
    }

    /// The published packs were signed before checks, key names and change records existed;
    /// their manifests must still encode to the bytes that were signed.
    #[test]
    fn a_manifest_without_snapshot_fields_encodes_as_before_them() {
        let encoded = serde_json::to_string(&manifest("ab".repeat(32), 7)).unwrap();
        assert_eq!(
            encoded,
            format!(
                r#"{{"pack":"san-diego-reports","packVersion":"2026.10.6","schemaVersion":"1.0","builtAt":"2026-10-06T00:00:00Z","licence":{{"id":"ODC-PDDL-1.0","url":"https://opendefinition.org/licenses/odc-pddl/","disclaimer":"Informational only."}},"files":[{{"path":"originals/police_calls/2025.csv","table":"police_calls","format":"ORIGINAL","sourceUrl":"https://seshat.datasd.org/police_calls_for_service/pd_calls_for_service_2025_datasd.csv","retrievedAt":"2026-10-06T00:00:00Z","sha256":"{}","bytes":7,"rows":1,"year":2025,"partial":false,"lastModified":null,"etag":null}}]}}"#,
                "ab".repeat(32)
            )
        );
    }

    /// A check states that a retrieval's bytes equalled a snapshot the manifest lists; one naming
    /// other bytes, or no listed original, is refused.
    #[test]
    fn a_check_verifies_only_against_the_original_it_matched() {
        let key = SigningKey::from_bytes(&[5_u8; 32]);
        let signed = |manifest: PackManifest| {
            serde_json::to_vec(&sign_package(manifest, &key).unwrap()).unwrap()
        };
        let check = PackCheck {
            table: "police_calls".into(),
            source_url: "https://seshat.datasd.org/x.csv".into(),
            checked_at: "2026-10-07T00:00:00Z".into(),
            sha256: "ab".repeat(32),
            bytes: 7,
            last_modified: None,
            etag: None,
            matches: "originals/police_calls/2025.csv".into(),
        };
        let mut checked = manifest("ab".repeat(32), 7);
        checked.checks.push(check.clone());
        assert!(verify_manifest_bytes(
            &signed(checked.clone()),
            &[PinnedPackKey::test(
                key.verifying_key(),
                PackKeyScope::Curated
            )]
        )
        .is_ok());
        for tampered in [
            PackCheck {
                sha256: "cd".repeat(32),
                ..check.clone()
            },
            PackCheck {
                bytes: 8,
                ..check.clone()
            },
            PackCheck {
                matches: "originals/police_calls/2024.csv".into(),
                ..check
            },
        ] {
            checked.checks = vec![tampered];
            let refused = verify_manifest_bytes(
                &signed(checked.clone()),
                &[PinnedPackKey::test(
                    key.verifying_key(),
                    PackKeyScope::Curated,
                )],
            )
            .unwrap_err();
            assert!(
                refused.to_string().contains("matching no original"),
                "{refused}"
            );
        }
    }

    #[test]
    fn a_listed_path_outside_the_pack_is_refused() {
        for path in ["../escape.csv", "/etc/hosts", "", "a/../../b"] {
            assert!(pack_relative(path).is_none(), "{path}");
        }
        assert!(pack_relative("originals/police_calls/2025.csv").is_some());
    }
}
