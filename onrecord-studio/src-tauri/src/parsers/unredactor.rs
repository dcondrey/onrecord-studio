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
use std::rc::Rc;

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
// A scanned page is one raster covering (nearly) the whole page box, drawn under everything
// else; 0.9 leaves room for scanner margins and bleed while excluding stamped image overlays.
const RASTER_BACKDROP_MINIMUM_PAGE_COVERAGE: f64 = 0.9;
const MAXIMUM_FONT_CMAP_BYTES: usize = 4 * 1024 * 1024;
const MAXIMUM_TOTAL_FONT_CMAP_BYTES: usize = 64 * 1024 * 1024;
const MAXIMUM_COORDINATE: f64 = 1_000_000_000.0;
const MINIMUM_RECTANGLE_DIMENSION: f64 = 2.0;
const MINIMUM_TEXT_OVERLAP_RATIO: f64 = 0.5;
const MINIMUM_REDACTION_FILL_ALPHA: f64 = 0.8;
const REDACTION_ANALYSIS_VERSION: &str = "pdf-redaction-geometry-v3";

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
    bytes_per_code: usize,
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
            bytes_per_code: 1,
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

#[derive(Clone, Copy, Debug)]
struct ImageFill {
    bounds: Bounds,
    operation_index: usize,
}

#[derive(Default)]
struct PageGeometry {
    text_runs: Vec<TextRun>,
    filled_rectangles: Vec<FilledRectangle>,
    /// Image paints in traversal order; classified against the text z-order once the page is
    /// fully traversed, see `classify_paint_operations`.
    image_fills: Vec<ImageFill>,
    shading_fills: Vec<usize>,
    malformed_operations: usize,
    unclassified_filled_rectangles: usize,
    unclassified_shading_fills: usize,
    raster_backdrops: usize,
    images_without_text_overlap: usize,
    shadings_without_text_overlap: usize,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum FontKey {
    Font(ObjectId),
    ToUnicode(ObjectId),
}

/// Font encodings are derived once per font object for the whole document. Without this,
/// every page and every form-XObject invocation re-inflated and re-parsed each /ToUnicode
/// CMap, and nothing counted that work against any budget.
#[derive(Default)]
struct FontEncodingCache<'d> {
    entries: BTreeMap<FontKey, Option<Rc<Encoding<'d>>>>,
    decoded_cmap_bytes: usize,
}

