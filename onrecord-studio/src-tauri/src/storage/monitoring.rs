use crate::{
    analysis::monitoring::{changed_facts, service_comparisons, significant_facts},
    error::AppError,
    integrity::{is_sha256_hex, sha256_hex},
    storage::sqlite::DuckDbRepository,
    types::monitoring::*,
};
use duckdb::{params, OptionalExt};
use std::io::Read;

fn err(e: impl std::fmt::Display) -> AppError {
    AppError::Storage(e.to_string())
}
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS monitor_heads(source_url TEXT PRIMARY KEY, source_sha256 TEXT NOT NULL, extraction_sha256 TEXT NOT NULL, observed_at TEXT NOT NULL, facts_zstd BLOB NOT NULL);
CREATE TABLE IF NOT EXISTS monitor_changes(id TEXT PRIMARY KEY, source_url TEXT NOT NULL, observed_at TEXT NOT NULL, payload TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS monitor_listings(source_url TEXT PRIMARY KEY, source_sha256 TEXT NOT NULL, links_zstd BLOB NOT NULL);
CREATE TABLE IF NOT EXISTS monitor_coverage(id TEXT PRIMARY KEY, topic TEXT NOT NULL, case_id TEXT, payload TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS monitor_observations(id TEXT PRIMARY KEY, topic TEXT NOT NULL, case_id TEXT, payload TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS monitor_geo_progress(snapshot_id TEXT PRIMARY KEY, last_row_number BIGINT NOT NULL, portal_key TEXT NOT NULL, document_id BIGINT NOT NULL);
";

pub(crate) fn packed<T: serde::Serialize>(value: &T) -> Result<Vec<u8>, AppError> {
    zstd::stream::encode_all(serde_json::to_vec(value).map_err(err)?.as_slice(), 9).map_err(err)
}
pub(crate) fn unpacked<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, AppError> {
    let mut output = Vec::new();
    zstd::stream::read::Decoder::new(bytes)
        .map_err(err)?
        .take(2_000_001)
        .read_to_end(&mut output)
        .map_err(err)?;
    if output.len() > 2_000_000 {
        return Err(AppError::Storage(
            "Monitoring derivative exceeds its bounded size".into(),
        ));
    }
    serde_json::from_slice(&output).map_err(err)
}
fn public_url(value: &str) -> bool {
    reqwest::Url::parse(value).is_ok_and(|u| {
        matches!(u.scheme(), "https" | "http")
            && u.host_str().is_some()
            && u.username().is_empty()
            && u.password().is_none()
    })
}
fn evidence(row: &duckdb::Row<'_>) -> duckdb::Result<MonitoringEvidence> {
    Ok(MonitoringEvidence {
        portal_key: row.get(0)?,
        document_id: row.get(1)?,
        title: row.get(2)?,
        source_url: row.get(3)?,
        source_sha256: row.get(4)?,
        extraction_sha256: row.get(5)?,
        quote: row.get(6)?,
        character_offset: 0,
        snapshot_id: None,
        row_number: None,
    })
}

pub(crate) fn monitoring_revision(conn: &duckdb::Connection) -> Result<String, AppError> {
    let exists: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM information_schema.tables WHERE table_name='monitor_heads')",[],|r|r.get(0)).map_err(err)?;
    if !exists {
        return Ok(String::new());
    }
    let heads:String=conn.query_row("SELECT COALESCE(md5(string_agg(source_url||source_sha256||extraction_sha256,'|' ORDER BY source_url)),'') FROM monitor_heads",[],|r|r.get(0)).map_err(err)?;
    let coverage: String = conn
        .query_row(
            "SELECT COALESCE(md5(string_agg(payload,'|' ORDER BY id)),'') FROM monitor_coverage",
            [],
            |r| r.get(0),
        )
        .map_err(err)?;
    let observations:String=conn.query_row("SELECT COALESCE(md5(string_agg(payload,'|' ORDER BY id)),'') FROM monitor_observations",[],|r|r.get(0)).map_err(err)?;
    Ok(sha256_hex(
        format!("{heads}:{coverage}:{observations}").as_bytes(),
    ))
}

pub(crate) fn purge_monitoring_references(
    conn: &duckdb::Connection,
    portal: &str,
    document: Option<i64>,
) -> Result<(), AppError> {
    let exists: bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM information_schema.tables WHERE table_name='monitor_heads')",[],|r|r.get(0)).map_err(err)?;
    if !exists {
        return Ok(());
    }
    let progress_exists: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM information_schema.tables WHERE table_name='monitor_geo_progress')", [], |r| r.get(0)).map_err(err)?;
    if progress_exists {
        conn.execute("DELETE FROM monitor_geo_progress WHERE portal_key=?1 AND (?2 IS NULL OR document_id=?2)", params![portal, document]).map_err(err)?;
    }
    let mut statement=conn.prepare("SELECT download_url FROM documents WHERE portal_subdomain=?1 AND (?2 IS NULL OR id=?2) AND download_url IS NOT NULL").map_err(err)?;
    let urls = statement
        .query_map(params![portal, document], |r| r.get::<_, String>(0))
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;
    drop(statement);
    for url in urls {
        for table in ["monitor_heads", "monitor_changes", "monitor_listings"] {
            conn.execute(
                &format!("DELETE FROM {table} WHERE source_url=?1"),
                params![url],
            )
            .map_err(err)?;
        }
    }
    let mut statement = conn
        .prepare("SELECT id,payload FROM monitor_observations")
        .map_err(err)?;
    let observations = statement
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;
    drop(statement);
    for (id, payload) in observations {
        let value: ServiceObservation = serde_json::from_str(&payload).map_err(err)?;
        if value.evidence.portal_key == portal
            && document.is_none_or(|d| value.evidence.document_id == d as u64)
        {
            conn.execute("DELETE FROM monitor_observations WHERE id=?1", params![id])
                .map_err(err)?;
        }
    }
    let mut statement = conn
        .prepare("SELECT id,payload FROM monitor_coverage")
        .map_err(err)?;
    let coverage = statement
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;
    drop(statement);
    for (id, payload) in coverage {
        let mut value: CoverageExpectation = serde_json::from_str(&payload).map_err(err)?;
        let old = value.evidence.len();
        value.evidence.retain(|s| {
            s.portal_key != portal || document.is_some_and(|d| s.document_id != d as u64)
        });
        if old != value.evidence.len() {
            if value.evidence.is_empty() {
                value.state = "EXPECTED".into();
            }
            conn.execute(
                "UPDATE monitor_coverage SET payload=?1 WHERE id=?2",
                params![serde_json::to_string(&value).map_err(err)?, id],
            )
            .map_err(err)?;
        }
    }
    let reports_exist:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM information_schema.tables WHERE table_name='research_report_sources')",[],|r|r.get(0)).map_err(err)?;
    if reports_exist {
        conn.execute("DELETE FROM monitor_coverage WHERE EXISTS(SELECT 1 FROM research_report_sources s WHERE s.portal_key=?1 AND (?2 IS NULL OR s.document_id=?2) AND starts_with(monitor_coverage.id,'research:'||s.report_id||':'))",params![portal,document]).map_err(err)?;
    }
    Ok(())
}
fn persist_change(conn: &duckdb::Connection, change: &RecordChange) -> Result<(), AppError> {
    conn.execute(
        "INSERT INTO monitor_changes VALUES (?1,?2,?3,?4) ON CONFLICT DO NOTHING",
        params![
            change.id,
            change.source_url,
            change.observed_at,
            serde_json::to_string(change).map_err(err)?
        ],
    )
    .map_err(err)?;
    Ok(())
}
fn verify_evidence(
    conn: &duckdb::Connection,
    citation: &MonitoringEvidence,
) -> Result<(), AppError> {
    let valid: bool = if let (Some(snapshot), Some(row)) =
        (&citation.snapshot_id, citation.row_number)
    {
        conn.query_row("SELECT EXISTS(SELECT 1 FROM analytical_rows r JOIN analytical_snapshots s ON s.snapshot_id=r.snapshot_id JOIN documents d ON d.download_url=s.source_uri AND d.raw_file_sha256=s.source_sha256 JOIN portals p ON p.subdomain=d.portal_subdomain WHERE r.snapshot_id=?1 AND r.row_number=?2 AND r.row_hash=?3 AND s.source_sha256=?4 AND s.source_uri=?5 AND r.normalized_payload_json=?6 AND d.id=?7 AND d.portal_subdomain=?8 AND lower(trim(d.visibility))='public' AND p.lifecycle_state<>'REMOVED')",params![snapshot,row,citation.extraction_sha256,citation.source_sha256,citation.source_url,citation.quote,citation.document_id,citation.portal_key],|r|r.get(0)).map_err(err)?
    } else {
        let offset = i64::try_from(citation.character_offset).map_err(err)?;
        let length = i64::try_from(citation.quote.chars().count()).map_err(err)?;
        conn.query_row("SELECT EXISTS(SELECT 1 FROM documents d JOIN documents_fts f ON f.document_id=d.id AND f.portal_subdomain=d.portal_subdomain JOIN portals p ON p.subdomain=d.portal_subdomain WHERE d.id=?1 AND d.portal_subdomain=?2 AND d.raw_file_sha256=?3 AND d.extracted_text_projection_sha256=?4 AND d.download_url=?5 AND substr(f.extracted_text,?6+1,?7)=?8 AND lower(trim(d.visibility))='public' AND p.lifecycle_state<>'REMOVED')",params![citation.document_id,citation.portal_key,citation.source_sha256,citation.extraction_sha256,citation.source_url,offset,length,citation.quote],|r|r.get(0)).map_err(err)?
    };
    if !valid || citation.quote.len() < 12 {
        return Err(AppError::DataParsing("The cited source changed, was removed, or no longer supports the exact excerpt. Refresh the evidence before saving.".into()));
    }
    Ok(())
}

