use std::path::Path;

use calamine::{open_workbook_auto, Data, Reader};
use chrono::{DateTime, NaiveDate, NaiveDateTime, SecondsFormat, Utc};

use crate::error::AppError;
use crate::normalization::normalize_delimited_field as sanitize_text_field;
use crate::parsers::source::inspect_regular_file;

const MAX_WORKBOOK_BYTES: u64 = 512 * 1024 * 1024;
const MAX_WORKSHEETS: usize = 2_000;
const MAX_CELLS: usize = 10_000_000;
const MAX_EXTRACTED_BYTES: usize = 128 * 1024 * 1024;

fn normalize_float(value: f64) -> String {
    if value == 0.0 {
        return "0".to_string();
    }

    if value.is_finite()
        && value.fract() == 0.0
        && value >= i64::MIN as f64
        && value <= i64::MAX as f64
    {
        return format!("{value:.0}");
    }

    value.to_string().replace('E', "e")
}

fn naive_datetime_to_rfc3339(value: NaiveDateTime) -> String {
    DateTime::<Utc>::from_naive_utc_and_offset(value, Utc)
        .to_rfc3339_opts(SecondsFormat::AutoSi, true)
}

fn normalize_iso_datetime(value: &str) -> Result<String, AppError> {
    let sanitized = sanitize_text_field(value);

    if let Ok(parsed) = DateTime::parse_from_rfc3339(&sanitized) {
        return Ok(parsed.to_rfc3339_opts(SecondsFormat::AutoSi, true));
    }

    if let Ok(parsed) = NaiveDateTime::parse_from_str(&sanitized, "%Y-%m-%dT%H:%M:%S%.f") {
        return Ok(naive_datetime_to_rfc3339(parsed));
    }

    if let Ok(parsed) = NaiveDate::parse_from_str(&sanitized, "%Y-%m-%d") {
        let midnight = parsed.and_hms_opt(0, 0, 0).ok_or_else(|| {
            AppError::DataParsing(format!(
                "EXCEL_DATETIME_INVALID: could not construct midnight for '{sanitized}'"
            ))
        })?;
        return Ok(naive_datetime_to_rfc3339(midnight));
    }

    Err(AppError::DataParsing(format!(
        "EXCEL_DATETIME_INVALID: '{sanitized}' is not a supported ISO-8601 date or date-time"
    )))
}

fn cell_to_searchable_text(cell: &Data) -> Result<String, AppError> {
    match cell {
        Data::Empty => Ok(String::new()),
        Data::String(value) => Ok(sanitize_text_field(value)),
        Data::Float(value) => Ok(normalize_float(*value)),
        Data::Int(value) => Ok(value.to_string()),
        Data::Bool(value) => Ok(value.to_string()),
        Data::Error(value) => Ok(format!("[ERROR:{value}]")),
        Data::DateTime(value) => {
            let datetime = value.as_datetime().ok_or_else(|| {
                AppError::DataParsing(format!(
                    "EXCEL_DATETIME_INVALID: serial value {} cannot be represented as a date-time",
                    value.as_f64()
                ))
            })?;
            Ok(naive_datetime_to_rfc3339(datetime))
        }
        Data::DateTimeIso(value) => normalize_iso_datetime(value),
        Data::DurationIso(value) => Ok(sanitize_text_field(value)),
    }
}

