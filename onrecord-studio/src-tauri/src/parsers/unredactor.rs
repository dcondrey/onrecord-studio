use crate::analysis::analyze_redaction_density;
use crate::error::AppError;
use crate::integrity::sha256_hex;
use crate::storage::pii::mask_pii;
use flate2::read::ZlibDecoder;
use lopdf::content::Content;
use lopdf::{Dictionary, Document, Encoding, Object, ObjectId, Stream, StringFormat};
use std::collections::{BTreeMap, HashSet};
use std::io::Read;
use std::path::Path;

use crate::parsers::source::read_regular_file;

const MAXIMUM_PDF_BYTES: u64 = 512 * 1024 * 1024;
const MAXIMUM_PAGES: usize = 10_000;
const MAXIMUM_PAGE_CONTENT_BYTES: usize = 64 * 1024 * 1024;
const MAXIMUM_TOTAL_CONTENT_BYTES: usize = 512 * 1024 * 1024;
const MAXIMUM_PAGE_OPERATIONS: usize = 250_000;
const MAXIMUM_TOTAL_OPERATIONS: usize = 2_000_000;
const MAXIMUM_GRAPHICS_STATE_DEPTH: usize = 256;
const MAXIMUM_FORM_DEPTH: usize = 32;
const MAXIMUM_FORM_INVOCATIONS_PER_PAGE: usize = 2_048;
const MAXIMUM_TEXT_RUNS_PER_PAGE: usize = 50_000;
const MAXIMUM_FILLED_RECTANGLES_PER_PAGE: usize = 4_096;
const MAXIMUM_INTERSECTION_CHECKS: usize = 5_000_000;
const MAXIMUM_REPORTED_INTERSECTIONS_PER_PAGE: usize = 512;
const MAXIMUM_PAGE_TEXT_BYTES: usize = 4 * 1024 * 1024;
const MAXIMUM_QUARANTINED_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
const MAXIMUM_METADATA_VALUE_BYTES: usize = 8 * 1024;
const MAXIMUM_METADATA_ENTRIES: usize = 256;
const MAXIMUM_XMP_BYTES: usize = 1024 * 1024;
const MAXIMUM_STRUCTURAL_SIGNALS: usize = 20_000;
const MAXIMUM_IMAGE_PIXELS: usize = 25_000_000;
const ADAPTIVE_WINDOW_RADIUS: usize = 8;
const MAXIMUM_COORDINATE: f64 = 1_000_000_000.0;
const MINIMUM_RECTANGLE_DIMENSION: f64 = 2.0;
const MINIMUM_TEXT_OVERLAP_RATIO: f64 = 0.5;
const MINIMUM_REDACTION_FILL_ALPHA: f64 = 0.8;
const REDACTION_ANALYSIS_VERSION: &str = "pdf-redaction-geometry-v2";

#[derive(Clone, Copy, Debug)]
struct Matrix {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    e: f64,
    f: f64,
}

impl Matrix {
    const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    fn multiply(self, right: Self) -> Self {
        Self {
            a: self.a * right.a + self.c * right.b,
            b: self.b * right.a + self.d * right.b,
            c: self.a * right.c + self.c * right.d,
            d: self.b * right.c + self.d * right.d,
            e: self.a * right.e + self.c * right.f + self.e,
            f: self.b * right.e + self.d * right.f + self.f,
        }
    }

    fn translation(x: f64, y: f64) -> Self {
        Self {
            e: x,
            f: y,
            ..Self::IDENTITY
        }
    }

    fn transform(self, x: f64, y: f64) -> (f64, f64) {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }
}

#[derive(Clone, Copy, Debug)]
struct Bounds {
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
}

impl Bounds {
    fn from_points(points: &[(f64, f64)]) -> Option<Self> {
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for &(point_x, point_y) in points {
            if !point_x.is_finite()
                || !point_y.is_finite()
                || point_x.abs() > MAXIMUM_COORDINATE
                || point_y.abs() > MAXIMUM_COORDINATE
            {
                return None;
            }
            min_x = min_x.min(point_x);
            min_y = min_y.min(point_y);
            max_x = max_x.max(point_x);
            max_y = max_y.max(point_y);
        }
        let bounds = Self {
            min_x,
            min_y,
            max_x,
            max_y,
        };
        (bounds.width() > 0.0 && bounds.height() > 0.0).then_some(bounds)
    }

    fn transformed_rectangle(
        matrix: Matrix,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    ) -> Option<Self> {
        let points = [
            matrix.transform(x, y),
            matrix.transform(x + width, y),
            matrix.transform(x, y + height),
            matrix.transform(x + width, y + height),
        ];
        Self::from_points(&points)
    }

    fn from_pdf_array(array: &[Object]) -> Option<Self> {
        let [left, bottom, right, top] = array else {
            return None;
        };
        let left = object_number(left)?;
        let bottom = object_number(bottom)?;
        let right = object_number(right)?;
        let top = object_number(top)?;
        Self::transformed_rectangle(
            Matrix::IDENTITY,
            left.min(right),
            bottom.min(top),
            (right - left).abs(),
            (top - bottom).abs(),
        )
    }

    fn width(self) -> f64 {
        self.max_x - self.min_x
    }

    fn height(self) -> f64 {
        self.max_y - self.min_y
    }

    fn area(self) -> f64 {
        self.width() * self.height()
    }

    fn center(self) -> (f64, f64) {
        (
            (self.min_x + self.max_x) * 0.5,
            (self.min_y + self.max_y) * 0.5,
        )
    }

    fn intersects(self, other: Self) -> bool {
        self.max_x > other.min_x
            && self.min_x < other.max_x
            && self.max_y > other.min_y
            && self.min_y < other.max_y
    }

    fn coordinates(self) -> [f64; 4] {
        [self.min_x, self.min_y, self.max_x, self.max_y]
    }

    fn covers_text(self, text: Self) -> bool {
        let (center_x, center_y) = text.center();
        if center_x < self.min_x
            || center_x > self.max_x
            || center_y < self.min_y
            || center_y > self.max_y
        {
            return false;
        }
        let intersection_width = self.max_x.min(text.max_x) - self.min_x.max(text.min_x);
        let intersection_height = self.max_y.min(text.max_y) - self.min_y.max(text.min_y);
        intersection_width > 0.0
            && intersection_height > 0.0
            && intersection_width * intersection_height / text.area() >= MINIMUM_TEXT_OVERLAP_RATIO
    }
}

#[derive(Clone, Copy, Debug)]
enum FillColorSpace {
    DeviceGray,
    DeviceRgb,
    DeviceCmyk,
    Unsupported,
}

#[derive(Clone, Copy, Debug)]
struct GraphicsState {
    ctm: Matrix,
    fill_color_space: FillColorSpace,
    fill_luminance: Option<f64>,
    fill_alpha: Option<f64>,
    standard_blend_mode: bool,
}

impl Default for GraphicsState {
    fn default() -> Self {
        Self {
            ctm: Matrix::IDENTITY,
            fill_color_space: FillColorSpace::DeviceGray,
            fill_luminance: Some(0.0),
            fill_alpha: Some(1.0),
            standard_blend_mode: true,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct TextState {
    matrix: Matrix,
    line_matrix: Matrix,
    font_size: f64,
    glyph_width_factor: f64,
    horizontal_scale: f64,
    character_spacing: f64,
    word_spacing: f64,
    leading: f64,
    rise: f64,
    rendering_mode: i64,
}

impl Default for TextState {
    fn default() -> Self {
        Self {
            matrix: Matrix::IDENTITY,
            line_matrix: Matrix::IDENTITY,
            font_size: 0.0,
            glyph_width_factor: 0.5,
            horizontal_scale: 1.0,
            character_spacing: 0.0,
            word_spacing: 0.0,
            leading: 0.0,
            rise: 0.0,
            rendering_mode: 0,
        }
    }
}

#[derive(Clone, Debug)]
struct TextRun {
    bounds: Bounds,
    operation_index: usize,
    rendering_mode: i64,
    extracted_text: String,
}

#[derive(Clone, Copy, Debug)]
struct FilledRectangle {
    bounds: Bounds,
    operation_index: usize,
    dark_fill: bool,
}

#[derive(Default)]
struct PageGeometry {
    text_runs: Vec<TextRun>,
    filled_rectangles: Vec<FilledRectangle>,
    malformed_operations: usize,
    unclassified_filled_rectangles: usize,
    processed_operations: usize,
    decoded_form_bytes: usize,
    form_invocations: usize,
    unresolved_form_xobjects: usize,
    cyclic_form_xobjects: usize,
    forms_without_bounding_boxes: usize,
    positioned_text_bytes: usize,
}

#[derive(Default)]
struct GeometryBudget {
    operation_sequence: usize,
    decoded_form_bytes: usize,
    form_invocations: usize,
}

struct GeometryTraversal<'a> {
    active_forms: &'a mut HashSet<ObjectId>,
    budget: &'a mut GeometryBudget,
    geometry: &'a mut PageGeometry,
}

#[derive(Default)]
struct RedactionAnnotationGeometry {
    regions: Vec<Bounds>,
    annotation_count: usize,
}

#[derive(Default)]
struct ClashSummary {
    overlay_text_runs: HashSet<usize>,
    annotation_text_runs: HashSet<usize>,
    invisible_overlay_text_runs: HashSet<usize>,
    reported_intersections: Vec<RedactionIntersectionEvidence>,
    evidence_truncated: bool,
}

#[derive(Debug, serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RedactionPageFinding {
    pub page_number: u32,
    pub potential_intersections: usize,
    pub dark_overlay_intersections: usize,
    pub redaction_annotation_intersections: usize,
    pub invisible_text_intersections: usize,
    pub unapplied_redaction_annotations: usize,
    pub form_invocations_analyzed: usize,
    pub coverage_complete: bool,
    pub intersections: Vec<RedactionIntersectionEvidence>,
}

#[derive(Debug, serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RedactionIntersectionEvidence {
    pub kind: &'static str,
    pub text_bounds: [f64; 4],
    pub mask_bounds: [f64; 4],
}

#[derive(Debug, serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct UnredactorForensicReport {
    pub analysis_version: &'static str,
    pub source_sha256: String,
    pub scanned_page_count: usize,
    pub coverage_complete: bool,
    pub requires_manual_review: bool,
    pub is_leaky_redaction: bool,
    pub extracted_hidden_text: String,
    pub bounding_box_clashes_count: usize,
    pub metadata_leaks: Vec<String>,
    pub isolated_structural_anomalies: Vec<String>,
    pub page_findings: Vec<RedactionPageFinding>,
    pub calculated_redaction_ratio: f64,
}