impl DuckDbRepository {
    pub async fn monitoring_research_context(
        &self,
        topic: &str,
    ) -> Result<serde_json::Value, AppError> {
        let status = self.monitoring_status(topic.to_string(), None).await?;
        Ok(
            serde_json::json!({"acquisitionPriorities":status.coverage.into_iter().filter(|c|c.expectation.state=="EXPECTED").take(12).map(|c|serde_json::json!({"agency":c.expectation.agency,"recordType":c.expectation.record_type,"period":c.expectation.period,"consequence":c.expectation.consequence,"requestedRecords":c.expectation.acquisition_action,"corpusStatus":c.status})).collect::<Vec<_>>(),"publicationChanges":status.changes.into_iter().take(8).collect::<Vec<_>>(),"serviceComparisons":status.comparisons.into_iter().take(8).collect::<Vec<_>>(),"caution":"Monitoring results are leads. Retrieve the underlying source and verify exact periods, definitions and quotations before making a finding."}),
        )
    }

    pub async fn initialize_monitoring(&self) -> Result<(), AppError> {
        self.query_connection("Initialize source and service monitoring", |conn| {
            conn.execute_batch(SCHEMA).map_err(err)?;
            Ok(())
        })
        .await
    }

    /// Capture accepted extractions before retention releases originals. Repeated calls are
    /// idempotent; only the newest current record for each exact source URL is considered.
    pub async fn capture_monitoring_versions(&self) -> Result<(), AppError> {
        self.capture_monitoring_selection(None, None).await
    }

    pub async fn capture_monitoring_document(
        &self,
        portal: &str,
        document: u64,
    ) -> Result<(), AppError> {
        self.capture_monitoring_selection(Some(portal.to_string()), Some(document))
            .await
    }

