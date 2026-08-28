// TODO(parsers): Replace the hand-maintained export list with a capability
// registry that reports format support, memory bounds, and extraction routes.
// FEATURE(parsers): Allow signed, sandboxed parser extensions for formats that
// should not increase the trusted core binary surface.
// PERF(parsers): Benchmark parser concurrency and memory permits by format;
// OCR and archive expansion currently have very different cost profiles.

pub mod archive;
pub mod database;
pub mod entropy;
pub mod excel;
pub mod format;
// FEATURE(forensics): These bounded modules define residual-evidence contracts. Keep collectors
// separate from ordinary searchable-text ingestion so their confidence and coverage remain clear.
pub mod compression_oracle;
pub mod forensic_evidence;
pub mod html;
pub mod hypothesis;
pub mod layout_side_channel;
pub mod legislation;
pub mod msg;
pub mod neural_inference;
pub mod ocr;
pub mod office;
pub mod office_audit;
pub mod pdf;
pub mod pdf_auxiliary;
pub mod pixelation;
pub mod raster_audit;
pub mod recovery;
pub mod rtf;
pub mod salvage;
mod source;
pub(crate) mod staging;
pub mod svg;
pub mod text;
pub mod unredactor;
pub mod zip;

pub use archive::extract_archive_container;
pub use database::extract_sqlite_text;
pub use entropy::{assess_file_entropy, calculate_shannon_entropy, EntropyAssessment};
pub use excel::extract_spreadsheet_layers;
pub use forensic_evidence::{
    audit_document_path, EvidenceBundle, EvidenceClass, EvidenceConfidence, EvidenceLocator,
    ForensicAuditOptions, ForensicFinding,
};
pub use format::{detect_document_format, DocumentFormat, ExtractionRoute};
pub use html::extract_html_text;
pub use legislation::{
    parse_legal_authority, phrase_match_count, policy_tokens, token_sequence_match_count,
    ParsedLegalAuthority, ParsedLegalProvision,
};
pub use msg::{extract_msg_layers, ExtractedEmailPayload, MsgExtractionContext};
pub use ocr::{execute_local_ocr_scan, probe_ocr_capability, OcrCapability};
pub use office::extract_office_package;
pub use pdf::{
    character_layer_density, extract_text_layers, requires_ocr_reprocessing,
    MINIMUM_CHARACTER_LAYER_DENSITY,
};
pub use recovery::{
    carve_historical_pdf_versions, RecoveredMetadataArtifact, RecoveredTextSegment,
    RecoveredVersionProfile,
};
pub use rtf::extract_rtf_text;
pub use salvage::extract_salvaged_text;
pub use svg::extract_svg_text;
pub use text::extract_plain_text;
pub use zip::{extract_zip_container, extract_zip_container_with_manifest};
