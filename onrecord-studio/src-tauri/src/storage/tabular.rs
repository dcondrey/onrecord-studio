//! Bounded, loss-conscious preparation for analytical snapshots. Original bytes are hashed by
//! the caller; conversion is only a projection. Suppression markers and identifiers stay text.
use crate::error::AppError;
use calamine::{Data, Reader};
use serde_json::Value;
use std::collections::BTreeSet;
use std::io::Cursor;

pub(crate) const MAX_SOURCE_BYTES: u64 = 96 * 1024 * 1024;
const MAX_ROWS: usize = 750_000;
const MAX_COLUMNS: usize = 512;
const MAX_CELLS: usize = 6_000_000;
/// Delimited text is streamed, but DuckDB holds an import's rows until its transaction commits:
/// measured peaks were 50-80 bytes per cell (CFS 2025, 6.5M cells, 379 MB; Get It Done 2025,
/// 8.7M cells, 707 MB). Twenty million cells bounds one import near 1.5 GB.
pub(crate) const MAX_STREAMED_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_STREAMED_CELLS: usize = 20_000_000;

pub(crate) struct PreparedTable {
    pub headers: Vec<String>,
    pub records: Vec<Result<csv::StringRecord, String>>,
    pub format: &'static str,
    pub warnings: Vec<String>,
    pub partial: bool,
}

fn invalid(message: impl Into<String>) -> AppError {
    AppError::DataParsing(message.into())
}

pub(crate) fn prepare(
    bytes: &[u8],
    name: &str,
    worksheet: Option<&str>,
) -> Result<PreparedTable, AppError> {
    let extension = name
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    // Decode only an explicit BOM; guessing an encoding could silently change evidence.
    let mut encoding_warning = None;
    let decoded;
    let text_bytes = if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        if !bytes.len().is_multiple_of(2) {
            return Err(invalid("UTF-16 export has an incomplete code unit"));
        }
        let little_endian = bytes[0] == 0xff;
        let units = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| {
                if little_endian {
                    u16::from_le_bytes(*pair)
                } else {
                    u16::from_be_bytes(*pair)
                }
            })
            .collect::<Vec<_>>();
        decoded = String::from_utf16(&units)
            .map_err(|_| invalid("UTF-16 export contains invalid Unicode"))?;
        encoding_warning = Some("Format repair: BOM-declared UTF-16 decoded to UTF-8 for analysis; the source digest still identifies the original bytes.".to_string());
        decoded.as_bytes()
    } else {
        bytes
    };
    let trimmed = text_bytes
        .strip_prefix(&[0xef, 0xbb, 0xbf])
        .unwrap_or(text_bytes);
    let first = trimmed.iter().find(|b| !b.is_ascii_whitespace()).copied();
    let mut table = if matches!(extension.as_str(), "xlsx" | "xls" | "ods")
        || bytes.starts_with(b"PK\x03\x04")
        || bytes.starts_with(&[0xd0, 0xcf, 0x11, 0xe0])
    {
        workbook(bytes, worksheet)?
    } else if matches!(extension.as_str(), "jsonl" | "ndjson") {
        let mut rows = Vec::new();
        for line in std::str::from_utf8(trimmed)
            .map_err(|_| invalid("JSONL must be UTF-8"))?
            .lines()
            .filter(|s| !s.trim().is_empty())
        {
            if rows.len() >= MAX_ROWS {
                return Err(invalid(
                    "Table exceeds 750000 rows; split the export before importing",
                ));
            }
            rows.push(
                serde_json::from_str(line)
                    .map_err(|e| invalid(format!("JSONL row {}: {e}", rows.len() + 1)))?,
            );
        }
        json_table(Value::Array(rows))?
    } else if matches!(extension.as_str(), "json" | "geojson") || matches!(first, Some(b'{' | b'['))
    {
        json_table(
            serde_json::from_slice(trimmed)
                .map_err(|e| invalid(format!("Invalid table JSON: {e}")))?,
        )?
    } else {
        if first == Some(b'<') || trimmed.contains(&0) {
            return Err(invalid("This is not a supported table export. Choose CSV, TSV, JSON, GeoJSON, XLSX, XLS, or ODS, rather than a webpage or PDF."));
        }
        let delimiter = detect_delimiter(trimmed, &extension)?;
        let mut reader = csv::ReaderBuilder::new()
            .delimiter(delimiter)
            .flexible(true)
            .from_reader(trimmed);
        let headers = reader
            .headers()
            .map_err(|e| invalid(format!("Table headers: {e}")))?
            .iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let mut records = Vec::new();
        for record in reader.records() {
            check_size(records.len() + 1, headers.len())?;
            records.push(record.map_err(|e| e.to_string()));
        }
        PreparedTable {
            headers,
            records,
            format: if delimiter == b'\t' { "TSV" } else { "CSV" },
            warnings: if delimiter != if extension == "tsv" { b'\t' } else { b',' } {
                vec![format!(
                    "Format repair: detected {} delimiter using quote-aware record consistency.",
                    match delimiter {
                        b'\t' => "tab",
                        b';' => "semicolon",
                        b'|' => "pipe",
                        _ => "comma",
                    }
                )]
            } else {
                vec![]
            },
            partial: false,
        }
    };
    if let Some(warning) = encoding_warning {
        table.warnings.push(warning);
    }
    check_size(table.records.len(), table.headers.len())?;
    if table.headers.is_empty() || table.headers.iter().all(|h| h.trim().is_empty()) {
        return Err(invalid("Table has no column headers"));
    }
    if table.headers.iter().collect::<BTreeSet<_>>().len() != table.headers.len() {
        table.warnings.push("Duplicate column headers were assigned distinct SQL names; raw rows are retained by column position.".into());
    }
    Ok(table)
}

/// How many leading bytes decide a file's format and delimiter.
const SNIFF_BYTES: u64 = 1024 * 1024;

