//! Fetches a named City of San Diego / SANDAG table into the managed store and imports it.

use super::*;
use crate::analysis::encampment_ordinance::{SCHOOLS_SOURCE_URI, TRANSIT_STOPS_SOURCE_URI};
use crate::types::{
    AnalyticalDatasetKind, AnalyticalImportRequest, AnalyticalImportResult, CityDataset,
    DerivedSubset,
};
use chrono::Datelike;

#[derive(Debug, PartialEq, Eq)]
struct CityDatasetSpec {
    url: String,
    kind: AnalyticalDatasetKind,
    display_name: String,
    file_name: String,
    partial: bool,
    row_filter: Option<(&'static str, &'static str)>,
}

/// The URL is always built here from a fixed template; years outside the publisher's series are
/// refused rather than guessed at.
fn city_dataset_spec(
    dataset: CityDataset,
    year: Option<u16>,
    current_year: i32,
) -> Result<CityDatasetSpec, AppError> {
    let yearly = |first: i32, name: &str| -> Result<i32, AppError> {
        let year = year
            .map(i32::from)
            .ok_or_else(|| AppError::DataParsing(format!("Choose a year for {name}")))?;
        if !(first..=current_year).contains(&year) {
            return Err(AppError::DataParsing(format!(
                "{name} is published for {first} through {current_year}"
            )));
        }
        Ok(year)
    };
    let unsplit = |name: &str| -> Result<(), AppError> {
        match year {
            Some(_) => Err(AppError::DataParsing(format!(
                "{name} is one published file, not split by year"
            ))),
            None => Ok(()),
        }
    };
    Ok(match dataset {
        CityDataset::NibrsOffenses => {
            let year = yearly(2020, "SDPD NIBRS offenses")?;
            let file_name = format!("pd_nibrs_{year}_datasd.csv");
            CityDatasetSpec {
                url: format!("https://seshat.datasd.org/police_nibrs/{file_name}"),
                kind: AnalyticalDatasetKind::NibrsOffenses,
                display_name: format!("SDPD NIBRS offenses {year}"),
                file_name,
                partial: year >= current_year,
                row_filter: None,
            }
        }
        CityDataset::GetItDoneEncampments => {
            let year = yearly(2016, "Get It Done closed reports")?;
            let file_name = format!("get_it_done_requests_closed_{year}_datasd.csv");
            CityDatasetSpec {
                url: format!("https://seshat.datasd.org/get_it_done_reports/{file_name}"),
                kind: AnalyticalDatasetKind::RegionalData,
                display_name: format!(
                    "Get It Done closed reports {year}, service_name = Encampment"
                ),
                file_name,
                partial: true,
                row_filter: Some(("service_name", "Encampment")),
            }
        }
        CityDataset::GetItDoneOpen => {
            unsplit("Get It Done open reports")?;
            let file_name = "get_it_done_requests_open_datasd.csv";
            CityDatasetSpec {
                url: format!("https://seshat.datasd.org/get_it_done_reports/{file_name}"),
                kind: AnalyticalDatasetKind::RegionalData,
                display_name: "Get It Done open reports (all services)".into(),
                file_name: file_name.into(),
                // The publisher replaces this file daily; a snapshot is the backlog on one day.
                partial: true,
                row_filter: None,
            }
        }
        CityDataset::Schools => {
            unsplit("SANDAG schools")?;
            CityDatasetSpec {
                url: SCHOOLS_SOURCE_URI.into(),
                kind: AnalyticalDatasetKind::RegionalData,
                display_name: "SANDAG schools (CDE addresses geocoded by SanGIS)".into(),
                file_name: "sandag_schools.geojson".into(),
                partial: false,
                row_filter: None,
            }
        }
        CityDataset::TransitStops => {
            unsplit("SANDAG transit stops")?;
            CityDatasetSpec {
                url: TRANSIT_STOPS_SOURCE_URI.into(),
                kind: AnalyticalDatasetKind::RegionalData,
                display_name: "SANDAG transit stops (MTS and NCTD GTFS)".into(),
                file_name: "Transit_Stops_GTFS.csv".into(),
                partial: false,
                row_filter: None,
            }
        }
    })
}

