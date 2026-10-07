//! Data packs: City datasets retrieved and compiled by the pack builder, signed with the
//! publisher key, and imported by the app from their original files (`docs/data-packs.md`).
//!
//! A pack is a directory holding `manifest.json` (a [`SignedPackage`] of [`PackManifest`]) and the
//! files it lists. Nothing in a pack is trusted until [`verify_pack`] has checked the signature,
//! that the key is one this app pins, and the size and SHA-256 of every listed file.

use crate::error::AppError;
use crate::integrity::{sha256_file, verify_package_detailed, SignedPackage};
use ed25519_dalek::VerifyingKey;
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

#[cfg(feature = "pack-builder")]
pub mod builder;
pub(crate) mod catalog;
pub mod conformed;
pub(crate) mod import;

/// The manifest's file name inside a pack directory.
pub(crate) const PACK_MANIFEST_FILE: &str = "manifest.json";

/// Pinned pack publisher keys as `(kid, hex of the 32-byte Ed25519 key)`. Separate from the
/// per-installation export key and the entitlement keys; rotation adds an entry. There is no
/// runtime route to add a key: tests pass their own keys to [`verify_pack`].
pub(crate) const PACK_PUBLIC_KEYS: &[(&str, &str)] = &[(
    "pack-2026-1",
    "b42202850cd9ecda4ab2b96548891d3a2a8275f74dde76bba2153826089ca8ff",
)];

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
    /// The publisher's year for a table split by year.
    pub year: Option<u16>,
    /// The file covers a period not yet over, so later rows may still be published.
    pub partial: bool,
    /// The publisher's freshness markers as served, used to fetch only what changed later.
    pub last_modified: Option<String>,
    pub etag: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum PackFileFormat {
    Original,
    Parquet,
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
}

/// A pack whose signature, publisher key and every file have been checked.
#[derive(Debug)]
pub(crate) struct VerifiedPack {
    pub directory: PathBuf,
    pub manifest: PackManifest,
    pub signer_key_id: String,
}

impl VerifiedPack {
    pub fn path_of(&self, file: &PackFile) -> PathBuf {
        self.directory.join(&file.path)
    }
}

/// The app's pinned publisher keys. A malformed entry is skipped, which can only reduce what
/// verifies.
pub(crate) fn pinned_pack_keys() -> Vec<VerifyingKey> {
    PACK_PUBLIC_KEYS
        .iter()
        .filter_map(|(_, hex)| {
            let bytes: [u8; 32] = decode_hex(hex)?.try_into().ok()?;
            VerifyingKey::from_bytes(&bytes).ok()
        })
        .collect()
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
fn pack_relative(path: &str) -> Option<&Path> {
    let relative = Path::new(path);
    (!path.is_empty()
        && relative
            .components()
            .all(|part| matches!(part, Component::Normal(_))))
    .then_some(relative)
}

/// Checks a pack before anything in it is used: the manifest's signature over its own canonical
/// bytes, that the signing key is one of `pinned`, and that every listed file is a regular file
/// inside the pack with the listed size and SHA-256.
pub(crate) fn verify_pack(
    directory: &Path,
    pinned: &[VerifyingKey],
) -> Result<VerifiedPack, AppError> {
    let manifest_path = directory.join(PACK_MANIFEST_FILE);
    let bytes = std::fs::read(&manifest_path)
        .map_err(|error| refused(format!("its manifest could not be read: {error}")))?;
    let package: SignedPackage<PackManifest> = serde_json::from_slice(&bytes)
        .map_err(|error| refused(format!("its manifest is not a pack manifest: {error}")))?;
    let checks = verify_package_detailed(&package);
    if !checks.all_passed() {
        return Err(refused(checks.detail.unwrap_or_else(|| {
            "its manifest does not match its signature".into()
        })));
    }
    let signer = package.signature.public_key_hex.to_ascii_lowercase();
    if !pinned
        .iter()
        .any(|key| crate::integrity::digest_hex(key.to_bytes()) == signer)
    {
        return Err(refused(
            "it is not signed by a pack publisher this app trusts",
        ));
    }
    for file in &package.manifest.files {
        let relative = pack_relative(&file.path)
            .ok_or_else(|| refused(format!("it lists a path outside the pack: {}", file.path)))?;
        let path = directory.join(relative);
        let metadata = std::fs::symlink_metadata(&path)
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
        let digest = sha256_file(&path)
            .map_err(|error| refused(format!("{} could not be read: {error}", file.path)))?;
        if !digest.eq_ignore_ascii_case(&file.sha256) {
            return Err(refused(format!(
                "{} does not match its manifest's SHA-256",
                file.path
            )));
        }
    }
    Ok(VerifiedPack {
        directory: directory.to_path_buf(),
        signer_key_id: package.signature.signer_key_id.clone(),
        manifest: package.manifest,
    })
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
                year: Some(2025),
                partial: false,
                last_modified: None,
                etag: None,
            }],
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
        let pinned = [key.verifying_key()];
        let pack = verify_pack(directory.path(), &pinned).unwrap();
        assert_eq!(pack.manifest.files.len(), 1);

        let stranger = [SigningKey::from_bytes(&[6_u8; 32]).verifying_key()];
        assert!(verify_pack(directory.path(), &stranger).is_err());

        // One byte of one file changed.
        let (directory, key) = signed_pack();
        flip_last_byte(&directory.path().join("originals/police_calls/2025.csv"));
        let refused = verify_pack(directory.path(), &[key.verifying_key()]).unwrap_err();
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
        assert!(verify_pack(directory.path(), &[key.verifying_key()]).is_err());
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
            .all(|file| pack.path_of(file).is_file()));
    }

    #[test]
    fn a_listed_path_outside_the_pack_is_refused() {
        for path in ["../escape.csv", "/etc/hosts", "", "a/../../b"] {
            assert!(pack_relative(path).is_none(), "{path}");
        }
        assert!(pack_relative("originals/police_calls/2025.csv").is_some());
    }
}