/// A table to import. Delimited text and XLSX worksheets are never held whole: each is parsed
/// once from disk (an XLSX after one hashing pass over its bytes), so its bound is the file size
/// and cell count rather than memory. XLS, XLSB, ODS and JSON are parsed whole by their libraries
/// and keep the in-memory caps, which are also what stop a small compressed workbook from
/// describing an unbounded sheet.
pub(crate) enum TableSource {
    Delimited {
        path: std::path::PathBuf,
        delimiter: u8,
        warnings: Vec<String>,
    },
    Loaded {
        table: PreparedTable,
        sha256: String,
    },
    /// An XLSX worksheet, read cell by cell as the box its values span: the first row with a
    /// value is the header, and a row with no value inside the box is an empty row.
    Worksheet {
        path: std::path::PathBuf,
        sheet: String,
        warnings: Vec<String>,
        partial: bool,
    },
}

/// What [`TableSource::read`] passes its callback.
pub(crate) enum TableRow<'a> {
    /// One record, counted from 0, and the headers it is read under. A record the parser cannot
    /// read is passed as its error.
    Record {
        headers: &'a [String],
        index: usize,
        record: Result<&'a csv::StringRecord, &'a str>,
    },
    /// Every record passed so far is void, and the read starts again from the first.
    Restart,
}

/// How a worksheet read ended.
enum WorksheetRead {
    Done {
        headers: Vec<String>,
        rows: usize,
    },
    /// A value lies outside the header row's columns, so the box is wider than assumed.
    Wider,
}

const OUT_OF_ROW_ORDER: &str = "This worksheet stores its cells out of row order, which spreadsheet applications do not write. Open and save it again in a spreadsheet application, then import it.";

