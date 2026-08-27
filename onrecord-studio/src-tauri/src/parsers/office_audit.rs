//! Placeholder for attributed OOXML/ODF package archaeology.

use crate::error::AppError;
use crate::parsers::forensic_evidence::{EvidenceBundle, ForensicAuditOptions};
use crate::parsers::format::DocumentFormat;
use std::path::Path;

pub fn audit_office_package<P: AsRef<Path>>(
    _path: P,
    _format: DocumentFormat,
    _options: ForensicAuditOptions,
) -> Result<EvidenceBundle, AppError> {
    // FEATURE(office-audit): Preserve part names, relationship IDs, XML paths, and run-level
    // provenance for w:ins/w:del/w:delText, move ranges, comments, rPrChange, and pPrChange.
    // TODO(office-audit): Inspect customXml, glossary, commentsExtended, people.xml, embedded
    // packages, duplicate ZIP entries, trailing bytes, and package central-directory anomalies.
    // NOTE(office-audit): Stripping rPr is a derived forensic view; do not modify document.xml.
    Ok(EvidenceBundle {
        coverage_complete: false,
        warnings: vec!["OFFICE_ATTRIBUTED_AUDIT_NOT_IMPLEMENTED".to_string()],
        ..EvidenceBundle::default()
    })
}
