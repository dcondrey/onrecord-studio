//! Placeholder for deterministic PDF residual-channel collection.

use crate::error::AppError;
use crate::parsers::forensic_evidence::{EvidenceBundle, ForensicAuditOptions};
use lopdf::Document;

/// Collects non-page-canvas PDF evidence that may survive an attempted redaction.
pub fn collect_pdf_auxiliary_evidence(
    _document: &Document,
    _options: ForensicAuditOptions,
) -> Result<EvidenceBundle, AppError> {
    // FEATURE(pdf-audit): Walk outlines, annotation contents/reply chains, AcroForm values,
    // StructTreeRoot ActualText/Alt attributes, page thumbnails, embedded files, XFA, and
    // optional-content relationships. Emit object IDs and page coordinates for every finding.
    // TODO(pdf-audit): Compare each incremental xref/object generation and report orphan objects.
    // NOTE(pdf-audit): A tagged structure tree usually references page content; it is not an
    // independent full text layer. Only present strings should be classified as Preserved.
    Ok(EvidenceBundle {
        coverage_complete: false,
        warnings: vec!["PDF_AUXILIARY_COLLECTOR_NOT_IMPLEMENTED".to_string()],
        ..EvidenceBundle::default()
    })
}

/// Produces a non-destructive preview with selected overlay operators suppressed.
pub fn render_overlay_suppressed_preview(
    _document: &Document,
    _options: ForensicAuditOptions,
) -> Result<Vec<u8>, AppError> {
    // TODO(pdf-audit): Preserve the original object graph, operator indexes, CTMs, clipping,
    // transparency groups, and Form XObject boundaries in the preview manifest.
    // NOTE(pdf-audit): This must return a derived artifact, never overwrite or rewrite evidence.
    Err(AppError::DataParsing(
        "PDF_OVERLAY_PREVIEW_NOT_IMPLEMENTED".to_string(),
    ))
}
