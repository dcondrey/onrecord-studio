//! Placeholder for deterministic PDF layout side-channel measurement.
//!
//! The goal is to report leaked geometry, not to identify a person's name. Any candidate
//! matching must remain in the separate, opt-in hypothesis layer.

use crate::error::AppError;
use crate::parsers::forensic_evidence::{EvidenceBundle, ForensicAuditOptions};
use lopdf::Document;

pub fn assess_glyph_position_leak(
    _document: &Document,
    _options: ForensicAuditOptions,
) -> Result<EvidenceBundle, AppError> {
    // FEATURE(pdf-audit): Compare adjacent glyph/text-run positions, text-state matrices,
    // TJ adjustments, character spacing, and embedded font metrics around redaction bounds.
    // TODO(pdf-audit): Model editor-specific padding, line justification, CTM rounding, and
    // workflow rewrites before reporting a usable interval for the missing span.
    // NOTE(pdf-audit): A leaked width is preserved side-channel evidence, not recovered text.
    Ok(EvidenceBundle {
        coverage_complete: false,
        warnings: vec!["PDF_GLYPH_POSITION_ANALYSIS_NOT_IMPLEMENTED".to_string()],
        ..EvidenceBundle::default()
    })
}