// This geometry-specific report remains deterministic and PII-masked. The companion
// audit_document_residual_evidence command runs pdf_auxiliary and layout_side_channel and returns
// their object-, page-, bounds-, and operator-attributed findings through the shared evidence
// contract instead of exposing a second unmasked text channel.
pub fn evaluate_vector_redaction_leaks<P: AsRef<Path>>(
    file_path: P,
) -> Result<UnredactorForensicReport, AppError> {
    let path = file_path.as_ref();
    let source_bytes = read_regular_file(path, "PDF_AUDIT", MAXIMUM_PDF_BYTES)?;
    if !source_bytes.starts_with(b"%PDF-") {
        return Err(AppError::DataParsing(format!(
            "Redaction geometry audit requires a PDF signature: '{}'",
            path.display()
        )));
    }
    let source_sha256 = sha256_hex(&source_bytes);
    let document = Document::load_mem(&source_bytes).map_err(|error| {
        AppError::DataParsing(format!(
            "Failed to parse PDF structure '{}': {error}",
            path.display()
        ))
    })?;
    drop(source_bytes);
    let pages = document.get_pages();
    let page_count = pages.len();
    if page_count > MAXIMUM_PAGES {
        return Err(AppError::DataParsing(format!(
            "PDF redaction audit exceeds the {MAXIMUM_PAGES}-page limit"
        )));
    }
    let mut metadata_leaks = collect_metadata(&document)?;
    metadata_leaks.extend(collect_xmp_metadata(&document));
    for value in &mut metadata_leaks {
        *value = mask_pii(value)?.masked_text;
    }
    let mut extracted_hidden_text = String::new();
    let mut bounding_box_clashes_count = 0_usize;
    let mut isolated_structural_anomalies = collect_structural_signals(&document)?;
    let mut redaction_character_count = 0_usize;
    let mut total_character_count = 0_usize;
    let mut total_content_bytes = 0_usize;
    let mut total_operations = 0_usize;
    let mut quarantined_output_full = false;
    let mut page_findings = Vec::new();

    for (page_number, page_id) in pages {
        validate_page_content_streams(&document, page_id, page_number)?;
        let content_bytes = match bounded_page_content(&document, page_id) {
            Ok(content) => content,
            Err(_) => {
                record_unresolved_page(
                    &document,
                    page_id,
                    page_number,
                    "CONTENT_STREAMS_UNRESOLVED",
                    &mut isolated_structural_anomalies,
                    &mut page_findings,
                )?;
                continue;
            }
        };
        if content_bytes.len() > MAXIMUM_PAGE_CONTENT_BYTES {
            return Err(AppError::DataParsing(format!(
                "PDF page {page_number} exceeds the {MAXIMUM_PAGE_CONTENT_BYTES}-byte decoded-content limit"
            )));
        }
        total_content_bytes = total_content_bytes
            .checked_add(content_bytes.len())
            .filter(|value| *value <= MAXIMUM_TOTAL_CONTENT_BYTES)
            .ok_or_else(|| {
                AppError::DataParsing(format!(
                    "PDF decoded content exceeds the {MAXIMUM_TOTAL_CONTENT_BYTES}-byte audit limit"
                ))
            })?;
        let operators = match Content::decode(&content_bytes) {
            Ok(operators) => operators,
            Err(_) => {
                record_unresolved_page(
                    &document,
                    page_id,
                    page_number,
                    "CONTENT_OPERATORS_UNRESOLVED",
                    &mut isolated_structural_anomalies,
                    &mut page_findings,
                )?;
                continue;
            }
        };
        if operators.operations.len() > MAXIMUM_PAGE_OPERATIONS {
            return Err(AppError::DataParsing(format!(
                "PDF page {page_number} exceeds the {MAXIMUM_PAGE_OPERATIONS}-operator audit limit"
            )));
        }
        let geometry = analyze_page_geometry(&document, page_id, &operators.operations)?;
        total_operations = total_operations
            .checked_add(geometry.processed_operations)
            .filter(|value| *value <= MAXIMUM_TOTAL_OPERATIONS)
            .ok_or_else(|| {
                AppError::DataParsing(format!(
                    "PDF content exceeds the {MAXIMUM_TOTAL_OPERATIONS}-operator audit limit"
                ))
            })?;
        total_content_bytes = total_content_bytes
            .checked_add(geometry.decoded_form_bytes)
            .filter(|value| *value <= MAXIMUM_TOTAL_CONTENT_BYTES)
            .ok_or_else(|| {
                AppError::DataParsing(format!(
                    "PDF decoded content exceeds the {MAXIMUM_TOTAL_CONTENT_BYTES}-byte audit limit"
                ))
            })?;
        let redaction_annotations = collect_redaction_annotations(&document, page_id)?;
        let clashes = correlate_redaction_geometry(
            &geometry.text_runs,
            &geometry.filled_rectangles,
            &redaction_annotations.regions,
        )?;
        let (page_text, page_text_complete) =
            match extract_page_text(&document, page_id, &operators.operations) {
                Ok(text) => (text, true),
                Err(_) => {
                    append_signal(
                        &mut isolated_structural_anomalies,
                        format!("PAGE_{page_number}_TEXT_EXTRACTION_UNRESOLVED"),
                    )?;
                    (String::new(), false)
                }
            };
        let redaction_metric = analyze_redaction_density(&page_text)?;
        redaction_character_count = redaction_character_count
            .checked_add(redaction_metric.match_count)
            .ok_or_else(|| AppError::Analysis("Redaction coverage overflowed".to_string()))?;
        total_character_count = total_character_count
            .checked_add(redaction_metric.total_string_length)
            .ok_or_else(|| AppError::Analysis("PDF text length overflowed".to_string()))?;

        if geometry.malformed_operations > 0 {
            append_signal(
                &mut isolated_structural_anomalies,
                format!(
                    "PAGE_{page_number}_MALFORMED_GEOMETRY_OPERATIONS_{}",
                    geometry.malformed_operations
                ),
            )?;
        }
        if geometry.unclassified_filled_rectangles > 0 {
            append_signal(
                &mut isolated_structural_anomalies,
                format!(
                    "PAGE_{page_number}_UNCLASSIFIED_FILLED_RECTANGLES_{}",
                    geometry.unclassified_filled_rectangles
                ),
            )?;
        }
        if geometry.form_invocations > 0 {
            append_signal(
                &mut isolated_structural_anomalies,
                format!(
                    "PAGE_{page_number}_FORM_XOBJECT_INVOCATIONS_ANALYZED_{}",
                    geometry.form_invocations
                ),
            )?;
        }
        if geometry.unresolved_form_xobjects > 0 {
            append_signal(
                &mut isolated_structural_anomalies,
                format!(
                    "PAGE_{page_number}_UNRESOLVED_FORM_XOBJECTS_{}",
                    geometry.unresolved_form_xobjects
                ),
            )?;
        }
        if geometry.cyclic_form_xobjects > 0 {
            append_signal(
                &mut isolated_structural_anomalies,
                format!(
                    "PAGE_{page_number}_CYCLIC_FORM_XOBJECTS_{}",
                    geometry.cyclic_form_xobjects
                ),
            )?;
        }
        if geometry.forms_without_bounding_boxes > 0 {
            append_signal(
                &mut isolated_structural_anomalies,
                format!(
                    "PAGE_{page_number}_FORM_XOBJECTS_WITHOUT_BOUNDING_BOXES_{}",
                    geometry.forms_without_bounding_boxes
                ),
            )?;
        }
        if redaction_annotations.annotation_count > 0 {
            append_signal(
                &mut isolated_structural_anomalies,
                format!(
                    "PAGE_{page_number}_UNAPPLIED_REDACTION_ANNOTATIONS_{}",
                    redaction_annotations.annotation_count
                ),
            )?;
        }
        if !clashes.overlay_text_runs.is_empty() {
            append_signal(
                &mut isolated_structural_anomalies,
                format!(
                    "PAGE_{page_number}_DARK_OVERLAY_TEXT_INTERSECTIONS_{}",
                    clashes.overlay_text_runs.len()
                ),
            )?;
        }
        if !clashes.annotation_text_runs.is_empty() {
            append_signal(
                &mut isolated_structural_anomalies,
                format!(
                    "PAGE_{page_number}_REDACTION_ANNOTATION_TEXT_INTERSECTIONS_{}",
                    clashes.annotation_text_runs.len()
                ),
            )?;
        }
        if !clashes.invisible_overlay_text_runs.is_empty() {
            append_signal(
                &mut isolated_structural_anomalies,
                format!(
                    "PAGE_{page_number}_INVISIBLE_TEXT_BEHIND_DARK_OVERLAY_{}",
                    clashes.invisible_overlay_text_runs.len()
                ),
            )?;
        }
        if clashes.evidence_truncated {
            append_signal(
                &mut isolated_structural_anomalies,
                format!("PAGE_{page_number}_INTERSECTION_EVIDENCE_TRUNCATED"),
            )?;
        }

        let dark_overlay_intersections = clashes.overlay_text_runs.len();
        let redaction_annotation_intersections = clashes.annotation_text_runs.len();
        let invisible_text_intersections = clashes.invisible_overlay_text_runs.len();
        let reported_intersections = clashes.reported_intersections;
        let mut unique_clashes = clashes.overlay_text_runs;
        unique_clashes.extend(clashes.annotation_text_runs);
        let mut unique_clash_indices = unique_clashes.iter().copied().collect::<Vec<_>>();
        unique_clash_indices.sort_unstable();
        let page_coverage_complete = geometry.malformed_operations == 0
            && geometry.unclassified_filled_rectangles == 0
            && geometry.unresolved_form_xobjects == 0
            && geometry.cyclic_form_xobjects == 0
            && geometry.forms_without_bounding_boxes == 0
            && page_text_complete;
        if !unique_clashes.is_empty()
            || redaction_annotations.annotation_count > 0
            || !page_coverage_complete
        {
            page_findings.push(RedactionPageFinding {
                page_number,
                potential_intersections: unique_clashes.len(),
                dark_overlay_intersections,
                redaction_annotation_intersections,
                invisible_text_intersections,
                unapplied_redaction_annotations: redaction_annotations.annotation_count,
                form_invocations_analyzed: geometry.form_invocations,
                coverage_complete: page_coverage_complete,
                intersections: reported_intersections,
            });
        }
        bounding_box_clashes_count = bounding_box_clashes_count
            .checked_add(unique_clashes.len())
            .ok_or_else(|| AppError::Analysis("Redaction clash count overflowed".to_string()))?;
        if !unique_clashes.is_empty() && !quarantined_output_full {
            let hidden_text = unique_clash_indices
                .iter()
                .filter_map(|index| geometry.text_runs.get(*index))
                .map(|run| run.extracted_text.trim())
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            if !hidden_text.is_empty() {
                let masked = mask_pii(&hidden_text)?.masked_text;
                quarantined_output_full = !append_quarantined_page(
                    &mut extracted_hidden_text,
                    page_number,
                    masked.trim(),
                );
            }
        }
    }

    if quarantined_output_full {
        append_signal(
            &mut isolated_structural_anomalies,
            "QUARANTINED_OUTPUT_TRUNCATED_AT_SAFETY_LIMIT".to_string(),
        )?;
    }

    let calculated_redaction_ratio = if total_character_count == 0 {
        0.0
    } else {
        redaction_character_count as f64 / total_character_count as f64
    };
    let coverage_complete = !isolated_structural_anomalies.iter().any(|signal| {
        signal.contains("MALFORMED_GEOMETRY_OPERATIONS")
            || signal.contains("UNCLASSIFIED_FILLED_RECTANGLES")
            || signal.contains("UNRESOLVED_FORM_XOBJECTS")
            || signal.contains("CYCLIC_FORM_XOBJECTS")
            || signal.contains("FORM_XOBJECTS_WITHOUT_BOUNDING_BOXES")
            || signal.contains("CONTENT_STREAMS_UNRESOLVED")
            || signal.contains("CONTENT_OPERATORS_UNRESOLVED")
            || signal.contains("TEXT_EXTRACTION_UNRESOLVED")
            || signal == "XFA_FORM_CONTENT_PRESENT"
            || signal == "OPTIONAL_CONTENT_LAYERS_PRESENT"
            || signal == "QUARANTINED_OUTPUT_TRUNCATED_AT_SAFETY_LIMIT"
    });
    let requires_manual_review = bounding_box_clashes_count > 0
        || !coverage_complete
        || page_findings
            .iter()
            .any(|finding| finding.unapplied_redaction_annotations > 0);
    Ok(UnredactorForensicReport {
        analysis_version: REDACTION_ANALYSIS_VERSION,
        source_sha256,
        scanned_page_count: page_count,
        coverage_complete,
        requires_manual_review,
        is_leaky_redaction: bounding_box_clashes_count > 0,
        extracted_hidden_text: extracted_hidden_text.trim().to_string(),
        bounding_box_clashes_count,
        metadata_leaks,
        isolated_structural_anomalies,
        page_findings,
        calculated_redaction_ratio,
    })
}