    async fn capture_monitoring_selection(
        &self,
        portal: Option<String>,
        document: Option<u64>,
    ) -> Result<(), AppError> {
        self.query_connection("Capture compact publication versions", move |conn| {
            conn.execute_batch(SCHEMA).map_err(err)?;
            let mut statement = conn.prepare("WITH current AS (SELECT d.portal_subdomain,d.id,COALESCE(d.file_name,'Document') AS title,d.download_url AS url,d.raw_file_sha256 AS digest,COALESCE(d.extracted_text_projection_sha256,'') AS extraction,substr(f.extracted_text,1,500000) AS text,row_number() OVER(PARTITION BY d.download_url ORDER BY d.updated_at DESC,d.id DESC) AS ordinal FROM documents d JOIN documents_fts f ON f.document_id=d.id AND f.portal_subdomain=d.portal_subdomain JOIN portals p ON p.subdomain=d.portal_subdomain WHERE lower(trim(d.visibility))='public' AND p.lifecycle_state<>'REMOVED' AND d.extraction_status IN ('SEARCHABLE','SALVAGED') AND length(d.raw_file_sha256)=64 AND d.download_url IS NOT NULL AND (?1 IS NULL OR d.portal_subdomain=?1) AND (?2 IS NULL OR d.id=?2)) SELECT c.portal_subdomain,c.id,c.title,c.url,c.digest,c.extraction,c.text FROM current c LEFT JOIN monitor_heads h ON h.source_url=c.url WHERE ordinal=1 AND (h.source_url IS NULL OR h.source_sha256<>c.digest OR h.extraction_sha256<>c.extraction) ORDER BY c.url LIMIT 60").map_err(err)?;
            let records = statement.query_map(params![portal, document], evidence).map_err(err)?.collect::<Result<Vec<_>,_>>().map_err(err)?;
            drop(statement);
            for record in records {
                if !public_url(&record.source_url) { continue; }
                let facts = significant_facts(&record.quote);
                let previous: Option<(String,String,Vec<u8>)> = conn.query_row("SELECT source_sha256,extraction_sha256,facts_zstd FROM monitor_heads WHERE source_url=?1",params![record.source_url],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(err)?;
                let now = chrono::Utc::now().to_rfc3339();
                let tx = conn.transaction().map_err(err)?;
                if let Some((old_hash,old_extraction,bytes)) = previous {
                    let old: Vec<MonitoringFact> = unpacked(&bytes)?;
                    let differences = changed_facts(&old,&facts);
                    let categories = if differences.is_empty() { vec![(if old_extraction == record.extraction_sha256 { "SOURCE_BYTES" } else { "TEXT_OUTSIDE_MONITORED_FACTS" }.to_string(),None,None)] } else { differences };
                    for (category,before_quote,after_quote) in categories {
                        persist_change(&tx,&RecordChange { id: uuid::Uuid::new_v4().to_string(), source_url: record.source_url.clone(), title: record.title.clone(), observed_at: now.clone(), category, before_sha256: old_hash.clone(), after_sha256: record.source_sha256.clone(), before_quote, after_quote, interpretation: "The retrieved source changed. Compare the quoted versions and their effective periods; changed wording or extraction does not establish a changed obligation, expenditure or outcome.".into(), review_state: "NEEDS_REVIEW".into() })?;
                    }
                }
                tx.execute("INSERT INTO monitor_heads VALUES (?1,?2,?3,?4,?5) ON CONFLICT(source_url) DO UPDATE SET source_sha256=excluded.source_sha256,extraction_sha256=excluded.extraction_sha256,observed_at=excluded.observed_at,facts_zstd=excluded.facts_zstd",params![record.source_url,record.source_sha256,record.extraction_sha256,now,packed(&facts)?]).map_err(err)?;
                tx.commit().map_err(err)?;
            }
            Ok(())
        }).await
    }

    pub async fn refresh_monitoring(&self) -> Result<(), AppError> {
        self.capture_monitoring_versions().await?;
        self.import_monitoring_geo_rows().await?;
        // Dossier requests become coverage expectations linked to their topic, with the
        // actual request draft retained as the acquisition action. No request is sent.
        if let Ok(status) = self.research_status().await {
            let period_pattern =
                regex::Regex::new(r"(?i)\bFY\s*20[0-9]{2}\b|\b20[0-9]{2}-[0-9]{2}-[0-9]{2}\b")
                    .expect("The coverage period expression is valid");
            for report in status.reports.into_iter().take(10) {
                for (index, request) in report.synthesis.requests.into_iter().enumerate() {
                    let id = format!("research:{}:{index}", report.id);
                    let mut terms = request
                        .title
                        .split(|c: char| !c.is_alphanumeric())
                        .filter(|s| s.len() > 4)
                        .filter(|s| {
                            !["records", "request", "supporting", "existing", "provide"]
                                .contains(&s.to_lowercase().as_str())
                        })
                        .take(5)
                        .map(str::to_lowercase)
                        .collect::<Vec<_>>();
                    if terms.is_empty() {
                        terms.push(report.topic.clone());
                    }
                    let period = period_pattern
                        .find(&request.draft)
                        .map(|m| m.as_str().to_string())
                        .unwrap_or_else(|| "Specify the requested period".into());
                    let expectation = CoverageExpectation {
                        id,
                        case_id: None,
                        topic: report.topic.clone(),
                        agency: request.custodian,
                        record_type: request.title,
                        period,
                        query_terms: terms,
                        consequence: request.resolves,
                        acquisition_action: request.draft,
                        state: "EXPECTED".into(),
                        evidence: vec![],
                    };
                    let raw = serde_json::to_string(&expectation).map_err(err)?;
                    self.query_connection("Register consequential acquisition gap",move|conn|{conn.execute("INSERT INTO monitor_coverage VALUES (?1,?2,NULL,?3) ON CONFLICT DO NOTHING",params![expectation.id,expectation.topic,raw]).map_err(err)?;Ok(())}).await?;
                }
            }
        }
        Ok(())
    }

    /// Call only after a successful inspection. A removed link means absent from this
    /// retrieved listing, not that its target was deleted or the agency concealed it.
    pub async fn record_publication_listing(
        &self,
        source_url: String,
        source_sha256: String,
        links: Vec<String>,
    ) -> Result<(), AppError> {
        if !public_url(&source_url) || !is_sha256_hex(&source_sha256) {
            return Ok(());
        }
        let mut links = links
            .into_iter()
            .filter(|u| public_url(u))
            .take(4000)
            .collect::<Vec<_>>();
        links.sort();
        links.dedup();
        self.query_connection("Compare successful publication listings",move|conn| {
            conn.execute_batch(SCHEMA).map_err(err)?;
            let previous: Option<(String,Vec<u8>)> = conn.query_row("SELECT source_sha256,links_zstd FROM monitor_listings WHERE source_url=?1",params![source_url],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(err)?;
            let tx=conn.transaction().map_err(err)?;
            if let Some((digest,bytes))=previous {
                if digest==source_sha256 { return Ok(()); }
                let old:Vec<String>=unpacked(&bytes)?;
                let now=chrono::Utc::now().to_rfc3339();
                for missing in old.iter().filter(|u|!links.contains(u)).take(40) {
                    persist_change(&tx,&RecordChange{id:uuid::Uuid::new_v4().to_string(),source_url:source_url.clone(),title:"Publication listing changed".into(),observed_at:now.clone(),category:"LISTING_REFERENCE_REMOVED".into(),before_sha256:digest.clone(),after_sha256:source_sha256.clone(),before_quote:Some(missing.clone()),after_quote:None,interpretation:"This URL was present in the preceding successful listing and is absent from the current bounded listing. The target may still exist, have moved, or belong to a different page; verify it before claiming disappearance.".into(),review_state:"NEEDS_REVIEW".into()})?;
                }
            }
            tx.execute("INSERT INTO monitor_listings VALUES (?1,?2,?3) ON CONFLICT(source_url) DO UPDATE SET source_sha256=excluded.source_sha256,links_zstd=excluded.links_zstd",params![source_url,source_sha256,packed(&links)?]).map_err(err)?;tx.commit().map_err(err)?;Ok(())
        }).await
    }

    pub async fn save_coverage_expectation(
        &self,
        mut value: CoverageExpectation,
    ) -> Result<(), AppError> {
        if value.id.is_empty() {
            value.id = uuid::Uuid::new_v4().to_string();
        }
        if value.agency.trim().is_empty()
            || value.record_type.trim().is_empty()
            || value.period.trim().is_empty()
            || value.consequence.trim().is_empty()
            || value.query_terms.is_empty()
            || value.query_terms.len() > 8
            || value
                .query_terms
                .iter()
                .any(|s| s.len() < 3 || s.len() > 100)
            || !["EXPECTED", "RECEIVED", "WAIVED"].contains(&value.state.as_str())
            || (value.state == "RECEIVED" && value.evidence.is_empty())
            || serde_json::to_vec(&value).map_err(err)?.len() > 24000
        {
            return Err(AppError::DataParsing("Record an agency, record type, period, useful search terms and consequence; receiving an expectation requires cited evidence.".into()));
        }
        self.query_connection("Save investigative coverage expectation",move|conn|{for source in &value.evidence {verify_evidence(conn,source)?;}conn.execute("INSERT INTO monitor_coverage VALUES (?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET topic=excluded.topic,case_id=excluded.case_id,payload=excluded.payload",params![value.id,value.topic,value.case_id,serde_json::to_string(&value).map_err(err)?]).map_err(err)?;Ok(())}).await
    }

    pub async fn save_service_observation(
        &self,
        mut value: ServiceObservation,
    ) -> Result<(), AppError> {
        validate_observation(&value)?;
        if value.id.is_empty() {
            value.id = uuid::Uuid::new_v4().to_string();
        }
        self.query_connection("Save sourced service observation",move|conn|{verify_evidence(conn,&value.evidence)?;conn.execute("INSERT INTO monitor_observations VALUES (?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET topic=excluded.topic,case_id=excluded.case_id,payload=excluded.payload",params![value.id,value.topic,value.case_id,serde_json::to_string(&value).map_err(err)?]).map_err(err)?;Ok(())}).await
    }

    pub async fn monitoring_status(
        &self,
        topic: String,
        case_id: Option<String>,
    ) -> Result<MonitoringStatus, AppError> {
        self.query_connection("Read investigative monitoring", move|conn| {
            conn.execute_batch(SCHEMA).map_err(err)?;
            let mut statement=conn.prepare("SELECT payload FROM monitor_coverage WHERE topic=?1 AND (case_id IS NULL OR case_id=?2) ORDER BY id LIMIT 100").map_err(err)?;
            let expectations=statement.query_map(params![topic,case_id],|r|r.get::<_,String>(0)).map_err(err)?.collect::<Result<Vec<_>,_>>().map_err(err)?.into_iter().map(|s|serde_json::from_str::<CoverageExpectation>(&s).map_err(err)).collect::<Result<Vec<_>,_>>()?;drop(statement);
            let mut statement=conn.prepare("SELECT d.portal_subdomain,d.id,COALESCE(d.file_name,'Document'),d.download_url,d.raw_file_sha256,COALESCE(d.extracted_text_projection_sha256,''),substr(f.extracted_text,1,12000) FROM documents d JOIN documents_fts f ON f.document_id=d.id AND f.portal_subdomain=d.portal_subdomain JOIN portals p ON p.subdomain=d.portal_subdomain WHERE lower(trim(d.visibility))='public' AND p.lifecycle_state<>'REMOVED' AND d.extraction_status IN ('SEARCHABLE','SALVAGED') AND d.download_url IS NOT NULL AND length(d.raw_file_sha256)=64 ORDER BY d.updated_at DESC,d.id DESC LIMIT 250").map_err(err)?;
            let sources=statement.query_map([],evidence).map_err(err)?.collect::<Result<Vec<_>,_>>().map_err(err)?;drop(statement);
            let haystacks=sources.iter().map(|s|format!("{} {}",s.title,s.quote).to_lowercase()).collect::<Vec<_>>();
            let mut coverage=Vec::new();
            for expectation in expectations {
                let candidates=sources.iter().zip(haystacks.iter()).filter(|(_,haystack)|expectation.query_terms.iter().filter(|term|haystack.contains(&term.to_lowercase())).count()*2>=expectation.query_terms.len()).map(|(s,_)|s).take(5).map(|s|citation_window(s,&expectation.query_terms)).collect::<Vec<_>>();
                let status=if expectation.state=="WAIVED"{"WAIVED"}else if expectation.state=="RECEIVED" && expectation.evidence.iter().all(|s|verify_evidence(conn,s).is_ok()){"RECEIPT_RECORDED"}else if expectation.state=="RECEIVED"{"SOURCE_CHANGED"}else if candidates.is_empty(){"NOT_LOCATED"}else{"MATCHES_NEED_REVIEW"};
                let explanation=match status{"NOT_LOCATED"=>"No matching record was located in the bounded corpus search. This does not establish that the agency lacks it.","MATCHES_NEED_REVIEW"=>"Related records were found. Check custodian, record type, exact period and completeness before recording receipt.","SOURCE_CHANGED"=>"Previously recorded supporting evidence changed or was removed; verify the production again.","RECEIPT_RECORDED"=>"Receipt was recorded against these source identities. Review completeness against the expectation.",_=>"Expectation waived by the researcher."};
                coverage.push(CoverageAssessment{expectation,status:status.into(),explanation:explanation.into(),candidates});
            }
            let mut statement=conn.prepare("SELECT c.payload FROM monitor_changes c WHERE EXISTS(SELECT 1 FROM documents d JOIN portals p ON p.subdomain=d.portal_subdomain WHERE d.download_url=c.source_url AND lower(trim(d.visibility))='public' AND p.lifecycle_state<>'REMOVED') ORDER BY c.observed_at DESC LIMIT 80").map_err(err)?;
            let changes=statement.query_map([],|r|r.get::<_,String>(0)).map_err(err)?.collect::<Result<Vec<_>,_>>().map_err(err)?.into_iter().map(|s|serde_json::from_str(&s).map_err(err)).collect::<Result<Vec<RecordChange>,_>>()?;drop(statement);
            let mut statement=conn.prepare("SELECT payload FROM monitor_observations WHERE topic=?1 AND (case_id IS NULL OR case_id=?2) ORDER BY id LIMIT 500").map_err(err)?;
            let mut observations=statement.query_map(params![topic,case_id],|r|r.get::<_,String>(0)).map_err(err)?.collect::<Result<Vec<_>,_>>().map_err(err)?.into_iter().map(|s|serde_json::from_str(&s).map_err(err)).collect::<Result<Vec<ServiceObservation>,_>>()?;
            observations.retain(|observation| conn.query_row("SELECT EXISTS(SELECT 1 FROM documents d JOIN portals p ON p.subdomain=d.portal_subdomain WHERE d.id=?1 AND d.portal_subdomain=?2 AND lower(trim(d.visibility))='public' AND p.lifecycle_state<>'REMOVED')",params![observation.evidence.document_id,observation.evidence.portal_key],|r|r.get::<_,bool>(0)).unwrap_or(false));
            for observation in &mut observations {if verify_evidence(conn,&observation.evidence).is_err(){observation.reviewed=false;observation.status="SOURCE_CHANGED".into();}}
            let comparisons=service_comparisons(&observations);
            Ok(MonitoringStatus {topic,case_id,coverage,changes,observations,comparisons,source_candidates:sources.iter().take(30).map(|s|citation_window(s,&[])).collect(),limitations:vec!["Coverage search examines up to 250 current public records and their first 12,000 characters. Matching is a candidate lead, not proof of receipt or completeness.".into(),"Version monitoring compares up to 160 material statements in the first 500,000 characters. Baselines begin when monitoring is enabled; retrieval time is not publication or effective time.".into(),"Service comparisons use reviewed observations only. Missing coordinates and hours remain unknown; complaints and administrative closure are not automatically classified as operations.".into()]})
        }).await
    }

    async fn import_monitoring_geo_rows(&self) -> Result<(), AppError> {
        self.query_connection("Read explicitly geocoded public table observations",|conn|{
            let mut statement=conn.prepare("SELECT r.snapshot_id,r.row_number,r.row_hash,r.normalized_payload_json,s.display_name,d.portal_subdomain,d.id,s.source_uri,s.source_sha256 FROM analytical_rows r JOIN analytical_snapshots s ON s.snapshot_id=r.snapshot_id JOIN documents d ON d.download_url=s.source_uri AND d.raw_file_sha256=s.source_sha256 JOIN portals p ON p.subdomain=d.portal_subdomain WHERE r.malformed=0 AND lower(trim(d.visibility))='public' AND p.lifecycle_state<>'REMOVED' AND EXISTS(SELECT 1 FROM analytical_columns c WHERE c.snapshot_id=s.snapshot_id AND lower(c.column_name) IN ('latitude','lat')) AND NOT EXISTS(SELECT 1 FROM monitor_geo_progress g WHERE g.snapshot_id=r.snapshot_id AND g.last_row_number>=r.row_number) AND NOT EXISTS(SELECT 1 FROM analytical_snapshots o JOIN monitor_observations m ON m.id=o.snapshot_id||':'||CAST(r.row_number AS VARCHAR) WHERE o.snapshot_id<>s.snapshot_id AND o.source_sha256=s.source_sha256 AND o.source_uri IS NOT DISTINCT FROM s.source_uri) ORDER BY s.retrieved_at DESC,r.row_number LIMIT 500").map_err(err)?;
            let rows=statement.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,u64>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,String>(5)?,r.get::<_,u64>(6)?,r.get::<_,String>(7)?,r.get::<_,String>(8)?))).map_err(err)?.collect::<Result<Vec<_>,_>>().map_err(err)?;drop(statement);
            let tx = conn.transaction().map_err(err)?;
            for (snapshot,row,hash,raw,title,portal,id,url,source_hash) in rows {
                if let Some(observation)=geo_row_observation(&snapshot,row,&hash,&raw,&title,&portal,id,&url,&source_hash) {
                    tx.execute("INSERT INTO monitor_observations VALUES (?1,?2,NULL,?3) ON CONFLICT DO NOTHING",params![observation.id,observation.topic,serde_json::to_string(&observation).map_err(err)?]).map_err(err)?;
                }
                // Snapshot rows are immutable. One cursor per snapshot lets a bounded
                // batch advance past unsupported rows without storing every rejection.
                tx.execute("INSERT INTO monitor_geo_progress VALUES (?1,?2,?3,?4) ON CONFLICT(snapshot_id) DO UPDATE SET last_row_number=greatest(monitor_geo_progress.last_row_number,excluded.last_row_number),portal_key=excluded.portal_key,document_id=excluded.document_id", params![snapshot,row,portal,id]).map_err(err)?;
            }
            tx.commit().map_err(err)?;
            Ok(())
        }).await
    }
}

