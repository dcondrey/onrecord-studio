//! The source registry: files outside the City's open-data list that registry packs are built
//! from, defined as data in `sources.json` so adding a source needs no Rust. Each entry names its
//! pack and conformed table, where its file is published, and the licence it is published under.

use crate::types::DataPackFileLicence;
use serde::Deserialize;
use std::collections::HashSet;
use std::sync::LazyLock;

/// One published file, or one file per year of a series, as `sources.json` defines it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RegistryEntry {
    pub pack: String,
    pub table: String,
    pub title: String,
    pub publisher: String,
    /// The publisher's landing page the entry was read from.
    pub page: String,
    /// The file's address; a year series names its year as `{year}`.
    pub url: String,
    /// First and last year of a series published one file per year.
    pub years: Option<[u16; 2]>,
    pub format: RegistryFormat,
    pub worksheet: Option<String>,
    pub licence: RegistryLicence,
    /// The header of a per-row identifier column.
    pub key: Option<String>,
    pub time: RegistryTime,
    pub partial: bool,
    pub headers: Vec<String>,
    // Curator notes the registry keeps beside each entry; nothing reads them at run time.
    #[allow(dead_code)]
    bytes: u64,
    #[allow(dead_code)]
    rows: Option<u64>,
    #[allow(dead_code)]
    #[serde(default)]
    notes: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RegistryFormat {
    Csv,
    Xlsx,
    Json,
    Geojson,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RegistryLicence {
    pub id: String,
    pub url: String,
    pub attribution: Option<String>,
    pub disclaimer: Option<String>,
    /// The publisher's own words granting the licence, and where they are.
    pub evidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RegistryTime {
    pub column: Option<String>,
    pub kind: RegistryTimeKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RegistryTimeKind {
    /// A wall-clock reading, as the City's `RecordedAt` columns.
    RecordedAt,
    /// A fiscal year published as its last two digits, as the City's.
    FiscalYear,
    /// A four-digit calendar year with no day.
    CalendarYear,
    Undated,
}

impl RegistryEntry {
    /// The file's address for `year`, which is `None` for a file published once.
    pub fn url_for(&self, year: Option<u16>) -> String {
        match year {
            Some(year) => self.url.replace("{year}", &year.to_string()),
            None => self.url.clone(),
        }
    }

    /// The years the entry is published for, or `[None]` for one file.
    pub fn published_years(&self) -> Vec<Option<u16>> {
        match self.years {
            Some([first, last]) => (first..=last).map(Some).collect(),
            None => vec![None],
        }
    }

    /// Whether the file for `year` may still gain rows: a series' earlier years are closed.
    pub fn partial_for(&self, year: Option<u16>) -> bool {
        self.partial && self.years.is_none_or(|[_, last]| year == Some(last))
    }

    pub fn file_licence(&self) -> DataPackFileLicence {
        DataPackFileLicence {
            id: self.licence.id.clone(),
            url: self.licence.url.clone(),
            attribution: self.licence.attribution.clone(),
            disclaimer: self.licence.disclaimer.clone(),
        }
    }
}

/// The last path segment of `url`, the name a pack stores its file under.
pub(crate) fn file_name(url: &str) -> Option<String> {
    let url = reqwest::Url::parse(url).ok()?;
    url.path_segments()?
        .next_back()
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

static REGISTRY: LazyLock<Vec<RegistryEntry>> = LazyLock::new(|| {
    parse(include_str!("sources.json")).unwrap_or_else(|error| panic!("sources.json: {error}"))
});

/// Every registry entry. `sources.json` is validated by a unit test, so loading cannot fail in a
/// tested build.
pub(crate) fn registry() -> &'static [RegistryEntry] {
    &REGISTRY
}

/// Registry pack names, in first-listed order.
pub(crate) fn registry_packs() -> Vec<&'static str> {
    let mut packs: Vec<&'static str> = Vec::new();
    for entry in registry() {
        if !packs.contains(&entry.pack.as_str()) {
            packs.push(&entry.pack);
        }
    }
    packs
}

fn https(field: &str, value: &str) -> Result<(), String> {
    match reqwest::Url::parse(value) {
        Ok(url) if url.scheme() == "https" && url.host_str().is_some() => Ok(()),
        _ => Err(format!("{field} '{value}' is not an https URL")),
    }
}

fn snake_case(name: &str) -> bool {
    name.starts_with(|ch: char| ch.is_ascii_lowercase())
        && name
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
}

/// Parses and validates a registry. `city_packs` and `city_tables` are names the City catalog
/// already uses, which a registry entry may not reuse.
fn parse_with(
    json: &str,
    city_packs: &[&str],
    city_tables: &[&str],
) -> Result<Vec<RegistryEntry>, String> {
    let entries: Vec<RegistryEntry> = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let mut seen = HashSet::new();
    for entry in &entries {
        let name = format!("{} {} {}", entry.pack, entry.table, entry.url);
        let fail = |reason: String| Err(format!("{name}: {reason}"));
        if !seen.insert((&entry.pack, &entry.table, &entry.url)) {
            return fail("listed twice".into());
        }
        if city_packs.contains(&entry.pack.as_str()) {
            return fail("the pack name is a City pack's".into());
        }
        if city_tables.contains(&entry.table.as_str()) {
            return fail("the table name is a City table's".into());
        }
        if !snake_case(&entry.pack.replace('-', "_")) || !snake_case(&entry.table) {
            return fail("pack must be kebab-case and table snake_case".into());
        }
        for (field, value) in [
            ("title", &entry.title),
            ("publisher", &entry.publisher),
            ("licence id", &entry.licence.id),
            ("licence evidence", &entry.licence.evidence),
        ] {
            if value.trim().is_empty() {
                return fail(format!("{field} is empty"));
            }
        }
        https("page", &entry.page).map_err(|reason| format!("{name}: {reason}"))?;
        https("licence url", &entry.licence.url).map_err(|reason| format!("{name}: {reason}"))?;
        if entry.url.contains("{year}") != entry.years.is_some() {
            return fail("url names {year} exactly when years is set".into());
        }
        if let Some([first, last]) = entry.years {
            if first > last {
                return fail(format!("years {first}..{last} run backwards"));
            }
        }
        for year in entry.published_years() {
            let url = entry.url_for(year);
            https("url", &url).map_err(|reason| format!("{name}: {reason}"))?;
            if file_name(&url).is_none() {
                return fail("url has no file name".into());
            }
        }
        if entry.worksheet.is_some() != (entry.format == RegistryFormat::Xlsx) {
            return fail("a worksheet is named exactly for an xlsx file".into());
        }
        if entry.time.column.is_some() == (entry.time.kind == RegistryTimeKind::Undated) {
            return fail("time names a column exactly when it is not undated".into());
        }
        for header in entry.key.iter().chain(&entry.time.column) {
            if !entry.headers.contains(header) {
                return fail(format!("'{header}' is not one of its headers"));
            }
        }
        if let Some(other) = entries.iter().find(|other| {
            other.table == entry.table && (other.key != entry.key || other.time != entry.time)
        }) {
            return fail(format!(
                "its table's key or time differs from the entry for {}",
                other.url
            ));
        }
    }
    Ok(entries)
}

fn parse(json: &str) -> Result<Vec<RegistryEntry>, String> {
    let city_packs = super::catalog::city_pack_names().collect::<Vec<_>>();
    parse_with(json, &city_packs, &super::conformed::city_tables())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(overrides: serde_json::Value) -> serde_json::Value {
        let mut entry = serde_json::json!({
            "pack": "test-pack", "table": "test_table", "title": "T", "publisher": "P",
            "page": "https://example.test/", "url": "https://example.test/data.csv",
            "years": null, "format": "csv", "worksheet": null,
            "licence": {"id": "CC0-1.0", "url": "https://example.test/l", "attribution": null,
                        "disclaimer": null, "evidence": "quoted"},
            "key": "id", "time": {"column": "year", "kind": "calendar_year"},
            "partial": false, "bytes": 1, "rows": 1, "headers": ["id", "year"], "notes": ""
        });
        for (field, value) in overrides.as_object().unwrap() {
            entry[field] = value.clone();
        }
        entry
    }

    fn check(entries: &[serde_json::Value]) -> Result<Vec<RegistryEntry>, String> {
        parse_with(
            &serde_json::to_string(entries).unwrap(),
            &["san-diego-money"],
            &["police_calls"],
        )
    }

    /// The shipped registry loads, and each malformed entry is refused at load rather than when a
    /// build reaches it.
    #[test]
    fn the_registry_loads_and_refuses_malformed_entries() {
        assert!(!registry().is_empty());
        let ok = entry(serde_json::json!({}));
        assert!(check(std::slice::from_ref(&ok)).is_ok());
        let series = entry(serde_json::json!({
            "url": "https://example.test/{year}.csv", "years": [2020, 2022], "partial": true
        }));
        let loaded = check(std::slice::from_ref(&series)).unwrap();
        assert_eq!(
            loaded[0].url_for(Some(2021)),
            "https://example.test/2021.csv"
        );
        assert_eq!(
            loaded[0]
                .published_years()
                .into_iter()
                .map(|year| loaded[0].partial_for(year))
                .collect::<Vec<_>>(),
            [false, false, true]
        );
        for (refused, reason) in [
            (vec![ok.clone(), ok.clone()], "listed twice"),
            (
                vec![entry(
                    serde_json::json!({"url": "http://example.test/data.csv"}),
                )],
                "https",
            ),
            (
                vec![entry(serde_json::json!({"format": "parquet"}))],
                "unknown variant",
            ),
            (
                vec![entry(
                    serde_json::json!({"years": [2022, 2020], "url": "https://example.test/{year}.csv"}),
                )],
                "backwards",
            ),
            (
                vec![entry(serde_json::json!({"years": [2020, 2022]}))],
                "{year}",
            ),
            (
                vec![entry(
                    serde_json::json!({"url": "https://example.test/{year}.csv"}),
                )],
                "{year}",
            ),
            (
                vec![entry(serde_json::json!({"pack": "san-diego-money"}))],
                "City pack",
            ),
            (
                vec![entry(serde_json::json!({"table": "police_calls"}))],
                "City table",
            ),
            (vec![entry(serde_json::json!({"key": "ID"}))], "headers"),
            (
                vec![entry(serde_json::json!({"worksheet": "Sheet1"}))],
                "worksheet",
            ),
            (
                vec![entry(
                    serde_json::json!({"time": {"column": null, "kind": "recorded_at"}}),
                )],
                "column",
            ),
            (
                vec![
                    ok.clone(),
                    entry(serde_json::json!({"url": "https://example.test/b.csv", "key": null})),
                ],
                "differs",
            ),
            (
                vec![entry(serde_json::json!({"extra": 1}))],
                "unknown field",
            ),
        ] {
            let error = check(&refused)
                .err()
                .unwrap_or_else(|| panic!("accepted: {reason}"));
            assert!(error.contains(reason), "{reason}: {error}");
        }
    }
}
