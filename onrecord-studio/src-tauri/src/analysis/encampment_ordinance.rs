//! Observations over imported tables for SDMC 63.0404 (unauthorized encampments) and 63.0405(b)
//! (no criminal citation under 63.0404(a) between 9:00 p.m. and 5:30 a.m., or when no shelter is
//! reasonably available).
//!
//! Nothing here classifies an event as unlawful. Shelter availability, sign postings and the City
//! Manager's (c)(5) park designations are in no imported table, and the distance bands are a
//! stand-in for "two blocks", not a distance the ordinance states. Times are compared as the
//! publisher recorded them (Pacific wall-clock), never converted.

use crate::error::AppError;
use crate::types::{
    EncampmentIncidentDistance, EncampmentLocationObservation, EncampmentOrdinanceReport,
    LocationSectionCount, NearestFeature, NightHoursObservation, NightHoursSectionCount,
    ObservationSource, RecordedActivity,
};
use chrono::{NaiveDateTime, Timelike};
use duckdb::{params, Connection, OptionalExt};
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub(crate) const SCHOOLS_SOURCE_URI: &str = "https://geo.sandag.org/server/rest/services/Hosted/Schools/FeatureServer/0/query?where=1%3D1&outFields=*&outSR=4326&f=geojson";
pub(crate) const TRANSIT_STOPS_SOURCE_URI: &str =
    "https://geo.sandag.org/server/rest/directories/downloads/Transit_Stops_GTFS.csv";
/// A quarter mile, a common reading of "two blocks"; the ordinance gives no distance.
pub(crate) const NEAR_BAND_FEET: f64 = 1_320.0;
pub(crate) const FAR_BAND_FEET: f64 = 2_640.0;
const WINDOW_START_SECONDS: u32 = 21 * 3600;
const WINDOW_END_SECONDS: u32 = 5 * 3600 + 30 * 60;
const EARTH_RADIUS_FEET: f64 = 6_371_008.8 / 0.3048;

