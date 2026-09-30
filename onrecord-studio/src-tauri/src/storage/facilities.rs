use crate::{
    analysis::facility_coverage::{parse_daily_hours, summarize_area},
    error::AppError,
    storage::sqlite::DuckDbRepository,
    types::*,
};
use duckdb::params;
use std::collections::BTreeSet;

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS facility_coverage_mappings (snapshot_id TEXT PRIMARY KEY, mapping_json TEXT NOT NULL, updated_at TEXT NOT NULL);";
const MAX_AREAS: usize = 50;
const MAX_IDS_PER_AREA: usize = 5000;
const MAX_CATEGORIES: usize = 100;
const MAX_TEXT: usize = 200;

fn error(e: impl std::fmt::Display) -> AppError {
    AppError::Storage(e.to_string())
}
fn invalid(message: impl Into<String>) -> AppError {
    AppError::DataParsing(message.into())
}
fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}
fn bounded(value: &str, label: &str) -> Result<(), AppError> {
    if value.trim().is_empty() || value.len() > MAX_TEXT {
        return Err(invalid(format!(
            "{label} must be between 1 and {MAX_TEXT} characters"
        )));
    }
    Ok(())
}

fn validate_mapping(
    mapping: &FacilityCoverageMapping,
    schema: &AnalyticalSchema,
) -> Result<AnalyticalDatasetSummary, AppError> {
    let dataset = schema
        .datasets
        .iter()
        .find(|d| d.snapshot_id == mapping.snapshot_id)
        .ok_or_else(|| invalid("The selected facility list snapshot no longer exists"))?;
    let columns = [
        Some(&mapping.name_column),
        mapping.id_column.as_ref(),
        mapping.address_column.as_ref(),
        mapping.latitude_column.as_ref(),
        mapping.longitude_column.as_ref(),
        mapping.hours_column.as_ref(),
        mapping.category_column.as_ref(),
    ];
    for column in columns.into_iter().flatten() {
        if !schema
            .columns
            .iter()
            .any(|c| c.table_name == dataset.table_name && c.column_name == *column)
        {
            return Err(invalid(format!(
                "Column {column} does not exist in the selected facility list"
            )));
        }
    }
    let coordinates = match (&mapping.latitude_column, &mapping.longitude_column) {
        (Some(_), Some(_)) => true,
        (None, None) => false,
        _ => return Err(invalid("Map both latitude and longitude, or neither")),
    };
    if !coordinates && mapping.address_column.is_none() {
        return Err(invalid(
            "Map an address column or both latitude and longitude columns",
        ));
    }
    match (&mapping.category_column, mapping.restroom_categories.len()) {
        (Some(_), 0) => {
            return Err(invalid(
                "List the category values that identify a public restroom",
            ))
        }
        (None, n) if n > 0 => {
            return Err(invalid(
                "Restroom category values require a mapped category column",
            ))
        }
        (_, n) if n > MAX_CATEGORIES => {
            return Err(invalid(format!(
                "At most {MAX_CATEGORIES} restroom category values"
            )))
        }
        _ => {}
    }
    for category in &mapping.restroom_categories {
        bounded(category, "A restroom category value")?;
    }
    if mapping.areas.is_empty() || mapping.areas.len() > MAX_AREAS {
        return Err(invalid(format!("Define between 1 and {MAX_AREAS} areas")));
    }
    let mut names = BTreeSet::new();
    for area in &mapping.areas {
        bounded(&area.name, "An area name")?;
        if !names.insert(area.name.trim().to_lowercase()) {
            return Err(invalid(format!("Area {} is defined twice", area.name)));
        }
        match (&area.bounding_box, area.facility_ids.is_empty()) {
            (Some(bbox), true) => {
                if !coordinates {
                    return Err(invalid(format!(
                        "Area {} uses a bounding box, which requires mapped latitude and longitude",
                        area.name
                    )));
                }
                let values = [
                    bbox.min_latitude,
                    bbox.max_latitude,
                    bbox.min_longitude,
                    bbox.max_longitude,
                ];
                if values.iter().any(|v| !v.is_finite())
                    || !(-90.0..=90.0).contains(&bbox.min_latitude)
                    || !(-90.0..=90.0).contains(&bbox.max_latitude)
                    || !(-180.0..=180.0).contains(&bbox.min_longitude)
                    || !(-180.0..=180.0).contains(&bbox.max_longitude)
                    || bbox.min_latitude > bbox.max_latitude
                    || bbox.min_longitude > bbox.max_longitude
                {
                    return Err(invalid(format!(
                        "Area {} needs a bounding box with minimums at or below maximums, latitude within ±90 and longitude within ±180",
                        area.name
                    )));
                }
            }
            (None, false) => {
                if mapping.id_column.is_none() {
                    return Err(invalid(format!(
                        "Area {} lists facility ids, which requires a mapped facility id column",
                        area.name
                    )));
                }
                if area.facility_ids.len() > MAX_IDS_PER_AREA {
                    return Err(invalid(format!(
                        "Area {} lists more than {MAX_IDS_PER_AREA} facility ids",
                        area.name
                    )));
                }
                for id in &area.facility_ids {
                    bounded(id, "A facility id")?;
                }
            }
            _ => {
                return Err(invalid(format!(
                    "Area {} needs exactly one of a bounding box or a facility id list",
                    area.name
                )))
            }
        }
    }
    Ok(dataset.clone())
}