/// Copies the header and every record whose `column` equals `value` exactly. Returns the
/// publisher's record count. Field values are copied byte for byte; only quoting may change.
fn keep_matching_rows(
    source: &Path,
    destination: &Path,
    column: &str,
    value: &str,
) -> Result<usize, AppError> {
    let csv_error = |error: csv::Error| AppError::DataParsing(format!("Publisher CSV: {error}"));
    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .from_path(source)
        .map_err(csv_error)?;
    let headers = reader.byte_headers().map_err(csv_error)?.clone();
    let index = headers
        .iter()
        .position(|header| header == column.as_bytes())
        .ok_or_else(|| {
            AppError::DataParsing(format!(
                "The publisher's file no longer has a '{column}' column; nothing was imported"
            ))
        })?;
    let file = std::fs::File::create(destination)
        .map_err(|error| AppError::Storage(format!("Failed to stage filtered rows: {error}")))?;
    let mut writer = csv::Writer::from_writer(file);
    writer.write_byte_record(&headers).map_err(csv_error)?;
    let mut record = csv::ByteRecord::new();
    let mut publisher_rows = 0;
    while reader.read_byte_record(&mut record).map_err(csv_error)? {
        publisher_rows += 1;
        if record.get(index) == Some(value.as_bytes()) {
            writer.write_byte_record(&record).map_err(csv_error)?;
        }
    }
    writer
        .into_inner()
        .map_err(|error| AppError::Storage(format!("Failed to write filtered rows: {error}")))?
        .sync_all()
        .map_err(|error| AppError::Storage(format!("Failed to sync filtered rows: {error}")))?;
    Ok(publisher_rows)
}