/// Reads `sheet` once as its value box. Without `bounds`, the box's columns are taken from the
/// header row, which holds for nearly every export; a value outside them ends the read as
/// [`WorksheetRead::Wider`] so the caller can measure the box and read again with it.
fn read_worksheet<RS: std::io::Read + std::io::Seek>(
    book: &mut calamine::Xlsx<RS>,
    sheet: &str,
    bounds: Option<&crate::parsers::excel::XlsxBounds>,
    each: &mut impl FnMut(TableRow<'_>) -> Result<(), AppError>,
) -> Result<WorksheetRead, AppError> {
    let changed = || {
        invalid(
            "The worksheet changed between its two reads; import stopped and nothing was stored.",
        )
    };
    let mut reader = book
        .worksheet_cells_reader(sheet)
        .map_err(worksheet_error)?;
    let mut previous: Option<(u32, u32)> = None;
    let mut header_row: Option<u32> = None;
    let mut header_cells: Vec<(u32, String)> = Vec::new();
    // The box's first column and width, and the headers, once the header row is complete.
    let mut layout: Option<(u32, usize, Vec<String>)> = None;
    let mut row: Vec<String> = Vec::new();
    // The sheet row `row` holds, while it has values not yet passed on.
    let mut pending: Option<u32> = None;
    let mut rows = 0;
    let finish_header =
        |header_cells: &[(u32, String)]| -> Result<(u32, usize, Vec<String>), AppError> {
            let (first_column, width) = match bounds {
                Some(bounds) => (
                    bounds.first_column,
                    (bounds.last_column - bounds.first_column + 1) as usize,
                ),
                // In row order, the header row's cells arrive left to right.
                None => {
                    let first = header_cells.first().map_or(0, |(column, _)| *column);
                    let last = header_cells.last().map_or(0, |(column, _)| *column);
                    (first, (last - first + 1) as usize)
                }
            };
            let mut headers = vec![String::new(); width];
            for (column, text) in header_cells {
                let slot = column.checked_sub(first_column).map(|slot| slot as usize);
                match slot.filter(|slot| *slot < width) {
                    Some(slot) => headers[slot] = text.clone(),
                    None => return Err(changed()),
                }
            }
            check_headers(&headers)?;
            Ok((first_column, width, headers))
        };
    // Passes rows from `from` up to, not including, `until`; rows with no value inside the box
    // are passed empty, as a loaded worksheet's dense range has them.
    // One record, refilled for every row passed on.
    let mut record = csv::StringRecord::new();
    let mut pass = |row: &mut Vec<String>,
                    from: u32,
                    until: u32,
                    width: usize,
                    headers: &[String],
                    rows: &mut usize|
     -> Result<(), AppError> {
        for _ in from..until {
            check_streamed_size(*rows + 1, width)?;
            record.clear();
            for cell in row.iter_mut() {
                record.push_field(cell);
                cell.clear();
            }
            each(TableRow::Record {
                headers,
                index: *rows,
                record: Ok(&record),
            })?;
            *rows += 1;
        }
        Ok(())
    };
    while let Some(cell) = reader.next_cell().map_err(worksheet_error)? {
        if matches!(cell.get_value(), calamine::DataRef::Empty) {
            continue;
        }
        let (at_row, at_column) = cell.get_position();
        if previous.is_some_and(|at| (at_row, at_column) <= at) {
            return Err(invalid(OUT_OF_ROW_ORDER));
        }
        previous = Some((at_row, at_column));
        match header_row {
            None => {
                if bounds.is_some_and(|bounds| bounds.first_row != at_row) {
                    return Err(changed());
                }
                header_row = Some(at_row);
                let mut text = String::new();
                render_into(cell.get_value(), &mut text);
                header_cells.push((at_column, text));
                continue;
            }
            Some(header) if header == at_row => {
                let mut text = String::new();
                render_into(cell.get_value(), &mut text);
                header_cells.push((at_column, text));
                continue;
            }
            Some(_) => {}
        }
        if layout.is_none() {
            layout = Some(finish_header(&header_cells)?);
        }
        let (first_column, width, headers) = layout.as_ref().expect("set above");
        if at_column < *first_column || (at_column - first_column) as usize >= *width {
            return match bounds {
                Some(_) => Err(changed()),
                None => Ok(WorksheetRead::Wider),
            };
        }
        if bounds.is_some_and(|bounds| at_row > bounds.last_row) {
            return Err(changed());
        }
        if pending != Some(at_row) {
            let from = match pending {
                Some(sheet_row) => {
                    pass(
                        &mut row,
                        sheet_row,
                        sheet_row + 1,
                        *width,
                        headers,
                        &mut rows,
                    )?;
                    sheet_row + 1
                }
                None => {
                    row = vec![String::new(); *width];
                    header_row.expect("set before any data row") + 1
                }
            };
            pass(&mut row, from, at_row, *width, headers, &mut rows)?;
            pending = Some(at_row);
        }
        render_into(
            cell.get_value(),
            &mut row[(at_column - first_column) as usize],
        );
    }
    let Some(header_row) = header_row else {
        return Err(invalid("Worksheet is empty"));
    };
    let (_, width, headers) = match layout {
        Some(layout) => layout,
        None => finish_header(&header_cells)?,
    };
    if let Some(sheet_row) = pending {
        pass(
            &mut row,
            sheet_row,
            sheet_row + 1,
            width,
            &headers,
            &mut rows,
        )?;
    }
    if bounds.is_some_and(|bounds| pending.unwrap_or(header_row) != bounds.last_row) {
        return Err(changed());
    }
    Ok(WorksheetRead::Done { headers, rows })
}

fn choose_sheet(names: &[String], worksheet: Option<&str>) -> Result<String, AppError> {
    match worksheet.filter(|s| !s.trim().is_empty()) {
        Some(name) if names.iter().any(|n| n == name) => Ok(name.to_string()),
        Some(_) => Err(invalid(format!(
            "Worksheet not found. Available sheets: {}",
            names.join(", ")
        ))),
        None if names.len() == 1 => Ok(names[0].clone()),
        _ => Err(invalid(format!(
            "Choose a worksheet to import. Available sheets: {}",
            names.join(", ")
        ))),
    }
}

/// A loaded workbook cell's text, by the same rule as a streamed one's.
fn render(data: &Data) -> String {
    use calamine::DataRef;
    let value = match data {
        Data::Empty => DataRef::Empty,
        Data::Int(value) => DataRef::Int(*value),
        Data::Float(value) => DataRef::Float(*value),
        Data::Bool(value) => DataRef::Bool(*value),
        Data::String(text) => DataRef::SharedString(text),
        Data::DateTime(date) => DataRef::DateTime(*date),
        Data::DateTimeIso(text) => DataRef::DateTimeIso(text.clone()),
        Data::DurationIso(text) => DataRef::DurationIso(text.clone()),
        Data::Error(error) => DataRef::Error(error.clone()),
    };
    let mut out = String::new();
    render_into(&value, &mut out);
    out
}

/// Writes a worksheet cell's text into `out`: its `Display` text, except that a date and time
/// is written as `%Y-%m-%d %H:%M:%S`.
fn render_into(value: &calamine::DataRef<'_>, out: &mut String) {
    use calamine::DataRef;
    use std::fmt::Write;
    match value {
        DataRef::Empty => {}
        DataRef::String(text) | DataRef::DateTimeIso(text) | DataRef::DurationIso(text) => {
            out.push_str(text)
        }
        DataRef::SharedString(text) => out.push_str(text),
        DataRef::DateTime(date) => match date.as_datetime() {
            Some(at) => write!(out, "{}", at.format("%Y-%m-%d %H:%M:%S")),
            None => write!(out, "{date}"),
        }
        .expect("writing to a String cannot fail"),
        DataRef::Int(value) => write!(out, "{value}").expect("writing to a String cannot fail"),
        DataRef::Float(value) => write!(out, "{value}").expect("writing to a String cannot fail"),
        DataRef::Bool(value) => write!(out, "{value}").expect("writing to a String cannot fail"),
        DataRef::Error(value) => write!(out, "{value}").expect("writing to a String cannot fail"),
    }
}

fn worksheet_error(error: impl std::fmt::Display) -> AppError {
    invalid(format!("Worksheet: {error}"))
}

/// An XLSX worksheet read cell by cell, or `None` when the file is not XLSX (ODS shares its
/// container signature) and must go to the loader for the other workbook formats.
fn streamed_worksheet(
    path: &std::path::Path,
    worksheet: Option<&str>,
) -> Result<Option<TableSource>, AppError> {
    let (file, opened) =
        crate::parsers::source::open_regular_file(path, "ANALYTICAL", MAX_STREAMED_BYTES)?;
    let Ok(book) = calamine::Xlsx::new(std::io::BufReader::new(&file)) else {
        return Ok(None);
    };
    let names = book.sheet_names().to_vec();
    let sheet = choose_sheet(&names, worksheet)?;
    crate::parsers::source::ensure_unchanged(&file, &opened, path, "ANALYTICAL")?;
    Ok(Some(TableSource::Worksheet {
        path: path.to_path_buf(),
        warnings: vec![format!("Imported worksheet '{sheet}'. Formula cells use the workbook’s cached values; formulas were not recalculated.")],
        partial: names.len() > 1,
        sheet,
    }))
}

/// What one complete read of a source found. `sha256` covers every byte read, so two reads that
/// agree on it read the same file.
pub(crate) struct SourceRead {
    pub headers: Vec<String>,
    pub sha256: String,
    pub rows: usize,
}

impl TableSource {
    pub(crate) fn open(
        path: &std::path::Path,
        name: &str,
        worksheet: Option<&str>,
    ) -> Result<Self, AppError> {
        let extension = name
            .rsplit('.')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let (file, opened) =
            crate::parsers::source::open_regular_file(path, "ANALYTICAL", MAX_STREAMED_BYTES)?;
        let mut sample = Vec::new();
        std::io::Read::read_to_end(&mut std::io::Read::take(&file, SNIFF_BYTES), &mut sample)
            .map_err(|error| AppError::Storage(format!("ANALYTICAL_READ_FAILED: {error}")))?;
        crate::parsers::source::ensure_unchanged(&file, &opened, path, "ANALYTICAL")?;
        let text = sample.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&sample);
        let first = text.iter().find(|b| !b.is_ascii_whitespace()).copied();
        let delimited = !(matches!(
            extension.as_str(),
            "xlsx" | "xls" | "ods" | "jsonl" | "ndjson" | "json" | "geojson"
        ) || sample.starts_with(b"PK\x03\x04")
            || sample.starts_with(&[0xd0, 0xcf, 0x11, 0xe0])
            || sample.starts_with(&[0xff, 0xfe])
            || sample.starts_with(&[0xfe, 0xff])
            || matches!(first, Some(b'{' | b'[')));
        if sample.starts_with(b"PK\x03\x04") && extension != "ods" {
            if let Some(source) = streamed_worksheet(path, worksheet)? {
                return Ok(source);
            }
        }
        if !delimited {
            let bytes =
                crate::parsers::source::read_regular_file(path, "ANALYTICAL", MAX_SOURCE_BYTES)?;
            return Ok(Self::Loaded {
                sha256: crate::integrity::sha256_hex(&bytes),
                table: prepare(&bytes, name, worksheet)?,
            });
        }
        if first == Some(b'<') {
            return Err(invalid("This is not a supported table export. Choose CSV, TSV, JSON, GeoJSON, XLSX, XLS, or ODS, rather than a webpage or PDF."));
        }
        // A record cut off by the end of the sample can only make an alternate dialect look less
        // consistent, never more, so sniffing a prefix cannot select a wrong delimiter.
        let delimiter = detect_delimiter(text, &extension)?;
        let mut warnings = Vec::new();
        if delimiter != if extension == "tsv" { b'\t' } else { b',' } {
            warnings.push(format!(
                "Format repair: detected {} delimiter using quote-aware record consistency.",
                match delimiter {
                    b'\t' => "tab",
                    b';' => "semicolon",
                    b'|' => "pipe",
                    _ => "comma",
                }
            ));
        }
        Ok(Self::Delimited {
            path: path.to_path_buf(),
            delimiter,
            warnings,
        })
    }

    pub(crate) fn warnings(&self) -> &[String] {
        match self {
            Self::Delimited { warnings, .. } => warnings,
            Self::Loaded { table, .. } => &table.warnings,
            Self::Worksheet { warnings, .. } => warnings,
        }
    }

    pub(crate) fn format(&self) -> &'static str {
        match self {
            Self::Delimited { delimiter, .. } if *delimiter == b'\t' => "TSV",
            Self::Delimited { .. } => "CSV",
            Self::Loaded { table, .. } => table.format,
            Self::Worksheet { .. } => "Workbook",
        }
    }

    pub(crate) fn declares_partial(&self) -> bool {
        match self {
            Self::Loaded { table, .. } => table.partial,
            Self::Worksheet { partial, .. } => *partial,
            Self::Delimited { .. } => false,
        }
    }

    /// Passes every record in order with the header row and its zero-based index. A record the
    /// parser cannot read is passed as its error and the read continues, as the row-level
    /// validator expects. After [`TableRow::Restart`], records are passed again from the first.
    pub(crate) fn read(
        &self,
        mut each: impl FnMut(TableRow<'_>) -> Result<(), AppError>,
    ) -> Result<SourceRead, AppError> {
        match self {
            Self::Loaded { table, sha256 } => {
                for (index, record) in table.records.iter().enumerate() {
                    each(TableRow::Record {
                        headers: &table.headers,
                        index,
                        record: record.as_ref().map_err(String::as_str),
                    })?;
                }
                Ok(SourceRead {
                    headers: table.headers.clone(),
                    sha256: sha256.clone(),
                    rows: table.records.len(),
                })
            }
            Self::Worksheet { path, sheet, .. } => {
                let (file, opened) = crate::parsers::source::open_regular_file(
                    path,
                    "ANALYTICAL",
                    MAX_STREAMED_BYTES,
                )?;
                let mut hashing = HashingReader::new(&file);
                std::io::copy(&mut hashing, &mut std::io::sink()).map_err(|error| {
                    AppError::Storage(format!("ANALYTICAL_READ_FAILED: {error}"))
                })?;
                let sha256 = hashing.finish();
                std::io::Seek::rewind(&mut &file).map_err(|error| {
                    AppError::Storage(format!("ANALYTICAL_READ_FAILED: {error}"))
                })?;
                let mut book = calamine::Xlsx::new(std::io::BufReader::new(&file))
                    .map_err(|e| invalid(format!("Workbook: {e}")))?;
                let read = match read_worksheet(&mut book, sheet, None, &mut each)? {
                    WorksheetRead::Done { headers, rows } => (headers, rows),
                    WorksheetRead::Wider => {
                        let bounds = crate::parsers::excel::xlsx_value_bounds(&mut book, sheet)
                            .map_err(worksheet_error)?
                            .ok_or_else(|| invalid("Worksheet is empty"))?;
                        if !bounds.in_row_order {
                            return Err(invalid(OUT_OF_ROW_ORDER));
                        }
                        each(TableRow::Restart)?;
                        match read_worksheet(&mut book, sheet, Some(&bounds), &mut each)? {
                            WorksheetRead::Done { headers, rows } => (headers, rows),
                            // A measured box holds every value unless the sheet changed.
                            WorksheetRead::Wider => return Err(invalid("The worksheet changed between its two reads; import stopped and nothing was stored.")),
                        }
                    }
                };
                crate::parsers::source::ensure_unchanged(&file, &opened, path, "ANALYTICAL")?;
                Ok(SourceRead {
                    headers: read.0,
                    sha256,
                    rows: read.1,
                })
            }
            Self::Delimited {
                path, delimiter, ..
            } => {
                let (file, opened) = crate::parsers::source::open_regular_file(
                    path,
                    "ANALYTICAL",
                    MAX_STREAMED_BYTES,
                )?;
                let mut hashing = HashingReader::new(&file);
                let mut buffered = std::io::BufReader::with_capacity(256 * 1024, &mut hashing);
                let starts_with_bom = std::io::BufRead::fill_buf(&mut buffered)
                    .map_err(|error| AppError::Storage(format!("ANALYTICAL_READ_FAILED: {error}")))?
                    .starts_with(&[0xef, 0xbb, 0xbf]);
                if starts_with_bom {
                    std::io::BufRead::consume(&mut buffered, 3);
                }
                let mut reader = csv::ReaderBuilder::new()
                    .delimiter(*delimiter)
                    .flexible(true)
                    .from_reader(buffered);
                let headers = reader
                    .headers()
                    .map_err(|e| invalid(format!("Table headers: {e}")))?
                    .iter()
                    .map(str::to_owned)
                    .collect::<Vec<_>>();
                check_headers(&headers)?;
                let mut record = csv::StringRecord::new();
                let mut rows = 0;
                loop {
                    match reader.read_record(&mut record) {
                        Ok(false) => break,
                        Ok(true) => {
                            check_streamed_size(rows + 1, headers.len())?;
                            each(TableRow::Record {
                                headers: &headers,
                                index: rows,
                                record: Ok(&record),
                            })?;
                        }
                        Err(error) => {
                            if matches!(error.kind(), csv::ErrorKind::Io(_)) {
                                return Err(AppError::Storage(format!(
                                    "ANALYTICAL_READ_FAILED: {error}"
                                )));
                            }
                            check_streamed_size(rows + 1, headers.len())?;
                            each(TableRow::Record {
                                headers: &headers,
                                index: rows,
                                record: Err(&error.to_string()),
                            })?;
                        }
                    }
                    rows += 1;
                }
                drop(reader);
                if hashing.saw_nul {
                    return Err(invalid("This is not a supported table export. Choose CSV, TSV, JSON, GeoJSON, XLSX, XLS, or ODS, rather than a webpage or PDF."));
                }
                crate::parsers::source::ensure_unchanged(&file, &opened, path, "ANALYTICAL")?;
                Ok(SourceRead {
                    headers,
                    sha256: hashing.finish(),
                    rows,
                })
            }
        }
    }
}