struct FacilityRow {
    table_row: u64,
    id: Option<String>,
    name: Option<String>,
    address: Option<String>,
    latitude: Option<f64>,
    longitude: Option<f64>,
    hours: Option<String>,
    category: Option<String>,
}

fn text_expr(column: Option<&String>) -> String {
    column.map_or("CAST(NULL AS VARCHAR)".into(), |c| {
        format!("NULLIF(trim(CAST({} AS VARCHAR)), '')", quote(c))
    })
}
fn number_expr(column: Option<&String>) -> String {
    column.map_or("CAST(NULL AS DOUBLE)".into(), |c| {
        format!("TRY_CAST({} AS DOUBLE)", quote(c))
    })
}

fn build_report(
    mapping: FacilityCoverageMapping,
    dataset: &AnalyticalDatasetSummary,
    rows: Vec<FacilityRow>,
) -> FacilityCoverageReport {
    let total_rows = rows.len() as u64;
    let wanted = mapping
        .restroom_categories
        .iter()
        .map(|c| c.trim().to_lowercase())
        .collect::<BTreeSet<_>>();
    let mut category_missing = 0u64;
    let restrooms = rows
        .into_iter()
        .filter(|row| {
            if mapping.category_column.is_none() {
                return true;
            }
            match &row.category {
                Some(category) => wanted.contains(&category.to_lowercase()),
                None => {
                    category_missing += 1;
                    false
                }
            }
        })
        .collect::<Vec<_>>();
    let coordinates = mapping.latitude_column.is_some();
    let located = |row: &FacilityRow| {
        row.latitude
            .zip(row.longitude)
            .filter(|(lat, lon)| lat.is_finite() && lon.is_finite())
    };
    let location_not_recorded =
        coordinates.then(|| restrooms.iter().filter(|r| located(r).is_none()).count() as u64);
    let facility = |row: &FacilityRow| {
        let (hours_status, open_intervals) = parse_daily_hours(row.hours.as_deref());
        FacilityCoverageFacility {
            table_row: row.table_row,
            facility_id: row.id.clone(),
            name: row.name.clone(),
            address: row.address.clone(),
            hours_text: row.hours.clone(),
            hours_status,
            open_intervals,
        }
    };
    let areas = mapping
        .areas
        .iter()
        .map(|area| match &area.bounding_box {
            Some(bbox) => {
                let members = restrooms
                    .iter()
                    .filter(|row| {
                        located(row).is_some_and(|(lat, lon)| {
                            (bbox.min_latitude..=bbox.max_latitude).contains(&lat)
                                && (bbox.min_longitude..=bbox.max_longitude).contains(&lon)
                        })
                    })
                    .map(facility)
                    .collect();
                let boundary = format!(
                    "Latitude {} to {}, longitude {} to {} (edges included)",
                    bbox.min_latitude, bbox.max_latitude, bbox.min_longitude, bbox.max_longitude
                );
                summarize_area(area.name.clone(), boundary, members, vec![])
            }
            None => {
                let listed = area
                    .facility_ids
                    .iter()
                    .map(|id| id.trim().to_string())
                    .collect::<BTreeSet<_>>();
                let members = restrooms
                    .iter()
                    .filter(|row| row.id.as_ref().is_some_and(|id| listed.contains(id)))
                    .map(facility)
                    .collect::<Vec<_>>();
                let found = members
                    .iter()
                    .filter_map(|f| f.facility_id.clone())
                    .collect::<BTreeSet<_>>();
                let unmatched = listed.difference(&found).cloned().collect();
                let boundary = format!("{} listed facility ids", listed.len());
                summarize_area(area.name.clone(), boundary, members, unmatched)
            }
        })
        .collect::<Vec<_>>();
    let mut limitations = vec![
        "Coverage and zero-open minutes use only facilities whose daily hours were recorded and interpretable. A facility with unrecorded or unreadable hours may be open during a minute counted as having none open.".to_string(),
        "Hours are read as one daily schedule. Weekday-specific, seasonal and closure-notice schedules are not interpreted.".to_string(),
    ];
    if dataset.partial_dataset || dataset.rejected_rows > 0 {
        limitations.push(format!(
            "This snapshot is partial or has {} rejected rows; counts cover imported rows only.",
            dataset.rejected_rows
        ));
    }
    if mapping.category_column.is_none() {
        limitations.push(
            "No category column is mapped, so every row in the list is counted as a public restroom."
                .into(),
        );
    } else if category_missing > 0 {
        limitations.push(format!(
            "{category_missing} rows have no recorded category and are not counted as public restrooms."
        ));
    }
    if mapping.hours_column.is_none() {
        limitations.push("No hours column is mapped, so no facility has recorded hours.".into());
    }
    if let Some(missing) = location_not_recorded.filter(|n| *n > 0) {
        limitations.push(format!(
            "{missing} public restroom rows have no numeric coordinates and cannot fall inside a bounding box. Addresses are not geocoded."
        ));
    }
    FacilityCoverageReport {
        dataset_name: dataset.display_name.clone(),
        table_name: dataset.table_name.clone(),
        source_uri: dataset.source_uri.clone(),
        source_sha256: dataset.source_sha256.clone(),
        total_rows,
        public_restroom_rows: restrooms.len() as u64,
        location_not_recorded,
        areas,
        limitations,
        mapping,
    }
}