pub fn extract_spreadsheet_layers<P: AsRef<Path>>(file_path: P) -> Result<String, AppError> {
    let path = file_path.as_ref();
    inspect_regular_file(path, "SPREADSHEET", MAX_WORKBOOK_BYTES)?;

    let mut workbook = open_workbook_auto(path).map_err(|error| {
        AppError::DataParsing(format!(
            "SPREADSHEET_LOAD_FAILED: failed to open '{}': {}",
            path.display(),
            error
        ))
    })?;

    let sheet_names = workbook.sheet_names();
    if sheet_names.len() > MAX_WORKSHEETS {
        return Err(AppError::DataParsing(format!(
            "SPREADSHEET_SHEET_COUNT_EXCEEDED: '{}' contains {} sheets; limit is {MAX_WORKSHEETS}",
            path.display(),
            sheet_names.len()
        )));
    }
    let mut sheet_blocks = Vec::with_capacity(sheet_names.len());
    let mut cell_count = 0_usize;
    let mut extracted_bytes = 0_usize;

    for sheet_name in sheet_names {
        // `worksheet_range` allocates a dense matrix spanning the sheet's bounding box before it
        // returns, so the `MAX_CELLS` check below runs far too late. A two-cell sheet holding only
        // A1 and XFD1048576 spans 17,179,869,184 cells; the allocation aborts the process, and an
        // abort does not unwind, so `spawn_blocking` cannot contain it.
        //
        // The declared `<dimension>` attribute cannot be trusted for this -- it is advisory, often
        // wrong, and absent entirely in a hand-built file -- so measure the real extent by
        // streaming the cells first. The streaming reader allocates nothing per cell.
        if let calamine::Sheets::Xlsx(xlsx) = &mut workbook {
            let extent = measure_xlsx_extent(xlsx, &sheet_name).map_err(|error| {
                AppError::DataParsing(format!(
                    "EXCEL_SHEET_READ_FAILED: failed to measure sheet '{}' in '{}': {error}",
                    sanitize_text_field(&sheet_name),
                    path.display()
                ))
            })?;
            if extent > MAX_CELLS as u64 {
                return Err(AppError::DataParsing(format!(
                    "SPREADSHEET_CELL_COUNT_EXCEEDED: sheet '{}' in '{}' spans {extent} cells; \
                     limit is {MAX_CELLS}",
                    sanitize_text_field(&sheet_name),
                    path.display()
                )));
            }
        }
        // NOTE(parsers): `.xls`, `.xlsb`, and `.ods` build the same dense matrix but expose no
        // streaming reader. Their formats cap the grid at 65,536 x 256, so the worst case is
        // bounded near 536 MB rather than unbounded, and `MAX_CELLS` rejects it after allocation.
        let range = workbook.worksheet_range(&sheet_name).map_err(|error| {
            AppError::DataParsing(format!(
                "EXCEL_SHEET_READ_FAILED: failed to read sheet '{}' from '{}': {}",
                sheet_name,
                path.display(),
                error
            ))
        })?;

        let mut block = format!("--- Sheet: {} ---", sanitize_text_field(&sheet_name));
        for row in range.rows() {
            cell_count = cell_count.checked_add(row.len()).ok_or_else(|| {
                AppError::DataParsing(
                    "SPREADSHEET_CELL_COUNT_OVERFLOW: cell count overflow".to_string(),
                )
            })?;
            if cell_count > MAX_CELLS {
                return Err(AppError::DataParsing(format!(
                    "SPREADSHEET_CELL_COUNT_EXCEEDED: '{}' exceeds {MAX_CELLS} cells",
                    path.display()
                )));
            }
            let fields = row
                .iter()
                .map(cell_to_searchable_text)
                .collect::<Result<Vec<_>, AppError>>()?;

            if fields.iter().any(|field| !field.is_empty()) {
                let row_text = fields.join(",");
                let next_size = extracted_bytes
                    .checked_add(block.len())
                    .and_then(|size| size.checked_add(row_text.len()))
                    .and_then(|size| size.checked_add(1))
                    .ok_or_else(|| {
                        AppError::DataParsing(
                            "SPREADSHEET_TEXT_SIZE_OVERFLOW: extracted size overflow".to_string(),
                        )
                    })?;
                if next_size > MAX_EXTRACTED_BYTES {
                    return Err(AppError::DataParsing(format!(
                        "SPREADSHEET_TEXT_SIZE_EXCEEDED: '{}' exceeds {MAX_EXTRACTED_BYTES} extracted bytes",
                        path.display()
                    )));
                }
                block.push('\n');
                block.push_str(&row_text);
            }
        }

        extracted_bytes = extracted_bytes
            .checked_add(block.len())
            .and_then(|size| size.checked_add(2))
            .ok_or_else(|| {
                AppError::DataParsing(
                    "SPREADSHEET_TEXT_SIZE_OVERFLOW: extracted size overflow".to_string(),
                )
            })?;
        if extracted_bytes > MAX_EXTRACTED_BYTES {
            return Err(AppError::DataParsing(format!(
                "SPREADSHEET_TEXT_SIZE_EXCEEDED: '{}' exceeds {MAX_EXTRACTED_BYTES} extracted bytes",
                path.display()
            )));
        }
        sheet_blocks.push(block);
    }

    Ok(sheet_blocks.join("\n\n"))
}