/// SHA-256 of every byte passed through, and whether any was NUL.
struct HashingReader<R> {
    inner: R,
    digest: sha2::Sha256,
    saw_nul: bool,
}

impl<R> HashingReader<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            digest: <sha2::Sha256 as sha2::Digest>::new(),
            saw_nul: false,
        }
    }

    fn finish(self) -> String {
        crate::integrity::digest_hex(sha2::Digest::finalize(self.digest))
    }
}

impl<R: std::io::Read> std::io::Read for HashingReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buffer)?;
        sha2::Digest::update(&mut self.digest, &buffer[..read]);
        self.saw_nul |= buffer[..read].contains(&0);
        Ok(read)
    }
}

fn check_headers_width(columns: usize) -> Result<(), AppError> {
    if columns > MAX_COLUMNS {
        return Err(invalid(format!(
            "Analytical import supports up to {MAX_COLUMNS} columns."
        )));
    }
    Ok(())
}

fn check_headers(headers: &[String]) -> Result<(), AppError> {
    check_headers_width(headers.len())?;
    if headers.is_empty() || headers.iter().all(|h| h.trim().is_empty()) {
        return Err(invalid("Table has no column headers"));
    }
    Ok(())
}

fn check_streamed_size(rows: usize, columns: usize) -> Result<(), AppError> {
    if rows.saturating_mul(columns) > MAX_STREAMED_CELLS {
        return Err(invalid(format!("Delimited import supports up to {MAX_STREAMED_CELLS} cells. Split this export into smaller tables.")));
    }
    Ok(())
}

