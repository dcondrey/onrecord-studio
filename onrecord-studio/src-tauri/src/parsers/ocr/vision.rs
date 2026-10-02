//! Apple Vision text recognition with PDFKit page rendering (native/VisionOcr.swift). Runs inside
//! the sandbox and needs nothing installed, unlike the Tesseract/Poppler subprocess path.

use std::ffi::{c_char, c_void, CStr, CString};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::ptr;
use std::time::Instant;

use super::{append_page_text, OcrCapability, MAXIMUM_REPORTED_LANGUAGE_PACKS, MAX_PDF_OCR_PAGES};
use crate::error::AppError;

/// Ceiling on one page's bitmap. A page rendered at 300 DPI (or an image decoded at native size)
/// beyond it is downsampled to fit, so memory per page stays bounded whatever the page geometry.
/// Pages are rendered and released one at a time.
const MAX_OCR_PAGE_PIXELS: isize = 36 * 1024 * 1024;

const STATUS_OK: i32 = 0;
const STATUS_TIMEOUT: i32 = 1;
const STATUS_INVALID_DOCUMENT: i32 = 2;

unsafe extern "C" {
    fn onrecord_vision_open(
        path: *const c_char,
        is_pdf: bool,
        maximum_page_pixels: isize,
        status: *mut i32,
        error: *mut *mut c_char,
    ) -> *mut c_void;
    fn onrecord_vision_page_count(handle: *mut c_void) -> isize;
    fn onrecord_vision_recognize_page(
        handle: *mut c_void,
        index: isize,
        timeout_seconds: f64,
        status: *mut i32,
        error: *mut *mut c_char,
    ) -> *mut c_char;
    fn onrecord_vision_close(handle: *mut c_void);
    fn onrecord_vision_describe() -> *mut c_char;
    fn onrecord_vision_free_string(value: *mut c_char);
}

/// Takes ownership of a string the bridge allocated with `strdup`.
fn take_native_string(pointer: *mut c_char) -> Option<String> {
    if pointer.is_null() {
        return None;
    }
    // SAFETY: the bridge returns NUL-terminated strings from `strdup`, freed exactly once here.
    let value = unsafe { CStr::from_ptr(pointer) }
        .to_string_lossy()
        .into_owned();
    unsafe { onrecord_vision_free_string(pointer) };
    Some(value)
}

fn native_failure(path: &Path, status: i32, error: *mut c_char) -> AppError {
    let message = take_native_string(error)
        .unwrap_or_else(|| "OCR_ENGINE_FAILED: Vision reported no detail".to_string());
    let message = format!("{message} ('{}')", path.display());
    if status == STATUS_TIMEOUT || status == STATUS_INVALID_DOCUMENT {
        AppError::DataParsing(message)
    } else {
        AppError::Internal(message)
    }
}

struct VisionDocument(*mut c_void);

impl Drop for VisionDocument {
    fn drop(&mut self) {
        // SAFETY: the handle came from a successful `onrecord_vision_open` and is released once.
        unsafe { onrecord_vision_close(self.0) };
    }
}

pub(super) fn recognize(path: &Path, is_pdf: bool, deadline: Instant) -> Result<String, AppError> {
    let native_path = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        AppError::DataParsing(format!(
            "OCR_INPUT_INVALID: '{}' contains a NUL byte",
            path.display()
        ))
    })?;
    let mut status = STATUS_OK;
    let mut error = ptr::null_mut();
    // SAFETY: `native_path` outlives the call; `status` and `error` are valid out-pointers.
    let handle = unsafe {
        onrecord_vision_open(
            native_path.as_ptr(),
            is_pdf,
            MAX_OCR_PAGE_PIXELS,
            &mut status,
            &mut error,
        )
    };
    if handle.is_null() {
        return Err(native_failure(path, status, error));
    }
    let document = VisionDocument(handle);
    // SAFETY: `document` holds a live handle.
    let page_count =
        usize::try_from(unsafe { onrecord_vision_page_count(document.0) }).unwrap_or(0);
    if page_count == 0 {
        return Err(AppError::DataParsing(format!(
            "OCR_PAGE_COUNT_INVALID: '{}' contains no pages",
            path.display()
        )));
    }
    if page_count > MAX_PDF_OCR_PAGES {
        return Err(AppError::DataParsing(format!(
            "OCR_PAGE_COUNT_EXCEEDED: '{}' contains {page_count} pages; limit is {MAX_PDF_OCR_PAGES}",
            path.display(),
        )));
    }

    let mut text = String::new();
    for index in 0..page_count {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(AppError::DataParsing(format!(
                "OCR_ENGINE_TIMEOUT: processing deadline expired for '{}'",
                path.display()
            )));
        }
        let mut status = STATUS_OK;
        let mut error = ptr::null_mut();
        // SAFETY: `index` < page count (bounded by MAX_PDF_OCR_PAGES, so it fits isize); the
        // handle is live and the out-pointers are valid.
        let page = unsafe {
            onrecord_vision_recognize_page(
                document.0,
                index as isize,
                remaining.as_secs_f64(),
                &mut status,
                &mut error,
            )
        };
        let Some(page_text) = take_native_string(page) else {
            return Err(native_failure(path, status, error));
        };
        append_page_text(&mut text, page_text.trim())?;
    }
    if text.trim().is_empty() {
        return Err(AppError::DataParsing(format!(
            "OCR_OUTPUT_EMPTY: '{}' produced no searchable text",
            path.display()
        )));
    }
    Ok(text)
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct VisionDescription {
    revision: u64,
    os_version: String,
    languages: Vec<String>,
    language_error: Option<String>,
}