/// A snapshot, its table, and the stored column alias for each lowercased original header.
type ImportedTable = (ObservationSource, String, HashMap<String, String>);
/// A layer's source and its named points as (latitude, longitude).
type PointLayer = (ObservationSource, Vec<(String, (f64, f64))>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TimeOfDay {
    Inside,
    /// Exactly 9:00 p.m. or 5:30 a.m.: the ordinance does not say whether "between" includes them.
    Boundary,
    Outside,
    NotRecorded,
}

fn recorded_datetime(value: &str) -> Option<NaiveDateTime> {
    let value = value.trim();
    [
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S",
        "%m/%d/%Y %H:%M:%S",
        "%m/%d/%Y %H:%M",
    ]
    .iter()
    .find_map(|format| NaiveDateTime::parse_from_str(value, format).ok())
}

fn time_of_day(at: Option<NaiveDateTime>) -> TimeOfDay {
    let Some(at) = at else {
        return TimeOfDay::NotRecorded;
    };
    match at.num_seconds_from_midnight() {
        WINDOW_START_SECONDS | WINDOW_END_SECONDS => TimeOfDay::Boundary,
        seconds if !(WINDOW_END_SECONDS..=WINDOW_START_SECONDS).contains(&seconds) => {
            TimeOfDay::Inside
        }
        _ => TimeOfDay::Outside,
    }
}

fn distance_feet((lat_a, lon_a): (f64, f64), (lat_b, lon_b): (f64, f64)) -> f64 {
    let (phi_a, phi_b) = (lat_a.to_radians(), lat_b.to_radians());
    let half_dphi = (phi_b - phi_a) / 2.0;
    let half_dlambda = (lon_b - lon_a).to_radians() / 2.0;
    let h = half_dphi.sin().powi(2) + phi_a.cos() * phi_b.cos() * half_dlambda.sin().powi(2);
    2.0 * EARTH_RADIUS_FEET * h.sqrt().min(1.0).asin()
}

/// The first feature at the least distance. A feature is skipped without the full formula when
/// its latitude difference alone, a lower bound on great-circle distance, is already farther than
/// the best so far.
fn nearest(point: (f64, f64), features: &[(String, (f64, f64))]) -> Option<NearestFeature> {
    let mut best: Option<(&String, f64)> = None;
    for (name, at) in features {
        if best.is_some_and(|(_, distance)| {
            (at.0 - point.0).abs().to_radians() * EARTH_RADIUS_FEET > distance
        }) {
            continue;
        }
        let distance = distance_feet(point, *at);
        if best.is_none_or(|(_, shortest)| distance < shortest) {
            best = Some((name, distance));
        }
    }
    best.map(|(name, distance)| NearestFeature {
        name: name.clone(),
        distance_feet: distance.round(),
    })
}

fn coordinate(value: Option<&str>, range: std::ops::RangeInclusive<f64>) -> Option<f64> {
    value
        .and_then(|value| value.trim().parse::<f64>().ok())
        .filter(|value| *value != 0.0 && range.contains(value))
}

fn storage(error: impl std::fmt::Display) -> AppError {
    AppError::Storage(error.to_string())
}

fn quote(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

/// Imported tables whose original headers include every `required` name, with the stored column
/// alias for each original header (lowercased).
fn tables_with_columns(
    conn: &Connection,
    filter: Option<(&str, &str)>,
    required: &[&str],
) -> Result<Vec<ImportedTable>, AppError> {
    let mut statement = conn
        .prepare(&format!(
            "SELECT s.snapshot_id, s.display_name, s.source_sha256, s.table_name,
                lower(c.original_name), c.column_name
             FROM analytical_snapshots s JOIN analytical_columns c ON c.snapshot_id = s.snapshot_id
             WHERE {} ORDER BY s.retrieved_at DESC, s.snapshot_id",
            filter.map_or("?1 IS NULL", |(clause, _)| clause)
        ))
        .map_err(storage)?;
    let mut tables: Vec<ImportedTable> = Vec::new();
    let rows = statement
        .query_map(params![filter.map(|(_, value)| value)], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .map_err(storage)?;
    for row in rows {
        let (snapshot_id, display_name, source_sha256, table, original, alias) =
            row.map_err(storage)?;
        let index = match tables.iter().position(|t| t.0.snapshot_id == snapshot_id) {
            Some(index) => index,
            None => {
                tables.push((
                    ObservationSource {
                        snapshot_id,
                        display_name,
                        source_sha256,
                    },
                    table,
                    HashMap::new(),
                ));
                tables.len() - 1
            }
        };
        tables[index]
            .2
            .insert(original.unwrap_or_else(|| alias.clone()), alias);
    }
    tables.retain(|(_, _, columns)| required.iter().all(|name| columns.contains_key(*name)));
    Ok(tables)
}

fn select_text(
    conn: &Connection,
    table: &str,
    columns: &[Option<&String>],
    filter: Option<(&String, &str)>,
) -> Result<Vec<Vec<Option<String>>>, AppError> {
    let projection = columns
        .iter()
        .map(|column| match column {
            Some(column) => format!("CAST({} AS VARCHAR)", quote(column)),
            None => "NULL".into(),
        })
        .collect::<Vec<_>>()
        .join(", ");
    let condition = filter
        .map(|(column, prefix)| {
            format!(
                " WHERE upper(trim(CAST({} AS VARCHAR))) LIKE '{prefix}%'",
                quote(column)
            )
        })
        .unwrap_or_default();
    let mut statement = conn
        .prepare(&format!(
            "SELECT {projection} FROM {}{condition}",
            quote(table)
        ))
        .map_err(storage)?;
    let width = columns.len();
    let rows = statement
        .query_map([], |row| {
            (0..width)
                .map(|index| row.get::<_, Option<String>>(index))
                .collect::<Result<Vec<_>, _>>()
        })
        .map_err(storage)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(storage)
}

fn night_hours(conn: &Connection) -> Result<Option<NightHoursObservation>, AppError> {
    let logs = tables_with_columns(
        conn,
        None,
        &["activitynumber", "activitydate", "violationsection"],
    )?;
    if logs.is_empty() {
        return Ok(None);
    }
    let mut seen = BTreeSet::new();
    let mut sections: BTreeMap<(String, String), NightHoursSectionCount> = BTreeMap::new();
    let mut listed = Vec::new();
    let (mut first, mut last): (Option<NaiveDateTime>, Option<NaiveDateTime>) = (None, None);
    let mut sources = Vec::new();
    for (source, table, columns) in &logs {
        let get = |name: &str| columns.get(name);
        let (Some(date_column), Some(section_column)) =
            (get("activitydate"), get("violationsection"))
        else {
            continue;
        };
        let rows = select_text(
            conn,
            table,
            &[
                get("activitynumber"),
                get("activitytype"),
                Some(date_column),
                Some(section_column),
                get("address"),
                get("streetnumber"),
                get("streetdirection"),
                get("streetname"),
                get("streettype"),
                get("beat"),
            ],
            Some((section_column, "63.0404")),
        )?;
        let (earliest, latest) = conn
            .query_row(
                &format!(
                    "SELECT strftime(min(t), '%Y-%m-%d %H:%M:%S'), strftime(max(t), '%Y-%m-%d %H:%M:%S')
                     FROM (SELECT try_strptime(CAST({} AS VARCHAR), '%Y-%m-%d %H:%M:%S') AS t FROM {})",
                    quote(date_column),
                    quote(table)
                ),
                [],
                |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .map_err(storage)?;
        for bound in [earliest, latest].iter().flatten() {
            if let Some(at) = recorded_datetime(bound) {
                first = Some(first.map_or(at, |first| first.min(at)));
                last = Some(last.map_or(at, |last| last.max(at)));
            }
        }
        let parsed = rows
            .iter()
            .map(|row| row[2].as_deref().and_then(recorded_datetime))
            .collect::<Vec<_>>();
        // A date-only export rendered with a zero time must never read as midnight activity.
        let has_time_of_day = parsed
            .iter()
            .flatten()
            .any(|at| at.num_seconds_from_midnight() != 0);
        for (row, at) in rows.iter().zip(parsed) {
            let at = at.filter(|_| has_time_of_day);
            let text = |index: usize| row[index].clone().unwrap_or_default();
            let section = text(3).trim().to_ascii_uppercase();
            let undated_key = if text(0).is_empty() {
                text(2)
            } else {
                String::new()
            };
            if !seen.insert((text(0), section.clone(), undated_key)) {
                continue;
            }
            let activity_type = text(1).trim().to_ascii_uppercase();
            let count = sections
                .entry((section.clone(), activity_type.clone()))
                .or_insert_with(|| NightHoursSectionCount {
                    section: section.clone(),
                    activity_type: activity_type.clone(),
                    activities: 0,
                    inside_window: 0,
                    at_window_boundary: 0,
                    outside_window: 0,
                    time_not_recorded: 0,
                });
            count.activities += 1;
            let class = time_of_day(at);
            match class {
                TimeOfDay::Inside => count.inside_window += 1,
                TimeOfDay::Boundary => count.at_window_boundary += 1,
                TimeOfDay::Outside => count.outside_window += 1,
                TimeOfDay::NotRecorded => count.time_not_recorded += 1,
            }
            if matches!(class, TimeOfDay::Inside | TimeOfDay::Boundary) {
                let street = (5..=8)
                    .filter_map(|index| row[index].as_deref())
                    .map(str::trim)
                    .filter(|part| !part.is_empty() && !part.eq_ignore_ascii_case("null"))
                    .collect::<Vec<_>>()
                    .join(" ");
                let mut location = row[4]
                    .clone()
                    .filter(|a| !a.trim().is_empty())
                    .unwrap_or(street);
                if let Some(beat) = row[9].as_deref().filter(|b| !b.trim().is_empty()) {
                    location = format!("{location} (beat {beat})");
                }
                listed.push(RecordedActivity {
                    activity_number: text(0),
                    activity_type,
                    section,
                    recorded_at: text(2),
                    at_window_boundary: class == TimeOfDay::Boundary,
                    location,
                });
            }
        }
        sources.push(source.clone());
    }
    listed.sort_by(|a, b| a.recorded_at.cmp(&b.recorded_at));
    Ok(Some(NightHoursObservation {
        sources,
        first_activity: first.map(|at| at.format("%Y-%m-%d %H:%M").to_string()),
        last_activity: last.map(|at| at.format("%Y-%m-%d %H:%M").to_string()),
        sections: sections.into_values().collect(),
        listed_activities: listed,
    }))
}

fn point_layer(conn: &Connection, source_uri: &str) -> Result<Option<ImportedTable>, AppError> {
    let snapshot = conn
        .query_row(
            "SELECT snapshot_id FROM analytical_snapshots WHERE source_uri = ?1
             ORDER BY retrieved_at DESC, snapshot_id LIMIT 1",
            params![source_uri],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(storage)?;
    let Some(snapshot) = snapshot else {
        return Ok(None);
    };
    Ok(
        tables_with_columns(conn, Some(("s.snapshot_id = ?1", &snapshot)), &[])?
            .into_iter()
            .next(),
    )
}

fn schools(conn: &Connection) -> Result<Option<PointLayer>, AppError> {
    let Some((source, table, columns)) = point_layer(conn, SCHOOLS_SOURCE_URI)? else {
        return Ok(None);
    };
    let rows = select_text(
        conn,
        &table,
        &[columns.get("name"), columns.get("_geometry")],
        None,
    )?;
    let points = rows
        .into_iter()
        .filter_map(|row| {
            let geometry: serde_json::Value = serde_json::from_str(row[1].as_deref()?).ok()?;
            let coordinates = geometry.get("coordinates")?.as_array()?;
            let lon = coordinates.first()?.as_f64()?;
            let lat = coordinates.get(1)?.as_f64()?;
            Some((row[0].clone().unwrap_or_default(), (lat, lon)))
        })
        .collect();
    Ok(Some((source, points)))
}

fn stations(conn: &Connection) -> Result<Option<PointLayer>, AppError> {
    let Some((source, table, columns)) = point_layer(conn, TRANSIT_STOPS_SOURCE_URI)? else {
        return Ok(None);
    };
    let rows = select_text(
        conn,
        &table,
        &[
            columns.get("stop_name"),
            columns.get("stop_lat"),
            columns.get("stop_lon"),
            columns.get("location_type"),
        ],
        None,
    )?;
    let points = rows
        .into_iter()
        .filter(|row| {
            row[3]
                .as_deref()
                .and_then(|value| value.trim().parse::<f64>().ok())
                == Some(1.0)
        })
        .filter_map(|row| {
            let lat = coordinate(row[1].as_deref(), -90.0..=90.0)?;
            let lon = coordinate(row[2].as_deref(), -180.0..=180.0)?;
            Some((row[0].clone().unwrap_or_default(), (lat, lon)))
        })
        .collect();
    Ok(Some((source, points)))
}

fn locations(
    conn: &Connection,
    not_measured: &mut Vec<String>,
) -> Result<Option<EncampmentLocationObservation>, AppError> {
    let incident_tables = tables_with_columns(
        conn,
        Some(("s.dataset_kind = ?1", "NIBRS_OFFENSES")),
        &["nibrs_uniq", "case_number", "code_section"],
    )?;
    if incident_tables.is_empty() {
        return Ok(None);
    }
    let schools = schools(conn)?;
    let stations = stations(conn)?;
    if schools.is_none() {
        not_measured.push("Distance to schools: fetch SANDAG schools to measure it.".into());
    }
    if stations.is_none() {
        not_measured
            .push("Distance to transit stations: fetch SANDAG transit stops to measure it.".into());
    }
    let banded = schools.is_some() && stations.is_some();
    let mut seen = BTreeSet::new();
    let mut incidents = Vec::new();
    let mut sources = Vec::new();
    for (source, table, columns) in &incident_tables {
        let get = |name: &str| columns.get(name);
        let Some(section_column) = get("code_section") else {
            continue;
        };
        let rows = select_text(
            conn,
            table,
            &[
                get("nibrs_uniq"),
                get("case_number"),
                get("occured_on"),
                Some(section_column),
                get("block_addr"),
                get("latitude"),
                get("longitude"),
            ],
            Some((section_column, "63.0404")),
        )?;
        for row in rows {
            let text = |index: usize| row[index].clone().unwrap_or_default();
            if !seen.insert(text(0)) {
                continue;
            }
            let latitude = coordinate(row[5].as_deref(), -90.0..=90.0);
            let longitude = coordinate(row[6].as_deref(), -180.0..=180.0);
            let point = latitude.zip(longitude);
            let section = text(3);
            incidents.push(EncampmentIncidentDistance {
                case_number: text(1),
                occurred_on: row[2].clone(),
                section: section
                    .split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .to_ascii_uppercase(),
                block_address: text(4),
                latitude,
                longitude,
                nearest_school: point
                    .and_then(|point| schools.as_ref().and_then(|(_, s)| nearest(point, s))),
                nearest_station: point
                    .and_then(|point| stations.as_ref().and_then(|(_, s)| nearest(point, s))),
            });
        }
        sources.push(source.clone());
    }
    let mut sections: BTreeMap<String, (LocationSectionCount, BTreeSet<String>)> = BTreeMap::new();
    for incident in &incidents {
        let (count, cases) = sections.entry(incident.section.clone()).or_insert_with(|| {
            (
                LocationSectionCount {
                    section: incident.section.clone(),
                    offenses: 0,
                    distinct_cases: 0,
                    not_geocoded: 0,
                    within_near_band: banded.then_some(0),
                    within_far_band: banded.then_some(0),
                    beyond_far_band: banded.then_some(0),
                },
                BTreeSet::new(),
            )
        });
        count.offenses += 1;
        cases.insert(incident.case_number.clone());
        let closest = incident
            .nearest_school
            .iter()
            .chain(incident.nearest_station.iter())
            .map(|feature| feature.distance_feet)
            .min_by(f64::total_cmp);
        match closest {
            None if incident.latitude.is_none() || incident.longitude.is_none() => {
                count.not_geocoded += 1
            }
            Some(feet) if banded => {
                let band = if feet <= NEAR_BAND_FEET {
                    &mut count.within_near_band
                } else if feet <= FAR_BAND_FEET {
                    &mut count.within_far_band
                } else {
                    &mut count.beyond_far_band
                };
                *band = band.map(|n| n + 1);
            }
            _ => {}
        }
    }
    incidents.sort_by(|a, b| a.occurred_on.cmp(&b.occurred_on));
    Ok(Some(EncampmentLocationObservation {
        incident_sources: sources,
        school_source: schools.map(|(source, _)| source),
        station_source: stations.map(|(source, _)| source),
        sections: sections
            .into_values()
            .map(|(mut count, cases)| {
                count.distinct_cases = cases.len();
                count
            })
            .collect(),
        incidents,
    }))
}

pub(crate) fn report(conn: &Connection) -> Result<EncampmentOrdinanceReport, AppError> {
    let mut not_measured = vec![
        "Shelter availability at the time of each citation (63.0405(b)): no imported table holds it.".to_string(),
        "Sign postings required by 63.0404(c)(1), (c)(2) and (c)(4): no imported table holds sign locations.".to_string(),
        "City Manager park designations under 63.0404(c)(5): not published as data.".to_string(),
        "Shelter locations for 63.0404(c)(2): not published as data.".to_string(),
    ];
    let night_hours = night_hours(conn)?;
    if night_hours.is_none() {
        not_measured.push("Time of day of 63.0404 citations: import an SDPD arrests/citations log (columns activityNumber, activityDate, violationSection).".into());
    }
    let locations = locations(conn, &mut not_measured)?;
    if locations.is_none() {
        not_measured.push("Where 63.0404 incidents occurred: fetch SDPD NIBRS offenses.".into());
    }
    Ok(EncampmentOrdinanceReport {
        night_hours,
        locations,
        not_measured,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::DuckDbRepository;
    use crate::types::{AnalyticalDatasetKind, AnalyticalImportRequest};
    use std::path::Path;

    fn at(value: &str) -> TimeOfDay {
        time_of_day(recorded_datetime(value))
    }

    #[test]
    fn pruning_finds_the_same_nearest_feature_as_measuring_every_one() {
        // A deterministic spread over San Diego: 2,000 features, 500 points.
        let mut seed = 0x2545_f491_4f6c_dd1d_u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % 1_000_000) as f64 / 1_000_000.0
        };
        let mut place = || (32.5 + next() * 0.6, -117.3 + next() * 0.4);
        let features = (0..2000)
            .map(|i| (format!("f{i}"), place()))
            .collect::<Vec<_>>();
        for _ in 0..500 {
            let point = place();
            let measured = features
                .iter()
                .map(|(name, at)| (name, distance_feet(point, *at)))
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .unwrap();
            let pruned = nearest(point, &features).unwrap();
            assert_eq!(
                (pruned.name.as_str(), pruned.distance_feet),
                (measured.0.as_str(), measured.1.round())
            );
        }
    }

    #[test]
    fn the_window_endpoints_are_listed_apart_from_both_sides() {
        assert_eq!(at("2025-03-01 20:59:59"), TimeOfDay::Outside);
        assert_eq!(at("2025-03-01 21:00:00"), TimeOfDay::Boundary);
        assert_eq!(at("2025-03-01 21:00:01"), TimeOfDay::Inside);
        assert_eq!(at("2025-03-01 05:29:59"), TimeOfDay::Inside);
        assert_eq!(at("2025-03-01 05:30:00"), TimeOfDay::Boundary);
        assert_eq!(at("2025-03-01 05:30:01"), TimeOfDay::Outside);
        assert_eq!(at("2025-03-01"), TimeOfDay::NotRecorded);
    }

    async fn import(
        repo: &DuckDbRepository,
        directory: &Path,
        file_name: &str,
        body: &str,
        kind: AnalyticalDatasetKind,
        source_uri: Option<&str>,
    ) -> crate::types::AnalyticalImportResult {
        let path = directory.join(file_name);
        std::fs::write(&path, body).unwrap();
        repo.import_analytical_csv(AnalyticalImportRequest {
            dataset_kind: kind,
            display_name: file_name.into(),
            source_uri: source_uri.map(str::to_owned),
            local_path: path.to_string_lossy().into_owned(),
            observation_period_start: None,
            observation_period_end: None,
            partial_dataset: false,
            worksheet: None,
            source_file_name: Some(file_name.into()),
            expected_source_sha256: None,
            derived_subset: None,
        })
        .await
        .unwrap()
    }

    async fn repository() -> DuckDbRepository {
        let repo = DuckDbRepository::new(":memory:").unwrap();
        repo.run_migrations(include_str!("../../migrations/0001_initial_schema.sql"))
            .await
            .unwrap();
        repo
    }

    const LOG_HEADER: &str =
        "activityNumber,ActivityType,activityDate,violationSection,streetNumber,streetName,beat\n";

    #[tokio::test]
    async fn overlapping_logs_count_an_activity_once_and_a_date_only_log_is_never_night() {
        let directory = tempfile::tempdir().unwrap();
        let repo = repository().await;
        import(
            &repo,
            directory.path(),
            "early.csv",
            &format!(
                "{LOG_HEADER}\
            1,CITATION,2025-01-02 22:15:00,63.0404(A),100,MAIN,521\n\
            1,CITATION,2025-01-02 22:15:00,647(E),100,MAIN,521\n\
            2,CITATION,2025-01-03 21:00:00,63.0404(A),200,PARK,521\n"
            ),
            AnalyticalDatasetKind::RegionalData,
            None,
        )
        .await;
        import(
            &repo,
            directory.path(),
            "late.csv",
            &format!(
                "{LOG_HEADER}\
            1,CITATION,2025-01-02 22:15:00,63.0404(A),100,MAIN,521\n\
            3,ARREST,2025-02-01 14:00:00,63.0404(C),300,ELM,122\n"
            ),
            AnalyticalDatasetKind::RegionalData,
            None,
        )
        .await;
        let report = repo.encampment_ordinance_report().await.unwrap();
        let night = report.night_hours.unwrap();
        let a = night
            .sections
            .iter()
            .find(|s| s.section == "63.0404(A)")
            .unwrap();
        assert_eq!(
            (
                a.activities,
                a.inside_window,
                a.at_window_boundary,
                a.outside_window
            ),
            (2, 1, 1, 0)
        );
        let c = night
            .sections
            .iter()
            .find(|s| s.section == "63.0404(C)")
            .unwrap();
        assert_eq!((c.activity_type.as_str(), c.outside_window), ("ARREST", 1));
        assert_eq!(night.listed_activities.len(), 2);
        assert_eq!(night.first_activity.as_deref(), Some("2025-01-02 22:15"));
        assert!(report.locations.is_none());

        let repo = repository().await;
        import(
            &repo,
            directory.path(),
            "dates.csv",
            &format!(
                "{LOG_HEADER}\
            4,CITATION,2025-01-02,63.0404(A),100,MAIN,521\n"
            ),
            AnalyticalDatasetKind::RegionalData,
            None,
        )
        .await;
        let night = repo
            .encampment_ordinance_report()
            .await
            .unwrap()
            .night_hours
            .unwrap();
        assert_eq!(night.sections[0].time_not_recorded, 1);
        assert!(night.listed_activities.is_empty());
    }

    // Header and first data row as published in pd_nibrs_2025_datasd.csv, then two rows moved to
    // known distances north of the one school: 0.003 deg (about 1,093 ft) and 0.01 deg (3,645 ft).
    const NIBRS: &str = "\"objectid\",\"nibrs_uniq\",\"case_number\",\"occured_on\",\"approved_on\",\"day_of_week\",\"month\",\"year\",\"code_section\",\"group_type\",\"ibr_category\",\"crime_against\",\"ibr_offense\",\"ibr_offense_description\",\"pd_offense_category\",\"violent_crime\",\"property_crime\",\"beat\",\"neighborhood\",\"service_area\",\"division_number\",\"division\",\"block_addr\",\"city\",\"state\",\"zip\",\"geocode_status\",\"geocode_score\",\"latitude\",\"longitude\"
\"414069\",\"3279672_90Z\",\"25000055\",\"2025-01-01\",\"2025-01-01 12:11:40\",\"4\",\"1\",\"2025\",\"63.0404(c) SDMC Unauthorized Encampments- Within Prohibited Areas || \",\"B\",\"GROUP \"\"B\"\" OFFENSES\",\"\",\"90Z\",\"All Other Offenses\",\"All Other Offenses\",\"0\",\"0\",\"121\",\"Mission Beach\",\"120.00000000\",\"1.00000000\",\"Northern\",\"1100 West Mission Bay DR\",\"SD\",\"CA\",\"92109\",\"M\",\"100.00000000\",\"32.773\",\"-117.245\"
\"414070\",\"3279673_90Z\",\"25000056\",\"2025-01-02\",\"2025-01-02 12:11:40\",\"4\",\"1\",\"2025\",\"63.0404(c) SDMC Unauthorized Encampments- Within Prohibited Areas || \",\"B\",\"GROUP \"\"B\"\" OFFENSES\",\"\",\"90Z\",\"All Other Offenses\",\"All Other Offenses\",\"0\",\"0\",\"121\",\"Mission Beach\",\"120.00000000\",\"1.00000000\",\"Northern\",\"1200 West Mission Bay DR\",\"SD\",\"CA\",\"92109\",\"M\",\"100.00000000\",\"32.78\",\"-117.245\"
\"414071\",\"3279674_90Z\",\"25000057\",\"2025-01-03\",\"2025-01-03 12:11:40\",\"4\",\"1\",\"2025\",\"63.0404(a) SDMC Unauthorized Encampments on Public Property || \",\"B\",\"GROUP \"\"B\"\" OFFENSES\",\"\",\"90Z\",\"All Other Offenses\",\"All Other Offenses\",\"0\",\"0\",\"121\",\"Mission Beach\",\"120.00000000\",\"1.00000000\",\"Northern\",\"1300 West Mission Bay DR\",\"SD\",\"CA\",\"92109\",\"U\",\"0E-8\",\"\",\"\"
";

    #[tokio::test]
    async fn distance_bands_need_both_layers_and_published_nibrs_rows_import_whole() {
        let directory = tempfile::tempdir().unwrap();
        let repo = repository().await;
        let nibrs = import(
            &repo,
            directory.path(),
            "pd_nibrs_2025_datasd.csv",
            NIBRS,
            AnalyticalDatasetKind::NibrsOffenses,
            None,
        )
        .await;
        assert_eq!((nibrs.imported_rows, nibrs.rejected_rows), (3, 0));
        import(&repo, directory.path(), "schools.geojson",
            r#"{"type":"FeatureCollection","features":[{"type":"Feature","id":1,"properties":{"name":"Mission Bay High"},"geometry":{"type":"Point","coordinates":[-117.245,32.77]}}]}"#,
            AnalyticalDatasetKind::RegionalData, Some(SCHOOLS_SOURCE_URI)).await;
        let report = repo.encampment_ordinance_report().await.unwrap();
        let locations = report.locations.unwrap();
        let c = locations
            .sections
            .iter()
            .find(|s| s.section == "63.0404(C)")
            .unwrap();
        assert_eq!(
            (c.offenses, c.within_near_band, c.beyond_far_band),
            (2, None, None)
        );
        assert!(report
            .not_measured
            .iter()
            .any(|note| note.contains("transit stations")));
        let near = locations
            .incidents
            .iter()
            .find(|i| i.case_number == "25000055")
            .unwrap();
        let feet = near.nearest_school.as_ref().unwrap().distance_feet;
        assert!((1_080.0..1_110.0).contains(&feet), "{feet}");

        import(&repo, directory.path(), "Transit_Stops_GTFS.csv",
            "\u{feff}OID_,stop_name,stop_lat,stop_lon,location_type\n1,Far Station,33.5,-117.245,1\n2,Bus Stop Beside School,32.7731,-117.245,0\n",
            AnalyticalDatasetKind::RegionalData, Some(TRANSIT_STOPS_SOURCE_URI)).await;
        let locations = repo
            .encampment_ordinance_report()
            .await
            .unwrap()
            .locations
            .unwrap();
        let c = locations
            .sections
            .iter()
            .find(|s| s.section == "63.0404(C)")
            .unwrap();
        assert_eq!(
            (c.within_near_band, c.within_far_band, c.beyond_far_band),
            (Some(1), Some(0), Some(1))
        );
        let a = locations
            .sections
            .iter()
            .find(|s| s.section == "63.0404(A)")
            .unwrap();
        assert_eq!((a.not_geocoded, a.within_near_band), (1, Some(0)));
        assert_eq!(
            locations.incidents[0]
                .nearest_station
                .as_ref()
                .unwrap()
                .name,
            "Far Station"
        );
    }
}

/// Measures the report over real published files: `ONRECORD_ORDINANCE_DIR` holding
/// `pd_nibrs_2025_datasd.csv`, `arrests.xlsx` (an SDPD ArrestsCites release), `schools.geojson`
/// and `Transit_Stops_GTFS.csv`, as fetched from the URLs in this module and in `city_datasets`.
#[cfg(test)]
mod live {
    use super::*;
    use crate::storage::DuckDbRepository;
    use crate::types::{AnalyticalDatasetKind, AnalyticalImportRequest};

    #[tokio::test]
    #[ignore = "needs ONRECORD_ORDINANCE_DIR with published files"]
    async fn measure_on_published_files() {
        let directory = std::path::PathBuf::from(std::env::var("ONRECORD_ORDINANCE_DIR").unwrap());
        let repo = DuckDbRepository::new(":memory:").unwrap();
        repo.run_migrations(include_str!("../../migrations/0001_initial_schema.sql"))
            .await
            .unwrap();
        for (file, kind, uri) in [
            (
                "pd_nibrs_2025_datasd.csv",
                AnalyticalDatasetKind::NibrsOffenses,
                None,
            ),
            ("arrests.xlsx", AnalyticalDatasetKind::RegionalData, None),
            (
                "schools.geojson",
                AnalyticalDatasetKind::RegionalData,
                Some(SCHOOLS_SOURCE_URI),
            ),
            (
                "Transit_Stops_GTFS.csv",
                AnalyticalDatasetKind::RegionalData,
                Some(TRANSIT_STOPS_SOURCE_URI),
            ),
        ] {
            let result = repo
                .import_analytical_csv(AnalyticalImportRequest {
                    dataset_kind: kind,
                    display_name: file.into(),
                    source_uri: uri.map(str::to_owned),
                    local_path: directory.join(file).to_string_lossy().into_owned(),
                    observation_period_start: None,
                    observation_period_end: None,
                    partial_dataset: false,
                    worksheet: None,
                    source_file_name: Some(file.into()),
                    expected_source_sha256: None,
                    derived_subset: None,
                })
                .await
                .unwrap();
            println!(
                "{file}: {} imported, {} rejected",
                result.imported_rows, result.rejected_rows
            );
        }
        let report = repo.encampment_ordinance_report().await.unwrap();
        let night = report.night_hours.unwrap();
        println!(
            "logs {:?} to {:?}",
            night.first_activity, night.last_activity
        );
        for row in &night.sections {
            println!("{row:?}");
        }
        for activity in &night.listed_activities {
            println!("{activity:?}");
        }
        let locations = report.locations.unwrap();
        for row in &locations.sections {
            println!("{row:?}");
        }
        println!("not measured: {:?}", report.not_measured);
    }
}