struct GeometryTraversal<'a, 'd> {
    inherited_encoding: Option<&'a Encoding<'d>>,
    active_forms: &'a mut HashSet<ObjectId>,
    budget: &'a mut GeometryBudget,
    geometry: &'a mut PageGeometry,
    font_cache: &'a mut FontEncodingCache<'d>,
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
    /// Pages whose content is a raster backdrop. Redactions burned into that image are outside
    /// this vector analysis; the count states that limit without claiming the page failed.
    pub raster_backdrop_pages: usize,
    pub metadata_leaks: Vec<String>,
    pub isolated_structural_anomalies: Vec<String>,
    pub page_findings: Vec<RedactionPageFinding>,
    /// Redacted share of the measured text. `None` when no text was measured: absence is not zero.
    pub calculated_redaction_ratio: Option<f64>,
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
    let document = crate::parsers::pdf::load_bounded_mem(&source_bytes).map_err(|error| {
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
    let mut analyzed_page_count = 0_usize;
    let mut raster_backdrop_pages = 0_usize;
    let mut font_cache = FontEncodingCache::default();

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
        let geometry =
            analyze_page_geometry(&document, page_id, &operators.operations, &mut font_cache)?;
        analyzed_page_count += 1;
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
            match extract_page_text(&document, page_id, &operators.operations, &mut font_cache) {
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
        if geometry.unclassified_shading_fills > 0 {
            append_signal(
                &mut isolated_structural_anomalies,
                format!(
                    "PAGE_{page_number}_UNCLASSIFIED_SHADING_FILLS_{}",
                    geometry.unclassified_shading_fills
                ),
            )?;
        }
        if geometry.raster_backdrops > 0 {
            raster_backdrop_pages += 1;
            append_signal(
                &mut isolated_structural_anomalies,
                format!("PAGE_{page_number}_RASTER_BACKDROP"),
            )?;
        }
        if geometry.images_without_text_overlap > 0 {
            append_signal(
                &mut isolated_structural_anomalies,
                format!(
                    "PAGE_{page_number}_IMAGES_WITHOUT_TEXT_OVERLAP_{}",
                    geometry.images_without_text_overlap
                ),
            )?;
        }
        if geometry.shadings_without_text_overlap > 0 {
            append_signal(
                &mut isolated_structural_anomalies,
                format!(
                    "PAGE_{page_number}_SHADINGS_WITHOUT_TEXT_OVERLAP_{}",
                    geometry.shadings_without_text_overlap
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
            && geometry.unclassified_shading_fills == 0
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

    let calculated_redaction_ratio = (total_character_count > 0)
        .then(|| redaction_character_count as f64 / total_character_count as f64);
    let coverage_complete = !isolated_structural_anomalies.iter().any(|signal| {
        signal.contains("MALFORMED_GEOMETRY_OPERATIONS")
            || signal.contains("UNCLASSIFIED_FILLED_RECTANGLES")
            || signal.contains("UNCLASSIFIED_SHADING_FILLS")
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
        scanned_page_count: analyzed_page_count,
        coverage_complete,
        requires_manual_review,
        is_leaky_redaction: bounding_box_clashes_count > 0,
        extracted_hidden_text: extracted_hidden_text.trim().to_string(),
        bounding_box_clashes_count,
        raster_backdrop_pages,
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

fn analyze_page_geometry<'d>(
    document: &'d Document,
    page_id: ObjectId,
    operations: &[lopdf::content::Operation],
    font_cache: &mut FontEncodingCache<'d>,
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
            inherited_encoding: None,
            active_forms: &mut active_forms,
            budget: &mut budget,
            geometry: &mut geometry,
            font_cache,
        },
    )?;
    geometry.processed_operations = budget.operation_sequence;
    geometry.decoded_form_bytes = budget.decoded_form_bytes;
    geometry.form_invocations = budget.form_invocations;
    classify_paint_operations(&mut geometry, inherited_page_box(document, page_id));
    Ok(geometry)
}

/// Sorts image and shading paints into three measured facts once the page's text z-order is
/// known: a raster backdrop (covers the page box and precedes every text run: the scan itself),
/// an overlay over earlier text (unclassified fill; the page cannot read as fully classified),
/// or a paint that intersects no earlier text (recorded, cannot hide anything).
fn classify_paint_operations(geometry: &mut PageGeometry, page_box: Option<Bounds>) {
    let first_text_operation = geometry
        .text_runs
        .iter()
        .map(|run| run.operation_index)
        .min();
    let covers_earlier_text = |bounds: Option<Bounds>, operation_index: usize| {
        geometry.text_runs.iter().any(|run| {
            run.operation_index < operation_index
                && bounds.is_none_or(|bounds| bounds.intersects(run.bounds))
        })
    };
    let image_fills = std::mem::take(&mut geometry.image_fills);
    let shading_fills = std::mem::take(&mut geometry.shading_fills);
    for image in image_fills {
        let is_backdrop = page_box.is_some_and(|page_box| {
            let covered_width =
                image.bounds.max_x.min(page_box.max_x) - image.bounds.min_x.max(page_box.min_x);
            let covered_height =
                image.bounds.max_y.min(page_box.max_y) - image.bounds.min_y.max(page_box.min_y);
            covered_width > 0.0
                && covered_height > 0.0
                && covered_width * covered_height / page_box.area()
                    >= RASTER_BACKDROP_MINIMUM_PAGE_COVERAGE
        }) && first_text_operation
            .is_none_or(|first| image.operation_index < first);
        if is_backdrop {
            geometry.raster_backdrops += 1;
        } else if covers_earlier_text(Some(image.bounds), image.operation_index) {
            geometry.unclassified_filled_rectangles += 1;
            geometry.filled_rectangles.push(FilledRectangle {
                bounds: image.bounds,
                operation_index: image.operation_index,
                dark_fill: false,
            });
        } else {
            geometry.images_without_text_overlap += 1;
        }
    }
    for operation_index in shading_fills {
        if covers_earlier_text(None, operation_index) {
            geometry.unclassified_shading_fills += 1;
        } else {
            geometry.shadings_without_text_overlap += 1;
        }
    }
}

fn inherited_page_box(document: &Document, page_id: ObjectId) -> Option<Bounds> {
    let mut identifier = page_id;
    let mut visited = HashSet::new();
    for _ in 0..32 {
        if !visited.insert(identifier) {
            return None;
        }
        let node = document.get_dictionary(identifier).ok()?;
        for key in [b"CropBox".as_slice(), b"MediaBox"] {
            if let Some(bounds) = node
                .get(key)
                .ok()
                .and_then(|value| resolve_object(document, value))
                .and_then(|value| value.as_array().ok())
                .and_then(|values| Bounds::from_pdf_array(values))
            {
                return Some(bounds);
            }
        }
        let Object::Reference(parent) = node.get(b"Parent").ok()? else {
            return None;
        };
        identifier = *parent;
    }
    None
}

fn analyze_geometry_operations<'d>(
    document: &'d Document,
    resources: Option<&'d Dictionary>,
    operations: &[lopdf::content::Operation],
    mut graphics: GraphicsState,
    mut text: TextState,
    form_depth: usize,
    traversal: GeometryTraversal<'_, 'd>,
) -> Result<(), AppError> {
    let GeometryTraversal {
        inherited_encoding,
        active_forms,
        budget,
        geometry,
        font_cache,
    } = traversal;
    // Positioned fragments must use the same PDF font mapping as page extraction.
    // Treating encoded glyph bytes as UTF-8 produced gibberish in real CPP packets.
    let encodings = resource_font_encodings(document, resources, font_cache);
    let mut current_encoding: Option<&Encoding<'d>> = inherited_encoding;
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
                graphics_stack.push((graphics, text, current_encoding));
            }
            "Q" => {
                if let Some((restored_graphics, mut restored_text, restored_encoding)) =
                    graphics_stack.pop()
                {
                    current_encoding = restored_encoding;
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
            "Tf" => {
                current_encoding = operation
                    .operands
                    .first()
                    .and_then(|value| value.as_name().ok())
                    .and_then(|name| encodings.get(name))
                    .map(Rc::as_ref);
                if current_encoding.is_none() {
                    geometry.malformed_operations += 1;
                }
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
                    text.bytes_per_code = operation
                        .operands
                        .first()
                        .and_then(|value| value.as_name().ok())
                        .map_or(1, |name| font_code_width(document, resources, name));
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
            "Tc" => set_text_number(
                &operation.operands,
                0,
                &mut text.character_spacing,
                &mut geometry.malformed_operations,
            ),
            "Tw" => set_text_number(
                &operation.operands,
                0,
                &mut text.word_spacing,
                &mut geometry.malformed_operations,
            ),
            "TL" => set_text_number(
                &operation.operands,
                0,
                &mut text.leading,
                &mut geometry.malformed_operations,
            ),
            "Ts" => set_text_number(
                &operation.operands,
                0,
                &mut text.rise,
                &mut geometry.malformed_operations,
            ),
            "Tz" => {
                if let Some(scale) = operand_number(&operation.operands, 0)
                    .map(|value| value / 100.0)
                    .filter(|value| value.abs() <= 10.0)
                {
                    text.horizontal_scale = scale;
                } else {
                    geometry.malformed_operations += 1;
                }
            }
            "Tr" => {
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
                    show_text_object(
                        value,
                        current_encoding,
                        &mut text,
                        graphics.ctm,
                        operation_index,
                        geometry,
                    )?;
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
                    show_text_array(
                        array,
                        current_encoding,
                        &mut text,
                        graphics.ctm,
                        operation_index,
                        geometry,
                    )?;
                } else {
                    geometry.malformed_operations += 1;
                }
            }
            "'" if inside_text => {
                move_to_next_text_line(&mut text);
                if let Some(value) = operation.operands.first() {
                    show_text_object(
                        value,
                        current_encoding,
                        &mut text,
                        graphics.ctm,
                        operation_index,
                        geometry,
                    )?;
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
                        current_encoding,
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
                        inherited_encoding: current_encoding,
                        active_forms: &mut *active_forms,
                        budget: &mut *budget,
                        geometry: &mut *geometry,
                        font_cache: &mut *font_cache,
                    },
                )?;
            }
            // An inline image paints the CTM unit square; classified against the text z-order
            // once the page is traversed.
            "BI" => record_image_fill(graphics.ctm, operation_index, geometry)?,
            // A shading paints the current clip region, whose bounds this pass does not track.
            "sh" => {
                if geometry.shading_fills.len() >= MAXIMUM_FILLED_RECTANGLES_PER_PAGE {
                    return Err(AppError::DataParsing(format!(
                        "PDF page exceeds {MAXIMUM_FILLED_RECTANGLES_PER_PAGE} shading fills"
                    )));
                }
                geometry.shading_fills.push(operation_index);
            }
            // A text-showing operator outside BT/ET is still rendered by common viewers.
            // Dropping it silently would report a complete audit that skipped that text.
            "Tj" | "TJ" | "'" | "\"" => geometry.malformed_operations += 1,
            _ => {}
        }
    }

    Ok(())
}

fn analyze_form_xobject<'d>(
    document: &'d Document,
    resources: Option<&'d Dictionary>,
    name: &[u8],
    graphics: GraphicsState,
    text: TextState,
    form_depth: usize,
    traversal: GeometryTraversal<'_, 'd>,
) -> Result<(), AppError> {
    let GeometryTraversal {
        inherited_encoding,
        active_forms,
        budget,
        geometry,
        font_cache,
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
    let subtype = stream
        .dict
        .get(b"Subtype")
        .ok()
        .and_then(|value| resolve_object(document, value))
        .and_then(|value| value.as_name().ok());
    match subtype {
        Some(b"Form") => {}
        // An image XObject paints the CTM unit square over whatever was drawn before it.
        Some(b"Image") => {
            return record_image_fill(graphics.ctm, budget.operation_sequence, geometry);
        }
        _ => return Ok(()),
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
    let image_start = geometry.image_fills.len();
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
            inherited_encoding,
            active_forms: &mut *active_forms,
            budget: &mut *budget,
            geometry: &mut *geometry,
            font_cache: &mut *font_cache,
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
        let visible_images = geometry
            .image_fills
            .drain(image_start..)
            .filter(|image| form_bounds.intersects(image.bounds))
            .collect::<Vec<_>>();
        geometry.image_fills.extend(visible_images);
    }
    Ok(())
}

fn record_image_fill(
    ctm: Matrix,
    operation_index: usize,
    geometry: &mut PageGeometry,
) -> Result<(), AppError> {
    let Some(bounds) = Bounds::transformed_rectangle(ctm, 0.0, 0.0, 1.0, 1.0) else {
        geometry.malformed_operations += 1;
        return Ok(());
    };
    if bounds.width() < MINIMUM_RECTANGLE_DIMENSION || bounds.height() < MINIMUM_RECTANGLE_DIMENSION
    {
        return Ok(());
    }
    if geometry.image_fills.len() >= MAXIMUM_FILLED_RECTANGLES_PER_PAGE {
        return Err(AppError::DataParsing(format!(
            "PDF page exceeds {MAXIMUM_FILLED_RECTANGLES_PER_PAGE} image paints"
        )));
    }
    geometry.image_fills.push(ImageFill {
        bounds,
        operation_index,
    });
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

fn encoding_for_font<'d>(
    font: &'d Dictionary,
    document: &'d Document,
    cache: &mut FontEncodingCache<'d>,
) -> Option<Encoding<'d>> {
    // /ToUnicode maps character codes to the published Unicode text even when /Encoding
    // is also present. lopdf otherwise prefers WinAnsi and loses custom/subset glyph maps.
    if let Ok(to_unicode) = font.get(b"ToUnicode") {
        // The CMap is inflated here, under this file's own bounds, so that its decoded size
        // is charged against a per-document budget before lopdf parses it.
        let Some(Object::Stream(stream)) = resolve_object(document, to_unicode) else {
            return None;
        };
        let remaining = MAXIMUM_FONT_CMAP_BYTES
            .min(MAXIMUM_TOTAL_FONT_CMAP_BYTES.saturating_sub(cache.decoded_cmap_bytes));
        let decoded = decode_content_stream_bounded(stream, remaining).ok()?;
        cache.decoded_cmap_bytes = cache.decoded_cmap_bytes.saturating_add(decoded.len());
        let plain_length = decoded.len();
        let mut unicode_font = font.clone();
        unicode_font.remove(b"Encoding");
        unicode_font.set(
            "ToUnicode",
            Object::Stream(Stream::new(Dictionary::new(), decoded)),
        );
        return match unicode_font.get_font_encoding_with_limit(document, plain_length) {
            Ok(Encoding::UnicodeMapEncoding(map)) => Some(Encoding::UnicodeMapEncoding(map)),
            // A malformed/bomb map must not silently turn glyph IDs into misleading text.
            _ => None,
        };
    }
    if font.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Type0".as_slice()) {
        // CID codes are glyph indices, not Unicode. Identity-H without /ToUnicode requires
        // an embedded-font cmap decoder that this bounded analyzer does not implement.
        return None;
    }
    font.get_font_encoding_with_limit(document, MAXIMUM_PAGE_CONTENT_BYTES)
        .ok()
}

fn resource_font_encodings<'d>(
    document: &'d Document,
    resources: Option<&'d Dictionary>,
    cache: &mut FontEncodingCache<'d>,
) -> BTreeMap<Vec<u8>, Rc<Encoding<'d>>> {
    let Some(fonts) = resources
        .and_then(|r| r.get(b"Font").ok())
        .and_then(|value| resolve_dictionary(document, value))
    else {
        return BTreeMap::new();
    };
    fonts
        .iter()
        .filter_map(|(name, value)| {
            cached_font_encoding(document, value, cache).map(|encoding| (name.clone(), encoding))
        })
        .collect()
}

