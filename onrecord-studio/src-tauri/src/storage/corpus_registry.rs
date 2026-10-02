//! Registry snapshots held beside the corpus index, and the lookup that anchors an extracted
//! identifier to one.
//!
//! The snapshot is an operator-supplied local file; nothing is fetched. Its digest and import
//! instant are stored with it and travel with every match, so "registered to NAME" is always
//! "registered to NAME in file F as of D", reproducible by anyone holding the same file.

use std::collections::BTreeMap;
use std::path::Path;

use duckdb::{params, Connection, OptionalExt};

use crate::error::AppError;
use crate::integrity::sha256_file;
use crate::parsers::source::inspect_regular_file;
use crate::types::{
    CorpusEntityKind, RegistryAnchor, RegistryKind, RegistryRecord, RegistrySnapshotSummary,
};

/// The free CSLB list runs to several hundred megabytes of CSV; the paid full file is larger.
const MAXIMUM_SNAPSHOT_BYTES: u64 = 1 << 30;
const MAXIMUM_RECORDS: usize = 2_000_000;
const MAXIMUM_FIELD_CHARACTERS: usize = 256;
/// A CSLB licence number is numeric and has never exceeded seven digits; the extractor accepts
/// eight so a future rollover is not silently dropped.
const MAXIMUM_LICENSE_DIGITS: usize = 8;

pub(super) struct ParsedRecord {
    pub normalized_identifier: String,
    pub registrant_name: String,
    pub status: String,
    pub detail: String,
}

pub(super) struct ParsedSnapshot {
    pub records: Vec<ParsedRecord>,
    pub skipped_rows: u64,
    pub file_sha256: String,
}

/// Header names, lowercased with everything but letters and digits removed, that may carry each
/// field. The free CSV and the paid fixed-block file label the same columns differently.
struct ColumnNames {
    identifier: &'static [&'static str],
    name: &'static [&'static str],
    status: &'static [&'static str],
    /// Columns folded into `detail` as "Header: value", in this order.
    detail: &'static [&'static str],
}

const CSLB_COLUMNS: ColumnNames = ColumnNames {
    identifier: &[
        "licenseno",
        "licensenumber",
        "licensenum",
        "licno",
        "license",
    ],
    name: &["businessname", "fullbusinessname", "busname", "name"],
    status: &["primarystatus", "licensestatus", "status"],
    detail: &[
        "classifications",
        "classificationss",
        "classification",
        "city",
        "expirationdate",
    ],
};

fn normalized_header(value: &str) -> String {
    value
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|character| character.to_ascii_lowercase())
        .collect()
}

fn clipped(value: &str) -> String {
    value
        .trim()
        .chars()
        .take(MAXIMUM_FIELD_CHARACTERS)
        .collect()
}

/// The form a licence number takes on both sides of the join: digits with leading zeros removed.
/// The extractor keeps the digits as written, so `0123456` and `123456` must meet here.
pub(super) fn normalize_license_number(value: &str) -> Option<String> {
    let digits = value.trim();
    if digits.is_empty()
        || digits.len() > MAXIMUM_LICENSE_DIGITS
        || !digits.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let stripped = digits.trim_start_matches('0');
    Some(if stripped.is_empty() { "0" } else { stripped }.to_string())
}

fn normalize_identifier(registry: RegistryKind, value: &str) -> Option<String> {
    match registry {
        RegistryKind::CslbLicenseMaster => normalize_license_number(value),
    }
}

fn parse_error(message: String) -> AppError {
    AppError::DataParsing(format!("REGISTRY_SNAPSHOT_INVALID: {message}"))
}

