//! Defensive, read-only ZIP structure and compression-risk audit.

use crate::error::AppError;
use crate::parsers::forensic_evidence::{
    finding_for_text, EvidenceBundle, EvidenceClass, EvidenceConfidence, EvidenceLocator,
    ForensicAuditOptions,
};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;

const MAXIMUM_PACKAGE_BYTES: usize = 128 * 1024 * 1024;
const MAXIMUM_ENTRIES: usize = 10_000;
const EXTREME_COMPRESSION_RATIO: u64 = 200;

#[derive(Debug)]
struct CentralDirectoryEntry {
    raw_name: Vec<u8>,
    display_name: String,
    compression_method: u16,
    compressed_size: u64,
    uncompressed_size: u64,
}

pub fn assess_package_compression_risk(
    package_bytes: &[u8],
    options: ForensicAuditOptions,
) -> Result<EvidenceBundle, AppError> {
    // FEATURE(container-audit): Inspect compression method, duplicate entries, local-header /
    // central-directory inconsistencies, and whether sensitive plaintext is still present.
    // TODO(container-audit): Add a safe redaction-export linter for attacker-controlled content
    // co-compressed with sensitive content.
    // NOTE(security): Do not implement an append/save/size oracle or automated guess loop here;
    // that would turn a defensive audit into an exfiltration primitive.
    if package_bytes.len() > MAXIMUM_PACKAGE_BYTES {
        return Err(AppError::DataParsing(format!(
            "ZIP_AUDIT_SIZE_EXCEEDED: package exceeds {MAXIMUM_PACKAGE_BYTES} bytes"
        )));
    }
    // The library intentionally indexes entries by name and can collapse duplicate names. It is
    // still useful for validating the archive as a whole, but the audit must read the bounded
    // central-directory records itself or a duplicate-name finding is structurally impossible.
    let _archive = zip::ZipArchive::new(Cursor::new(package_bytes))
        .map_err(|error| AppError::DataParsing(format!("ZIP_AUDIT_INVALID: {error}")))?;
    let entries = parse_central_directory(package_bytes)?;
    if entries.len() > MAXIMUM_ENTRIES {
        return Err(AppError::DataParsing(format!(
            "ZIP_AUDIT_ENTRY_LIMIT_EXCEEDED: package contains more than {MAXIMUM_ENTRIES} entries"
        )));
    }

    let mut findings = Vec::new();
    let mut names = BTreeMap::<Vec<u8>, (String, usize)>::new();
    let mut methods = BTreeSet::new();
    for entry in entries {
        let name = entry.display_name.replace('\\', "/");
        methods.insert(entry.compression_method);
        let (_, count) = names
            .entry(entry.raw_name)
            .or_insert_with(|| (name.clone(), 0));
        *count += 1;

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
    for (_, (name, count)) in names.into_iter().filter(|(_, (_, count))| *count > 1) {
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
    let mut warnings = Vec::new();
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
        coverage_complete: true,
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
        let compressed_size = read_u32(bytes, cursor + 20).unwrap_or(u32::MAX) as u64;
        let uncompressed_size = read_u32(bytes, cursor + 24).unwrap_or(u32::MAX) as u64;
        let name_length = read_u16(bytes, cursor + 28).unwrap_or(u16::MAX) as usize;
        let extra_length = read_u16(bytes, cursor + 30).unwrap_or(u16::MAX) as usize;
        let comment_length = read_u16(bytes, cursor + 32).unwrap_or(u16::MAX) as usize;
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
            compression_method,
            compressed_size,
            uncompressed_size,
        });
        cursor = next;
    }
    Ok(entries)
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
        // `ZipArchive::by_index` validates against and reports the local header's name, so the
        // fixture must encode the same duplicate in both directory records. This is byte-level
        // fixture construction only; production never rewrites a package under audit.
        let local_headers = bytes
            .windows(4)
            .enumerate()
            .filter_map(|(index, window)| (window == b"PK\x03\x04").then_some(index))
            .collect::<Vec<_>>();
        assert_eq!(local_headers.len(), 2);
        let second_local_name = local_headers[1] + 30;
        bytes[second_local_name..second_local_name + "two.txt".len()].copy_from_slice(b"one.txt");
        let result = assess_package_compression_risk(&bytes, ForensicAuditOptions::default())
            .expect("audit");
        assert!(result.coverage_complete);
        assert!(result
            .findings
            .iter()
            .any(|finding| finding.kind == "ZIP_DUPLICATE_ENTRY_NAME"));
    }
}