fn cached_font_encoding<'d>(
    document: &'d Document,
    font_value: &'d Object,
    cache: &mut FontEncodingCache<'d>,
) -> Option<Rc<Encoding<'d>>> {
    let font = resolve_dictionary(document, font_value)?;
    let key = match font_value {
        Object::Reference(identifier) => Some(FontKey::Font(*identifier)),
        _ => match font.get(b"ToUnicode") {
            Ok(Object::Reference(identifier)) => Some(FontKey::ToUnicode(*identifier)),
            _ => None,
        },
    };
    if let Some(cached) = key.and_then(|key| cache.entries.get(&key)) {
        return cached.clone();
    }
    let encoding = encoding_for_font(font, document, cache).map(Rc::new);
    if let Some(key) = key {
        cache.entries.insert(key, encoding.clone());
    }
    encoding
}

fn extract_page_text<'d>(
    document: &'d Document,
    page_id: ObjectId,
    operations: &[lopdf::content::Operation],
    font_cache: &mut FontEncodingCache<'d>,
) -> Result<String, AppError> {
    let encodings = resource_font_encodings(
        document,
        inherited_page_resources(document, page_id),
        font_cache,
    );
    let mut current_encoding = None;
    let mut text = String::new();
    for operation in operations {
        match operation.operator.as_str() {
            "Tf" => {
                current_encoding = operation
                    .operands
                    .first()
                    .and_then(|value| value.as_name().ok())
                    .and_then(|name| encodings.get(name))
                    .map(Rc::as_ref);
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
    encoding: Option<&Encoding<'_>>,
    text: &mut TextState,
    ctm: Matrix,
    operation_index: usize,
    geometry: &mut PageGeometry,
) -> Result<(), AppError> {
    let Some(bytes) = object.as_str().ok() else {
        geometry.malformed_operations += 1;
        return Ok(());
    };
    show_text_bytes(bytes, encoding, text, ctm, operation_index, geometry)
}

fn show_text_bytes(
    bytes: &[u8],
    encoding: Option<&Encoding<'_>>,
    text: &mut TextState,
    ctm: Matrix,
    operation_index: usize,
    geometry: &mut PageGeometry,
) -> Result<(), AppError> {
    let (advance, glyph_count) = text_string_advance(bytes, text)?;
    record_text_run(
        advance,
        glyph_count,
        encoding
            .map(|mapping| decode_pdf_text(Some(mapping), bytes))
            .unwrap_or_default(),
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
    encoding: Option<&Encoding<'_>>,
    text: &mut TextState,
    ctm: Matrix,
    operation_index: usize,
    geometry: &mut PageGeometry,
) -> Result<(), AppError> {
    for object in array {
        if let Ok(bytes) = object.as_str() {
            show_text_bytes(bytes, encoding, text, ctm, operation_index, geometry)?;
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
    let glyph_count = if text.bytes_per_code > 1 {
        bytes.len().div_ceil(text.bytes_per_code)
    } else if bytes.starts_with(&[0xfe, 0xff]) {
        bytes.len().saturating_sub(2) / 2
    } else {
        bytes.len()
    };
    // Word spacing applies only to the single-byte code 32 (ISO 32000-1 9.3.3).
    let spaces = if text.bytes_per_code > 1 {
        0
    } else {
        bytes.iter().filter(|value| **value == b' ').count()
    };
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

fn font_code_width(document: &Document, resources: Option<&Dictionary>, font_name: &[u8]) -> usize {
    let is_two_byte = resources
        .and_then(|resources| resources.get(b"Font").ok())
        .and_then(|value| resolve_dictionary(document, value))
        .and_then(|fonts| fonts.get(font_name).ok())
        .and_then(|value| resolve_dictionary(document, value))
        .is_some_and(|font| {
            let name_is = |key: &[u8], accepted: &[&[u8]]| {
                font.get(key)
                    .ok()
                    .and_then(|value| resolve_object(document, value))
                    .and_then(|value| value.as_name().ok())
                    .is_some_and(|name| accepted.contains(&name))
            };
            name_is(b"Subtype", &[b"Type0".as_slice()])
                && name_is(b"Encoding", &[b"Identity-H", b"Identity-V"])
        });
    if is_two_byte {
        2
    } else {
        1
    }
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

#[cfg(test)]
mod positioned_text_tests {
    use super::*;
    use lopdf::{content::Operation, dictionary};

    #[test]
    fn cid_glyph_indices_without_unicode_map_are_not_presented_as_text() {
        let document = Document::with_version("1.7");
        let font = dictionary! { "Type"=>"Font", "Subtype"=>"Type0", "Encoding"=>"Identity-H" };
        let mut cache = FontEncodingCache::default();
        assert!(encoding_for_font(&font, &document, &mut cache).is_none());
        let mut geometry = PageGeometry::default();
        let mut text = TextState::default();
        show_text_bytes(
            &[0, 73, 0, 72, 0, 81],
            None,
            &mut text,
            Matrix::IDENTITY,
            1,
            &mut geometry,
        )
        .unwrap();
        assert!(geometry
            .text_runs
            .iter()
            .all(|run| run.extracted_text.is_empty()));
    }

    #[test]
    fn positioned_fragments_decode_font_maps_and_restore_encoding_after_graphics_state() {
        let mut document = Document::with_version("1.7");
        let mut fonts = Dictionary::new();
        for (name, values) in [("F1", [0x43, 0x41, 0x54]), ("F2", [0x44, 0x4f, 0x47])] {
            let cmap = format!("/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapName /Test def /CMapType 2 def 1 begincodespacerange <00> <FF> endcodespacerange 3 beginbfchar <01> <{:04X}> <02> <{:04X}> <03> <{:04X}> endbfchar endcmap CMapName currentdict /CMap defineresource pop end end", values[0],values[1],values[2]);
            let mapping = document.add_object(Stream::new(Dictionary::new(), cmap.into_bytes()));
            let font = document.add_object(dictionary! { "Type"=>"Font", "Subtype"=>"Type1", "BaseFont"=>"Helvetica", "Encoding"=>"WinAnsiEncoding", "ToUnicode"=>mapping });
            fonts.set(name, font);
        }
        let resources = dictionary! { "Font"=>fonts };
        let show = || {
            Operation::new(
                "Tj",
                vec![Object::String(vec![1, 2, 3], StringFormat::Hexadecimal)],
            )
        };
        let operations = vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new("Td", vec![100.into(), 100.into()]),
            show(),
            Operation::new("q", vec![]),
            Operation::new("Tf", vec!["F2".into(), 12.into()]),
            show(),
            Operation::new("Q", vec![]),
            show(),
            Operation::new("ET", vec![]),
            Operation::new("re", vec![99.into(), 96.into(), 100.into(), 20.into()]),
            Operation::new("f", vec![]),
        ];
        let mut geometry = PageGeometry::default();
        let mut active_forms = HashSet::new();
        let mut budget = GeometryBudget::default();
        let mut cache = FontEncodingCache::default();
        analyze_geometry_operations(
            &document,
            Some(&resources),
            &operations,
            GraphicsState::default(),
            TextState::default(),
            0,
            GeometryTraversal {
                inherited_encoding: None,
                active_forms: &mut active_forms,
                budget: &mut budget,
                geometry: &mut geometry,
                font_cache: &mut cache,
            },
        )
        .unwrap();
        assert_eq!(
            geometry
                .text_runs
                .iter()
                .map(|run| run.extracted_text.as_str())
                .collect::<Vec<_>>(),
            vec!["CAT", "DOG", "CAT"]
        );
        assert_eq!(cache.entries.len(), 2);
        let clashes =
            correlate_redaction_geometry(&geometry.text_runs, &geometry.filled_rectangles, &[])
                .unwrap();
        assert_eq!(clashes.overlay_text_runs.len(), 3);
    }

    fn run_operations(
        resources: Option<&Dictionary>,
        operations: &[Operation],
    ) -> (Document, PageGeometry) {
        let document = Document::with_version("1.7");
        let geometry = run_operations_in(&document, resources, operations);
        (document, geometry)
    }

    fn run_operations_in(
        document: &Document,
        resources: Option<&Dictionary>,
        operations: &[Operation],
    ) -> PageGeometry {
        let mut geometry = PageGeometry::default();
        let mut active_forms = HashSet::new();
        let mut budget = GeometryBudget::default();
        let mut cache = FontEncodingCache::default();
        analyze_geometry_operations(
            document,
            resources,
            operations,
            GraphicsState::default(),
            TextState::default(),
            0,
            GeometryTraversal {
                inherited_encoding: None,
                active_forms: &mut active_forms,
                budget: &mut budget,
                geometry: &mut geometry,
                font_cache: &mut cache,
            },
        )
        .unwrap();
        classify_paint_operations(&mut geometry, Some(LETTER_PAGE_BOX));
        geometry
    }

    const LETTER_PAGE_BOX: Bounds = Bounds {
        min_x: 0.0,
        min_y: 0.0,
        max_x: 612.0,
        max_y: 792.0,
    };

    fn image_document() -> (Document, Dictionary) {
        let mut document = Document::with_version("1.7");
        let image = document.add_object(Stream::new(
            dictionary! { "Type"=>"XObject", "Subtype"=>"Image", "Width"=>1, "Height"=>1 },
            vec![0],
        ));
        let resources = dictionary! { "XObject" => dictionary! { "Im1" => image } };
        (document, resources)
    }

    fn draw_image(a: i64, d: i64, e: i64, f: i64) -> Vec<Operation> {
        vec![
            Operation::new("q", vec![]),
            Operation::new(
                "cm",
                vec![a.into(), 0.into(), 0.into(), d.into(), e.into(), f.into()],
            ),
            Operation::new("Do", vec!["Im1".into()]),
            Operation::new("Q", vec![]),
        ]
    }

    fn text_then(overlay: Vec<Operation>) -> Vec<Operation> {
        let mut operations = vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new("Td", vec![100.into(), 100.into()]),
            Operation::new(
                "Tj",
                vec![Object::String(b"SECRET".to_vec(), StringFormat::Literal)],
            ),
            Operation::new("ET", vec![]),
        ];
        operations.extend(overlay);
        operations
    }

    #[test]
    fn image_stamped_over_live_text_is_an_unclassified_overlay() {
        let (document, resources) = image_document();
        let operations = text_then(draw_image(100, 20, 99, 96));
        let geometry = run_operations_in(&document, Some(&resources), &operations);
        assert_eq!(geometry.unclassified_filled_rectangles, 1);
        assert_eq!(geometry.raster_backdrops, 0);
        assert_eq!(geometry.filled_rectangles.len(), 1);
        assert!(!geometry.filled_rectangles[0].dark_fill);
        assert!(geometry.filled_rectangles[0]
            .bounds
            .covers_text(geometry.text_runs[0].bounds));
    }

    #[test]
    fn full_page_scan_under_an_ocr_layer_is_a_raster_backdrop_not_an_overlay() {
        let (document, resources) = image_document();
        let mut operations = draw_image(612, 792, 0, 0);
        operations.extend(text_then(vec![]));
        let geometry = run_operations_in(&document, Some(&resources), &operations);
        assert_eq!(geometry.raster_backdrops, 1);
        assert_eq!(geometry.unclassified_filled_rectangles, 0);
        assert_eq!(geometry.images_without_text_overlap, 0);
        assert!(geometry.filled_rectangles.is_empty());
        assert_eq!(geometry.text_runs.len(), 1);
    }

    #[test]
    fn full_page_image_drawn_after_text_is_still_an_overlay() {
        let (document, resources) = image_document();
        let operations = text_then(draw_image(612, 792, 0, 0));
        let geometry = run_operations_in(&document, Some(&resources), &operations);
        assert_eq!(geometry.raster_backdrops, 0);
        assert_eq!(geometry.unclassified_filled_rectangles, 1);
    }

    #[test]
    fn image_intersecting_no_earlier_text_is_recorded_without_failing_coverage() {
        let (document, resources) = image_document();
        let operations = text_then(draw_image(50, 20, 400, 700));
        let geometry = run_operations_in(&document, Some(&resources), &operations);
        assert_eq!(geometry.images_without_text_overlap, 1);
        assert_eq!(geometry.unclassified_filled_rectangles, 0);
        assert_eq!(geometry.raster_backdrops, 0);
    }

    #[test]
    fn inline_images_and_shadings_over_earlier_text_leave_the_page_unclassified() {
        let inline = Operation::new(
            "BI",
            vec![Object::Stream(Stream::new(
                dictionary! { "W"=>1, "H"=>1, "BPC"=>8, "CS"=>"G" },
                vec![0],
            ))],
        );
        let (_, geometry) = run_operations(
            None,
            &text_then(vec![
                Operation::new(
                    "cm",
                    vec![
                        100.into(),
                        0.into(),
                        0.into(),
                        20.into(),
                        99.into(),
                        96.into(),
                    ],
                ),
                inline,
                Operation::new("sh", vec!["Sh1".into()]),
            ]),
        );
        assert_eq!(geometry.unclassified_filled_rectangles, 1);
        assert_eq!(geometry.unclassified_shading_fills, 1);
        assert_eq!(geometry.raster_backdrops, 0);
    }

    #[test]
    fn shading_drawn_before_any_text_cannot_hide_it() {
        let mut operations = vec![Operation::new("sh", vec!["Sh1".into()])];
        operations.extend(text_then(vec![]));
        let (_, geometry) = run_operations(None, &operations);
        assert_eq!(geometry.unclassified_shading_fills, 0);
        assert_eq!(geometry.shadings_without_text_overlap, 1);
    }

    #[test]
    fn to_unicode_cmaps_are_decoded_once_per_font_object_and_budgeted() {
        let mut document = Document::with_version("1.7");
        let cmap = b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapName /Test def /CMapType 2 def 1 begincodespacerange <00> <FF> endcodespacerange 1 beginbfchar <01> <0041> endbfchar endcmap CMapName currentdict /CMap defineresource pop end end".to_vec();
        let cmap_length = cmap.len();
        let mapping = document.add_object(Stream::new(Dictionary::new(), cmap));
        let font = document.add_object(dictionary! {
            "Type"=>"Font", "Subtype"=>"Type1", "BaseFont"=>"Helvetica", "ToUnicode"=>mapping
        });
        let resources = dictionary! { "Font" => dictionary! { "F1" => font, "F2" => font } };
        let oversized = document.add_object(Stream::new(
            Dictionary::new(),
            vec![b' '; MAXIMUM_FONT_CMAP_BYTES + 1],
        ));
        let mut cache = FontEncodingCache::default();
        for _ in 0..3 {
            let encodings = resource_font_encodings(&document, Some(&resources), &mut cache);
            assert_eq!(encodings.len(), 2);
        }
        assert_eq!(cache.entries.len(), 1);
        assert_eq!(cache.decoded_cmap_bytes, cmap_length);

        let bomb_font = dictionary! { "Type"=>"Font", "Subtype"=>"Type1", "ToUnicode"=>oversized };
        assert!(encoding_for_font(&bomb_font, &document, &mut cache).is_none());
        assert_eq!(cache.decoded_cmap_bytes, cmap_length);
    }

    #[test]
    fn text_state_set_outside_a_text_object_applies_to_later_text() {
        let string = || Object::String(b"SECRET".to_vec(), StringFormat::Literal);
        let operations = vec![
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new("BT", vec![]),
            Operation::new("Td", vec![100.into(), 100.into()]),
            Operation::new("Tj", vec![string()]),
            Operation::new("ET", vec![]),
            Operation::new("re", vec![99.into(), 96.into(), 100.into(), 20.into()]),
            Operation::new("f", vec![]),
        ];
        let (_, geometry) = run_operations(None, &operations);
        let clashes =
            correlate_redaction_geometry(&geometry.text_runs, &geometry.filled_rectangles, &[])
                .unwrap();
        assert_eq!(geometry.text_runs.len(), 1);
        assert_eq!(clashes.overlay_text_runs.len(), 1);
    }

    #[test]
    fn text_shown_outside_a_text_object_makes_coverage_incomplete() {
        let operations = vec![Operation::new(
            "Tj",
            vec![Object::String(b"X".to_vec(), StringFormat::Literal)],
        )];
        let (_, geometry) = run_operations(None, &operations);
        assert_eq!(geometry.malformed_operations, 1);
    }

    #[test]
    fn identity_h_codes_advance_two_bytes_per_glyph() {
        let resources = dictionary! {
            "Font" => dictionary! {
                "F1" => dictionary! { "Type"=>"Font", "Subtype"=>"Type0", "Encoding"=>"Identity-H" }
            }
        };
        let operations = vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new(
                "Tj",
                vec![Object::String(vec![0, 1, 0, 2], StringFormat::Hexadecimal)],
            ),
            Operation::new("ET", vec![]),
        ];
        let (_, geometry) = run_operations(Some(&resources), &operations);
        assert_eq!(geometry.text_runs.len(), 1);
        assert_eq!(geometry.text_runs[0].bounds.width(), 12.0);
    }
}