fn record_unresolved_page(
    document: &Document,
    page_id: ObjectId,
    page_number: u32,
    unresolved_signal: &str,
    structural_anomalies: &mut Vec<String>,
    page_findings: &mut Vec<RedactionPageFinding>,
) -> Result<(), AppError> {
    let annotations = collect_redaction_annotations(document, page_id)?;
    append_signal(
        structural_anomalies,
        format!("PAGE_{page_number}_{unresolved_signal}"),
    )?;
    if annotations.annotation_count > 0 {
        append_signal(
            structural_anomalies,
            format!(
                "PAGE_{page_number}_UNAPPLIED_REDACTION_ANNOTATIONS_{}",
                annotations.annotation_count
            ),
        )?;
    }
    page_findings.push(RedactionPageFinding {
        page_number,
        potential_intersections: 0,
        dark_overlay_intersections: 0,
        redaction_annotation_intersections: 0,
        invisible_text_intersections: 0,
        unapplied_redaction_annotations: annotations.annotation_count,
        form_invocations_analyzed: 0,
        coverage_complete: false,
        intersections: Vec::new(),
    });
    Ok(())
}

pub fn execute_adaptive_threshold_unredaction(
    image_bytes: &[u8],
    width: usize,
    height: usize,
    contrast_scalar: f32,
) -> Result<Vec<u8>, AppError> {
    if !contrast_scalar.is_finite() || !(0.1..=4.0).contains(&contrast_scalar) {
        return Err(AppError::DataParsing(
            "Contrast scalar must be finite and between 0.1 and 4.0".to_string(),
        ));
    }
    let total_pixels = width.checked_mul(height).ok_or_else(|| {
        AppError::DataParsing("Image geometry exceeds addressable memory".to_string())
    })?;
    if total_pixels == 0 || total_pixels > MAXIMUM_IMAGE_PIXELS || image_bytes.len() != total_pixels
    {
        return Err(AppError::DataParsing(
            "Image buffer and bounded grayscale geometry do not match".to_string(),
        ));
    }
    let mut horizontal_sums = vec![0_u32; total_pixels];
    for y in 0..height {
        let row_start = y * width;
        let initial_end = ADAPTIVE_WINDOW_RADIUS.min(width - 1);
        let mut row_sum = image_bytes[row_start..=row_start + initial_end]
            .iter()
            .map(|value| u32::from(*value))
            .sum::<u32>();
        for x in 0..width {
            if x > 0 {
                if let Some(add_x) = x.checked_add(ADAPTIVE_WINDOW_RADIUS) {
                    if add_x < width {
                        row_sum += u32::from(image_bytes[row_start + add_x]);
                    }
                }
                if x > ADAPTIVE_WINDOW_RADIUS {
                    row_sum -= u32::from(image_bytes[row_start + x - ADAPTIVE_WINDOW_RADIUS - 1]);
                }
            }
            horizontal_sums[row_start + x] = row_sum;
        }
    }
    let mut output = vec![0_u8; total_pixels];
    for x in 0..width {
        let initial_end = ADAPTIVE_WINDOW_RADIUS.min(height - 1);
        let mut window_sum = (0..=initial_end)
            .map(|y| u64::from(horizontal_sums[y * width + x]))
            .sum::<u64>();
        for y in 0..height {
            if y > 0 {
                if let Some(add_y) = y.checked_add(ADAPTIVE_WINDOW_RADIUS) {
                    if add_y < height {
                        window_sum += u64::from(horizontal_sums[add_y * width + x]);
                    }
                }
                if y > ADAPTIVE_WINDOW_RADIUS {
                    window_sum -=
                        u64::from(horizontal_sums[(y - ADAPTIVE_WINDOW_RADIUS - 1) * width + x]);
                }
            }
            let x0 = x.saturating_sub(ADAPTIVE_WINDOW_RADIUS);
            let x1 = x.saturating_add(ADAPTIVE_WINDOW_RADIUS).min(width - 1);
            let y0 = y.saturating_sub(ADAPTIVE_WINDOW_RADIUS);
            let y1 = y.saturating_add(ADAPTIVE_WINDOW_RADIUS).min(height - 1);
            let count = (x1 - x0 + 1) * (y1 - y0 + 1);
            let threshold = window_sum as f32 / count as f32 * contrast_scalar;
            output[y * width + x] = if image_bytes[y * width + x] as f32 <= threshold {
                0
            } else {
                255
            };
        }
    }
    Ok(output)
}