impl DuckDbRepository {
    pub async fn facility_coverage_workspace(&self) -> Result<FacilityCoverageWorkspace, AppError> {
        let schema = self.analytical_schema().await?;
        let saved = self
            .query_connection("Read facility coverage mappings", |conn| {
                conn.execute_batch(SCHEMA).map_err(error)?;
                let mut statement = conn
                    .prepare(
                        "SELECT mapping_json FROM facility_coverage_mappings ORDER BY snapshot_id",
                    )
                    .map_err(error)?;
                let values = statement
                    .query_map([], |row| row.get::<_, String>(0))
                    .map_err(error)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(error)?;
                values
                    .into_iter()
                    .map(|v| serde_json::from_str::<FacilityCoverageMapping>(&v).map_err(error))
                    .collect::<Result<Vec<_>, _>>()
            })
            .await?;
        let mut reports = vec![];
        let mut warnings = vec![];
        for mapping in saved {
            match validate_mapping(&mapping, &schema) {
                Ok(dataset) => reports.push(self.facility_coverage_report(mapping, dataset).await?),
                Err(e) => warnings.push(format!(
                    "Facility list {} needs attention: {e}",
                    mapping.snapshot_id
                )),
            }
        }
        Ok(FacilityCoverageWorkspace { reports, warnings })
    }

    pub async fn save_facility_coverage_mapping(
        &self,
        mapping: FacilityCoverageMapping,
    ) -> Result<FacilityCoverageWorkspace, AppError> {
        let schema = self.analytical_schema().await?;
        validate_mapping(&mapping, &schema)?;
        self.query_connection("Save facility coverage mapping", move |conn| {
            conn.execute_batch(SCHEMA).map_err(error)?;
            conn.execute(
                "INSERT INTO facility_coverage_mappings VALUES (?1,?2,?3) ON CONFLICT(snapshot_id) DO UPDATE SET mapping_json=excluded.mapping_json,updated_at=excluded.updated_at",
                params![
                    mapping.snapshot_id,
                    serde_json::to_string(&mapping).map_err(error)?,
                    chrono::Utc::now().to_rfc3339()
                ],
            )
            .map_err(error)?;
            Ok(())
        })
        .await?;
        self.facility_coverage_workspace().await
    }