fn citation_window(source: &MonitoringEvidence, terms: &[String]) -> MonitoringEvidence {
    let mut value = source.clone();
    let lower = source.quote.to_lowercase();
    let byte = terms
        .iter()
        .filter_map(|t| lower.find(&t.to_lowercase()))
        .min()
        .unwrap_or(0);
    // Search indices in a lowercased copy can differ for Unicode; the window remains
    // anchored to a character position and is sliced from the unmodified source.
    let start = lower[..byte]
        .chars()
        .count()
        .saturating_sub(160)
        .min(source.quote.chars().count());
    value.character_offset = start;
    value.quote = source.quote.chars().skip(start).take(1400).collect();
    value
}

fn validate_observation(value: &ServiceObservation) -> Result<(), AppError> {
    if !["FACILITY", "OPERATION", "OBSERVATION"].contains(&value.kind.as_str())
        || value.place.trim().is_empty()
        || value.place.len() > 240
        || chrono::NaiveDate::parse_from_str(&value.observed_on, "%Y-%m-%d").is_err()
        || value.latitude.is_some() != value.longitude.is_some()
        || value
            .latitude
            .is_some_and(|x| !x.is_finite() || !(-90.0..=90.0).contains(&x))
        || value
            .longitude
            .is_some_and(|x| !x.is_finite() || !(-180.0..=180.0).contains(&x))
        || value
            .hours_per_week
            .is_some_and(|x| !x.is_finite() || !(0.0..=168.0).contains(&x))
        || value.evidence.quote.len() > 12000
    {
        return Err(AppError::DataParsing("Use a cited public place, ISO observation date, paired valid coordinates and 0–168 weekly hours when known.".into()));
    }
    let numbers = regex::Regex::new(r"-?[0-9]+(?:\.[0-9]+)?")
        .unwrap()
        .find_iter(&value.evidence.quote)
        .filter_map(|m| m.as_str().parse::<f64>().ok())
        .collect::<Vec<_>>();
    if [value.latitude, value.longitude]
        .into_iter()
        .flatten()
        .any(|coordinate| {
            !numbers
                .iter()
                .any(|number| (number - coordinate).abs() < 0.000001)
        })
    {
        return Err(AppError::DataParsing("Coordinates must appear explicitly in the cited source; no inferred geocoding is accepted.".into()));
    }
    let quoted = value.evidence.quote.to_lowercase();
    let date = chrono::NaiveDate::parse_from_str(&value.observed_on, "%Y-%m-%d").map_err(err)?;
    if !quoted.contains(&value.place.to_lowercase())
        || ![
            value.observed_on.clone(),
            date.format("%B %-d, %Y").to_string().to_lowercase(),
            date.format("%-m/%-d/%Y").to_string(),
        ]
        .iter()
        .any(|date| quoted.contains(date))
    {
        return Err(AppError::DataParsing(
            "The recorded place and observation date must appear in the cited excerpt.".into(),
        ));
    }
    if let Some(hours) = value.hours_per_week {
        if ![
            "hours_per_week",
            "weekly_hours",
            "hours per week",
            "hours/week",
            "weekly hours",
        ]
        .iter()
        .any(|label| quoted.contains(label))
            || !numbers
                .iter()
                .any(|number| (number - hours).abs() < 0.000001)
        {
            return Err(AppError::DataParsing("Weekly hours need an explicit weekly-hours measurement in the cited source; leave the field blank when unknown.".into()));
        }
    }
    if value.kind == "OPERATION"
        && ![
            "operation",
            "abatement",
            "cleanup",
            "clean-up",
            "encampment removal",
        ]
        .iter()
        .any(|label| quoted.contains(label))
    {
        return Err(AppError::DataParsing("An operation observation needs a source explicitly documenting an operation, not only a service request.".into()));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn geo_row_observation(
    snapshot: &str,
    row: u64,
    hash: &str,
    raw: &str,
    title: &str,
    portal: &str,
    id: u64,
    url: &str,
    source_hash: &str,
) -> Option<ServiceObservation> {
    if raw.len() > 12000 {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    let object = value.as_object()?;
    let string = |names: &[&str]| {
        names.iter().find_map(|name| {
            object.get(*name).and_then(|v| {
                v.as_str()
                    .map(str::to_string)
                    .or_else(|| v.as_i64().map(|x| x.to_string()))
            })
        })
    };
    let number = |names: &[&str]| {
        names.iter().find_map(|name| {
            object
                .get(*name)
                .and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse().ok()))
        })
    };
    let place = string(&["facility_name", "site_name", "name", "location", "address"])?;
    let date = string(&[
        "operation_date",
        "observed_on",
        "observation_date",
        "date",
        "requested_datetime",
        "date_requested",
    ])?;
    let observed_on = date.chars().take(10).collect::<String>();
    let descriptor = format!(
        "{title} {}",
        string(&["service_name", "service_type", "operation_type", "type"]).unwrap_or_default()
    )
    .to_lowercase();
    let kind = if object.contains_key("operation_id") && object.contains_key("operation_date") {
        "OPERATION"
    } else if ["restroom", "bathroom", "facility", "shelter"]
        .iter()
        .any(|s| descriptor.contains(s))
    {
        "FACILITY"
    } else {
        "OBSERVATION"
    };
    let topic = if descriptor.contains("restroom") || descriptor.contains("bathroom") {
        "bathrooms"
    } else if descriptor.contains("encampment") || descriptor.contains("abatement") {
        "abatement"
    } else {
        "homelessness"
    };
    let result = ServiceObservation {
        id: format!("{snapshot}:{row}"),
        case_id: None,
        topic: topic.into(),
        kind: kind.into(),
        place,
        observed_on,
        latitude: number(&["latitude", "lat"]),
        longitude: number(&["longitude", "lon", "lng"]),
        hours_per_week: number(&["hours_per_week", "weekly_hours"]),
        status: string(&["status", "operating_status"]).unwrap_or_else(|| "NOT_RECORDED".into()),
        evidence: MonitoringEvidence {
            portal_key: portal.into(),
            document_id: id,
            title: title.into(),
            source_url: url.into(),
            source_sha256: source_hash.into(),
            extraction_sha256: hash.into(),
            quote: raw.into(),
            character_offset: 0,
            snapshot_id: Some(snapshot.into()),
            row_number: Some(row),
        },
        reviewed: false,
    };
    (result.latitude.is_some()
        && result.longitude.is_some()
        && validate_observation(&result).is_ok())
    .then_some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn setup() -> DuckDbRepository {
        let db = DuckDbRepository::new(":memory:").unwrap();
        db.run_migrations(include_str!("../../migrations/0001_initial_schema.sql"))
            .await
            .unwrap();
        let conn = db.analytics_connection().await.unwrap();
        conn.execute_batch("INSERT INTO portals(subdomain,display_name,base_url,lifecycle_state) VALUES ('city','City of San Diego','https://www.sandiego.gov','ACTIVE'); INSERT INTO requests(id,portal_subdomain,title,status,created_at,updated_at) VALUES (42,'city','Public service records','closed','2026-09-26','2026-09-26')").unwrap();
        db
    }
    fn insert(conn: &duckdb::Connection, text: &str) {
        conn.execute("INSERT INTO documents(id,request_id,portal_subdomain,file_name,download_url,visibility,created_at,updated_at,raw_file_sha256,extracted_text_projection_sha256,extraction_status) VALUES (1,42,'city','Riverbed contract','https://www.sandiego.gov/riverbed.pdf','public','2026-09-26','2026-09-26',?1,?2,'SEARCHABLE')",params!["a".repeat(64),sha256_hex(text.as_bytes())]).unwrap();
        conn.execute("INSERT INTO documents_fts(document_id,portal_subdomain,file_name,extracted_text) VALUES (1,'city','Riverbed contract',?1)",params![text]).unwrap();
    }
    #[tokio::test]
    async fn existing_database_gains_monitoring_without_rewriting_legacy_records() {
        let db = DuckDbRepository::new(":memory:").unwrap();
        let conn = db.analytics_connection().await.unwrap();
        conn.execute_batch("CREATE TABLE documents(id BIGINT, title TEXT); INSERT INTO documents VALUES(9,'Existing public record')").unwrap();
        assert_eq!(monitoring_revision(&conn).unwrap(), "");
        purge_monitoring_references(&conn, "legacy", None).unwrap();
        db.initialize_monitoring().await.unwrap();
        db.initialize_monitoring().await.unwrap();
        assert_eq!(
            conn.query_row("SELECT title FROM documents WHERE id=9", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "Existing public record"
        );
    }
    #[tokio::test]
    async fn records_material_versions_after_original_release_and_only_on_changes() {
        let db = setup().await;
        let conn = db.analytics_connection().await.unwrap();
        insert(
            &conn,
            "Grant allocation is $3,000,000.\nThe provider shall deliver 40 housing placements.\n",
        );
        db.refresh_monitoring().await.unwrap();
        assert_eq!(
            conn.query_row("SELECT count(*) FROM monitor_changes", [], |r| r
                .get::<_, u64>(0))
                .unwrap(),
            0
        );
        let replacement =
            "Grant allocation is $4,000,000.\nThe provider shall deliver 20 housing placements.\n";
        conn.execute(
            "UPDATE documents SET raw_file_sha256=?1,extracted_text_projection_sha256=?2",
            params!["b".repeat(64), sha256_hex(replacement.as_bytes())],
        )
        .unwrap();
        conn.execute(
            "UPDATE documents_fts SET extracted_text=?1",
            params![replacement],
        )
        .unwrap();
        db.refresh_monitoring().await.unwrap();
        let status = db
            .monitoring_status("homelessness".into(), None)
            .await
            .unwrap();
        assert!(status.changes.iter().any(|c| c.category == "MONEY"
            && c.before_quote.as_ref().unwrap().contains("3,000,000")
            && c.after_quote.as_ref().unwrap().contains("4,000,000")));
        let revision = monitoring_revision(&conn).unwrap();
        let count = status.changes.len();
        db.refresh_monitoring().await.unwrap();
        assert_eq!(
            db.monitoring_status("homelessness".into(), None)
                .await
                .unwrap()
                .changes
                .len(),
            count
        );
        conn.execute("UPDATE monitor_heads SET observed_at='2099-01-01'", [])
            .unwrap();
        assert_eq!(
            monitoring_revision(&conn).unwrap(),
            revision,
            "Polling timestamps must not wake another research cycle"
        );
        purge_monitoring_references(&conn, "city", Some(1)).unwrap();
        assert_eq!(
            conn.query_row("SELECT count(*) FROM monitor_heads", [], |r| r
                .get::<_, u64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            conn.query_row("SELECT count(*) FROM monitor_changes", [], |r| r
                .get::<_, u64>(0))
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn extraction_hook_captures_its_document_even_with_a_large_unprocessed_backlog() {
        let db = setup().await;
        let conn = db.analytics_connection().await.unwrap();
        let text = "Public service allocation is $1,000.";
        conn.execute("INSERT INTO documents(id,request_id,portal_subdomain,file_name,download_url,visibility,created_at,updated_at,raw_file_sha256,extracted_text_projection_sha256,extraction_status) SELECT id,42,'city','Record','https://www.sandiego.gov/record-'||CAST(id AS VARCHAR)||'.pdf','public','2026-09-26','2026-09-26',?1,?2,'SEARCHABLE' FROM range(1,71) t(id)",params!["a".repeat(64),sha256_hex(text.as_bytes())]).unwrap();
        conn.execute("INSERT INTO documents_fts(document_id,portal_subdomain,file_name,extracted_text) SELECT id,'city','Record',?1 FROM range(1,71) t(id)",params![text]).unwrap();
        db.capture_monitoring_document("city", 70).await.unwrap();
        let urls = conn
            .prepare("SELECT source_url FROM monitor_heads")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(urls, vec!["https://www.sandiego.gov/record-70.pdf"]);
    }
    #[tokio::test]
    async fn only_successful_listing_snapshots_can_establish_removed_references() {
        let db = setup().await;
        let url = "https://www.sandiego.gov/meetings".to_string();
        db.record_publication_listing(
            url.clone(),
            "a".repeat(64),
            vec![
                "https://www.sandiego.gov/contract.pdf".into(),
                "https://www.sandiego.gov/agenda.pdf".into(),
            ],
        )
        .await
        .unwrap();
        db.record_publication_listing(
            url.clone(),
            "b".repeat(64),
            vec!["https://www.sandiego.gov/agenda.pdf".into()],
        )
        .await
        .unwrap();
        let conn = db.analytics_connection().await.unwrap();
        let payload: String = conn
            .query_row("SELECT payload FROM monitor_changes", [], |r| r.get(0))
            .unwrap();
        let change: RecordChange = serde_json::from_str(&payload).unwrap();
        assert_eq!(change.category, "LISTING_REFERENCE_REMOVED");
        assert!(change.interpretation.contains("target may still exist"));
        assert_eq!(
            change.before_quote.as_deref(),
            Some("https://www.sandiego.gov/contract.pdf")
        );
        db.record_publication_listing(url, "not a valid source hash".into(), vec![])
            .await
            .unwrap();
        assert_eq!(
            conn.query_row("SELECT count(*) FROM monitor_changes", [], |r| r
                .get::<_, u64>(0))
                .unwrap(),
            1
        );
    }
    #[tokio::test]
    async fn saves_only_live_exact_citations_and_keeps_missing_measurements_unknown() {
        let db = setup().await;
        let conn = db.analytics_connection().await.unwrap();
        let text="Public river trail operation, 2026-09-01. Latitude 32.75, longitude -117.1. Weekly hours not recorded.";
        insert(&conn, text);
        db.initialize_monitoring().await.unwrap();
        let citation = MonitoringEvidence {
            portal_key: "city".into(),
            document_id: 1,
            title: "Riverbed contract".into(),
            source_url: "https://www.sandiego.gov/riverbed.pdf".into(),
            source_sha256: "a".repeat(64),
            extraction_sha256: sha256_hex(text.as_bytes()),
            quote: text.into(),
            character_offset: 0,
            snapshot_id: None,
            row_number: None,
        };
        let observation = ServiceObservation {
            id: "operation-1".into(),
            case_id: Some("riverbed".into()),
            topic: "abatement".into(),
            kind: "OPERATION".into(),
            place: "Public river trail".into(),
            observed_on: "2026-09-01".into(),
            latitude: Some(32.75),
            longitude: Some(-117.1),
            hours_per_week: None,
            status: "DOCUMENTED".into(),
            evidence: citation.clone(),
            reviewed: true,
        };
        db.save_service_observation(observation.clone())
            .await
            .unwrap();
        let status = db
            .monitoring_status("abatement".into(), Some("riverbed".into()))
            .await
            .unwrap();
        assert_eq!(status.observations[0].hours_per_week, None);
        let mut guessed = observation.clone();
        guessed.latitude = Some(33.1);
        assert!(db
            .save_service_observation(guessed)
            .await
            .unwrap_err()
            .to_string()
            .contains("Coordinates must appear"));
        conn.execute(
            "UPDATE documents SET raw_file_sha256=?1",
            params!["b".repeat(64)],
        )
        .unwrap();
        assert!(db
            .save_service_observation(observation)
            .await
            .unwrap_err()
            .to_string()
            .contains("source changed"));
        assert!(
            !db.monitoring_status("abatement".into(), Some("riverbed".into()))
                .await
                .unwrap()
                .observations[0]
                .reviewed
        );
    }
    #[test]
    fn imported_complaints_are_unreviewed_observations_not_completed_operations() {
        let raw = r#"{"latitude":32.75,"longitude":-117.1,"location":"Public river trail","requested_datetime":"2026-09-01T12:00:00Z","service_type":"Encampment","status":"Closed"}"#;
        let value = geo_row_observation(
            "snapshot",
            1,
            &"a".repeat(64),
            raw,
            "Get It Done reports",
            "city",
            1,
            "https://data.sandiego.gov/fixture.csv",
            &"b".repeat(64),
        )
        .unwrap();
        assert_eq!(value.kind, "OBSERVATION");
        assert!(!value.reviewed);
        assert_eq!(value.hours_per_week, None);
        assert_eq!(value.observed_on, "2026-09-01");
        assert!(geo_row_observation(
            "snapshot",
            2,
            &"a".repeat(64),
            &raw.replace("32.75", "999"),
            "Get It Done reports",
            "city",
            1,
            "https://data.sandiego.gov/fixture.csv",
            &"b".repeat(64)
        )
        .is_none());
    }

    #[tokio::test]
    async fn unsupported_geospatial_rows_do_not_starve_later_supported_rows() {
        let db = setup().await;
        let conn = db.analytics_connection().await.unwrap();
        insert(&conn, "Public restroom operating inventory and schedules.");
        conn.execute("INSERT INTO analytical_snapshots(snapshot_id,dataset_kind,display_name,table_name,source_uri,local_path,source_sha256,retrieved_at,validation_summary_json) VALUES ('geo','generic','Public restroom operating hours','geo_fixture','https://www.sandiego.gov/riverbed.pdf','',?1,'2026-09-26','{}')",params!["a".repeat(64)]).unwrap();
        conn.execute_batch("INSERT INTO analytical_columns(snapshot_id,table_name,column_name,original_name,logical_type,ordinal) VALUES ('geo','geo_fixture','latitude','latitude','DOUBLE',0)").unwrap();
        let unsupported = r#"{"latitude":32.75,"longitude":-117.1,"name":"Public restroom"}"#;
        conn.execute("INSERT INTO analytical_row_ledger SELECT 'geo',row_number,?1,?2,?2,0,'[]' FROM range(1,501) t(row_number)", params![sha256_hex(unsupported.as_bytes()), unsupported]).unwrap();
        let supported = r#"{"latitude":32.75,"longitude":-117.1,"name":"Public restroom","observed_on":"2026-09-26","weekly_hours":56}"#;
        conn.execute(
            "INSERT INTO analytical_row_ledger VALUES ('geo',501,?1,?2,?2,0,'[]')",
            params![sha256_hex(supported.as_bytes()), supported],
        )
        .unwrap();
        db.initialize_monitoring().await.unwrap();
        db.import_monitoring_geo_rows().await.unwrap();
        assert_eq!(
            conn.query_row("SELECT count(*) FROM monitor_observations", [], |r| r
                .get::<_, u64>(0))
                .unwrap(),
            0
        );
        db.import_monitoring_geo_rows().await.unwrap();
        let status = db
            .monitoring_status("bathrooms".into(), None)
            .await
            .unwrap();
        assert_eq!(status.observations.len(), 1);
        assert_eq!(status.observations[0].evidence.row_number, Some(501));
        assert_eq!(
            conn.query_row("SELECT count(*) FROM monitor_geo_progress", [], |r| r
                .get::<_, u64>(0))
                .unwrap(),
            1,
            "Progress storage grows by snapshot, not by rejected row"
        );
        purge_monitoring_references(&conn, "city", Some(1)).unwrap();
        assert_eq!(
            conn.query_row("SELECT count(*) FROM monitor_geo_progress", [], |r| r
                .get::<_, u64>(0))
                .unwrap(),
            0
        );
    }

    /// A re-import of the same file under a newer parser is a new snapshot of the same rows. A row
    /// already observed from the earlier snapshot must not come back as a second, unreviewed
    /// observation; a row only the new snapshot holds still does.
    #[tokio::test]
    async fn a_reimported_file_does_not_observe_the_same_row_twice() {
        let db = setup().await;
        let conn = db.analytics_connection().await.unwrap();
        insert(&conn, "Public restroom operating inventory and schedules.");
        let row = |date: &str| {
            serde_json::json!({"name":"Public restroom","observed_on":date,"latitude":32.75,"longitude":-117.1,"weekly_hours":56}).to_string()
        };
        for (snapshot, retrieved_at, rows) in [("v3", "2026-09-01", 1), ("v4", "2026-10-01", 2)] {
            conn.execute("INSERT INTO analytical_snapshots(snapshot_id,dataset_kind,display_name,table_name,source_uri,local_path,source_sha256,retrieved_at,validation_summary_json) VALUES (?1,'generic','Public restroom operating hours','geo_fixture','https://www.sandiego.gov/riverbed.pdf','',?2,?3,'{}')", params![snapshot, "a".repeat(64), retrieved_at]).unwrap();
            conn.execute("INSERT INTO analytical_columns(snapshot_id,table_name,column_name,original_name,logical_type,ordinal) VALUES (?1,'geo_fixture','latitude','latitude','DOUBLE',0)", params![snapshot]).unwrap();
            for number in 1..=rows {
                let raw = row(&format!("2026-09-0{number}"));
                conn.execute(
                    "INSERT INTO analytical_row_ledger VALUES (?1,?2,?3,?4,?4,0,'[]')",
                    params![snapshot, number, sha256_hex(raw.as_bytes()), raw],
                )
                .unwrap();
            }
            if snapshot == "v3" {
                db.initialize_monitoring().await.unwrap();
                db.import_monitoring_geo_rows().await.unwrap();
            }
        }
        db.import_monitoring_geo_rows().await.unwrap();
        let observed = conn
            .prepare("SELECT id FROM monitor_observations ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(observed, ["v3:1", "v4:2"]);
    }

    #[tokio::test]
    async fn native_table_rows_become_cited_candidates_and_review_unlocks_hours_comparison() {
        let db = setup().await;
        let conn = db.analytics_connection().await.unwrap();
        insert(&conn, "Public restroom operating inventory and schedules.");
        conn.execute("INSERT INTO analytical_snapshots(snapshot_id,dataset_kind,display_name,table_name,source_uri,local_path,source_sha256,retrieved_at,validation_summary_json) VALUES ('geo','generic','Public restroom operating hours','geo_fixture','https://www.sandiego.gov/riverbed.pdf','',?1,'2026-09-26','{}')",params!["a".repeat(64)]).unwrap();
        conn.execute_batch("INSERT INTO analytical_columns(snapshot_id,table_name,column_name,original_name,logical_type,ordinal) VALUES ('geo','geo_fixture','latitude','latitude','DOUBLE',0)").unwrap();
        for (row, date, hours) in [(1, "2026-09-01", 56.0), (2, "2026-09-15", 28.0)] {
            let raw=serde_json::json!({"name":"Public restroom","observed_on":date,"latitude":32.75,"longitude":-117.1,"weekly_hours":hours}).to_string();
            conn.execute(
                "INSERT INTO analytical_row_ledger VALUES ('geo',?1,?2,?3,?3,0,'[]')",
                params![row, sha256_hex(raw.as_bytes()), raw],
            )
            .unwrap();
        }
        db.refresh_monitoring().await.unwrap();
        let status = db
            .monitoring_status("bathrooms".into(), None)
            .await
            .unwrap();
        assert_eq!(status.observations.len(), 2);
        assert!(status.comparisons.is_empty());
        for mut observation in status.observations {
            assert_eq!(observation.evidence.snapshot_id.as_deref(), Some("geo"));
            observation.reviewed = true;
            db.save_service_observation(observation).await.unwrap();
        }
        let reviewed = db
            .monitoring_status("bathrooms".into(), None)
            .await
            .unwrap();
        assert!(
            reviewed
                .comparisons
                .iter()
                .any(|c| c.kind == "ACCESS_HOURS_CHANGE"
                    && c.calculation.contains("-28.0 hours/week"))
        );
        conn.execute_batch("UPDATE portals SET lifecycle_state='REMOVED' WHERE subdomain='city'")
            .unwrap();
        assert!(db
            .monitoring_status("bathrooms".into(), None)
            .await
            .unwrap()
            .observations
            .is_empty());
    }
}