/// Returns the number of cells the sheet's true bounding box spans, without materializing it.
///
/// Measured by streaming the cell records rather than reading the declared `<dimension>`, because
/// that attribute is advisory: real government workbooks frequently disagree with it, and a
/// hand-crafted file can omit it entirely.
pub(crate) fn measure_xlsx_extent<RS>(
    workbook: &mut calamine::Xlsx<RS>,
    sheet_name: &str,
) -> Result<u64, calamine::XlsxError>
where
    RS: std::io::Read + std::io::Seek,
{
    let mut reader = workbook.worksheet_cells_reader(sheet_name)?;
    let mut bounds: Option<(u32, u32, u32, u32)> = None;
    while let Some(cell) = reader.next_cell()? {
        let (row, column) = cell.get_position();
        bounds = Some(match bounds {
            None => (row, column, row, column),
            Some((min_row, min_column, max_row, max_column)) => (
                min_row.min(row),
                min_column.min(column),
                max_row.max(row),
                max_column.max(column),
            ),
        });
    }
    Ok(match bounds {
        None => 0,
        Some((min_row, min_column, max_row, max_column)) => {
            u64::from(max_row - min_row + 1) * u64::from(max_column - min_column + 1)
        }
    })
}

#[cfg(test)]
mod sparse_bounding_box_tests {
    use super::*;
    use std::io::Write;

    /// Writes a minimal but valid `.xlsx` whose sheet declares only two cells, A1 and the
    /// bottom-right corner of the grid.
    fn sparse_corner_workbook(path: &Path) {
        let mut writer = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        let options: zip::write::FileOptions<()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let parts: [(&str, &str); 5] = [
            (
                "[Content_Types].xml",
                r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#,
            ),
            (
                "_rels/.rels",
                r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
            ),
            (
                "xl/workbook.xml",
                r#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
            ),
            (
                "xl/_rels/workbook.xml.rels",
                r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#,
            ),
            (
                "xl/worksheets/sheet1.xml",
                r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>a</t></is></c></row><row r="1048576"><c r="XFD1048576" t="inlineStr"><is><t>b</t></is></c></row></sheetData></worksheet>"#,
            ),
        ];
        for (name, body) in parts {
            writer.start_file(name, options).unwrap();
            writer.write_all(body.as_bytes()).unwrap();
        }
        writer.finish().unwrap();
    }

    /// A sheet whose two populated cells sit at opposite corners of the grid spans 17,179,869,184
    /// cells. calamine builds a dense matrix of that size inside `worksheet_range`, so the limit
    /// has to be enforced on the bounding box beforehand: the allocation failure is an abort, not
    /// a panic, so it kills the whole application rather than failing one document.
    #[test]
    fn rejects_a_sparse_sheet_whose_bounding_box_would_allocate_a_dense_matrix() {
        let directory = tempfile::tempdir().unwrap();
        let workbook = directory.path().join("sparse.xlsx");
        sparse_corner_workbook(&workbook);

        assert!(
            std::fs::metadata(&workbook).unwrap().len() < 8_192,
            "the attack file is tiny; size-based limits cannot catch it"
        );

        let error = extract_spreadsheet_layers(&workbook)
            .expect_err("a sheet spanning 17 billion cells must be rejected");
        let message = error.to_string();
        assert!(
            message.contains("SPREADSHEET_CELL_COUNT_EXCEEDED"),
            "unexpected error: {message}"
        );
        assert!(
            message.contains("17179869184"),
            "the bounding box size should be reported: {message}"
        );
    }
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::io::Write;
    use std::path::Path;

    use calamine::{Data, ExcelDateTime, ExcelDateTimeType};
    use tempfile::tempdir;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    use super::{
        cell_to_searchable_text, extract_spreadsheet_layers, normalize_float, sanitize_text_field,
    };
    use crate::error::AppError;

    fn write_xlsx_part(
        archive: &mut ZipWriter<File>,
        name: &str,
        contents: &str,
    ) -> Result<(), AppError> {
        archive
            .start_file(name, SimpleFileOptions::default())
            .map_err(|error| {
                AppError::Internal(format!("EXCEL_TEST_ARCHIVE_ENTRY_FAILED: {error}"))
            })?;
        archive.write_all(contents.as_bytes()).map_err(|error| {
            AppError::Internal(format!("EXCEL_TEST_ARCHIVE_WRITE_FAILED: {error}"))
        })
    }

