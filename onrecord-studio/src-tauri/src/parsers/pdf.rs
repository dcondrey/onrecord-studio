use std::path::Path;

use lopdf::{Document, LoadOptions};

use crate::error::{AppError, OcrRequirementReason};
use crate::parsers::source::inspect_regular_file;

pub const MINIMUM_CHARACTER_LAYER_DENSITY: f64 = 0.15;
const MAX_PDF_BYTES: u64 = 256 * 1024 * 1024;
const MAX_PDF_PAGES: usize = 25_000;
const MAX_EXTRACTED_BYTES: usize = 128 * 1024 * 1024;
const MAX_DECOMPRESSED_STREAM_BYTES: usize = 16 * 1024 * 1024;

pub fn character_layer_density(text: &str) -> f64 {
    let total_character_count = text.chars().count();
    if total_character_count == 0 {
        return 0.0;
    }

    let alphanumeric_count = text
        .chars()
        .filter(|character| character.is_alphanumeric())
        .count();
    alphanumeric_count as f64 / total_character_count as f64
}

pub fn requires_ocr_reprocessing(text: &str) -> bool {
    text.is_empty() || character_layer_density(text) < MINIMUM_CHARACTER_LAYER_DENSITY
}

pub fn extract_text_layers<P: AsRef<Path>>(file_path: P) -> Result<String, AppError> {
    let path = file_path.as_ref();
    inspect_regular_file(path, "PDF", MAX_PDF_BYTES)?;
    // lopdf indexes the complete input before page access, so the regular-file gate above caps its
    // resident source buffer. Its bounded load option separately caps eager object/xref stream
    // inflation, and the page loop below uses the bounded extractor so content and font streams
    // cannot bypass that protection during lazy page decoding.
    let document = Document::load_with_options(
        path,
        LoadOptions::with_max_decompressed_size(MAX_DECOMPRESSED_STREAM_BYTES),
    )
    .map_err(|error| {
        AppError::DataParsing(format!(
            "PDF_LOAD_FAILED: failed to load '{}': {}",
            path.display(),
            error
        ))
    })?;

    let pages = document.get_pages();
    if pages.len() > MAX_PDF_PAGES {
        return Err(AppError::DataParsing(format!(
            "PDF_PAGE_COUNT_EXCEEDED: '{}' contains {} pages; limit is {MAX_PDF_PAGES}",
            path.display(),
            pages.len()
        )));
    }
    let mut page_layers = Vec::with_capacity(pages.len());
    let mut alphanumeric_count = 0usize;
    let mut total_character_count = 0usize;
    let mut extracted_bytes = 0_usize;

    for page_number in pages.keys().copied() {
        let page_text = document
            .extract_text_with_limit(&[page_number], MAX_DECOMPRESSED_STREAM_BYTES)
            .map_err(|error| {
                AppError::DataParsing(format!(
                    "PDF_TEXT_EXTRACTION_FAILED: failed to extract page {} from '{}': {}",
                    page_number,
                    path.display(),
                    error
                ))
            })?;

        let page_character_count = page_text.chars().count();
        extracted_bytes = extracted_bytes
            .checked_add(page_text.len())
            .and_then(|size| size.checked_add(usize::from(!page_layers.is_empty())))
            .ok_or_else(|| {
                AppError::DataParsing(
                    "PDF_TEXT_SIZE_OVERFLOW: extracted byte count overflow".to_string(),
                )
            })?;
        if extracted_bytes > MAX_EXTRACTED_BYTES {
            return Err(AppError::DataParsing(format!(
                "PDF_TEXT_SIZE_EXCEEDED: '{}' exceeds {MAX_EXTRACTED_BYTES} extracted bytes",
                path.display()
            )));
        }
        total_character_count = total_character_count
            .checked_add(page_character_count)
            .ok_or_else(|| {
                AppError::DataParsing(
                    "PDF_TEXT_SIZE_OVERFLOW: extracted character count overflow".to_string(),
                )
            })?;
        alphanumeric_count = alphanumeric_count
            .checked_add(
                page_text
                    .chars()
                    .filter(|character| character.is_alphanumeric())
                    .count(),
            )
            .ok_or_else(|| {
                AppError::DataParsing(
                    "PDF_TEXT_SIZE_OVERFLOW: alphanumeric character count overflow".to_string(),
                )
            })?;
        page_layers.push(page_text);
    }

    if total_character_count == 0 {
        return Err(AppError::OcrRequired {
            path: path.display().to_string(),
            reason: OcrRequirementReason::EmptyCharacterLayer,
            density: 0.0,
            minimum_density: MINIMUM_CHARACTER_LAYER_DENSITY,
        });
    }

    let density = alphanumeric_count as f64 / total_character_count as f64;
    if density < MINIMUM_CHARACTER_LAYER_DENSITY {
        return Err(AppError::OcrRequired {
            path: path.display().to_string(),
            reason: OcrRequirementReason::LowCharacterDensity,
            density,
            minimum_density: MINIMUM_CHARACTER_LAYER_DENSITY,
        });
    }

    Ok(page_layers.join("\n"))
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::path::Path;

    use flate2::write::ZlibEncoder;
    use flate2::Compression;
    use lopdf::content::{Content, Operation};
    use lopdf::{dictionary, Document, Object, Stream};
    use tempfile::tempdir;

    use super::{
        character_layer_density, extract_text_layers, requires_ocr_reprocessing,
        MAX_DECOMPRESSED_STREAM_BYTES, MINIMUM_CHARACTER_LAYER_DENSITY,
    };
    use crate::error::AppError;

    fn write_text_pdf(path: &Path, page_texts: &[&str]) -> Result<(), AppError> {
        let mut document = Document::with_version("1.5");
        let pages_id = document.new_object_id();
        let font_id = document.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Courier",
        });
        let resources_id = document.add_object(dictionary! {
            "Font" => dictionary! {
                "F1" => font_id,
            },
        });
        let mut page_ids = Vec::with_capacity(page_texts.len());

        for page_text in page_texts {
            let content = Content {
                operations: vec![
                    Operation::new("BT", Vec::new()),
                    Operation::new("Tf", vec!["F1".into(), 12.into()]),
                    Operation::new("Td", vec![72.into(), 720.into()]),
                    Operation::new("Tj", vec![Object::string_literal(*page_text)]),
                    Operation::new("ET", Vec::new()),
                ],
            };
            let encoded = content.encode().map_err(|error| {
                AppError::DataParsing(format!("PDF_TEST_CONTENT_ENCODING_FAILED: {error}"))
            })?;
            let content_id = document.add_object(Stream::new(dictionary! {}, encoded));
            let page_id = document.add_object(dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "Contents" => content_id,
            });
            page_ids.push(page_id.into());
        }

        document.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => page_ids,
                "Count" => page_texts.len() as i64,
                "Resources" => resources_id,
                "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            }),
        );
        let catalog_id = document.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        document.trailer.set("Root", catalog_id);
        document.compress();
        document.save(path).map_err(|error| {
            AppError::Internal(format!(
                "PDF_TEST_FIXTURE_WRITE_FAILED: failed to write '{}': {}",
                path.display(),
                error
            ))
        })?;
        Ok(())
    }

    fn write_page_stream_bomb(path: &Path) -> Result<(), AppError> {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::best());
        encoder
            .write_all(&vec![b' '; MAX_DECOMPRESSED_STREAM_BYTES + 1])
            .map_err(|error| AppError::Internal(format!("PDF_TEST_COMPRESSION_FAILED: {error}")))?;
        let compressed = encoder
            .finish()
            .map_err(|error| AppError::Internal(format!("PDF_TEST_COMPRESSION_FAILED: {error}")))?;
        let mut document = Document::with_version("1.5");
        let pages_id = document.new_object_id();
        let content_id = document.add_object(Stream::new(
            dictionary! { "Filter" => "FlateDecode" },
            compressed,
        ));
        let page_id = document.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
        });
        document.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![Object::Reference(page_id)],
                "Count" => 1,
            }),
        );
        let catalog_id = document.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        document.trailer.set("Root", catalog_id);
        document.save(path).map_err(|error| {
            AppError::Internal(format!("PDF_TEST_FIXTURE_WRITE_FAILED: {error}"))
        })?;
        Ok(())
    }

    #[test]
    fn density_uses_unicode_character_counts() {
        let text = "Aé9-------";
        assert!((character_layer_density(text) - 0.3).abs() < f64::EPSILON);
    }

    #[test]
    fn empty_and_low_density_layers_require_ocr() {
        assert!(requires_ocr_reprocessing(""));
        assert!(requires_ocr_reprocessing("A---------"));
    }

    #[test]
    fn threshold_density_is_accepted() {
        let text = "ABC-----------------";
        assert_eq!(
            character_layer_density(text),
            MINIMUM_CHARACTER_LAYER_DENSITY
        );
        assert!(!requires_ocr_reprocessing(text));
    }

    #[test]
    fn extracts_real_multi_page_pdf_in_page_order() -> Result<(), AppError> {
        let directory = tempdir().map_err(|error| {
            AppError::Internal(format!("PDF_TEST_TEMP_DIRECTORY_FAILED: {error}"))
        })?;
        let path = directory.path().join("evidence.pdf");
        write_text_pdf(&path, &["First page evidence", "Second page findings"])?;

        let extracted = extract_text_layers(&path)?;
        let first_page = extracted
            .find("First page evidence")
            .ok_or_else(|| AppError::DataParsing("PDF_TEST_FIRST_PAGE_MISSING".to_string()))?;
        let second_page = extracted
            .find("Second page findings")
            .ok_or_else(|| AppError::DataParsing("PDF_TEST_SECOND_PAGE_MISSING".to_string()))?;
        assert!(first_page < second_page);
        Ok(())
    }

    #[test]
    fn rejects_page_stream_decompression_bombs() -> Result<(), AppError> {
        let directory = tempdir().map_err(|error| {
            AppError::Internal(format!("PDF_TEST_TEMP_DIRECTORY_FAILED: {error}"))
        })?;
        let path = directory.path().join("bomb.pdf");
        write_page_stream_bomb(&path)?;
        let error = extract_text_layers(&path).expect_err("the page stream must be bounded");
        assert!(matches!(
            error,
            AppError::DataParsing(message) if message.contains("PDF_TEXT_EXTRACTION_FAILED")
        ));
        Ok(())
    }
}