/// A declared/default dialect wins if plausible. Alternate dialects must have consistently
/// shaped records; ambiguous alternatives require review. Never count separators inside quotes.
fn detect_delimiter(bytes: &[u8], extension: &str) -> Result<u8, AppError> {
    let preferred = if extension == "tsv" { b'\t' } else { b',' };
    let mut plausible = Vec::new();
    let mut alternate_header = false;
    for delimiter in [preferred, b',', b'\t', b';', b'|'] {
        if plausible.contains(&delimiter) {
            continue;
        }
        let mut reader = csv::ReaderBuilder::new()
            .delimiter(delimiter)
            .flexible(true)
            .from_reader(bytes);
        let Ok(headers) = reader.headers() else {
            continue;
        };
        let width = headers.len();
        if width < 2 {
            continue;
        }
        // For the default dialect keep malformed rows visible to the row-level validator.
        if delimiter == preferred {
            return Ok(preferred);
        }
        alternate_header = true;
        let mut observed = 0;
        let mut matching = 0;
        for row in reader.records().take(32) {
            observed += 1;
            if row.is_ok_and(|row| {
                row.len() == width
                    || (row.len() > width && row.iter().skip(width).all(|s| s.trim().is_empty()))
            }) {
                matching += 1;
            }
        }
        if observed > 0 && matching * 5 >= observed * 3 {
            plausible.push(delimiter);
        }
    }
    match plausible.as_slice() {
        [] if alternate_header => Err(invalid(
            "No consistent table delimiter; review the export format before importing",
        )),
        [] => Ok(preferred), // Legitimate single-column export, or row errors handled below.
        [delimiter] => Ok(*delimiter),
        _ => Err(invalid(
            "Ambiguous table delimiter; export an explicit CSV or TSV before importing",
        )),
    }
}

fn check_size(rows: usize, columns: usize) -> Result<(), AppError> {
    if rows > MAX_ROWS || columns > MAX_COLUMNS || rows.saturating_mul(columns) > MAX_CELLS {
        return Err(invalid("Analytical import supports up to 750000 rows, 512 columns, and 6000000 cells. Split this export into smaller tables."));
    }
    Ok(())
}

fn cell(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        value => value.to_string(),
    }
}

/// Whether a JSON response says the server holds rows it did not send: ArcGIS sets
/// `exceededTransferLimit` (under `properties` in GeoJSON), paged APIs a `next` link.
pub(crate) fn reports_more_rows(value: &Value) -> bool {
    value
        .get("exceededTransferLimit")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || value
            .pointer("/properties/exceededTransferLimit")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        || value.get("next").is_some_and(|v| !v.is_null())
}