    fn write_multi_sheet_xlsx(path: &Path) -> Result<(), AppError> {
        let file = File::create(path).map_err(|error| {
            AppError::Internal(format!(
                "EXCEL_TEST_FIXTURE_CREATE_FAILED: failed to create '{}': {}",
                path.display(),
                error
            ))
        })?;
        let mut archive = ZipWriter::new(file);
        write_xlsx_part(
            &mut archive,
            "[Content_Types].xml",
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/></Types>"#,
        )?;
        write_xlsx_part(
            &mut archive,
            "_rels/.rels",
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
        )?;
        write_xlsx_part(
            &mut archive,
            "xl/workbook.xml",
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Intake" sheetId="1" r:id="rId1"/><sheet name="Review" sheetId="2" r:id="rId2"/></sheets></workbook>"#,
        )?;
        write_xlsx_part(
            &mut archive,
            "xl/_rels/workbook.xml.rels",
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#,
        )?;
        write_xlsx_part(
            &mut archive,
            "xl/styles.xml",
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts count="1"><font><sz val="11"/><name val="Calibri"/></font></fonts><fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills><borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="2"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="14" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/></cellXfs></styleSheet>"#,
        )?;
        write_xlsx_part(
            &mut archive,
            "xl/worksheets/sheet1.xml",
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>District</t></is></c><c r="B1" t="inlineStr"><is><t>Count</t></is></c><c r="C1" t="inlineStr"><is><t>Date</t></is></c></row><row r="2"><c r="A2" t="inlineStr"><is><t>North,&#13;&#10; District</t></is></c><c r="B2"><v>42</v></c><c r="C2" s="1"><v>25569</v></c></row></sheetData></worksheet>"#,
        )?;
        write_xlsx_part(
            &mut archive,
            "xl/worksheets/sheet2.xml",
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>Finding</t></is></c></row><row r="2"><c r="A2" t="inlineStr"><is><t>Verified</t></is></c></row></sheetData></worksheet>"#,
        )?;
        archive.finish().map_err(|error| {
            AppError::Internal(format!("EXCEL_TEST_ARCHIVE_FINALIZE_FAILED: {error}"))
        })?;
        Ok(())
    }

    #[test]
    fn text_fields_remove_csv_and_line_delimiters() {
        assert_eq!(sanitize_text_field("North,\r\n District"), "North District");
    }

    #[test]
    fn numeric_values_are_normalized() -> Result<(), AppError> {
        assert_eq!(normalize_float(-0.0), "0");
        assert_eq!(normalize_float(42.0), "42");
        assert_eq!(normalize_float(42.25), "42.25");
        assert_eq!(cell_to_searchable_text(&Data::Int(-7))?, "-7");
        Ok(())
    }

    #[test]
    fn excel_datetime_is_rendered_as_rfc3339() -> Result<(), AppError> {
        let value = Data::DateTime(ExcelDateTime::new(
            25_569.0,
            ExcelDateTimeType::DateTime,
            false,
        ));
        assert_eq!(cell_to_searchable_text(&value)?, "1970-01-01T00:00:00Z");
        Ok(())
    }

    #[test]
    fn extracts_real_multi_sheet_workbook_as_flat_csv() -> Result<(), AppError> {
        let directory = tempdir().map_err(|error| {
            AppError::Internal(format!("EXCEL_TEST_TEMP_DIRECTORY_FAILED: {error}"))
        })?;
        let path = directory.path().join("records.xlsx");
        write_multi_sheet_xlsx(&path)?;

        let extracted = extract_spreadsheet_layers(&path)?;
        assert!(extracted.contains("--- Sheet: Intake ---"));
        assert!(extracted.contains("District,Count,Date"));
        assert!(extracted.contains("North District,42,1970-01-01T00:00:00Z"));
        assert!(extracted.contains("--- Sheet: Review ---"));
        assert!(extracted.contains("Finding\nVerified"));
        assert!(!extracted.contains('\r'));
        Ok(())
    }
}
