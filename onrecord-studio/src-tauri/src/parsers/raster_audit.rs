//! Placeholder for conservative raster residual and embedded-preview analysis.

use crate::error::AppError;
use crate::parsers::forensic_evidence::{EvidenceBundle, ForensicAuditOptions};
use std::path::Path;

pub fn audit_raster_file<P: AsRef<Path>>(
    _path: P,
    _options: ForensicAuditOptions,
) -> Result<EvidenceBundle, AppError> {
    // FEATURE(raster-audit): Extract EXIF/IFD thumbnails, PNG text/XMP chunks, embedded previews,
    // alpha-channel statistics, and hashes as separate evidence artifacts.
    // TODO(raster-audit): Add derived gamma/curve previews and boundary-residual measurements.
    // NOTE(raster-audit): JPEG DCT artifacts can indicate editing or uncovered edge remnants;
    // they cannot restore pixels that were completely overwritten.
    Ok(EvidenceBundle {
        coverage_complete: false,
        warnings: vec!["RASTER_AUDIT_NOT_IMPLEMENTED".to_string()],
        ..EvidenceBundle::default()
    })
}
