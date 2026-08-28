//! Defensive, read-only ZIP structure and compression-risk audit.

use crate::error::AppError;
use crate::parsers::forensic_evidence::{
    finding_for_text, EvidenceBundle, EvidenceClass, EvidenceConfidence, EvidenceLocator,
    ForensicAuditOptions,
};
use crate::storage::pii::mask_pii;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read};

const MAXIMUM_PACKAGE_BYTES: usize = 128 * 1024 * 1024;
const MAXIMUM_ENTRIES: usize = 10_000;
const EXTREME_COMPRESSION_RATIO: u64 = 200;
const MAXIMUM_TEXT_SCAN_ENTRY_BYTES: u64 = 4 * 1024 * 1024;
const MAXIMUM_TOTAL_TEXT_SCAN_BYTES: u64 = 16 * 1024 * 1024;
const MAXIMUM_FINDINGS: usize = 20_000;

#[derive(Debug)]
struct CentralDirectoryEntry {
    raw_name: Vec<u8>,
    display_name: String,
    flags: u16,
    compression_method: u16,
    crc32: u32,
    compressed_size: u64,
    uncompressed_size: u64,
    local_header_offset: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportContentClass {
    Public,
    Sensitive,
    AttackerControlled,
}

#[derive(Debug, Clone)]
pub struct ExportCompressionMember {
    pub scope_id: String,
    pub member_name: String,
    pub content_class: ExportContentClass,
    pub compression_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedactionExportLint {
    pub safe: bool,
    pub rejected_scope_ids: Vec<String>,
    pub explanation: String,
}

/// Rejects any compression stream that combines attacker-controlled bytes with sensitive bytes.
/// Independent ZIP entries use independent scopes; a solid archive or compressed response body
/// must use one shared scope so the unsafe combination is caught before export.
pub fn lint_redaction_export_compression(
    members: &[ExportCompressionMember],
) -> Result<RedactionExportLint, AppError> {
    if members.len() > MAXIMUM_ENTRIES {
        return Err(AppError::DataParsing(format!(
            "REDACTION_EXPORT_MEMBER_LIMIT_EXCEEDED:{MAXIMUM_ENTRIES}"
        )));
    }
    let mut scopes = BTreeMap::<&str, (bool, bool)>::new();
    for member in members {
        if member.scope_id.trim().is_empty()
            || member.member_name.trim().is_empty()
            || member.scope_id.chars().count() > 256
            || member.member_name.chars().count() > 1_024
        {
            return Err(AppError::DataParsing(
                "REDACTION_EXPORT_MEMBER_INVALID".to_string(),
            ));
        }
        if !member.compression_enabled {
            continue;
        }
        let state = scopes.entry(&member.scope_id).or_default();
        state.0 |= member.content_class == ExportContentClass::Sensitive;
        state.1 |= member.content_class == ExportContentClass::AttackerControlled;
    }
    let rejected_scope_ids = scopes
        .into_iter()
        .filter_map(|(scope, (sensitive, attacker))| {
            (sensitive && attacker).then(|| scope.to_string())
        })
        .collect::<Vec<_>>();
    Ok(RedactionExportLint {
        safe: rejected_scope_ids.is_empty(),
        explanation: if rejected_scope_ids.is_empty() {
            "No enabled compression scope mixes attacker-controlled and sensitive bytes."
                .to_string()
        } else {
            "At least one enabled compression scope mixes attacker-controlled and sensitive bytes; use independent uncompressed or separately compressed members."
                .to_string()
        },
        rejected_scope_ids,
    })
}

pub fn assess_package_compression_risk(
    package_bytes: &[u8],
    options: ForensicAuditOptions,
) -> Result<EvidenceBundle, AppError> {
    // Compression method, duplicates, local/central inconsistencies, encryption, ratios, unsafe
    // paths, and bounded plaintext PII presence are inspected without modifying the archive.
    // Do not implement an append/save/size oracle or automated guess loop here;
    // that would turn a defensive audit into an exfiltration primitive.
    if package_bytes.len() > MAXIMUM_PACKAGE_BYTES {
        return Err(AppError::DataParsing(format!(
            "ZIP_AUDIT_SIZE_EXCEEDED: package exceeds {MAXIMUM_PACKAGE_BYTES} bytes"
        )));
    }
    // The library intentionally indexes entries by name and can collapse duplicate names. It is
    // still useful for validating the archive as a whole, but the audit must read the bounded
    // central-directory records itself or a duplicate-name finding is structurally impossible.
    let mut archive = zip::ZipArchive::new(Cursor::new(package_bytes))
        .map_err(|error| AppError::DataParsing(format!("ZIP_AUDIT_INVALID: {error}")))?;
    let entries = parse_central_directory(package_bytes)?;
    if entries.len() > MAXIMUM_ENTRIES {
        return Err(AppError::DataParsing(format!(
            "ZIP_AUDIT_ENTRY_LIMIT_EXCEEDED: package contains more than {MAXIMUM_ENTRIES} entries"
        )));
    }

    let mut findings = Vec::new();
    let mut warnings = Vec::new();
    let mut coverage_complete = true;
    let mut names = BTreeMap::<Vec<u8>, (String, usize)>::new();
    let mut methods = BTreeSet::new();
    for entry in &entries {
        if findings.len() >= MAXIMUM_FINDINGS {
            coverage_complete = false;
            warnings.push("ZIP_AUDIT_FINDING_LIMIT_REACHED".to_string());
            break;
        }
        let name = entry.display_name.replace('\\', "/");
        methods.insert(entry.compression_method);
        let (_, count) = names
            .entry(entry.raw_name.clone())
            .or_insert_with(|| (name.clone(), 0));
        *count += 1;

        if entry.flags & 1 != 0 {
            findings.push(finding_for_text(
                EvidenceClass::Preserved,
                EvidenceConfidence::Exact,
                "ZIP_ENCRYPTED_ENTRY_REQUIRES_MANUAL_DECRYPTION",
                EvidenceLocator { container_part: Some(name.clone()), ..EvidenceLocator::default() },
                &name,
                "The ZIP flags mark this entry as encrypted. OnRecord never accepts or stores archive credentials; decrypt a working copy with an investigator-controlled tool, then import that copy while retaining the encrypted original as evidence.",
                options,
            )?);
        }
        for mismatch in compare_local_header(package_bytes, entry)? {
            findings.push(finding_for_text(
                EvidenceClass::Preserved,
                EvidenceConfidence::Exact,
                "ZIP_LOCAL_CENTRAL_HEADER_MISMATCH",
                EvidenceLocator { container_part: Some(name.clone()), byte_offset: Some(entry.local_header_offset as u64), ..EvidenceLocator::default() },
                &format!("{name}:{mismatch}"),
                "The local file header and central-directory record disagree. Different ZIP readers may select or describe the preserved payload differently.",
                options,
            )?);
        }

        if unsafe_entry_path(&name) {
            findings.push(finding_for_text(
                EvidenceClass::Preserved,
                EvidenceConfidence::Exact,
                "ZIP_UNSAFE_ENTRY_PATH",
                EvidenceLocator { container_part: Some(name.clone()), ..EvidenceLocator::default() },
                &name,
                "The central directory contains an entry path that cannot be safely enclosed in an extraction root.",
                options,
            )?);
        }
        let compressed = entry.compressed_size;
        let uncompressed = entry.uncompressed_size;
        if compressed > 0 && uncompressed / compressed >= EXTREME_COMPRESSION_RATIO {
            findings.push(finding_for_text(
                EvidenceClass::Preserved,
                EvidenceConfidence::Exact,
                "ZIP_EXTREME_COMPRESSION_RATIO",
                EvidenceLocator { container_part: Some(name.clone()), ..EvidenceLocator::default() },
                &format!("{name}: {uncompressed} uncompressed bytes / {compressed} compressed bytes"),
                "The declared uncompressed-to-compressed ratio is extreme. This is a resource-risk indicator, not evidence that secret text can be recovered.",
                options,
            )?);
        }
    }
    let mut scanned_bytes = 0_u64;
    'plaintext: for index in 0..archive.len().min(MAXIMUM_ENTRIES) {
        let mut entry = match archive.by_index(index) {
            Ok(entry) => entry,
            Err(error) => {
                coverage_complete = false;
                warnings.push(format!(
                    "ZIP_PLAINTEXT_SCAN_ENTRY_UNAVAILABLE:{index}:{error}"
                ));
                continue;
            }
        };
        if entry.is_dir() {
            continue;
        }
        if entry.size() > MAXIMUM_TEXT_SCAN_ENTRY_BYTES {
            coverage_complete = false;
            warnings.push(format!(
                "ZIP_PLAINTEXT_SCAN_ENTRY_SIZE_LIMIT:{}:{}",
                entry.name(),
                entry.size()
            ));
            continue;
        }
        if scanned_bytes
            .checked_add(entry.size())
            .is_none_or(|total| total > MAXIMUM_TOTAL_TEXT_SCAN_BYTES)
        {
            coverage_complete = false;
            warnings.push("ZIP_PLAINTEXT_SCAN_BUDGET_REACHED".to_string());
            break;
        }
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        entry
            .by_ref()
            .take(MAXIMUM_TEXT_SCAN_ENTRY_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| {
                AppError::DataParsing(format!(
                    "ZIP_PLAINTEXT_SCAN_FAILED:{}:{error}",
                    entry.name()
                ))
            })?;
        scanned_bytes += bytes.len() as u64;
        let Ok(text) = std::str::from_utf8(&bytes) else {
            continue;
        };
        let pii = mask_pii(text)?;
        for pii_finding in pii.findings {
            if findings.len() >= MAXIMUM_FINDINGS {
                coverage_complete = false;
                warnings.push("ZIP_AUDIT_FINDING_LIMIT_REACHED".to_string());
                break 'plaintext;
            }
            findings.push(finding_for_text(
                EvidenceClass::Preserved,
                EvidenceConfidence::Strong,
                "ZIP_SENSITIVE_PLAINTEXT_PRESENT",
                EvidenceLocator {
                    container_part: Some(entry.name().to_string()),
                    byte_offset: Some(pii_finding.byte_start as u64),
                    ..EvidenceLocator::default()
                },
                &format!("kind={:?};evidenceSha256={}", pii_finding.kind, pii_finding.evidence_sha256),
                "A bounded structural PII detector found sensitive-looking plaintext in the decompressed member. The value is not returned; review the member before releasing the package.",
                options,
            )?);
        }
    }
    for (_, (name, count)) in names.into_iter().filter(|(_, (_, count))| *count > 1) {
        if findings.len() >= MAXIMUM_FINDINGS {
            coverage_complete = false;
            if !warnings
                .iter()
                .any(|warning| warning == "ZIP_AUDIT_FINDING_LIMIT_REACHED")
            {
                warnings.push("ZIP_AUDIT_FINDING_LIMIT_REACHED".to_string());
            }
            break;
        }
        findings.push(finding_for_text(
            EvidenceClass::Preserved,
            EvidenceConfidence::Exact,
            "ZIP_DUPLICATE_ENTRY_NAME",
            EvidenceLocator { container_part: Some(name.clone()), ..EvidenceLocator::default() },
            &format!("{name} occurs {count} times"),
            "Duplicate central-directory names can make viewers select different preserved bytes for the same logical part.",
            options,
        )?);
    }
    if methods.len() > 1 {
        warnings.push(format!(
            "ZIP_MIXED_COMPRESSION_METHODS:{}",
            methods
                .into_iter()
                .map(|method| method.to_string())
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    Ok(EvidenceBundle {
        findings,
        warnings,
        coverage_complete,
    })
}

fn parse_central_directory(bytes: &[u8]) -> Result<Vec<CentralDirectoryEntry>, AppError> {
    if bytes.len() < 22 {
        return Err(AppError::DataParsing(
            "ZIP_AUDIT_INVALID: end-of-central-directory record is missing".to_string(),
        ));
    }
    let search_start = bytes.len().saturating_sub(22 + u16::MAX as usize);
    let eocd = (search_start..=bytes.len() - 22)
        .rev()
        .find(|index| {
            bytes.get(*index..*index + 4) == Some(b"PK\x05\x06")
                && read_u16(bytes, *index + 20).is_some_and(|comment_length| {
                    index
                        .checked_add(22 + comment_length as usize)
                        .is_some_and(|end| end <= bytes.len())
                })
        })
        .ok_or_else(|| {
            AppError::DataParsing(
                "ZIP_AUDIT_INVALID: end-of-central-directory record is missing".to_string(),
            )
        })?;
    let disk = read_u16(bytes, eocd + 4).unwrap_or(u16::MAX);
    let central_disk = read_u16(bytes, eocd + 6).unwrap_or(u16::MAX);
    let disk_entries = read_u16(bytes, eocd + 8).unwrap_or(u16::MAX);
    let entry_count = read_u16(bytes, eocd + 10).unwrap_or(u16::MAX);
    let central_size = read_u32(bytes, eocd + 12).unwrap_or(u32::MAX) as usize;
    let relative_central_start = read_u32(bytes, eocd + 16).unwrap_or(u32::MAX) as usize;
    if disk != 0 || central_disk != 0 || disk_entries != entry_count {
        return Err(AppError::DataParsing(
            "ZIP_AUDIT_UNSUPPORTED: multi-disk ZIP archives are not audited".to_string(),
        ));
    }
    if entry_count == u16::MAX
        || central_size == u32::MAX as usize
        || relative_central_start == u32::MAX as usize
    {
        return Err(AppError::DataParsing(
            "ZIP_AUDIT_UNSUPPORTED: ZIP64 central directories are not audited".to_string(),
        ));
    }
    if entry_count as usize > MAXIMUM_ENTRIES {
        return Err(AppError::DataParsing(format!(
            "ZIP_AUDIT_ENTRY_LIMIT_EXCEEDED: package contains more than {MAXIMUM_ENTRIES} entries"
        )));
    }
    let archive_prefix = eocd
        .checked_sub(central_size)
        .and_then(|central_without_prefix| {
            central_without_prefix.checked_sub(relative_central_start)
        })
        .ok_or_else(|| {
            AppError::DataParsing(
                "ZIP_AUDIT_INVALID: central-directory bounds are inconsistent".to_string(),
            )
        })?;
    let central_start = archive_prefix
        .checked_add(relative_central_start)
        .ok_or_else(|| AppError::DataParsing("ZIP_AUDIT_INVALID: offset overflow".to_string()))?;
    let central_end = central_start
        .checked_add(central_size)
        .filter(|end| *end <= eocd)
        .ok_or_else(|| {
            AppError::DataParsing(
                "ZIP_AUDIT_INVALID: central-directory extends beyond its end record".to_string(),
            )
        })?;

    let mut cursor = central_start;
    let mut entries = Vec::with_capacity(entry_count as usize);
    for index in 0..entry_count as usize {
        if bytes.get(cursor..cursor + 4) != Some(b"PK\x01\x02") {
            return Err(AppError::DataParsing(format!(
                "ZIP_AUDIT_INVALID: central-directory entry {index} has no header"
            )));
        }
        let flags = read_u16(bytes, cursor + 8).ok_or_else(|| {
            AppError::DataParsing(format!(
                "ZIP_AUDIT_INVALID: central-directory entry {index} is truncated"
            ))
        })?;
        let compression_method = read_u16(bytes, cursor + 10).unwrap_or(u16::MAX);
        let crc32 = read_u32(bytes, cursor + 16).unwrap_or(u32::MAX);
        let compressed_size = read_u32(bytes, cursor + 20).unwrap_or(u32::MAX) as u64;
        let uncompressed_size = read_u32(bytes, cursor + 24).unwrap_or(u32::MAX) as u64;
        let name_length = read_u16(bytes, cursor + 28).unwrap_or(u16::MAX) as usize;
        let extra_length = read_u16(bytes, cursor + 30).unwrap_or(u16::MAX) as usize;
        let comment_length = read_u16(bytes, cursor + 32).unwrap_or(u16::MAX) as usize;
        let relative_local_header = read_u32(bytes, cursor + 42).unwrap_or(u32::MAX) as usize;
        let name_start = cursor + 46;
        let name_end = name_start.checked_add(name_length).ok_or_else(|| {
            AppError::DataParsing("ZIP_AUDIT_INVALID: filename offset overflow".to_string())
        })?;
        let next = name_end
            .checked_add(extra_length)
            .and_then(|end| end.checked_add(comment_length))
            .filter(|end| *end <= central_end)
            .ok_or_else(|| {
                AppError::DataParsing(format!(
                    "ZIP_AUDIT_INVALID: central-directory entry {index} exceeds its bounds"
                ))
            })?;
        let raw_name = bytes[name_start..name_end].to_vec();
        let display_name = if flags & (1 << 11) != 0 {
            std::str::from_utf8(&raw_name)
                .map_err(|_| {
                    AppError::DataParsing(format!(
                        "ZIP_AUDIT_INVALID: entry {index} declares a non-UTF-8 filename as UTF-8"
                    ))
                })?
                .to_string()
        } else {
            String::from_utf8_lossy(&raw_name).into_owned()
        };
        entries.push(CentralDirectoryEntry {
            raw_name,
            display_name,
            flags,
            compression_method,
            crc32,
            compressed_size,
            uncompressed_size,
            local_header_offset: archive_prefix
                .checked_add(relative_local_header)
                .ok_or_else(|| {
                    AppError::DataParsing(
                        "ZIP_AUDIT_INVALID: local-header offset overflow".to_string(),
                    )
                })?,
        });
        cursor = next;
    }
    Ok(entries)
}

fn compare_local_header(
    bytes: &[u8],
    entry: &CentralDirectoryEntry,
) -> Result<Vec<String>, AppError> {
    let offset = entry.local_header_offset;
    if bytes.get(offset..offset + 4) != Some(b"PK\x03\x04") {
        return Ok(vec!["localSignatureMissing".to_string()]);
    }
    let flags = read_u16(bytes, offset + 6).ok_or_else(|| {
        AppError::DataParsing("ZIP_AUDIT_INVALID: local header is truncated".to_string())
    })?;
    let method = read_u16(bytes, offset + 8).unwrap_or(u16::MAX);
    let crc32 = read_u32(bytes, offset + 14).unwrap_or(u32::MAX);
    let compressed_size = read_u32(bytes, offset + 18).unwrap_or(u32::MAX) as u64;
    let uncompressed_size = read_u32(bytes, offset + 22).unwrap_or(u32::MAX) as u64;
    let name_length = read_u16(bytes, offset + 26).unwrap_or(u16::MAX) as usize;
    let extra_length = read_u16(bytes, offset + 28).unwrap_or(u16::MAX) as usize;
    let name_start = offset + 30;
    let name_end = name_start
        .checked_add(name_length)
        .filter(|end| *end <= bytes.len())
        .ok_or_else(|| {
            AppError::DataParsing("ZIP_AUDIT_INVALID: local filename is truncated".to_string())
        })?;
    name_end
        .checked_add(extra_length)
        .filter(|end| *end <= bytes.len())
        .ok_or_else(|| {
            AppError::DataParsing("ZIP_AUDIT_INVALID: local extra field is truncated".to_string())
        })?;
    let mut mismatches = Vec::new();
    if bytes[name_start..name_end] != entry.raw_name {
        mismatches.push("filename".to_string());
    }
    if flags != entry.flags {
        mismatches.push(format!(
            "flagsLocal={flags:#06x},central={:#06x}",
            entry.flags
        ));
    }
    if method != entry.compression_method {
        mismatches.push(format!(
            "methodLocal={method},central={}",
            entry.compression_method
        ));
    }
    if flags & (1 << 3) == 0 {
        if crc32 != entry.crc32 {
            mismatches.push(format!(
                "crcLocal={crc32:#010x},central={:#010x}",
                entry.crc32
            ));
        }
        if compressed_size != entry.compressed_size {
            mismatches.push(format!(
                "compressedSizeLocal={compressed_size},central={}",
                entry.compressed_size
            ));
        }
        if uncompressed_size != entry.uncompressed_size {
            mismatches.push(format!(
                "uncompressedSizeLocal={uncompressed_size},central={}",
                entry.uncompressed_size
            ));
        }
    }
    Ok(mismatches)
}

fn unsafe_entry_path(name: &str) -> bool {
    let normalized = name.replace('\\', "/");
    normalized.starts_with('/')
        || normalized.contains('\0')
        || normalized
            .split('/')
            .any(|component| component == ".." || component.contains(':'))
}

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        bytes.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn reports_duplicate_entry_names_without_extracting() {
        let mut bytes = Vec::new();
        {
            let mut writer = zip::ZipWriter::new(Cursor::new(&mut bytes));
            let options = zip::write::SimpleFileOptions::default();
            writer.start_file("one.txt", options).expect("first entry");
            writer.write_all(b"first").expect("first payload");
            writer.start_file("two.txt", options).expect("second entry");
            writer.write_all(b"second").expect("second payload");
            writer.finish().expect("finish zip");
        }
        let central_directories = bytes
            .windows(4)
            .enumerate()
            .filter_map(|(index, window)| (window == b"PK\x01\x02").then_some(index))
            .collect::<Vec<_>>();
        assert_eq!(central_directories.len(), 2);
        let second_name = central_directories[1] + 46;
        bytes[second_name..second_name + "two.txt".len()].copy_from_slice(b"one.txt");
        let result = assess_package_compression_risk(&bytes, ForensicAuditOptions::default())
            .expect("audit");
        assert!(result.coverage_complete);
        assert!(result
            .findings
            .iter()
            .any(|finding| finding.kind == "ZIP_DUPLICATE_ENTRY_NAME"));
        assert!(result
            .findings
            .iter()
            .any(|finding| finding.kind == "ZIP_LOCAL_CENTRAL_HEADER_MISMATCH"));
    }

    #[test]
    fn detects_sensitive_plaintext_and_rejects_shared_compression_scopes() {
        let mut bytes = Vec::new();
        {
            let mut writer = zip::ZipWriter::new(Cursor::new(&mut bytes));
            writer
                .start_file("release.txt", zip::write::SimpleFileOptions::default())
                .expect("entry");
            writer
                .write_all(b"Resident identifier 123-45-6789 must remain private.")
                .expect("payload");
            writer.finish().expect("finish zip");
        }
        let result = assess_package_compression_risk(&bytes, ForensicAuditOptions::default())
            .expect("audit");
        assert!(result
            .findings
            .iter()
            .any(|finding| finding.kind == "ZIP_SENSITIVE_PLAINTEXT_PRESENT"));

        let lint = lint_redaction_export_compression(&[
            ExportCompressionMember {
                scope_id: "response-body".into(),
                member_name: "redacted-record".into(),
                content_class: ExportContentClass::Sensitive,
                compression_enabled: true,
            },
            ExportCompressionMember {
                scope_id: "response-body".into(),
                member_name: "request-label".into(),
                content_class: ExportContentClass::AttackerControlled,
                compression_enabled: true,
            },
        ])
        .expect("lint");
        assert!(!lint.safe);
        assert_eq!(lint.rejected_scope_ids, vec!["response-body"]);
    }
}