fn validate_page_content_streams(
    document: &Document,
    page_id: ObjectId,
    page_number: u32,
) -> Result<(), AppError> {
    let content_streams = document.get_page_contents(page_id);
    if content_streams.len() > MAXIMUM_FILLED_RECTANGLES_PER_PAGE {
        return Err(AppError::DataParsing(format!(
            "PDF page {page_number} exceeds {} content streams",
            MAXIMUM_FILLED_RECTANGLES_PER_PAGE
        )));
    }
    let mut compressed_bytes = 0_usize;
    for identifier in content_streams {
        let Some(Object::Stream(stream)) = document
            .get_object(identifier)
            .ok()
            .and_then(|value| resolve_object(document, value))
        else {
            continue;
        };
        compressed_bytes = compressed_bytes
            .checked_add(stream.content.len())
            .filter(|value| *value <= MAXIMUM_PAGE_CONTENT_BYTES)
            .ok_or_else(|| {
                AppError::DataParsing(format!(
                    "PDF page {page_number} exceeds the {MAXIMUM_PAGE_CONTENT_BYTES}-byte compressed-content limit"
                ))
            })?;
        for key in [b"DL".as_slice(), b"Length1"] {
            if let Some(declared_size) = stream
                .dict
                .get(key)
                .ok()
                .and_then(|value| resolve_object(document, value))
                .and_then(object_integer)
            {
                if declared_size < 0 || declared_size as u64 > MAXIMUM_PAGE_CONTENT_BYTES as u64 {
                    return Err(AppError::DataParsing(format!(
                        "PDF page {page_number} declares an oversized decoded content stream"
                    )));
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn bounded_page_content(
    document: &Document,
    page_id: ObjectId,
) -> Result<Vec<u8>, String> {
    let mut content = Vec::new();
    for identifier in document.get_page_contents(page_id) {
        let stream = document
            .get_object(identifier)
            .ok()
            .and_then(|value| resolve_object(document, value))
            .and_then(|value| value.as_stream().ok())
            .ok_or_else(|| "unresolved page content stream".to_string())?;
        let remaining = MAXIMUM_PAGE_CONTENT_BYTES.saturating_sub(content.len());
        let decoded = decode_content_stream_bounded(stream, remaining)?;
        content.extend_from_slice(&decoded);
    }
    Ok(content)
}

fn decode_content_stream_bounded(stream: &Stream, maximum_bytes: usize) -> Result<Vec<u8>, String> {
    if maximum_bytes == 0 {
        return Err("decoded content limit reached".to_string());
    }
    if stream.dict.get(b"Filter").is_err() {
        if stream.content.len() > maximum_bytes {
            return Err("uncompressed stream exceeds the decoded-content limit".to_string());
        }
        return Ok(stream.content.clone());
    }
    let filters = stream
        .filters()
        .map_err(|error| format!("invalid content filter declaration: {error}"))?;
    if filters.len() != 1 || (filters[0] != b"FlateDecode" && filters[0] != b"Fl") {
        return Err("unsupported or chained content filter".to_string());
    }
    if let Some(predictor) = stream
        .dict
        .get(b"DecodeParms")
        .ok()
        .and_then(|value| value.as_dict().ok())
        .and_then(|parameters| parameters.get(b"Predictor").ok())
        .and_then(object_integer)
    {
        if predictor > 1 {
            return Err("predictor-encoded content streams require manual review".to_string());
        }
    }
    let limit = u64::try_from(maximum_bytes)
        .map_err(|_| "decoded-content limit is not addressable".to_string())?;
    let mut decoder = ZlibDecoder::new(stream.content.as_slice()).take(limit.saturating_add(1));
    let mut decoded = Vec::new();
    decoder
        .read_to_end(&mut decoded)
        .map_err(|error| format!("Flate stream decoding failed: {error}"))?;
    if decoded.len() > maximum_bytes {
        return Err("Flate stream exceeds the decoded-content limit".to_string());
    }
    Ok(decoded)
}

fn analyze_page_geometry(
    document: &Document,
    page_id: ObjectId,
    operations: &[lopdf::content::Operation],
) -> Result<PageGeometry, AppError> {
    let mut geometry = PageGeometry::default();
    let mut budget = GeometryBudget::default();
    let top_level_forms = invoked_form_xobjects(document, page_id, operations);
    let mut active_forms = HashSet::with_capacity(top_level_forms.min(MAXIMUM_FORM_DEPTH));
    analyze_geometry_operations(
        document,
        inherited_page_resources(document, page_id),
        operations,
        GraphicsState::default(),
        TextState::default(),
        0,
        GeometryTraversal {
            active_forms: &mut active_forms,
            budget: &mut budget,
            geometry: &mut geometry,
        },
    )?;
    geometry.processed_operations = budget.operation_sequence;
    geometry.decoded_form_bytes = budget.decoded_form_bytes;
    geometry.form_invocations = budget.form_invocations;
    Ok(geometry)
}

fn analyze_geometry_operations(
    document: &Document,
    resources: Option<&Dictionary>,
    operations: &[lopdf::content::Operation],
    mut graphics: GraphicsState,
    mut text: TextState,
    form_depth: usize,
    traversal: GeometryTraversal<'_>,
) -> Result<(), AppError> {
    let GeometryTraversal {
        active_forms,
        budget,
        geometry,
    } = traversal;
    let mut graphics_stack = Vec::new();
    let mut pending_rectangles = Vec::new();
    let mut pending_polygon = Vec::new();
    let mut polygon_eligible = true;
    let mut inside_text = false;

    for operation in operations {
        budget.operation_sequence = budget
            .operation_sequence
            .checked_add(1)
            .filter(|value| *value <= MAXIMUM_PAGE_OPERATIONS)
            .ok_or_else(|| {
                AppError::DataParsing(format!(
                    "PDF page geometry exceeds {MAXIMUM_PAGE_OPERATIONS} operations including nested forms"
                ))
            })?;
        let operation_index = budget.operation_sequence;
        match operation.operator.as_str() {
            "q" => {
                if graphics_stack.len() >= MAXIMUM_GRAPHICS_STATE_DEPTH {
                    return Err(AppError::DataParsing(format!(
                        "PDF graphics-state depth exceeds {MAXIMUM_GRAPHICS_STATE_DEPTH}"
                    )));
                }
                graphics_stack.push((graphics, text));
            }
            "Q" => {
                if let Some((restored_graphics, mut restored_text)) = graphics_stack.pop() {
                    restored_text.matrix = text.matrix;
                    restored_text.line_matrix = text.line_matrix;
                    graphics = restored_graphics;
                    text = restored_text;
                } else {
                    geometry.malformed_operations += 1;
                }
            }
            "cm" => {
                if let Some(matrix) = matrix_from_operands(&operation.operands) {
                    graphics.ctm = graphics.ctm.multiply(matrix);
                } else {
                    geometry.malformed_operations += 1;
                }
            }
            "g" => {
                if let Some(gray) = operand_number(&operation.operands, 0) {
                    graphics.fill_color_space = FillColorSpace::DeviceGray;
                    graphics.fill_luminance = normalized_component(gray);
                } else {
                    geometry.malformed_operations += 1;
                }
            }
            "rg" => {
                if let (Some(red), Some(green), Some(blue)) = (
                    operand_number(&operation.operands, 0),
                    operand_number(&operation.operands, 1),
                    operand_number(&operation.operands, 2),
                ) {
                    graphics.fill_color_space = FillColorSpace::DeviceRgb;
                    graphics.fill_luminance = normalized_rgb_luminance(red, green, blue);
                } else {
                    geometry.malformed_operations += 1;
                }
            }
            "k" => {
                if let (Some(cyan), Some(magenta), Some(yellow), Some(black)) = (
                    operand_number(&operation.operands, 0),
                    operand_number(&operation.operands, 1),
                    operand_number(&operation.operands, 2),
                    operand_number(&operation.operands, 3),
                ) {
                    graphics.fill_color_space = FillColorSpace::DeviceCmyk;
                    graphics.fill_luminance =
                        normalized_cmyk_luminance(cyan, magenta, yellow, black);
                } else {
                    geometry.malformed_operations += 1;
                }
            }
            "cs" => set_fill_color_space(
                document,
                resources,
                &operation.operands,
                &mut graphics,
                &mut geometry.malformed_operations,
            ),
            "sc" | "scn" => set_fill_color(
                &operation.operands,
                &mut graphics,
                &mut geometry.malformed_operations,
            ),
            "gs" => apply_extended_graphics_state(
                document,
                resources,
                &operation.operands,
                &mut graphics,
                &mut geometry.malformed_operations,
            ),
            "m" => {
                finalize_polygon(
                    &mut pending_polygon,
                    &mut polygon_eligible,
                    &mut pending_rectangles,
                )?;
                if let (Some(x), Some(y)) = (
                    operand_number(&operation.operands, 0),
                    operand_number(&operation.operands, 1),
                ) {
                    pending_polygon.push(graphics.ctm.transform(x, y));
                } else {
                    geometry.malformed_operations += 1;
                    polygon_eligible = false;
                }
            }
            "l" => {
                if let (Some(x), Some(y)) = (
                    operand_number(&operation.operands, 0),
                    operand_number(&operation.operands, 1),
                ) {
                    if pending_polygon.len() < 5 && polygon_eligible {
                        pending_polygon.push(graphics.ctm.transform(x, y));
                    } else {
                        polygon_eligible = false;
                    }
                } else {
                    geometry.malformed_operations += 1;
                    polygon_eligible = false;
                }
            }
            "h" => finalize_polygon(
                &mut pending_polygon,
                &mut polygon_eligible,
                &mut pending_rectangles,
            )?,
            "c" | "v" | "y" => {
                pending_polygon.clear();
                polygon_eligible = false;
            }
            "re" => {
                finalize_polygon(
                    &mut pending_polygon,
                    &mut polygon_eligible,
                    &mut pending_rectangles,
                )?;
                if let (Some(x), Some(y), Some(width), Some(height)) = (
                    operand_number(&operation.operands, 0),
                    operand_number(&operation.operands, 1),
                    operand_number(&operation.operands, 2),
                    operand_number(&operation.operands, 3),
                ) {
                    if let Some(bounds) =
                        Bounds::transformed_rectangle(graphics.ctm, x, y, width, height)
                    {
                        if pending_rectangles.len() >= MAXIMUM_FILLED_RECTANGLES_PER_PAGE {
                            return Err(AppError::DataParsing(format!(
                                "PDF page exceeds {MAXIMUM_FILLED_RECTANGLES_PER_PAGE} rectangle path candidates"
                            )));
                        }
                        pending_rectangles.push(bounds);
                    }
                } else {
                    geometry.malformed_operations += 1;
                }
            }
            "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" => {
                finalize_polygon(
                    &mut pending_polygon,
                    &mut polygon_eligible,
                    &mut pending_rectangles,
                )?;
                for bounds in pending_rectangles.drain(..) {
                    if bounds.width() >= MINIMUM_RECTANGLE_DIMENSION
                        && bounds.height() >= MINIMUM_RECTANGLE_DIMENSION
                    {
                        if geometry.filled_rectangles.len() >= MAXIMUM_FILLED_RECTANGLES_PER_PAGE {
                            return Err(AppError::DataParsing(format!(
                                "PDF page exceeds {MAXIMUM_FILLED_RECTANGLES_PER_PAGE} filled rectangles"
                            )));
                        }
                        let classified = graphics.fill_luminance.is_some()
                            && graphics.fill_alpha.is_some()
                            && graphics.standard_blend_mode;
                        if !classified {
                            geometry.unclassified_filled_rectangles += 1;
                        }
                        geometry.filled_rectangles.push(FilledRectangle {
                            bounds,
                            operation_index,
                            dark_fill: classified
                                && graphics
                                    .fill_luminance
                                    .is_some_and(|luminance| luminance <= 0.25)
                                && graphics
                                    .fill_alpha
                                    .is_some_and(|alpha| alpha >= MINIMUM_REDACTION_FILL_ALPHA),
                        });
                    }
                }
            }
            "S" | "s" | "n" => {
                pending_rectangles.clear();
                pending_polygon.clear();
                polygon_eligible = true;
            }
            "BT" => {
                inside_text = true;
                text.matrix = Matrix::IDENTITY;
                text.line_matrix = Matrix::IDENTITY;
            }
            "ET" => inside_text = false,
            "Tf" if inside_text => {
                if let Some(font_size) =
                    operand_number(&operation.operands, 1).filter(|value| value.abs() <= 10_000.0)
                {
                    text.font_size = font_size.abs();
                    text.glyph_width_factor = operation
                        .operands
                        .first()
                        .and_then(|value| value.as_name().ok())
                        .and_then(|name| average_font_width(document, resources, name))
                        .unwrap_or(0.5);
                } else {
                    geometry.malformed_operations += 1;
                }
            }
            "Tm" if inside_text => {
                if let Some(matrix) = matrix_from_operands(&operation.operands) {
                    text.matrix = matrix;
                    text.line_matrix = matrix;
                } else {
                    geometry.malformed_operations += 1;
                }
            }
            "Td" if inside_text => {
                if let (Some(x), Some(y)) = (
                    operand_number(&operation.operands, 0),
                    operand_number(&operation.operands, 1),
                ) {
                    move_text_line(&mut text, x, y);
                } else {
                    geometry.malformed_operations += 1;
                }
            }
            "TD" if inside_text => {
                if let (Some(x), Some(y)) = (
                    operand_number(&operation.operands, 0),
                    operand_number(&operation.operands, 1),
                ) {
                    text.leading = -y;
                    move_text_line(&mut text, x, y);
                } else {
                    geometry.malformed_operations += 1;
                }
            }
            "T*" if inside_text => move_to_next_text_line(&mut text),
            "Tc" if inside_text => set_text_number(
                &operation.operands,
                0,
                &mut text.character_spacing,
                &mut geometry.malformed_operations,
            ),
            "Tw" if inside_text => set_text_number(
                &operation.operands,
                0,
                &mut text.word_spacing,
                &mut geometry.malformed_operations,
            ),
            "TL" if inside_text => set_text_number(
                &operation.operands,
                0,
                &mut text.leading,
                &mut geometry.malformed_operations,
            ),
            "Ts" if inside_text => set_text_number(
                &operation.operands,
                0,
                &mut text.rise,
                &mut geometry.malformed_operations,
            ),
            "Tz" if inside_text => {
                if let Some(scale) = operand_number(&operation.operands, 0)
                    .map(|value| value / 100.0)
                    .filter(|value| value.abs() <= 10.0)
                {
                    text.horizontal_scale = scale;
                } else {
                    geometry.malformed_operations += 1;
                }
            }
            "Tr" if inside_text => {
                if let Some(mode) = operation
                    .operands
                    .first()
                    .and_then(object_integer)
                    .filter(|value| (0..=7).contains(value))
                {
                    text.rendering_mode = mode;
                } else {
                    geometry.malformed_operations += 1;
                }
            }
            "Tj" if inside_text => {
                if let Some(value) = operation.operands.first() {
                    show_text_object(value, &mut text, graphics.ctm, operation_index, geometry)?;
                } else {
                    geometry.malformed_operations += 1;
                }
            }
            "TJ" if inside_text => {
                if let Some(array) = operation
                    .operands
                    .first()
                    .and_then(|value| value.as_array().ok())
                {
                    show_text_array(array, &mut text, graphics.ctm, operation_index, geometry)?;
                } else {
                    geometry.malformed_operations += 1;
                }
            }
            "'" if inside_text => {
                move_to_next_text_line(&mut text);
                if let Some(value) = operation.operands.first() {
                    show_text_object(value, &mut text, graphics.ctm, operation_index, geometry)?;
                } else {
                    geometry.malformed_operations += 1;
                }
            }
            "\"" if inside_text => {
                if operation.operands.len() == 3 {
                    if let Some(word_spacing) = operand_number(&operation.operands, 0) {
                        text.word_spacing = word_spacing;
                    }
                    if let Some(character_spacing) = operand_number(&operation.operands, 1) {
                        text.character_spacing = character_spacing;
                    }
                    move_to_next_text_line(&mut text);
                    show_text_object(
                        &operation.operands[2],
                        &mut text,
                        graphics.ctm,
                        operation_index,
                        geometry,
                    )?;
                } else {
                    geometry.malformed_operations += 1;
                }
            }
            "Do" => {
                let Some(name) = operation
                    .operands
                    .first()
                    .and_then(|value| value.as_name().ok())
                else {
                    geometry.malformed_operations += 1;
                    continue;
                };
                analyze_form_xobject(
                    document,
                    resources,
                    name,
                    graphics,
                    text,
                    form_depth,
                    GeometryTraversal {
                        active_forms: &mut *active_forms,
                        budget: &mut *budget,
                        geometry: &mut *geometry,
                    },
                )?;
            }
            _ => {}
        }
    }

    Ok(())
}

fn analyze_form_xobject(
    document: &Document,
    resources: Option<&Dictionary>,
    name: &[u8],
    graphics: GraphicsState,
    text: TextState,
    form_depth: usize,
    traversal: GeometryTraversal<'_>,
) -> Result<(), AppError> {
    let GeometryTraversal {
        active_forms,
        budget,
        geometry,
    } = traversal;
    let Some(entry) = resources
        .and_then(|resources| resources.get(b"XObject").ok())
        .and_then(|value| resolve_dictionary(document, value))
        .and_then(|xobjects| xobjects.get(name).ok())
    else {
        geometry.malformed_operations += 1;
        return Ok(());
    };
    let identifier = match entry {
        Object::Reference(identifier) => Some(*identifier),
        _ => None,
    };
    let Some(Object::Stream(stream)) = resolve_object(document, entry) else {
        geometry.malformed_operations += 1;
        return Ok(());
    };
    let is_form = stream
        .dict
        .get(b"Subtype")
        .ok()
        .and_then(|value| resolve_object(document, value))
        .and_then(|value| value.as_name().ok())
        .is_some_and(|subtype| subtype == b"Form");
    if !is_form {
        return Ok(());
    }
    budget.form_invocations = budget
        .form_invocations
        .checked_add(1)
        .filter(|value| *value <= MAXIMUM_FORM_INVOCATIONS_PER_PAGE)
        .ok_or_else(|| {
            AppError::DataParsing(format!(
                "PDF page exceeds {MAXIMUM_FORM_INVOCATIONS_PER_PAGE} form invocations"
            ))
        })?;
    if form_depth >= MAXIMUM_FORM_DEPTH {
        geometry.unresolved_form_xobjects += 1;
        return Ok(());
    }
    if identifier.is_some_and(|identifier| active_forms.contains(&identifier)) {
        geometry.cyclic_form_xobjects += 1;
        return Ok(());
    }
    if stream.content.len() > MAXIMUM_PAGE_CONTENT_BYTES {
        return Err(AppError::DataParsing(format!(
            "PDF form stream exceeds the {MAXIMUM_PAGE_CONTENT_BYTES}-byte compressed-content limit"
        )));
    }
    let remaining_form_bytes = MAXIMUM_PAGE_CONTENT_BYTES.saturating_sub(budget.decoded_form_bytes);
    let content_bytes = match decode_content_stream_bounded(stream, remaining_form_bytes) {
        Ok(content) => content,
        Err(_) => {
            geometry.unresolved_form_xobjects += 1;
            return Ok(());
        }
    };
    budget.decoded_form_bytes = budget
        .decoded_form_bytes
        .checked_add(content_bytes.len())
        .filter(|value| *value <= MAXIMUM_PAGE_CONTENT_BYTES)
        .ok_or_else(|| {
            AppError::DataParsing(format!(
                "PDF nested forms exceed the {MAXIMUM_PAGE_CONTENT_BYTES}-byte decoded-content limit"
            ))
        })?;
    let content = match Content::decode(&content_bytes) {
        Ok(content) => content,
        Err(_) => {
            geometry.unresolved_form_xobjects += 1;
            return Ok(());
        }
    };
    let form_matrix = stream
        .dict
        .get(b"Matrix")
        .ok()
        .and_then(|value| resolve_object(document, value))
        .and_then(|value| value.as_array().ok())
        .and_then(|values| matrix_from_operands(values))
        .unwrap_or(Matrix::IDENTITY);
    let child_graphics = GraphicsState {
        ctm: graphics.ctm.multiply(form_matrix),
        ..graphics
    };
    let child_text = TextState {
        matrix: Matrix::IDENTITY,
        line_matrix: Matrix::IDENTITY,
        ..text
    };
    let form_resources = stream
        .dict
        .get(b"Resources")
        .ok()
        .and_then(|value| resolve_dictionary(document, value))
        .or(resources);
    let form_bounds = stream
        .dict
        .get(b"BBox")
        .ok()
        .and_then(|value| resolve_object(document, value))
        .and_then(|value| value.as_array().ok())
        .and_then(|values| Bounds::from_pdf_array(values))
        .and_then(|bounds| {
            Bounds::transformed_rectangle(
                child_graphics.ctm,
                bounds.min_x,
                bounds.min_y,
                bounds.width(),
                bounds.height(),
            )
        });
    if form_bounds.is_none() {
        geometry.forms_without_bounding_boxes += 1;
    }
    let text_start = geometry.text_runs.len();
    let rectangle_start = geometry.filled_rectangles.len();
    if let Some(identifier) = identifier {
        active_forms.insert(identifier);
    }
    let result = analyze_geometry_operations(
        document,
        form_resources,
        &content.operations,
        child_graphics,
        child_text,
        form_depth + 1,
        GeometryTraversal {
            active_forms: &mut *active_forms,
            budget: &mut *budget,
            geometry: &mut *geometry,
        },
    );
    if let Some(identifier) = identifier {
        active_forms.remove(&identifier);
    }
    result?;
    if let Some(form_bounds) = form_bounds {
        let visible_text = geometry
            .text_runs
            .drain(text_start..)
            .filter(|run| form_bounds.intersects(run.bounds))
            .collect::<Vec<_>>();
        geometry.text_runs.extend(visible_text);
        let visible_rectangles = geometry
            .filled_rectangles
            .drain(rectangle_start..)
            .filter(|rectangle| form_bounds.intersects(rectangle.bounds))
            .collect::<Vec<_>>();
        geometry.filled_rectangles.extend(visible_rectangles);
    }
    Ok(())
}

fn finalize_polygon(
    points: &mut Vec<(f64, f64)>,
    eligible: &mut bool,
    rectangles: &mut Vec<Bounds>,
) -> Result<(), AppError> {
    if points.len() == 5 {
        let first = points[0];
        let last = points[4];
        if (first.0 - last.0).abs() <= 0.001 && (first.1 - last.1).abs() <= 0.001 {
            points.pop();
        }
    }
    if *eligible && points.len() == 4 {
        let twice_area = points
            .iter()
            .zip(points.iter().cycle().skip(1))
            .take(points.len())
            .map(|((x1, y1), (x2, y2))| x1 * y2 - x2 * y1)
            .sum::<f64>()
            .abs();
        if let Some(bounds) = Bounds::from_points(points) {
            let coverage = twice_area * 0.5 / bounds.area();
            if coverage.is_finite()
                && coverage >= 0.25
                && bounds.width() >= MINIMUM_RECTANGLE_DIMENSION
                && bounds.height() >= MINIMUM_RECTANGLE_DIMENSION
            {
                if rectangles.len() >= MAXIMUM_FILLED_RECTANGLES_PER_PAGE {
                    return Err(AppError::DataParsing(format!(
                        "PDF page exceeds {MAXIMUM_FILLED_RECTANGLES_PER_PAGE} rectangle path candidates"
                    )));
                }
                rectangles.push(bounds);
            }
        }
    }
    points.clear();
    *eligible = true;
    Ok(())
}

fn extract_page_text(
    document: &Document,
    page_id: ObjectId,
    operations: &[lopdf::content::Operation],
) -> Result<String, AppError> {
    let fonts = document.get_page_fonts(page_id).map_err(|error| {
        AppError::DataParsing(format!("Failed to resolve PDF page fonts: {error}"))
    })?;
    let encodings = fonts
        .into_iter()
        .filter_map(|(name, font)| {
            font.get_font_encoding_with_limit(document, MAXIMUM_PAGE_CONTENT_BYTES)
                .ok()
                .map(|encoding| (name, encoding))
        })
        .collect::<BTreeMap<Vec<u8>, Encoding<'_>>>();
    let mut current_encoding = None;
    let mut text = String::new();
    for operation in operations {
        match operation.operator.as_str() {
            "Tf" => {
                current_encoding = operation
                    .operands
                    .first()
                    .and_then(|value| value.as_name().ok())
                    .and_then(|name| encodings.get(name));
            }
            "Tj" | "TJ" => {
                append_text_operands(&mut text, current_encoding, &operation.operands, 0)?;
            }
            "'" => {
                append_bounded_text(&mut text, "\n")?;
                append_text_operands(&mut text, current_encoding, &operation.operands, 0)?;
            }
            "\"" => {
                append_bounded_text(&mut text, "\n")?;
                if let Some(value) = operation.operands.get(2) {
                    append_text_operands(
                        &mut text,
                        current_encoding,
                        std::slice::from_ref(value),
                        0,
                    )?;
                }
            }
            "ET" if !text.ends_with('\n') => append_bounded_text(&mut text, "\n")?,
            _ => {}
        }
    }
    Ok(text)
}

fn append_text_operands(
    output: &mut String,
    encoding: Option<&Encoding<'_>>,
    operands: &[Object],
    depth: usize,
) -> Result<(), AppError> {
    if depth > 32 {
        return Err(AppError::DataParsing(
            "PDF text operands exceed the nesting limit".to_string(),
        ));
    }
    for operand in operands {
        match operand {
            Object::String(bytes, _) => {
                append_bounded_text(output, &decode_pdf_text(encoding, bytes))?;
            }
            Object::Array(values) => {
                append_text_operands(output, encoding, values, depth + 1)?;
            }
            Object::Integer(value) if *value < -100 => append_bounded_text(output, " ")?,
            Object::Real(value) if *value < -100.0 => append_bounded_text(output, " ")?,
            _ => {}
        }
    }
    Ok(())
}

fn append_bounded_text(output: &mut String, value: &str) -> Result<(), AppError> {
    output
        .len()
        .checked_add(value.len())
        .filter(|length| *length <= MAXIMUM_PAGE_TEXT_BYTES)
        .ok_or_else(|| {
            AppError::DataParsing(format!(
                "PDF page exceeds the {MAXIMUM_PAGE_TEXT_BYTES}-byte text audit limit"
            ))
        })?;
    output.push_str(value);
    Ok(())
}

fn decode_pdf_text(encoding: Option<&Encoding<'_>>, bytes: &[u8]) -> String {
    encoding
        .and_then(|encoding| Document::decode_text(encoding, bytes).ok())
        .or_else(|| {
            lopdf::decode_text_string(&Object::String(bytes.to_vec(), StringFormat::Literal)).ok()
        })
        .unwrap_or_else(|| String::from_utf8_lossy(bytes).into_owned())
}

fn show_text_object(
    object: &Object,
    text: &mut TextState,
    ctm: Matrix,
    operation_index: usize,
    geometry: &mut PageGeometry,
) -> Result<(), AppError> {
    let Some(bytes) = object.as_str().ok() else {
        geometry.malformed_operations += 1;
        return Ok(());
    };
    show_text_bytes(bytes, text, ctm, operation_index, geometry)
}

fn show_text_bytes(
    bytes: &[u8],
    text: &mut TextState,
    ctm: Matrix,
    operation_index: usize,
    geometry: &mut PageGeometry,
) -> Result<(), AppError> {
    let (advance, glyph_count) = text_string_advance(bytes, text)?;
    record_text_run(
        advance,
        glyph_count,
        decode_pdf_text(None, bytes),
        text,
        ctm,
        operation_index,
        geometry,
    )?;
    text.matrix = text.matrix.multiply(Matrix::translation(advance, 0.0));
    Ok(())
}

fn show_text_array(
    array: &[Object],
    text: &mut TextState,
    ctm: Matrix,
    operation_index: usize,
    geometry: &mut PageGeometry,
) -> Result<(), AppError> {
    for object in array {
        if let Ok(bytes) = object.as_str() {
            show_text_bytes(bytes, text, ctm, operation_index, geometry)?;
        } else if let Some(adjustment) = object_number(object) {
            let advance = -adjustment / 1_000.0 * text.font_size * text.horizontal_scale;
            text.matrix = text.matrix.multiply(Matrix::translation(advance, 0.0));
        } else {
            geometry.malformed_operations += 1;
        }
    }
    Ok(())
}

fn text_string_advance(bytes: &[u8], text: &TextState) -> Result<(f64, usize), AppError> {
    if bytes.len() > MAXIMUM_PAGE_TEXT_BYTES {
        return Err(AppError::DataParsing(format!(
            "PDF text operand exceeds {MAXIMUM_PAGE_TEXT_BYTES} bytes"
        )));
    }
    let glyph_count = if bytes.starts_with(&[0xfe, 0xff]) {
        bytes.len().saturating_sub(2) / 2
    } else {
        bytes.len()
    };
    let spaces = bytes.iter().filter(|value| **value == b' ').count();
    let advance = (glyph_count as f64 * text.font_size * text.glyph_width_factor
        + glyph_count as f64 * text.character_spacing
        + spaces as f64 * text.word_spacing)
        * text.horizontal_scale;
    if advance.is_finite() && advance.abs() <= MAXIMUM_COORDINATE {
        Ok((advance, glyph_count))
    } else {
        Err(AppError::DataParsing(
            "PDF text advance exceeds safe coordinate bounds".to_string(),
        ))
    }
}

fn record_text_run(
    advance: f64,
    glyph_count: usize,
    extracted_text: String,
    text: &TextState,
    ctm: Matrix,
    operation_index: usize,
    geometry: &mut PageGeometry,
) -> Result<(), AppError> {
    if glyph_count == 0 || text.font_size <= 0.0 {
        return Ok(());
    }
    if geometry.text_runs.len() >= MAXIMUM_TEXT_RUNS_PER_PAGE {
        return Err(AppError::DataParsing(format!(
            "PDF page exceeds {MAXIMUM_TEXT_RUNS_PER_PAGE} positioned text runs"
        )));
    }
    geometry.positioned_text_bytes = geometry
        .positioned_text_bytes
        .checked_add(extracted_text.len())
        .filter(|value| *value <= MAXIMUM_PAGE_TEXT_BYTES)
        .ok_or_else(|| {
            AppError::DataParsing(format!(
                "PDF positioned text exceeds {MAXIMUM_PAGE_TEXT_BYTES} bytes"
            ))
        })?;
    let width = advance.abs().max(text.font_size * 0.25);
    let x = advance.min(0.0);
    let matrix = ctm.multiply(text.matrix);
    if let Some(bounds) = Bounds::transformed_rectangle(
        matrix,
        x,
        text.rise - text.font_size * 0.25,
        width,
        text.font_size * 1.25,
    ) {
        geometry.text_runs.push(TextRun {
            bounds,
            operation_index,
            rendering_mode: text.rendering_mode,
            extracted_text,
        });
    } else {
        geometry.malformed_operations += 1;
    }
    Ok(())
}

fn correlate_redaction_geometry(
    text_runs: &[TextRun],
    filled_rectangles: &[FilledRectangle],
    redaction_annotations: &[Bounds],
) -> Result<ClashSummary, AppError> {
    let mut summary = ClashSummary::default();
    let mut sorted_text = (0..text_runs.len()).collect::<Vec<_>>();
    sorted_text.sort_by(|left, right| {
        text_runs[*left]
            .bounds
            .center()
            .0
            .total_cmp(&text_runs[*right].bounds.center().0)
    });
    let mut checks = 0_usize;

    for rectangle in filled_rectangles.iter().filter(|value| value.dark_fill) {
        for text_index in text_candidates(&sorted_text, text_runs, rectangle.bounds) {
            checks = checked_intersection_count(checks)?;
            let text = &text_runs[*text_index];
            if rectangle.bounds.covers_text(text.bounds)
                && (rectangle.operation_index >= text.operation_index
                    || text_rendering_is_invisible(text.rendering_mode))
            {
                let first_overlay = summary.overlay_text_runs.insert(*text_index);
                if text_rendering_is_invisible(text.rendering_mode) {
                    summary.invisible_overlay_text_runs.insert(*text_index);
                }
                if first_overlay {
                    append_intersection_evidence(
                        &mut summary,
                        if text_rendering_is_invisible(text.rendering_mode) {
                            "INVISIBLE_TEXT_DARK_OVERLAY"
                        } else {
                            "DARK_OVERLAY"
                        },
                        text.bounds,
                        rectangle.bounds,
                    );
                }
            }
        }
    }
    for annotation in redaction_annotations {
        for text_index in text_candidates(&sorted_text, text_runs, *annotation) {
            checks = checked_intersection_count(checks)?;
            if annotation.covers_text(text_runs[*text_index].bounds)
                && summary.annotation_text_runs.insert(*text_index)
            {
                append_intersection_evidence(
                    &mut summary,
                    "REDACTION_ANNOTATION",
                    text_runs[*text_index].bounds,
                    *annotation,
                );
            }
        }
    }
    Ok(summary)
}

fn append_intersection_evidence(
    summary: &mut ClashSummary,
    kind: &'static str,
    text_bounds: Bounds,
    mask_bounds: Bounds,
) {
    if summary.reported_intersections.len() >= MAXIMUM_REPORTED_INTERSECTIONS_PER_PAGE {
        summary.evidence_truncated = true;
        return;
    }
    summary
        .reported_intersections
        .push(RedactionIntersectionEvidence {
            kind,
            text_bounds: text_bounds.coordinates(),
            mask_bounds: mask_bounds.coordinates(),
        });
}

fn text_rendering_is_invisible(rendering_mode: i64) -> bool {
    matches!(rendering_mode, 3 | 7)
}

fn text_candidates<'a>(
    sorted_text: &'a [usize],
    text_runs: &[TextRun],
    bounds: Bounds,
) -> &'a [usize] {
    let start =
        sorted_text.partition_point(|index| text_runs[*index].bounds.center().0 < bounds.min_x);
    let end =
        sorted_text.partition_point(|index| text_runs[*index].bounds.center().0 <= bounds.max_x);
    &sorted_text[start..end]
}

fn checked_intersection_count(current: usize) -> Result<usize, AppError> {
    current
        .checked_add(1)
        .filter(|value| *value <= MAXIMUM_INTERSECTION_CHECKS)
        .ok_or_else(|| {
            AppError::DataParsing(format!(
                "PDF geometry exceeds {MAXIMUM_INTERSECTION_CHECKS} bounded intersection checks"
            ))
        })
}

fn collect_redaction_annotations(
    document: &Document,
    page_id: ObjectId,
) -> Result<RedactionAnnotationGeometry, AppError> {
    let page = document.get_dictionary(page_id).map_err(|error| {
        AppError::DataParsing(format!("Failed to resolve PDF page dictionary: {error}"))
    })?;
    let Ok(annotations) = page.get(b"Annots") else {
        return Ok(RedactionAnnotationGeometry::default());
    };
    let Some(annotations) = resolve_object(document, annotations) else {
        return Ok(RedactionAnnotationGeometry::default());
    };
    let annotation_objects: Vec<&Object> = match annotations {
        Object::Array(values) => values.iter().collect(),
        Object::Dictionary(_) => vec![annotations],
        _ => return Ok(RedactionAnnotationGeometry::default()),
    };
    if annotation_objects.len() > MAXIMUM_FILLED_RECTANGLES_PER_PAGE {
        return Err(AppError::DataParsing(format!(
            "PDF page exceeds {MAXIMUM_FILLED_RECTANGLES_PER_PAGE} annotations"
        )));
    }
    let mut geometry = RedactionAnnotationGeometry::default();
    for annotation in annotation_objects {
        let Some(Object::Dictionary(dictionary)) = resolve_object(document, annotation) else {
            continue;
        };
        let is_redaction = dictionary
            .get(b"Subtype")
            .ok()
            .and_then(|value| resolve_object(document, value))
            .and_then(|value| value.as_name().ok())
            .is_some_and(|name| name == b"Redact");
        if !is_redaction {
            continue;
        }
        geometry.annotation_count += 1;
        let quad_start = geometry.regions.len();
        if let Some(quad_points) = dictionary
            .get(b"QuadPoints")
            .ok()
            .and_then(|value| resolve_object(document, value))
            .and_then(|value| value.as_array().ok())
            .filter(|values| {
                values.len() % 8 == 0 && values.len() <= MAXIMUM_FILLED_RECTANGLES_PER_PAGE * 8
            })
        {
            for quad in quad_points.chunks_exact(8) {
                let points = quad
                    .chunks_exact(2)
                    .filter_map(|point| {
                        Some((object_number(&point[0])?, object_number(&point[1])?))
                    })
                    .collect::<Vec<_>>();
                if points.len() == 4 {
                    if geometry.regions.len() >= MAXIMUM_FILLED_RECTANGLES_PER_PAGE {
                        return Err(AppError::DataParsing(format!(
                            "PDF page exceeds {MAXIMUM_FILLED_RECTANGLES_PER_PAGE} redaction quadrilaterals"
                        )));
                    }
                    if let Some(bounds) = Bounds::from_points(&points) {
                        geometry.regions.push(bounds);
                    }
                }
            }
        }
        if geometry.regions.len() > quad_start {
            continue;
        }
        let Some(rectangle) = dictionary
            .get(b"Rect")
            .ok()
            .and_then(|value| resolve_object(document, value))
            .and_then(|value| value.as_array().ok())
            .and_then(|array| Bounds::from_pdf_array(array))
        else {
            continue;
        };
        if geometry.regions.len() >= MAXIMUM_FILLED_RECTANGLES_PER_PAGE {
            return Err(AppError::DataParsing(format!(
                "PDF page exceeds {MAXIMUM_FILLED_RECTANGLES_PER_PAGE} redaction regions"
            )));
        }
        geometry.regions.push(rectangle);
    }
    Ok(geometry)
}

fn invoked_form_xobjects(
    document: &Document,
    page_id: ObjectId,
    operations: &[lopdf::content::Operation],
) -> usize {
    let mut form_names = HashSet::new();
    for operation in operations
        .iter()
        .filter(|operation| operation.operator == "Do")
    {
        let Some(name) = operation
            .operands
            .first()
            .and_then(|value| value.as_name().ok())
        else {
            continue;
        };
        if page_xobject(document, page_id, name).is_some_and(|object| {
            let Some(Object::Stream(stream)) = resolve_object(document, object) else {
                return false;
            };
            stream
                .dict
                .get(b"Subtype")
                .ok()
                .and_then(|value| resolve_object(document, value))
                .and_then(|value| value.as_name().ok())
                .is_some_and(|subtype| subtype == b"Form")
        }) {
            form_names.insert(name.to_vec());
        }
    }
    form_names.len()
}

fn page_xobject<'a>(document: &'a Document, page_id: ObjectId, name: &[u8]) -> Option<&'a Object> {
    inherited_page_resources(document, page_id)
        .and_then(|resources| resources.get(b"XObject").ok())
        .and_then(|value| resolve_dictionary(document, value))
        .and_then(|xobjects| xobjects.get(name).ok())
}

fn inherited_page_resources(document: &Document, page_id: ObjectId) -> Option<&Dictionary> {
    let mut identifier = page_id;
    let mut visited = HashSet::new();
    for _ in 0..32 {
        if !visited.insert(identifier) {
            return None;
        }
        let node = document.get_dictionary(identifier).ok()?;
        if let Ok(resources) = node.get(b"Resources") {
            return resolve_dictionary(document, resources);
        }
        let Object::Reference(parent) = node.get(b"Parent").ok()? else {
            return None;
        };
        identifier = *parent;
    }
    None
}

fn collect_structural_signals(document: &Document) -> Result<Vec<String>, AppError> {
    // NOTE(pdf-audit): These strings are compatibility signals for the existing UI. New work
    // should emit a typed locator and preserve the originating object reference as well.
    let mut signals = Vec::new();
    if document.trailer.get(b"Prev").is_ok() {
        append_signal(&mut signals, "INCREMENTAL_UPDATE_CHAIN_PRESENT".to_string())?;
    }
    let Ok(catalog) = document.catalog() else {
        return Ok(signals);
    };
    for (key, signal) in [
        (b"PieceInfo".as_slice(), "PIECE_INFO_HISTORY_PRESENT"),
        (b"OCProperties", "OPTIONAL_CONTENT_LAYERS_PRESENT"),
        (b"OpenAction", "DOCUMENT_OPEN_ACTION_PRESENT"),
        (b"AA", "DOCUMENT_ADDITIONAL_ACTIONS_PRESENT"),
    ] {
        if catalog.get(key).is_ok() {
            append_signal(&mut signals, signal.to_string())?;
        }
    }
    if let Some(acroform) = catalog
        .get(b"AcroForm")
        .ok()
        .and_then(|value| resolve_dictionary(document, value))
    {
        if acroform.get(b"XFA").is_ok() {
            append_signal(&mut signals, "XFA_FORM_CONTENT_PRESENT".to_string())?;
        }
    }
    if let Some(names) = catalog
        .get(b"Names")
        .ok()
        .and_then(|value| resolve_dictionary(document, value))
    {
        if names.get(b"EmbeddedFiles").is_ok() {
            append_signal(&mut signals, "EMBEDDED_FILES_PRESENT".to_string())?;
        }
        if names.get(b"JavaScript").is_ok() {
            append_signal(&mut signals, "DOCUMENT_JAVASCRIPT_PRESENT".to_string())?;
        }
    }
    Ok(signals)
}

fn append_signal(signals: &mut Vec<String>, signal: String) -> Result<(), AppError> {
    if signals.len() >= MAXIMUM_STRUCTURAL_SIGNALS {
        return Err(AppError::DataParsing(format!(
            "PDF audit exceeds {MAXIMUM_STRUCTURAL_SIGNALS} structural signals"
        )));
    }
    signals.push(signal);
    Ok(())
}

fn append_quarantined_page(output: &mut String, page_number: u32, text: &str) -> bool {
    let block =
        format!("--- QUARANTINED POTENTIAL REDACTION OVERLAY PAGE {page_number} ---\n{text}\n");
    let remaining = MAXIMUM_QUARANTINED_OUTPUT_BYTES.saturating_sub(output.len());
    if block.len() <= remaining {
        output.push_str(&block);
        return true;
    }
    let mut end = remaining.min(block.len());
    while end > 0 && !block.is_char_boundary(end) {
        end -= 1;
    }
    output.push_str(&block[..end]);
    false
}

fn move_text_line(text: &mut TextState, x: f64, y: f64) {
    text.line_matrix = text.line_matrix.multiply(Matrix::translation(x, y));
    text.matrix = text.line_matrix;
}

fn move_to_next_text_line(text: &mut TextState) {
    move_text_line(text, 0.0, -text.leading);
}

fn set_text_number(
    operands: &[Object],
    index: usize,
    target: &mut f64,
    malformed_operations: &mut usize,
) {
    if let Some(value) =
        operand_number(operands, index).filter(|value| value.abs() <= MAXIMUM_COORDINATE)
    {
        *target = value;
    } else {
        *malformed_operations += 1;
    }
}

fn matrix_from_operands(operands: &[Object]) -> Option<Matrix> {
    if operands.len() != 6 {
        return None;
    }
    let matrix = Matrix {
        a: operand_number(operands, 0)?,
        b: operand_number(operands, 1)?,
        c: operand_number(operands, 2)?,
        d: operand_number(operands, 3)?,
        e: operand_number(operands, 4)?,
        f: operand_number(operands, 5)?,
    };
    [matrix.a, matrix.b, matrix.c, matrix.d, matrix.e, matrix.f]
        .iter()
        .all(|value| value.is_finite() && value.abs() <= MAXIMUM_COORDINATE)
        .then_some(matrix)
}

fn set_fill_color_space(
    document: &Document,
    resources: Option<&Dictionary>,
    operands: &[Object],
    graphics: &mut GraphicsState,
    malformed_operations: &mut usize,
) {
    let Some(name) = (operands.len() == 1)
        .then(|| operands.first())
        .flatten()
        .and_then(|value| value.as_name().ok())
    else {
        *malformed_operations += 1;
        graphics.fill_color_space = FillColorSpace::Unsupported;
        graphics.fill_luminance = None;
        return;
    };
    graphics.fill_color_space = device_color_space(name)
        .or_else(|| {
            resources
                .and_then(|resources| resources.get(b"ColorSpace").ok())
                .and_then(|value| resolve_dictionary(document, value))
                .and_then(|color_spaces| color_spaces.get(name).ok())
                .and_then(|value| resolve_object(document, value))
                .and_then(|value| match value {
                    Object::Name(name) => device_color_space(name),
                    Object::Array(values) => values
                        .first()
                        .and_then(|value| value.as_name().ok())
                        .and_then(|family| match family {
                            b"CalGray" => Some(FillColorSpace::DeviceGray),
                            b"CalRGB" => Some(FillColorSpace::DeviceRgb),
                            _ => None,
                        }),
                    _ => None,
                })
        })
        .unwrap_or(FillColorSpace::Unsupported);
    graphics.fill_luminance = match graphics.fill_color_space {
        FillColorSpace::DeviceGray | FillColorSpace::DeviceRgb => Some(0.0),
        FillColorSpace::DeviceCmyk => Some(0.0),
        FillColorSpace::Unsupported => None,
    };
}

fn device_color_space(name: &[u8]) -> Option<FillColorSpace> {
    match name {
        b"DeviceGray" | b"G" => Some(FillColorSpace::DeviceGray),
        b"DeviceRGB" | b"RGB" => Some(FillColorSpace::DeviceRgb),
        b"DeviceCMYK" | b"CMYK" => Some(FillColorSpace::DeviceCmyk),
        _ => None,
    }
}

fn set_fill_color(
    operands: &[Object],
    graphics: &mut GraphicsState,
    malformed_operations: &mut usize,
) {
    let luminance = match graphics.fill_color_space {
        FillColorSpace::DeviceGray if operands.len() == 1 => {
            operand_number(operands, 0).and_then(normalized_component)
        }
        FillColorSpace::DeviceRgb if operands.len() == 3 => normalized_rgb_luminance(
            operand_number(operands, 0).unwrap_or(f64::NAN),
            operand_number(operands, 1).unwrap_or(f64::NAN),
            operand_number(operands, 2).unwrap_or(f64::NAN),
        ),
        FillColorSpace::DeviceCmyk if operands.len() == 4 => normalized_cmyk_luminance(
            operand_number(operands, 0).unwrap_or(f64::NAN),
            operand_number(operands, 1).unwrap_or(f64::NAN),
            operand_number(operands, 2).unwrap_or(f64::NAN),
            operand_number(operands, 3).unwrap_or(f64::NAN),
        ),
        FillColorSpace::Unsupported => None,
        _ => {
            *malformed_operations += 1;
            graphics.fill_luminance = None;
            return;
        }
    };
    if luminance.is_none() && !matches!(graphics.fill_color_space, FillColorSpace::Unsupported) {
        *malformed_operations += 1;
    }
    graphics.fill_luminance = luminance;
}

fn apply_extended_graphics_state(
    document: &Document,
    resources: Option<&Dictionary>,
    operands: &[Object],
    graphics: &mut GraphicsState,
    malformed_operations: &mut usize,
) {
    let Some(name) = operands.first().and_then(|value| value.as_name().ok()) else {
        *malformed_operations += 1;
        graphics.fill_alpha = None;
        graphics.standard_blend_mode = false;
        return;
    };
    let Some(state) = resources
        .and_then(|resources| resources.get(b"ExtGState").ok())
        .and_then(|value| resolve_dictionary(document, value))
        .and_then(|states| states.get(name).ok())
        .and_then(|value| resolve_dictionary(document, value))
    else {
        *malformed_operations += 1;
        graphics.fill_alpha = None;
        graphics.standard_blend_mode = false;
        return;
    };
    if let Ok(alpha) = state.get(b"ca") {
        graphics.fill_alpha = resolve_object(document, alpha)
            .and_then(object_number)
            .and_then(normalized_component);
        if graphics.fill_alpha.is_none() {
            *malformed_operations += 1;
        }
    }
    if let Ok(blend_mode) = state.get(b"BM") {
        graphics.standard_blend_mode = resolve_object(document, blend_mode)
            .and_then(|value| value.as_name().ok())
            .is_some_and(|name| matches!(name, b"Normal" | b"Compatible"));
    }
    if let Ok(soft_mask) = state.get(b"SMask") {
        let no_soft_mask = resolve_object(document, soft_mask)
            .and_then(|value| value.as_name().ok())
            .is_some_and(|name| name == b"None");
        if !no_soft_mask {
            graphics.standard_blend_mode = false;
        }
    }
}

fn average_font_width(
    document: &Document,
    resources: Option<&Dictionary>,
    font_name: &[u8],
) -> Option<f64> {
    let font = resources
        .and_then(|resources| resources.get(b"Font").ok())
        .and_then(|value| resolve_dictionary(document, value))
        .and_then(|fonts| fonts.get(font_name).ok())
        .and_then(|value| resolve_dictionary(document, value))?;
    if let Some(widths) = font
        .get(b"Widths")
        .ok()
        .and_then(|value| resolve_object(document, value))
        .and_then(|value| value.as_array().ok())
    {
        let mut total = 0.0;
        let mut count = 0_usize;
        for width in widths.iter().filter_map(object_number) {
            if (0.0..=5_000.0).contains(&width) {
                total += width;
                count += 1;
            }
        }
        if count > 0 {
            return normalized_font_width(total / count as f64);
        }
    }
    font.get(b"DW")
        .ok()
        .and_then(|value| resolve_object(document, value))
        .and_then(object_number)
        .and_then(normalized_font_width)
        .or_else(|| {
            font.get(b"FontDescriptor")
                .ok()
                .and_then(|value| resolve_dictionary(document, value))
                .and_then(|descriptor| descriptor.get(b"MissingWidth").ok())
                .and_then(|value| resolve_object(document, value))
                .and_then(object_number)
                .and_then(normalized_font_width)
        })
}

fn normalized_font_width(width: f64) -> Option<f64> {
    let factor = width / 1_000.0;
    (factor.is_finite() && (0.05..=5.0).contains(&factor)).then_some(factor)
}

fn normalized_component(value: f64) -> Option<f64> {
    value.is_finite().then_some(value.clamp(0.0, 1.0))
}

fn normalized_rgb_luminance(red: f64, green: f64, blue: f64) -> Option<f64> {
    let red = normalized_component(red)?;
    let green = normalized_component(green)?;
    let blue = normalized_component(blue)?;
    Some(0.2126 * red + 0.7152 * green + 0.0722 * blue)
}

fn normalized_cmyk_luminance(cyan: f64, magenta: f64, yellow: f64, black: f64) -> Option<f64> {
    let cyan = normalized_component(cyan)?;
    let magenta = normalized_component(magenta)?;
    let yellow = normalized_component(yellow)?;
    let black = normalized_component(black)?;
    normalized_rgb_luminance(
        1.0 - (cyan + black).min(1.0),
        1.0 - (magenta + black).min(1.0),
        1.0 - (yellow + black).min(1.0),
    )
}

fn operand_number(operands: &[Object], index: usize) -> Option<f64> {
    operands.get(index).and_then(object_number)
}

fn object_number(object: &Object) -> Option<f64> {
    object
        .as_float()
        .ok()
        .map(f64::from)
        .filter(|value| value.is_finite() && value.abs() <= MAXIMUM_COORDINATE)
}

fn object_integer(object: &Object) -> Option<i64> {
    object.as_i64().ok()
}

fn resolve_object<'a>(document: &'a Document, object: &'a Object) -> Option<&'a Object> {
    let mut current = object;
    let mut visited = HashSet::new();
    for _ in 0..32 {
        let Object::Reference(identifier) = current else {
            return Some(current);
        };
        if !visited.insert(*identifier) {
            return None;
        }
        current = document.get_object(*identifier).ok()?;
    }
    None
}

fn collect_metadata(document: &Document) -> Result<Vec<String>, AppError> {
    let Some(dictionary) = document
        .trailer
        .get(b"Info")
        .ok()
        .and_then(|value| resolve_dictionary(document, value))
    else {
        return Ok(Vec::new());
    };
    let mut values = Vec::new();
    let standard_keys = [
        b"Title".as_slice(),
        b"Subject",
        b"Author",
        b"Keywords",
        b"Creator",
        b"Producer",
        b"CreationDate",
        b"ModDate",
    ];
    for key in standard_keys.iter().copied() {
        let Ok(value) = dictionary.get(key) else {
            continue;
        };
        let Some(value) = resolve_object(document, value) else {
            continue;
        };
        let display = match value {
            Object::String(bytes, _) | Object::Name(bytes) => bounded_metadata_value(bytes),
            _ => continue,
        };
        if !display.is_empty() {
            values.push(format!("{}: {display}", String::from_utf8_lossy(key)));
        }
    }
    let mut custom_entries = dictionary
        .iter()
        .filter(|(key, _)| !standard_keys.contains(&key.as_slice()))
        .collect::<Vec<_>>();
    custom_entries.sort_by(|left, right| left.0.cmp(right.0));
    for (key, value) in custom_entries {
        if values.len() >= MAXIMUM_METADATA_ENTRIES {
            values.push("METADATA_ENTRY_LIST_TRUNCATED".to_string());
            break;
        }
        let Some(value) = resolve_object(document, value) else {
            continue;
        };
        let display = match value {
            Object::String(bytes, _) | Object::Name(bytes) => bounded_metadata_value(bytes),
            Object::Boolean(value) => value.to_string(),
            Object::Integer(value) => value.to_string(),
            Object::Real(value) => value.to_string(),
            _ => continue,
        };
        if !display.is_empty() {
            values.push(format!(
                "{}: {display}",
                bounded_metadata_value(key.as_slice())
            ));
        }
    }
    Ok(values)
}

fn bounded_metadata_value(bytes: &[u8]) -> String {
    let end = bytes.len().min(MAXIMUM_METADATA_VALUE_BYTES);
    let bounded = &bytes[..end];
    let mut value = if let Some(utf16) = bounded.strip_prefix(&[0xfe, 0xff]) {
        decode_utf16_metadata(utf16, u16::from_be_bytes)
    } else if let Some(utf16) = bounded.strip_prefix(&[0xff, 0xfe]) {
        decode_utf16_metadata(utf16, u16::from_le_bytes)
    } else {
        String::from_utf8_lossy(bounded).into_owned()
    }
    .trim()
    .to_string();
    if bytes.len() > end {
        value.push_str(" [TRUNCATED]");
    }
    value
}

fn collect_xmp_metadata(document: &Document) -> Vec<String> {
    let Some(Object::Stream(stream)) = document
        .catalog()
        .ok()
        .and_then(|catalog| catalog.get(b"Metadata").ok())
        .and_then(|value| resolve_object(document, value))
    else {
        return Vec::new();
    };
    match decode_content_stream_bounded(stream, MAXIMUM_XMP_BYTES) {
        Ok(bytes) => {
            let value = bounded_metadata_value(&bytes);
            (!value.is_empty())
                .then(|| format!("XMP: {value}"))
                .into_iter()
                .collect()
        }
        Err(_) => vec!["XMP_METADATA_PRESENT_BUT_UNRESOLVED".to_string()],
    }
}

fn decode_utf16_metadata(bytes: &[u8], decode: fn([u8; 2]) -> u16) -> String {
    let units = bytes
        .chunks_exact(2)
        .map(|chunk| decode([chunk[0], chunk[1]]))
        .collect::<Vec<_>>();
    String::from_utf16_lossy(&units)
}

fn resolve_dictionary<'a>(document: &'a Document, object: &'a Object) -> Option<&'a Dictionary> {
    resolve_object(document, object)?.as_dict().ok()
}