impl QueueManager {
    pub async fn fetch_city_dataset(
        &self,
        dataset: CityDataset,
        year: Option<u16>,
    ) -> Result<AnalyticalImportResult, AppError> {
        let spec = city_dataset_spec(dataset, year, Utc::now().year())?;
        let url = reqwest::Url::parse(&spec.url)
            .map_err(|error| AppError::Internal(format!("City dataset URL: {error}")))?;
        let staging = self.download_root.join(".staging");
        tokio::fs::create_dir_all(&staging).await.map_err(|error| {
            AppError::Storage(format!(
                "Failed to create download staging directory: {error}"
            ))
        })?;
        let staged = staging.join(format!(".city.{}.partial", Uuid::new_v4()));
        let downloaded = self.client.download_url_to_path(url, &staged).await?;
        if downloaded.byte_count == 0 {
            let _ = tokio::fs::remove_file(&staged).await;
            return Err(AppError::DataParsing(format!(
                "{} download was empty",
                spec.display_name
            )));
        }
        let published =
            objects::commit_staged_object(&self.download_root, &staged, &downloaded.sha256).await?;
        let (local_path, sha256, derived_subset) = match spec.row_filter {
            None => (published, downloaded.sha256, None),
            Some((column, value)) => {
                let subset = staging.join(format!(".city.{}.partial", Uuid::new_v4()));
                let target = subset.clone();
                let filtered = spawn_blocking("Filter City dataset", move || {
                    let rows = keep_matching_rows(&published, &target, column, value)?;
                    let digest = crate::integrity::sha256_file(&target).map_err(|error| {
                        AppError::Storage(format!("Failed to hash filtered rows: {error}"))
                    })?;
                    Ok((rows, digest))
                })
                .await
                .and_then(|result| result);
                let (publisher_rows, digest) = match filtered {
                    Ok(filtered) => filtered,
                    Err(error) => {
                        let _ = tokio::fs::remove_file(&subset).await;
                        return Err(error);
                    }
                };
                let path =
                    objects::commit_staged_object(&self.download_root, &subset, &digest).await?;
                let subset = DerivedSubset {
                    publisher_sha256: downloaded.sha256,
                    publisher_rows,
                    filter: format!("{column} = '{value}'"),
                };
                (path, digest, Some(subset))
            }
        };
        self.repository
            .import_analytical_csv(AnalyticalImportRequest {
                dataset_kind: spec.kind,
                display_name: spec.display_name,
                source_uri: Some(spec.url),
                local_path: local_path.to_string_lossy().into_owned(),
                observation_period_start: None,
                observation_period_end: None,
                partial_dataset: spec.partial,
                worksheet: None,
                source_file_name: Some(spec.file_name),
                expected_source_sha256: Some(sha256),
                derived_subset,
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_year_outside_the_publisher_series_is_refused_and_unsplit_files_take_none() {
        let spec = |dataset, year| city_dataset_spec(dataset, year, 2026);
        assert!(spec(CityDataset::NibrsOffenses, Some(2019)).is_err());
        let first = spec(CityDataset::NibrsOffenses, Some(2020)).unwrap();
        assert_eq!(
            first.url,
            "https://seshat.datasd.org/police_nibrs/pd_nibrs_2020_datasd.csv"
        );
        assert!(!first.partial);
        assert!(
            spec(CityDataset::NibrsOffenses, Some(2026))
                .unwrap()
                .partial
        );
        assert!(spec(CityDataset::NibrsOffenses, Some(2027)).is_err());
        assert!(spec(CityDataset::NibrsOffenses, None).is_err());
        assert!(spec(CityDataset::GetItDoneEncampments, Some(2015)).is_err());
        assert!(spec(CityDataset::GetItDoneEncampments, Some(2016)).is_ok());
        assert!(spec(CityDataset::GetItDoneOpen, Some(2025)).is_err());
        assert!(spec(CityDataset::GetItDoneOpen, None).is_ok());
        assert!(spec(CityDataset::Schools, Some(2025)).is_err());
        assert!(spec(CityDataset::TransitStops, None).is_ok());
    }

    #[test]
    fn the_subset_keeps_exact_matches_only_with_their_fields_intact() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.csv");
        let target = directory.path().join("target.csv");
        std::fs::write(
            &source,
            "id,service_name,note\n1,Encampment,\"a, quoted\"\n2,Encampment Cleanup,x\n3,encampment,y\n4,Encampment,\n",
        )
        .unwrap();
        let rows = keep_matching_rows(&source, &target, "service_name", "Encampment").unwrap();
        assert_eq!(rows, 4);
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "id,service_name,note\n1,Encampment,\"a, quoted\"\n4,Encampment,\n"
        );
        let missing = keep_matching_rows(&source, &target, "service", "Encampment");
        assert!(missing.is_err());
    }
}

#[cfg(test)]
mod live {
    use super::*;
    use crate::ingestion::client::HttpClientConfig;

    #[tokio::test]
    #[ignore = "downloads published City and SANDAG files"]
    async fn fetches_the_published_files_and_keeps_only_encampment_reports() {
        let client = ApiClient::new(HttpClientConfig::default()).unwrap();
        let directory = tempfile::tempdir().unwrap();
        for (dataset, year) in [
            (CityDataset::Schools, None),
            (CityDataset::TransitStops, None),
            (CityDataset::GetItDoneOpen, None),
            (CityDataset::GetItDoneEncampments, Some(2025)),
        ] {
            let spec = city_dataset_spec(dataset, year, 2026).unwrap();
            let path = directory.path().join(&spec.file_name);
            let downloaded = client
                .download_url_to_path(reqwest::Url::parse(&spec.url).unwrap(), &path)
                .await
                .unwrap();
            println!(
                "{}: {} bytes, {}",
                spec.file_name, downloaded.byte_count, downloaded.sha256
            );
            if let Some((column, value)) = spec.row_filter {
                let subset = directory.path().join("subset.csv");
                let rows = keep_matching_rows(&path, &subset, column, value).unwrap();
                let kept = csv::Reader::from_path(&subset).unwrap().records().count();
                println!("{column} = {value}: kept {kept} of {rows}");
            }
        }
    }
}