    async fn facility_coverage_report(
        &self,
        mapping: FacilityCoverageMapping,
        dataset: AnalyticalDatasetSummary,
    ) -> Result<FacilityCoverageReport, AppError> {
        let sql = format!(
            "SELECT rowid, {}, {}, {}, {}, {}, {}, {} FROM {} ORDER BY rowid",
            text_expr(mapping.id_column.as_ref()),
            text_expr(Some(&mapping.name_column)),
            text_expr(mapping.address_column.as_ref()),
            number_expr(mapping.latitude_column.as_ref()),
            number_expr(mapping.longitude_column.as_ref()),
            text_expr(mapping.hours_column.as_ref()),
            text_expr(mapping.category_column.as_ref()),
            quote(&dataset.table_name)
        );
        let rows = self
            .query_connection("Read facility list", move |conn| {
                let mut statement = conn.prepare(&sql).map_err(error)?;
                let rows = statement
                    .query_map([], |row| {
                        Ok(FacilityRow {
                            table_row: row.get::<_, i64>(0)?.max(0) as u64,
                            id: row.get(1)?,
                            name: row.get(2)?,
                            address: row.get(3)?,
                            latitude: row.get(4)?,
                            longitude: row.get(5)?,
                            hours: row.get(6)?,
                            category: row.get(7)?,
                        })
                    })
                    .map_err(error)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(error)?;
                Ok(rows)
            })
            .await?;
        Ok(build_report(mapping, &dataset, rows))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn imported_facility_list_is_summarized_per_user_area_from_recorded_rows_only() {
        let temp = tempfile::tempdir().unwrap();
        let repo = DuckDbRepository::new(temp.path().join("core.db")).unwrap();
        repo.run_migrations(include_str!("../../migrations/0001_initial_schema.sql"))
            .await
            .unwrap();
        let conn = repo.analytics_connection().await.unwrap();
        let columns = [
            "facility_id",
            "name",
            "address",
            "lat",
            "lon",
            "hours",
            "category",
        ];
        conn.execute_batch(
            "CREATE TABLE analysis_regional_data_fac (facility_id VARCHAR, name VARCHAR, address VARCHAR, lat VARCHAR, lon DOUBLE, hours VARCHAR, category VARCHAR);
             INSERT INTO analysis_regional_data_fac VALUES
               ('R1','Plaza','1 Main St','32.7000',-117.1600,'06:00-22:00','Public Restroom'),
               ('R2','Library','2 Main St','32.7200',-117.1500,'22:00-00:00','public restroom'),
               ('R3','Park','3 Main St','32.7100',-117.1550,'Mon-Fri 8am-5pm','Public Restroom'),
               ('R4','Pier','4 Main St',NULL,NULL,NULL,'Public Restroom'),
               ('R5','Fountain','5 Main St','32.7100',-117.1550,'24/7','Drinking Fountain'),
               ('R6','Uptown','6 Elm St','32.7500',-117.1550,'00:00-24:00','Public Restroom'),
               ('R7','Unknown','7 Elm St','32.7100',-117.1550,'24/7',NULL);",
        )
        .unwrap();
        conn.execute("INSERT INTO analytical_snapshots(snapshot_id,dataset_kind,display_name,table_name,source_uri,local_path,source_sha256,retrieved_at,validation_summary_json,imported_rows) VALUES ('fac','REGIONAL_DATA','Restrooms','analysis_regional_data_fac','https://example.test/restrooms.csv','/fixture',repeat('d',64),'2026-09-30','{}',7)",[]).unwrap();
        for (ordinal, column) in columns.iter().enumerate() {
            conn.execute("INSERT INTO analytical_columns(snapshot_id,table_name,column_name,logical_type,ordinal) VALUES ('fac','analysis_regional_data_fac',?1,'TEXT',?2)",params![column,ordinal as i64]).unwrap();
        }
        drop(conn);
        let downtown = FacilityBoundingBox {
            min_latitude: 32.70,
            max_latitude: 32.72,
            min_longitude: -117.17,
            max_longitude: -117.15,
        };
        let mut mapping = FacilityCoverageMapping {
            snapshot_id: "fac".into(),
            name_column: "name".into(),
            id_column: Some("facility_id".into()),
            address_column: Some("address".into()),
            latitude_column: None,
            longitude_column: None,
            hours_column: Some("hours".into()),
            category_column: Some("category".into()),
            restroom_categories: vec!["Public Restroom".into()],
            areas: vec![FacilityArea {
                name: "Downtown".into(),
                bounding_box: Some(downtown.clone()),
                facility_ids: vec![],
            }],
        };
        assert!(repo
            .save_facility_coverage_mapping(mapping.clone())
            .await
            .is_err());
        mapping.latitude_column = Some("lat".into());
        mapping.longitude_column = Some("lon".into());
        mapping.areas.push(FacilityArea {
            name: "Listed".into(),
            bounding_box: None,
            facility_ids: vec!["R4".into(), "R6".into(), "R9".into()],
        });
        let workspace = repo.save_facility_coverage_mapping(mapping).await.unwrap();
        assert!(workspace.warnings.is_empty());
        let report = &workspace.reports[0];
        assert_eq!(report.total_rows, 7);
        assert_eq!(report.public_restroom_rows, 5);
        assert_eq!(report.location_not_recorded, Some(1));
        let downtown = &report.areas[0];
        // R1 and R2 sit exactly on the box edges; R3's hours are weekday-qualified.
        assert_eq!(downtown.public_restrooms, 3);
        assert_eq!(downtown.with_recorded_hours, 2);
        assert_eq!(downtown.hours_uninterpretable, 1);
        assert_eq!(downtown.open_minutes_covered, Some(16 * 60 + 2 * 60));
        assert_eq!(downtown.minutes_with_none_open, Some(6 * 60));
        let listed = &report.areas[1];
        assert_eq!(listed.public_restrooms, 2);
        assert_eq!(listed.with_recorded_hours, 1);
        assert_eq!(listed.hours_not_recorded, 1);
        assert_eq!(listed.open_minutes_covered, Some(1440));
        assert_eq!(listed.minutes_with_none_open, Some(0));
        assert_eq!(listed.unmatched_facility_ids, vec!["R9".to_string()]);
    }
}