pub(super) fn capability() -> OcrCapability {
    // SAFETY: no arguments; the returned string is owned and freed by `take_native_string`.
    let description = take_native_string(unsafe { onrecord_vision_describe() })
        .ok_or_else(|| "Vision returned no capability description".to_string())
        .and_then(|json| {
            serde_json::from_str::<VisionDescription>(&json)
                .map_err(|error| format!("Vision capability description was unreadable: {error}"))
        });
    let description = match description {
        Ok(description) => description,
        Err(diagnostic) => {
            return OcrCapability {
                engine: "Apple Vision".to_string(),
                engine_available: false,
                engine_version: None,
                language_packs: Vec::new(),
                language_probe_complete: false,
                pdf_renderer: Some("PDFKit".to_string()),
                pdf_renderer_available: false,
                pdf_renderer_version: None,
                image_ocr_available: false,
                pdf_ocr_available: false,
                diagnostics: vec![diagnostic],
            }
        }
    };
    let mut languages = description.languages;
    let mut diagnostics = Vec::new();
    diagnostics.extend(description.language_error);
    let mut language_probe_complete = diagnostics.is_empty();
    if languages.len() > MAXIMUM_REPORTED_LANGUAGE_PACKS {
        languages.truncate(MAXIMUM_REPORTED_LANGUAGE_PACKS);
        language_probe_complete = false;
        diagnostics.push(format!(
            "OCR language reporting exceeded {MAXIMUM_REPORTED_LANGUAGE_PACKS} entries"
        ));
    }
    OcrCapability {
        engine: "Apple Vision".to_string(),
        engine_available: true,
        engine_version: Some(format!(
            "text recognition revision {}, macOS {}",
            description.revision, description.os_version
        )),
        language_packs: languages,
        language_probe_complete,
        pdf_renderer: Some("PDFKit".to_string()),
        pdf_renderer_available: true,
        pdf_renderer_version: Some(format!("macOS {}", description.os_version)),
        image_ocr_available: true,
        pdf_ocr_available: true,
        diagnostics,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use super::recognize;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/ocr")
            .join(name)
    }

    // The first Vision request in a process can spend tens of seconds loading models.
    const DEADLINE: Duration = Duration::from_secs(300);

    #[test]
    fn scanned_image_is_read_by_vision() {
        let text = recognize(
            &fixture("scanned-ledger.png"),
            false,
            Instant::now() + DEADLINE,
        )
        .expect("Vision OCR of the image fixture");
        assert!(text.contains("QUARTERLY LEDGER"), "{text}");
        assert!(text.contains("4471"), "{text}");
    }

    /// The fixture has no text layer, so its text can only come from a PDFKit render read by
    /// Vision. (It carries /Rotate 90, but Vision also reads sideways text, so this does not prove
    /// rotation is honored.)
    #[test]
    fn image_only_pdf_is_rendered_by_pdfkit_and_read_by_vision() {
        let path = fixture("scanned-ledger-rotated.pdf");
        assert!(crate::parsers::extract_text_layers(&path).is_err());
        let text = recognize(&path, true, Instant::now() + DEADLINE)
            .expect("Vision OCR of the image-only PDF fixture");
        assert!(text.contains("QUARTERLY LEDGER"), "{text}");
        assert!(text.contains("Invoice 4471 approved"), "{text}");
    }

    #[test]
    fn expired_deadline_stops_before_recognition() {
        let error = recognize(&fixture("scanned-ledger.png"), false, Instant::now())
            .expect_err("an expired deadline must not run OCR");
        assert!(error.to_string().contains("OCR_ENGINE_TIMEOUT"), "{error}");
    }
}