/// Reads the whole file, keyed by normalized identifier. The first row for an identifier wins and
/// every later one is counted as skipped, so a file that repeats a number cannot make the lookup
/// answer differently from one import to the next.
pub(super) fn parse_snapshot(
    registry: RegistryKind,
    path: &Path,
) -> Result<ParsedSnapshot, AppError> {
    inspect_regular_file(path, "REGISTRY_SNAPSHOT", MAXIMUM_SNAPSHOT_BYTES)?;
    let file_sha256 = sha256_file(path).map_err(|error| {
        AppError::Storage(format!(
            "Failed to hash the registry snapshot '{}': {error}",
            path.display()
        ))
    })?;
    let columns = match registry {
        RegistryKind::CslbLicenseMaster => &CSLB_COLUMNS,
    };
    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .trim(csv::Trim::All)
        .from_path(path)
        .map_err(|error| parse_error(format!("could not open the file as CSV: {error}")))?;
    let headers = reader
        .headers()
        .map_err(|error| parse_error(format!("could not read the header row: {error}")))?
        .clone();
    let normalized: Vec<String> = headers.iter().map(normalized_header).collect();
    let find = |names: &[&str]| {
        names
            .iter()
            .find_map(|name| normalized.iter().position(|header| header == name))
    };
    let identifier_column = find(columns.identifier).ok_or_else(|| {
        parse_error(format!(
            "no identifier column found; headers are {}",
            headers.iter().collect::<Vec<_>>().join(", ")
        ))
    })?;
    let name_column = find(columns.name).ok_or_else(|| {
        parse_error(format!(
            "no registrant-name column found; headers are {}",
            headers.iter().collect::<Vec<_>>().join(", ")
        ))
    })?;
    let status_column = find(columns.status);
    let detail_columns: Vec<(String, usize)> = columns
        .detail
        .iter()
        .filter_map(|name| {
            let index = normalized.iter().position(|header| header == name)?;
            Some((headers[index].to_string(), index))
        })
        .collect();

    let mut records: BTreeMap<String, ParsedRecord> = BTreeMap::new();
    let mut skipped_rows = 0_u64;
    for row in reader.records() {
        let row = row.map_err(|error| parse_error(format!("malformed row: {error}")))?;
        let Some(identifier) = row
            .get(identifier_column)
            .and_then(|value| normalize_identifier(registry, value))
        else {
            skipped_rows += 1;
            continue;
        };
        if records.contains_key(&identifier) {
            skipped_rows += 1;
            continue;
        }
        if records.len() >= MAXIMUM_RECORDS {
            return Err(parse_error(format!(
                "more than {MAXIMUM_RECORDS} records; this is not a registry list this build can hold"
            )));
        }
        let detail = detail_columns
            .iter()
            .filter_map(|(label, index)| {
                let value = clipped(row.get(*index)?);
                (!value.is_empty()).then(|| format!("{label}: {value}"))
            })
            .collect::<Vec<_>>()
            .join("; ");
        records.insert(
            identifier.clone(),
            ParsedRecord {
                normalized_identifier: identifier,
                registrant_name: clipped(row.get(name_column).unwrap_or_default()),
                status: clipped(
                    status_column
                        .and_then(|index| row.get(index))
                        .unwrap_or_default(),
                ),
                detail,
            },
        );
    }
    if records.is_empty() {
        return Err(parse_error("the file holds no keyed records".to_string()));
    }
    Ok(ParsedSnapshot {
        records: records.into_values().collect(),
        skipped_rows,
        file_sha256,
    })
}

fn storage(message: &str) -> impl FnOnce(duckdb::Error) -> AppError + '_ {
    move |error| AppError::Storage(format!("{message}: {error}"))
}

/// Replaces the registry's snapshot and every record under it in one transaction, so a lookup
/// never sees the old snapshot row with the new records or the reverse.
pub(super) fn replace_snapshot(
    connection: &mut Connection,
    summary: &RegistrySnapshotSummary,
    records: Vec<ParsedRecord>,
) -> Result<(), AppError> {
    let transaction = connection
        .transaction()
        .map_err(storage("Failed to begin the registry import"))?;
    let registry = summary.registry.as_str();
    transaction
        .execute(
            "DELETE FROM corpus_registry_records WHERE registry = ?1",
            params![registry],
        )
        .map_err(storage("Failed to clear the previous registry records"))?;
    transaction
        .execute(
            "DELETE FROM corpus_registry_snapshots WHERE registry = ?1",
            params![registry],
        )
        .map_err(storage("Failed to clear the previous registry snapshot"))?;
    transaction
        .execute(
            "INSERT INTO corpus_registry_snapshots
                (registry, identifier_kind, source_path, file_sha256, imported_at, record_count,
                 skipped_row_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                registry,
                summary.identifier_kind.as_str(),
                summary.source_path,
                summary.file_sha256,
                summary.imported_at,
                i64::try_from(summary.record_count).unwrap_or(i64::MAX),
                i64::try_from(summary.skipped_row_count).unwrap_or(i64::MAX),
            ],
        )
        .map_err(storage("Failed to record the registry snapshot"))?;
    {
        let mut appender = transaction
            .appender("corpus_registry_records")
            .map_err(storage("Failed to append registry records"))?;
        for record in &records {
            appender
                .append_row(params![
                    registry,
                    record.normalized_identifier,
                    record.registrant_name,
                    record.status,
                    record.detail,
                ])
                .map_err(storage("Failed to persist a registry record"))?;
        }
        appender
            .flush()
            .map_err(storage("Failed to flush registry records"))?;
    }
    transaction
        .commit()
        .map_err(storage("Failed to commit the registry import"))
}

