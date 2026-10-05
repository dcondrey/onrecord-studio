use crate::{error::AppError, storage::sqlite::DuckDbRepository, types::*};
use duckdb::{params, OptionalExt};

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS organisation_filings (ein TEXT PRIMARY KEY, name TEXT, document_json TEXT NOT NULL, checked_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS organisation_return_xml (object_id TEXT PRIMARY KEY, ein TEXT NOT NULL, member_name TEXT NOT NULL, sha256 TEXT NOT NULL, retrieved_at TEXT NOT NULL, xml TEXT NOT NULL);";

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
    }
}