fn json_table(value: Value) -> Result<PreparedTable, AppError> {
    if let Some(error) = value.get("error") {
        return Err(invalid(format!(
            "Provider returned an error instead of rows: {error}"
        )));
    }
    let partial = reports_more_rows(&value);
    let features = value.get("features").and_then(Value::as_array);
    let mut objects = Vec::new();
    let mut headers = Vec::new();
    let mut seen = BTreeSet::new();
    if let (Some(data), Some(columns)) = (
        value.get("data").and_then(Value::as_array),
        value
            .pointer("/meta/view/columns")
            .and_then(Value::as_array),
    ) {
        headers = columns
            .iter()
            .enumerate()
            .map(|(i, c)| {
                c.get("fieldName")
                    .or_else(|| c.get("name"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("field_{}", i + 1))
            })
            .collect();
        check_size(data.len(), headers.len())?;
        let records = data
            .iter()
            .map(|row| {
                row.as_array()
                    .map(|r| csv::StringRecord::from(r.iter().map(cell).collect::<Vec<_>>()))
                    .ok_or_else(|| "Socrata row is not an array".into())
            })
            .collect();
        return Ok(PreparedTable {
            headers,
            records,
            format: "Socrata JSON",
            warnings: vec![],
            partial,
        });
    }
    let rows = features.or_else(|| value.as_array()).or_else(|| ["results", "records", "data"].iter().find_map(|key| value.get(key).and_then(Value::as_array)))
        .ok_or_else(|| invalid("JSON must contain an array of row objects, a feature collection, or Socrata rows and column metadata. Catalog metadata is not a table."))?;
    check_size(rows.len(), 1)?;
    for row in rows {
        let mut object = if features.is_some() {
            match row.get("properties").or_else(|| row.get("attributes")) {
                Some(Value::Object(object)) => object.clone(),
                Some(Value::Null) | None => serde_json::Map::new(),
                _ => return Err(invalid("Feature properties must be an object or null")),
            }
        } else {
            row.as_object()
                .cloned()
                .ok_or_else(|| invalid("Every JSON row must be an object"))?
        };
        if features.is_some() {
            for (key, data) in [
                ("_feature_id", row.get("id")),
                ("_geometry", row.get("geometry")),
                (
                    "_crs",
                    value.get("crs").or_else(|| value.get("spatialReference")),
                ),
            ] {
                if let Some(data) = data {
                    if object.insert(key.into(), data.clone()).is_some() {
                        return Err(invalid(format!(
                            "Feature property '{key}' conflicts with preserved feature metadata"
                        )));
                    }
                }
            }
        }
        for key in object.keys() {
            if seen.insert(key.clone()) {
                headers.push(key.clone());
            }
        }
        check_size(rows.len(), headers.len())?;
        objects.push(object);
    }
    let records = objects
        .iter()
        .map(|o| {
            Ok(csv::StringRecord::from(
                headers
                    .iter()
                    .map(|h| o.get(h).map(cell).unwrap_or_default())
                    .collect::<Vec<_>>(),
            ))
        })
        .collect();
    Ok(PreparedTable {
        headers,
        records,
        format: if features.is_some() {
            "Feature JSON"
        } else {
            "JSON"
        },
        warnings: vec![],
        partial,
    })
}

fn workbook(bytes: &[u8], worksheet: Option<&str>) -> Result<PreparedTable, AppError> {
    let mut book = calamine::open_workbook_auto_from_rs(Cursor::new(bytes))
        .map_err(|e| invalid(format!("Workbook: {e}")))?;
    let names = book.sheet_names().to_vec();
    let name = choose_sheet(&names, worksheet)?;
    // XLSX is streamed by `streamed_worksheet`. The formats loaded here (XLS, XLSB, ODS) expose no
    // streaming reader and build a dense matrix, but cap their grid at 65,536 x 256, bounding the
    // worst case near 536 MB; `check_size` after allocation is the backstop, as in
    // `parsers::excel`.
    let range = book
        .worksheet_range(&name)
        .map_err(|e| invalid(format!("Worksheet: {e}")))?;
    check_size(range.height(), range.width())?;
    let stringify = render;
    let mut rows = range.rows();
    let headers = rows
        .next()
        .ok_or_else(|| invalid("Worksheet is empty"))?
        .iter()
        .map(&stringify)
        .collect();
    let records = rows
        .map(|row| {
            Ok(csv::StringRecord::from(
                row.iter().map(&stringify).collect::<Vec<_>>(),
            ))
        })
        .collect();
    Ok(PreparedTable { headers, records, format: "Workbook", warnings: vec![format!("Imported worksheet '{name}'. Formula cells use the workbook’s cached values; formulas were not recalculated.")], partial: names.len() > 1 })
}

/// What a column's values allow it to be typed as. Every nonempty value is observed, so a late
/// suppression token cannot silently become null.
#[derive(Debug, Clone)]
pub(crate) struct TypeEvidence {
    any: bool,
    integer: bool,
    number: bool,
    float: bool,
    date: bool,
}

impl Default for TypeEvidence {
    fn default() -> Self {
        Self {
            any: false,
            integer: true,
            number: true,
            float: true,
            date: true,
        }
    }
}

impl TypeEvidence {
    pub(crate) fn observe(&mut self, value: &str) {
        let value = value.trim();
        if value.is_empty() {
            return;
        }
        self.any = true;
        let significant = value.trim_start_matches(['-', '+']);
        let leading_zero = significant.starts_with('0')
            && significant
                .as_bytes()
                .get(1)
                .is_some_and(u8::is_ascii_digit);
        // A candidate one value has ruled out stays ruled out, so it is not tested again.
        self.integer = self.integer && !leading_zero && value.parse::<i64>().is_ok();
        self.number = self.number && !leading_zero && exact_decimal(value);
        self.float = self.float && !leading_zero && plain_decimal(value);
        self.date = self.date && chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").is_ok();
    }

    pub(crate) fn inferred(&self, header: &str) -> &'static str {
        let name = header.to_ascii_lowercase();
        if name == "id"
            || name.ends_with("_id")
            || name.ends_with("code")
            || name.contains("fips")
            || name.contains("geoid")
            || name.contains("zip")
            || name.contains("tract")
            || name.ends_with("_number")
        {
            "IDENTIFIER"
        } else if !self.any {
            "CATEGORY"
        } else if self.integer {
            "INTEGER"
        } else if self.number {
            "NUMBER"
        } else if self.float {
            "FLOAT"
        } else if self.date {
            "DATE"
        } else {
            "CATEGORY"
        }
    }
}

