use crate::{
    error::AppError,
    ingestion::organisation_filings::{read_index, IndexRow},
    storage::sqlite::DuckDbRepository,
    types::*,
};
use duckdb::arrow::array::{ArrayRef, StringArray};
use duckdb::arrow::datatypes::{DataType, Field, Schema};
use duckdb::arrow::record_batch::RecordBatch;
use duckdb::{params, OptionalExt};
use std::path::Path;
use std::sync::Arc;

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS organisation_filings (ein TEXT PRIMARY KEY, name TEXT, document_json TEXT NOT NULL, checked_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS organisation_return_xml (object_id TEXT PRIMARY KEY, ein TEXT NOT NULL, member_name TEXT NOT NULL, sha256 TEXT NOT NULL, retrieved_at TEXT NOT NULL, xml TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS organisation_payment_mappings (snapshot_id TEXT PRIMARY KEY, mapping_json TEXT NOT NULL, updated_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS irs_efile_index (index_year INTEGER NOT NULL, ein TEXT NOT NULL, object_id TEXT NOT NULL, return_type TEXT NOT NULL, tax_period TEXT NOT NULL, taxpayer_name TEXT NOT NULL, batch_id TEXT);
CREATE TABLE IF NOT EXISTS irs_efile_index_years (index_year INTEGER PRIMARY KEY, retrieved_at TEXT NOT NULL, rows BIGINT NOT NULL, source_url TEXT NOT NULL);";
/// Rows gathered before one bulk append; see `analytical_import`.
const APPEND_BATCH_ROWS: usize = 16_384;

fn error(e: impl std::fmt::Display) -> AppError {
    AppError::Storage(e.to_string())
}

/// A return's XML as the IRS archive held it, kept so a later check re-reads it without the
/// archive and a reader can compare it with the published file.
pub struct StoredReturnXml {
    pub member_name: String,
    pub sha256: String,
    pub retrieved_at: String,
    pub xml: String,
}

impl DuckDbRepository {
    /// The stored record alone, without the library's payment tables.
    pub async fn organisation_filings_document(
        &self,
        ein: &str,
    ) -> Result<Option<OrganisationFilings>, AppError> {
        let ein = ein.to_string();
        self.query_connection("Read organisation filings", move |conn| {
            conn.execute_batch(SCHEMA).map_err(error)?;
            conn.query_row(
                "SELECT document_json FROM organisation_filings WHERE ein = ?1",
                params![ein],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(error)?
            .map(|json| serde_json::from_str(&json).map_err(error))
            .transpose()
        })
        .await
    }

    /// The stored record with the payments the library's mapped tables hold for its name.
    pub async fn organisation_filings(
        &self,
        ein: &str,
    ) -> Result<Option<OrganisationFilings>, AppError> {
        let Some(mut document) = self.organisation_filings_document(ein).await? else {
            return Ok(None);
        };
        if let Some(name) = document.name.clone() {
            (document.public_money, document.public_money_notes) =
                self.organisation_public_money(&name).await?;
        }
        Ok(Some(document))
    }

    pub async fn save_organisation_filings(
        &self,
        document: &OrganisationFilings,
    ) -> Result<(), AppError> {
        let mut stored = document.clone();
        stored.public_money.clear();
        stored.public_money_notes.clear();
        self.query_connection("Save organisation filings", move |conn| {
            conn.execute_batch(SCHEMA).map_err(error)?;
            conn.execute(
                "INSERT INTO organisation_filings VALUES (?1,?2,?3,?4) ON CONFLICT(ein) DO UPDATE SET name=excluded.name,document_json=excluded.document_json,checked_at=excluded.checked_at",
                params![
                    stored.ein,
                    stored.name,
                    serde_json::to_string(&stored).map_err(error)?,
                    stored.checked_at
                ],
            )
            .map_err(error)?;
            Ok(())
        })
        .await
    }

    pub async fn list_organisation_checks(
        &self,
    ) -> Result<Vec<OrganisationCheckSummary>, AppError> {
        self.query_connection("List organisation checks", |conn| {
            conn.execute_batch(SCHEMA).map_err(error)?;
            let mut statement = conn
                .prepare("SELECT ein, name, checked_at FROM organisation_filings ORDER BY checked_at DESC, ein")
                .map_err(error)?;
            let rows = statement
                .query_map([], |row| {
                    Ok(OrganisationCheckSummary {
                        ein: row.get(0)?,
                        name: row.get(1)?,
                        checked_at: row.get(2)?,
                    })
                })
                .map_err(error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(error)?;
            Ok(rows)
        })
        .await
    }

    pub async fn organisation_return_xml(
        &self,
        object_id: &str,
    ) -> Result<Option<StoredReturnXml>, AppError> {
        let object_id = object_id.to_string();
        self.query_connection("Read organisation return", move |conn| {
            conn.execute_batch(SCHEMA).map_err(error)?;
            conn.query_row(
                "SELECT member_name, sha256, retrieved_at, xml FROM organisation_return_xml WHERE object_id = ?1",
                params![object_id],
                |row| {
                    Ok(StoredReturnXml {
                        member_name: row.get(0)?,
                        sha256: row.get(1)?,
                        retrieved_at: row.get(2)?,
                        xml: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(error)
        })
        .await
    }

    pub async fn save_organisation_return_xml(
        &self,
        object_id: &str,
        ein: &str,
        member_name: &str,
        sha256: &str,
        retrieved_at: &str,
        xml: &str,
    ) -> Result<(), AppError> {
        let values = [object_id, ein, member_name, sha256, retrieved_at, xml].map(str::to_string);
        self.query_connection("Save organisation return", move |conn| {
            conn.execute_batch(SCHEMA).map_err(error)?;
            conn.execute(
                "INSERT INTO organisation_return_xml VALUES (?1,?2,?3,?4,?5,?6) ON CONFLICT(object_id) DO NOTHING",
                params![values[0], values[1], values[2], values[3], values[4], values[5]],
            )
            .map_err(error)?;
            Ok(())
        })
        .await
    }
}

fn index_batch(rows: &[IndexRow], year: i32) -> Result<RecordBatch, AppError> {
    let text = |pick: fn(&IndexRow) -> Option<&str>| -> ArrayRef {
        Arc::new(StringArray::from(rows.iter().map(pick).collect::<Vec<_>>()))
    };
    let columns: Vec<ArrayRef> = vec![
        Arc::new(StringArray::from(vec![year.to_string(); rows.len()])),
        text(|r| Some(&r.ein)),
        text(|r| Some(&r.object_id)),
        text(|r| Some(&r.return_type)),
        text(|r| Some(&r.tax_period)),
        text(|r| Some(&r.taxpayer_name)),
        text(|r| r.batch_id.as_deref()),
    ];
    let schema = Schema::new(
        columns
            .iter()
            .enumerate()
            .map(|(i, _)| Field::new(format!("c{i}"), DataType::Utf8, true))
            .collect::<Vec<_>>(),
    );
    RecordBatch::try_new(Arc::new(schema), columns)
        .map_err(|e| AppError::Internal(format!("IRS index batch: {e}")))
}

impl DuckDbRepository {
    pub async fn organisation_payment_mappings(
        &self,
    ) -> Result<Vec<OrganisationPaymentMapping>, AppError> {
        self.query_connection("Read organisation payment mappings", |conn| {
            conn.execute_batch(SCHEMA).map_err(error)?;
            let mut statement = conn
                .prepare(
                    "SELECT mapping_json FROM organisation_payment_mappings ORDER BY snapshot_id",
                )
                .map_err(error)?;
            let values = statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(error)?;
            values
                .into_iter()
                .map(|v| serde_json::from_str(&v).map_err(error))
                .collect()
        })
        .await
    }

    /// Saves a payee-table mapping after checking that the table and its columns exist.
    pub async fn save_organisation_payment_mapping(
        &self,
        mapping: OrganisationPaymentMapping,
    ) -> Result<Vec<OrganisationPaymentMapping>, AppError> {
        let schema = self.analytical_schema().await?;
        let dataset = schema
            .datasets
            .iter()
            .find(|d| d.snapshot_id == mapping.snapshot_id)
            .ok_or_else(|| AppError::DataParsing("The selected table no longer exists".into()))?;
        for (label, column) in [
            ("vendor", Some(&mapping.vendor_column)),
            ("amount", Some(&mapping.amount_column)),
            ("fiscal year", mapping.fiscal_year_column.as_ref()),
        ] {
            let Some(column) = column else { continue };
            if !schema
                .columns
                .iter()
                .any(|c| c.table_name == dataset.table_name && &c.column_name == column)
            {
                return Err(AppError::DataParsing(format!(
                    "The {label} column does not exist in {}",
                    dataset.display_name
                )));
            }
        }
        self.query_connection("Save organisation payment mapping", move |conn| {
            conn.execute_batch(SCHEMA).map_err(error)?;
            conn.execute(
                "INSERT INTO organisation_payment_mappings VALUES (?1,?2,?3) ON CONFLICT(snapshot_id) DO UPDATE SET mapping_json=excluded.mapping_json,updated_at=excluded.updated_at",
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
        self.organisation_payment_mappings().await
    }

    pub async fn remove_organisation_payment_mapping(
        &self,
        snapshot_id: &str,
    ) -> Result<Vec<OrganisationPaymentMapping>, AppError> {
        let snapshot_id = snapshot_id.to_string();
        self.query_connection("Remove organisation payment mapping", move |conn| {
            conn.execute_batch(SCHEMA).map_err(error)?;
            conn.execute(
                "DELETE FROM organisation_payment_mappings WHERE snapshot_id = ?1",
                params![snapshot_id],
            )
            .map_err(error)?;
            Ok(())
        })
        .await?;
        self.organisation_payment_mappings().await
    }

    /// Replaces the library's copy of one IRS index year with the file's rows, in one
    /// transaction, so a failed read leaves the earlier copy.
    pub async fn replace_irs_index_year(
        &self,
        year: i32,
        path: &Path,
        source_url: &str,
    ) -> Result<IndexYearCheck, AppError> {
        let path = path.to_path_buf();
        let source_url = source_url.to_string();
        self.query_connection("Replace IRS index year", move |conn| {
            conn.execute_batch(SCHEMA).map_err(error)?;
            let file = std::fs::File::open(&path)
                .map_err(|e| AppError::Storage(format!("IRS index staging file: {e}")))?;
            let tx = conn.transaction().map_err(error)?;
            tx.execute(
                "DELETE FROM irs_efile_index WHERE index_year = ?1",
                params![year],
            )
            .map_err(error)?;
            let rows = {
                // The year column is appended as text so every column shares one arrow type;
                // DuckDB casts it to the table's INTEGER on append.
                let mut appender = tx
                    .appender("irs_efile_index")
                    .map_err(error)?;
                let mut pending = Vec::with_capacity(APPEND_BATCH_ROWS);
                let mut flush = |pending: &mut Vec<IndexRow>| -> Result<(), AppError> {
                    if !pending.is_empty() {
                        appender
                            .append_record_batch(index_batch(pending, year)?)
                            .map_err(error)?;
                        pending.clear();
                    }
                    Ok(())
                };
                let rows = read_index(std::io::BufReader::new(file), |row| {
                    pending.push(row);
                    if pending.len() == APPEND_BATCH_ROWS {
                        flush(&mut pending)?;
                    }
                    Ok(())
                })?;
                flush(&mut pending)?;
                appender.flush().map_err(error)?;
                rows
            };
            let retrieved_at = chrono::Utc::now().to_rfc3339();
            tx.execute(
                "INSERT INTO irs_efile_index_years VALUES (?1,?2,?3,?4) ON CONFLICT(index_year) DO UPDATE SET retrieved_at=excluded.retrieved_at,rows=excluded.rows,source_url=excluded.source_url",
                params![year, retrieved_at, rows as i64, source_url],
            )
            .map_err(error)?;
            tx.commit().map_err(error)?;
            Ok(IndexYearCheck {
                year,
                checked_at: retrieved_at,
                returns_listed: 0,
            })
        })
        .await
    }

    /// When the library's copy of a year's index was retrieved, if it holds one.
    pub async fn irs_index_year(&self, year: i32) -> Result<Option<IndexYearCheck>, AppError> {
        self.query_connection("Read IRS index year", move |conn| {
            conn.execute_batch(SCHEMA).map_err(error)?;
            conn.query_row(
                "SELECT retrieved_at FROM irs_efile_index_years WHERE index_year = ?1",
                params![year],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(error)
            .map(|retrieved| {
                retrieved.map(|checked_at| IndexYearCheck {
                    year,
                    checked_at,
                    returns_listed: 0,
                })
            })
        })
        .await
    }

    pub async fn irs_index_rows(&self, year: i32, ein: &str) -> Result<Vec<IndexRow>, AppError> {
        let ein = ein.to_string();
        self.query_connection("Read IRS index rows", move |conn| {
            conn.execute_batch(SCHEMA).map_err(error)?;
            let mut statement = conn
                .prepare("SELECT ein, object_id, return_type, tax_period, taxpayer_name, batch_id FROM irs_efile_index WHERE index_year = ?1 AND ein = ?2 ORDER BY object_id")
                .map_err(error)?;
            let rows = statement
                .query_map(params![year, ein], |row| {
                    Ok(IndexRow {
                        ein: row.get(0)?,
                        object_id: row.get(1)?,
                        return_type: row.get(2)?,
                        tax_period: row.get(3)?,
                        taxpayer_name: row.get(4)?,
                        batch_id: row.get(5)?,
                    })
                })
                .map_err(error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(error)?;
            Ok(rows)
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_record_round_trips_without_its_computed_payment_lines() {
        let temp = tempfile::tempdir().unwrap();
        let repo = DuckDbRepository::new(temp.path().join("core.db")).unwrap();
        repo.run_migrations(include_str!("../../migrations/0001_initial_schema.sql"))
            .await
            .unwrap();
        assert!(repo
            .organisation_filings("330215585")
            .await
            .unwrap()
            .is_none());
        let document = OrganisationFilings {
            ein: "330215585".into(),
            name: Some("Alpha Project For The Homeless".into()),
            checked_at: "2026-10-05T00:00:00Z".into(),
            index_years: vec![IndexYearCheck {
                year: 2024,
                checked_at: "2026-10-05T00:00:00Z".into(),
                returns_listed: 1,
            }],
            returns: vec![],
            summary: None,
            single_audits: None,
            warnings: vec![],
            public_money: vec![PublicMoneyLine {
                role: "payment".into(),
                dataset_name: "stale".into(),
                source_url: None,
                vendor_value: "stale".into(),
                fiscal_year: None,
                rows: 1,
                amount: None,
            }],
            public_money_notes: vec![],
        };
        repo.save_organisation_filings(&document).await.unwrap();
        let read = repo
            .organisation_filings("330215585")
            .await
            .unwrap()
            .unwrap();
        assert!(read.public_money.is_empty());
        assert_eq!(read.public_money_notes.len(), 1);
        assert_eq!(read.index_years, document.index_years);
        assert_eq!(
            repo.list_organisation_checks().await.unwrap()[0].ein,
            "330215585"
        );

        repo.save_organisation_return_xml("1", "330215585", "m.xml", "aa", "t1", "<Return/>")
            .await
            .unwrap();
        repo.save_organisation_return_xml("1", "330215585", "m.xml", "bb", "t2", "<Changed/>")
            .await
            .unwrap();
        let stored = repo.organisation_return_xml("1").await.unwrap().unwrap();
        assert_eq!(
            (stored.sha256.as_str(), stored.xml.as_str()),
            ("aa", "<Return/>")
        );
    }

    #[tokio::test]
    async fn payments_are_matched_by_vendor_name_and_an_unreadable_amount_withholds_the_sum() {
        let temp = tempfile::tempdir().unwrap();
        let repo = DuckDbRepository::new(temp.path().join("core.db")).unwrap();
        repo.run_migrations(include_str!("../../migrations/0001_initial_schema.sql"))
            .await
            .unwrap();
        let conn = repo.analytics_connection().await.unwrap();
        conn.execute_batch("CREATE TABLE analysis_org_payments(fy VARCHAR, fund VARCHAR, contract VARCHAR, vendor VARCHAR, txn VARCHAR, invoice VARCHAR, amount VARCHAR, status VARCHAR);
          INSERT INTO analysis_org_payments VALUES
            ('2023','100','C1','Alpha Project for the Homeless, Inc.','T1','I1','100.50','PAID'),
            ('2023','100','C1','Alpha Project for the Homeless, Inc.','T2','I2','200.00','PAID'),
            ('2024','100','C1','ALPHA PROJECT FOR THE HOMELESS','T3','I3','n/a','PAID'),
            ('2024','100','C2','Alpha Projects Unrelated LLC','T4','I4','999.00','PAID');").unwrap();
        drop(conn);
        crate::test_support::register_analytical_table(
            &repo,
            "payments",
            "analysis_org_payments",
            "https://example.org/payments.csv",
            'c',
        )
        .await
        .unwrap();
        let columns = [
            ("fy", "fy"),
            ("fund", "fund"),
            ("contract_id", "contract"),
            ("vendor_id", "vendor"),
            ("transaction_id", "txn"),
            ("invoice_id", "invoice"),
            ("amount", "amount"),
            ("status", "status"),
        ];
        repo.save_financial_mapping(FinancialMapping {
            role: "payment".into(),
            snapshot_id: "payments".into(),
            columns: columns
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            currency: Some("USD".into()),
            agency: Some("City".into()),
        })
        .await
        .unwrap();

        let (lines, notes) = repo
            .organisation_public_money("Alpha Project For The Homeless")
            .await
            .unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].fiscal_year.as_deref(), Some("2023"));
        assert_eq!(lines[0].rows, 2);
        assert_eq!(lines[0].amount.as_deref(), Some("300.5000"));
        assert_eq!(lines[1].vendor_value, "ALPHA PROJECT FOR THE HOMELESS");
        assert_eq!(lines[1].amount, None);

        let (none, notes) = repo
            .organisation_public_money("Father Joe's Villages")
            .await
            .unwrap();
        assert!(none.is_empty());
        assert!(notes[0].contains("column vendor"));

        let conn = repo.analytics_connection().await.unwrap();
        conn.execute_batch(
            "CREATE TABLE analysis_org_grants(payee VARCHAR, paid VARCHAR);
          INSERT INTO analysis_org_grants VALUES ('Father Joes Villages','5'),('Other','1');",
        )
        .unwrap();
        drop(conn);
        crate::test_support::register_analytical_table(
            &repo,
            "grants",
            "analysis_org_grants",
            "https://example.org/grants.csv",
            'd',
        )
        .await
        .unwrap();
        let mapping = |amount: &str| OrganisationPaymentMapping {
            snapshot_id: "grants".into(),
            vendor_column: "payee".into(),
            amount_column: amount.into(),
            fiscal_year_column: None,
        };
        assert!(repo
            .save_organisation_payment_mapping(mapping("missing"))
            .await
            .is_err());
        assert_eq!(
            repo.save_organisation_payment_mapping(mapping("paid"))
                .await
                .unwrap()
                .len(),
            1
        );
        let (lines, _) = repo
            .organisation_public_money("Father Joe's Villages")
            .await
            .unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(
            (lines[0].role.as_str(), lines[0].amount.as_deref()),
            ("payee table", Some("5.0000"))
        );
        let payees = repo.organisation_payees().await.unwrap();
        assert_eq!(
            payees[0].vendor_value,
            "Alpha Project for the Homeless, Inc."
        );
        assert_eq!(payees[0].rows, 2);
        assert!(payees
            .iter()
            .any(|p| p.vendor_value == "Father Joes Villages" && p.dataset_names == ["grants"]));
        assert!(repo
            .remove_organisation_payment_mapping("grants")
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn an_index_year_is_replaced_whole_and_queried_by_organisation() {
        let temp = tempfile::tempdir().unwrap();
        let repo = DuckDbRepository::new(temp.path().join("core.db")).unwrap();
        repo.run_migrations(include_str!("../../migrations/0001_initial_schema.sql"))
            .await
            .unwrap();
        assert!(repo.irs_index_year(2024).await.unwrap().is_none());
        let index = temp.path().join("index.csv");
        std::fs::write(&index, "RETURN_ID,FILING_TYPE,EIN,TAX_PERIOD,SUB_DATE,TAXPAYER_NAME,RETURN_TYPE,DLN,OBJECT_ID,XML_BATCH_ID\n1,EFILE,330215585,202306,2024,A,990,1,10,2024_TEOS_XML_05a\n2,EFILE,630809530,202312,2024,B,990EZ,1,11,2024_TEOS_XML_01A\n").unwrap();
        repo.replace_irs_index_year(2024, &index, "u")
            .await
            .unwrap();
        let rows = repo.irs_index_rows(2024, "330215585").await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].object_id, "10");
        assert_eq!(rows[0].batch_id.as_deref(), Some("2024_TEOS_XML_05a"));
        assert!(repo.irs_index_year(2024).await.unwrap().is_some());

        std::fs::write(
            &index,
            "EIN,TAX_PERIOD,TAXPAYER_NAME,RETURN_TYPE,OBJECT_ID\n330215585,202306,A,990,x\n",
        )
        .unwrap();
        assert!(repo
            .replace_irs_index_year(2024, &index, "u")
            .await
            .is_err());
        assert_eq!(
            repo.irs_index_rows(2024, "330215585").await.unwrap().len(),
            1
        );

        std::fs::write(
            &index,
            "EIN,TAX_PERIOD,TAXPAYER_NAME,RETURN_TYPE,OBJECT_ID\n330215585,202406,A,990,12\n",
        )
        .unwrap();
        repo.replace_irs_index_year(2024, &index, "u")
            .await
            .unwrap();
        let rows = repo.irs_index_rows(2024, "330215585").await.unwrap();
        assert_eq!(
            (
                rows.len(),
                rows[0].object_id.as_str(),
                rows[0].batch_id.as_deref()
            ),
            (1, "12", None)
        );
        assert!(repo
            .irs_index_rows(2024, "630809530")
            .await
            .unwrap()
            .is_empty());
    }
}
