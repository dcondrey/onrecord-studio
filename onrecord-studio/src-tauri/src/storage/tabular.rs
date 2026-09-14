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
            .chunks_exact(2)
            .map(|pair| {
                if little_endian {
                    u16::from_le_bytes([pair[0], pair[1]])
                } else {
                    u16::from_be_bytes([pair[0], pair[1]])
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

fn json_table(value: Value) -> Result<PreparedTable, AppError> {
    if let Some(error) = value.get("error") {
        return Err(invalid(format!(
            "Provider returned an error instead of rows: {error}"
        )));
    }
    let partial = value
        .get("exceededTransferLimit")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || value
            .pointer("/properties/exceededTransferLimit")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        || value.get("next").is_some_and(|v| !v.is_null());
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
    let name = match worksheet.filter(|s| !s.trim().is_empty()) {
        Some(name) if names.iter().any(|n| n == name) => name.to_string(),
        Some(_) => {
            return Err(invalid(format!(
                "Worksheet not found. Available sheets: {}",
                names.join(", ")
            )))
        }
        None if names.len() == 1 => names[0].clone(),
        _ => {
            return Err(invalid(format!(
                "Choose a worksheet to import. Available sheets: {}",
                names.join(", ")
            )))
        }
    };
    let range = book
        .worksheet_range(&name)
        .map_err(|e| invalid(format!("Worksheet: {e}")))?;
    check_size(range.height(), range.width())?;
    let stringify = |data: &Data| match data {
        Data::Empty => String::new(),
        Data::DateTime(date) => date
            .as_datetime()
            .map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| date.to_string()),
        other => other.to_string(),
    };
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

/// Inspect every nonempty value, so a late suppression token cannot silently become null.
pub(crate) fn infer_type(
    header: &str,
    index: usize,
    records: &[Result<csv::StringRecord, String>],
) -> &'static str {
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
        return "IDENTIFIER";
    }
    let mut any = false;
    let (mut integer, mut number, mut date) = (true, true, true);
    for value in records
        .iter()
        .filter_map(|r| r.as_ref().ok())
        .filter_map(|r| r.get(index))
        .filter(|v| !v.trim().is_empty())
    {
        any = true;
        let value = value.trim();
        let significant = value.trim_start_matches(['-', '+']);
        let leading_zero = significant.starts_with('0')
            && significant
                .as_bytes()
                .get(1)
                .is_some_and(u8::is_ascii_digit);
        integer &= !leading_zero && value.parse::<i64>().is_ok();
        number &= !leading_zero && exact_decimal(value);
        date &= chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").is_ok();
    }
    if !any {
        "CATEGORY"
    } else if integer {
        "INTEGER"
    } else if number {
        "NUMBER"
    } else if date {
        "DATE"
    } else {
        "CATEGORY"
    }
}

pub(crate) fn exact_decimal(value: &str) -> bool {
    let value = value.strip_prefix(['-', '+']).unwrap_or(value);
    let parts = value.split('.').collect::<Vec<_>>();
    parts.len() <= 2
        && !parts[0].is_empty()
        && parts[0].len() <= 28
        && parts[0].bytes().all(|b| b.is_ascii_digit())
        && (parts.len() == 1
            || (!parts[1].is_empty()
                && parts[1].len() <= 10
                && parts[1].bytes().all(|b| b.is_ascii_digit())))
}

#[cfg(test)]
mod tests {
    use super::*;
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
            .map(|(i, h)| infer_type(h, i, &table.records))
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