/// `-12.345` notation with any number of digits: what a decimal exceeding [`exact_decimal`]'s
/// scale looks like (coordinates published to 15 places). Exponents, `inf` and `NaN` are refused.
pub(crate) fn plain_decimal(value: &str) -> bool {
    let value = value.strip_prefix(['-', '+']).unwrap_or(value);
    let mut parts = value.splitn(2, '.');
    let whole = parts.next().unwrap_or_default();
    !whole.is_empty()
        && whole.bytes().all(|b| b.is_ascii_digit())
        && parts.next().is_none_or(|fraction| {
            !fraction.is_empty() && fraction.bytes().all(|b| b.is_ascii_digit())
        })
}

pub(crate) fn exact_decimal(value: &str) -> bool {
    let value = value.strip_prefix(['-', '+']).unwrap_or(value);
    let (whole, fraction) = match value.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (value, None),
    };
    !whole.is_empty()
        && whole.len() <= 28
        && whole.bytes().all(|b| b.is_ascii_digit())
        && fraction.is_none_or(|fraction| {
            !fraction.is_empty()
                && fraction.len() <= 10
                && fraction.bytes().all(|b| b.is_ascii_digit())
        })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    #[test]
    fn a_decimal_beyond_exact_scale_is_approximate_and_other_notations_stay_text() {
        let typed = |values: &[&str]| {
            let mut evidence = TypeEvidence::default();
            values.iter().for_each(|value| evidence.observe(value));
            evidence.inferred("value")
        };
        assert_eq!(typed(&["0.5", "12.0123456789"]), "NUMBER");
        assert_eq!(typed(&["32.716267940000002", "-117.1"]), "FLOAT");
        for other in ["1e5", "inf", "NaN", ".5", "5.", "012.5"] {
            assert_eq!(typed(&["32.716267940000002", other]), "CATEGORY", "{other}");
        }
    }
    pub(crate) fn workbook_file(
        directory: &std::path::Path,
        sheet_data: &str,
    ) -> std::path::PathBuf {
        use std::io::Write;
        let path = directory.join("sheet.xlsx");
        let mut writer = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let options: zip::write::FileOptions<()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let sheet = format!(
            r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>{sheet_data}</sheetData></worksheet>"#
        );
        for (name, body) in [
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
                r#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Data" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
            ),
            (
                "xl/_rels/workbook.xml.rels",
                r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#,
            ),
            ("xl/worksheets/sheet1.xml", sheet.as_str()),
        ] {
            writer.start_file(name, options).unwrap();
            writer.write_all(body.as_bytes()).unwrap();
        }
        writer.finish().unwrap();
        path
    }

    fn all_rows(source: &TableSource) -> (Vec<String>, Vec<Vec<String>>) {
        let mut rows = Vec::new();
        let read = source
            .read(|row| {
                match row {
                    TableRow::Restart => rows.clear(),
                    TableRow::Record { record, .. } => {
                        rows.push(record.unwrap().iter().map(str::to_owned).collect())
                    }
                }
                Ok(())
            })
            .unwrap();
        (read.headers, rows)
    }

    /// The box starts at the first value, a row with no value inside it is an empty row, and a
    /// missing cell is "": the shape calamine's dense range gives a loaded workbook.
    #[test]
    fn a_worksheet_is_read_as_the_box_its_values_span() {
        let directory = tempfile::tempdir().unwrap();
        let inline = |cell: &str, text: &str| {
            format!(r#"<c r="{cell}" t="inlineStr"><is><t>{text}</t></is></c>"#)
        };
        let data = format!(
            r#"<row r="2">{}{}</row><row r="3">{}<c r="C3"><v>12.5</v></c></row><row r="5"><c r="C5"><v>7</v></c></row><row r="6">{}</row>"#,
            inline("B2", "name"),
            inline("C2", "amount"),
            inline("B3", "Vista"),
            inline("B6", "Escondido"),
        );
        let path = workbook_file(directory.path(), &data);
        let streamed = TableSource::open(&path, "sheet.xlsx", None).unwrap();
        assert!(matches!(streamed, TableSource::Worksheet { .. }));
        let (headers, rows) = all_rows(&streamed);
        let row = |cells: [&str; 2]| cells.map(str::to_owned).to_vec();
        assert_eq!(headers, row(["name", "amount"]));
        assert_eq!(
            rows,
            vec![
                row(["Vista", "12.5"]),
                row(["", ""]),
                row(["", "7"]),
                row(["Escondido", ""])
            ]
        );

        // Cells out of row order are refused with a way forward, not read in a different order.
        let path = workbook_file(
            directory.path(),
            &format!(
                r#"<row r="2">{}</row><row r="1">{}</row>"#,
                inline("A2", "x"),
                inline("A1", "h")
            ),
        );
        let refused = TableSource::open(&path, "sheet.xlsx", None)
            .unwrap()
            .read(|_| Ok(()))
            .err()
            .unwrap();
        assert!(
            refused.to_string().contains("out of row order"),
            "{refused}"
        );

        // A box wider than the column limit is refused before any row is passed on.
        let path = workbook_file(
            directory.path(),
            &format!(
                r#"<row r="1">{}{}</row><row r="2">{}</row>"#,
                inline("A1", "a"),
                inline("XFD1", "b"),
                inline("A2", "x")
            ),
        );
        let mut passed = 0;
        let refused = TableSource::open(&path, "sheet.xlsx", None)
            .unwrap()
            .read(|_| {
                passed += 1;
                Ok(())
            });
        assert!(refused.is_err());
        assert_eq!(passed, 0);
    }

    /// The box's columns are first taken from the header row. A value outside them widens the
    /// box, so the rows passed so far are voided and the sheet is read again as the measured box:
    /// the result is the box a full measurement gives, with blank headers where none was written.
    #[test]
    fn a_value_outside_the_header_columns_widens_the_box() {
        let directory = tempfile::tempdir().unwrap();
        let inline = |cell: &str, text: &str| {
            format!(r#"<c r="{cell}" t="inlineStr"><is><t>{text}</t></is></c>"#)
        };
        let data = format!(
            r#"<row r="1">{}{}</row><row r="2">{}{}</row><row r="3">{}{}</row>"#,
            inline("B1", "name"),
            inline("C1", "amount"),
            inline("B2", "Vista"),
            inline("C2", "4"),
            inline("A3", "note"),
            inline("D3", "late"),
        );
        let path = workbook_file(directory.path(), &data);
        let mut restarts = 0;
        let mut rows = Vec::new();
        let read = TableSource::open(&path, "sheet.xlsx", None)
            .unwrap()
            .read(|row| {
                match row {
                    TableRow::Restart => {
                        restarts += 1;
                        rows.clear();
                    }
                    TableRow::Record { record, .. } => rows.push(
                        record
                            .unwrap()
                            .iter()
                            .map(str::to_owned)
                            .collect::<Vec<_>>(),
                    ),
                }
                Ok(())
            })
            .unwrap();
        let row = |cells: [&str; 4]| cells.map(str::to_owned).to_vec();
        assert_eq!(restarts, 1);
        assert_eq!(read.headers, row(["", "name", "amount", ""]));
        assert_eq!(
            rows,
            vec![row(["", "Vista", "4", ""]), row(["note", "", "", "late"])]
        );
        assert_eq!(read.rows, 2);
    }

    #[test]
    fn a_streamed_import_stops_at_its_cell_bound() {
        assert!(check_streamed_size(MAX_STREAMED_CELLS / 4 - 1, 4).is_ok());
        assert!(check_streamed_size(MAX_STREAMED_CELLS / 4, 4).is_ok());
        assert!(check_streamed_size(MAX_STREAMED_CELLS / 4 + 1, 4).is_err());
    }
    #[test]
    fn explicit_unicode_encoding_and_dialect_repairs_preserve_cell_values() {
        let text = "city;count\n\"San Diego; Central\";3\nMálaga;4";
        for little in [true, false] {
            let mut bytes = if little {
                vec![0xff, 0xfe]
            } else {
                vec![0xfe, 0xff]
            };
            for unit in text.encode_utf16() {
                bytes.extend(if little {
                    unit.to_le_bytes()
                } else {
                    unit.to_be_bytes()
                });
            }
            let table = prepare(&bytes, "export.csv", None).unwrap();
            assert_eq!(&table.records[0].as_ref().unwrap()[0], "San Diego; Central");
            assert_eq!(&table.records[1].as_ref().unwrap()[0], "Málaga");
            assert!(table.warnings.iter().any(|s| s.contains("UTF-16")));
            assert!(table.warnings.iter().any(|s| s.contains("semicolon")));
        }
        assert!(prepare(&[0xff, 0xfe, 0], "bad.csv", None).is_err());
        assert!(prepare(&[0xff, 0xfe, 0x00, 0xd8], "bad.csv", None).is_err());
        let comma = prepare(b"city,count\n\"San Diego; Central\",3", "data.csv", None).unwrap();
        assert_eq!(comma.headers.len(), 2);
        assert!(comma.warnings.is_empty());
    }
    #[test]
    fn ambiguous_alternative_delimiters_require_review() {
        assert!(prepare(b"city;name|count\nA;B|3\nC;D|4", "data.csv", None).is_err());
        let table = prepare(b"city\tcount\nVista\t3", "data.csv", None).unwrap();
        assert_eq!(table.format, "TSV");
    }
    #[test]
    fn suppression_and_leading_zero_codes_survive_inference() {
        let table = prepare(b"zip,population,rate,estimate,date\n00123,15,0.000001,*,2026-01-01\n92101,20,1.123456,12,2026-02-01", "data.csv", None).unwrap();
        let types = table
            .headers
            .iter()
            .enumerate()
            .map(|(i, h)| {
                let mut evidence = TypeEvidence::default();
                for record in table.records.iter().flatten() {
                    evidence.observe(record.get(i).unwrap_or(""));
                }
                evidence.inferred(h)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            types,
            vec!["IDENTIFIER", "INTEGER", "NUMBER", "CATEGORY", "DATE"]
        );
    }
    #[test]
    fn feature_projection_preserves_geometry_nulls_and_late_columns() {
        let table = prepare(br#"{"type":"FeatureCollection","exceededTransferLimit":true,"features":[{"type":"Feature","id":7,"properties":{"tract":"00100","count":3},"geometry":{"type":"Point","coordinates":[-117,32]}},{"type":"Feature","properties":{"new":"suppressed"},"geometry":null}]}"#, "data.geojson", None).unwrap();
        assert!(table.partial);
        assert!(table.headers.contains(&"new".into()));
        let geometry = table.headers.iter().position(|h| h == "_geometry").unwrap();
        assert!(table.records[0].as_ref().unwrap()[geometry].contains("coordinates"));
        assert_eq!(&table.records[1].as_ref().unwrap()[geometry], "");
    }
    #[test]
    fn rejects_webpages_and_api_errors_and_retains_ragged_rows() {
        assert!(prepare(b"<html>Sign in</html>", "data.csv", None).is_err());
        assert!(prepare(br#"{"error":{"code":403}}"#, "data.json", None).is_err());
        let table = prepare(b"a,b\n1,2,3", "data.csv", None).unwrap();
        assert_eq!(table.records[0].as_ref().unwrap().len(), 3);
    }
}