fn read_snapshot(row: &duckdb::Row<'_>) -> duckdb::Result<Option<RegistrySnapshotSummary>> {
    let registry: String = row.get(0)?;
    let identifier_kind: String = row.get(1)?;
    let Some(registry) = RegistryKind::parse(&registry) else {
        return Ok(None);
    };
    Ok(Some(RegistrySnapshotSummary {
        registry,
        identifier_kind: super::entity_kind_from_str(&identifier_kind)
            .unwrap_or_else(|| registry.identifier_kind()),
        source_path: row.get(2)?,
        file_sha256: row.get(3)?,
        imported_at: row.get(4)?,
        record_count: row.get::<_, i64>(5)?.max(0) as u64,
        skipped_row_count: row.get::<_, i64>(6)?.max(0) as u64,
    }))
}

const SNAPSHOT_COLUMNS: &str = "registry, identifier_kind, source_path, file_sha256, imported_at,
                                record_count, skipped_row_count";

pub(super) fn list_snapshots(
    connection: &Connection,
) -> Result<Vec<RegistrySnapshotSummary>, AppError> {
    let mut statement = connection
        .prepare(&format!(
            "SELECT {SNAPSHOT_COLUMNS} FROM corpus_registry_snapshots ORDER BY registry"
        ))
        .map_err(storage("Failed to prepare the registry listing"))?;
    let rows = statement
        .query_map([], read_snapshot)
        .map_err(storage("Failed to list registry snapshots"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage("Failed to read registry snapshots"))?;
    Ok(rows.into_iter().flatten().collect())
}

pub(super) fn lookup(
    connection: &Connection,
    identifier_kind: CorpusEntityKind,
    value: &str,
) -> Result<RegistryAnchor, AppError> {
    let Some(registry) = RegistryKind::for_identifier(identifier_kind) else {
        return Ok(RegistryAnchor::NoRegistry { identifier_kind });
    };
    let Some(identifier) = normalize_identifier(registry, value) else {
        return Err(AppError::DataParsing(format!(
            "'{value}' is not a {} value this registry is keyed by",
            identifier_kind.as_str()
        )));
    };
    let snapshot = connection
        .query_row(
            &format!(
                "SELECT {SNAPSHOT_COLUMNS} FROM corpus_registry_snapshots WHERE registry = ?1"
            ),
            params![registry.as_str()],
            read_snapshot,
        )
        .optional()
        .map_err(storage("Failed to read the registry snapshot"))?
        .flatten();
    let Some(snapshot) = snapshot else {
        return Ok(RegistryAnchor::NotImported { registry });
    };
    let record = connection
        .query_row(
            "SELECT registrant_name, status, detail FROM corpus_registry_records
             WHERE registry = ?1 AND normalized_identifier = ?2",
            params![registry.as_str(), identifier],
            |row| {
                Ok(RegistryRecord {
                    registrant_name: row.get(0)?,
                    status: row.get(1)?,
                    detail: row.get(2)?,
                })
            },
        )
        .optional()
        .map_err(storage("Failed to read the registry record"))?;
    Ok(match record {
        Some(record) => RegistryAnchor::Registered {
            snapshot,
            identifier,
            record,
        },
        None => RegistryAnchor::NotListed {
            snapshot,
            identifier,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::corpus::CorpusIndex;
    use tempfile::tempdir;

    fn write_csv(directory: &Path, name: &str, body: &str) -> std::path::PathBuf {
        let path = directory.join(name);
        std::fs::write(&path, body).expect("fixture must be written");
        path
    }

    /// The four lookup answers are produced by four different corpus states, and a re-import
    /// replaces the earlier snapshot rather than merging into it.
    #[tokio::test]
    async fn a_lookup_distinguishes_no_registry_not_imported_not_listed_and_registered() {
        let directory = tempdir().expect("temp dir must be created");
        let index = CorpusIndex::open(directory.path().join("corpus.db")).expect("index opens");
        let license = CorpusEntityKind::ContractorLicense;

        assert_eq!(
            index
                .registry_anchor(CorpusEntityKind::Ein, "12-3456789".into())
                .await
                .unwrap(),
            RegistryAnchor::NoRegistry {
                identifier_kind: CorpusEntityKind::Ein
            }
        );
        assert_eq!(
            index
                .registry_anchor(license, "123456".into())
                .await
                .unwrap(),
            RegistryAnchor::NotImported {
                registry: RegistryKind::CslbLicenseMaster
            }
        );

        let first = write_csv(
            directory.path(),
            "master.csv",
            "LicenseNo,BusinessName,PrimaryStatus,City,Classifications(s)\n\
             0123456,ACME BUILDERS INC,ACTIVE,SAN DIEGO,B | C-10\n\
             123456,DUPLICATE ROW,ACTIVE,,\n\
             not-a-number,NO KEY,ACTIVE,,\n\
             222222,OTHER CO,EXPIRED,,\n",
        );
        let summary = index
            .import_registry_snapshot(RegistryKind::CslbLicenseMaster, first.clone())
            .await
            .expect("import must succeed");
        assert_eq!((summary.record_count, summary.skipped_row_count), (2, 2));
        assert_eq!(summary.file_sha256, sha256_file(&first).unwrap());

        let anchor = index
            .registry_anchor(license, "123456".into())
            .await
            .unwrap();
        let RegistryAnchor::Registered {
            snapshot,
            identifier,
            record,
        } = anchor
        else {
            panic!("a listed licence must be Registered, got {anchor:?}");
        };
        assert_eq!(snapshot, summary);
        assert_eq!(identifier, "123456");
        assert_eq!(record.registrant_name, "ACME BUILDERS INC");
        assert_eq!(record.status, "ACTIVE");
        assert_eq!(
            record.detail,
            "Classifications(s): B | C-10; City: SAN DIEGO"
        );
        assert!(matches!(
            index.registry_anchor(license, "999999".into()).await.unwrap(),
            RegistryAnchor::NotListed { identifier, .. } if identifier == "999999"
        ));

        let second = write_csv(
            directory.path(),
            "later.csv",
            "LicenseNo,BusinessName\n222222,OTHER CO RENAMED\n",
        );
        let replaced = index
            .import_registry_snapshot(RegistryKind::CslbLicenseMaster, second)
            .await
            .unwrap();
        assert_eq!(replaced.record_count, 1);
        assert_eq!(
            index.registry_snapshots().await.unwrap(),
            vec![replaced.clone()]
        );
        assert!(matches!(
            index.registry_anchor(license, "123456".into()).await.unwrap(),
            RegistryAnchor::NotListed { snapshot, .. } if snapshot == replaced
        ));
        assert!(matches!(
            index.registry_anchor(license, "222222".into()).await.unwrap(),
            RegistryAnchor::Registered { record, .. } if record.registrant_name == "OTHER CO RENAMED"
        ));
    }

    #[tokio::test]
    async fn a_file_without_an_identifier_column_is_refused_and_leaves_no_snapshot() {
        let directory = tempdir().expect("temp dir must be created");
        let index = CorpusIndex::open(directory.path().join("corpus.db")).expect("index opens");
        let path = write_csv(directory.path(), "wrong.csv", "Name,Town\nACME,SD\n");
        let error = index
            .import_registry_snapshot(RegistryKind::CslbLicenseMaster, path)
            .await
            .expect_err("a keyless file must be refused");
        assert!(
            error.to_string().contains("no identifier column"),
            "{error}"
        );
        assert!(index.registry_snapshots().await.unwrap().is_empty());
    }
}
