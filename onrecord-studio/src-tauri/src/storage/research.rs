use super::sqlite::DuckDbRepository;
use crate::{error::AppError, types::*};
use duckdb::{params, OptionalExt};
use std::collections::BTreeMap;
use std::io::Read;

fn err(e: impl std::fmt::Display) -> AppError {
    AppError::Storage(e.to_string())
}
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS research_settings (singleton BOOLEAN PRIMARY KEY, policy_json TEXT NOT NULL, phase TEXT NOT NULL, last_error TEXT, last_finished_at TEXT, last_topic TEXT);
CREATE TABLE IF NOT EXISTS research_reports (id TEXT PRIMARY KEY, topic TEXT NOT NULL, created_at TEXT NOT NULL, fingerprint TEXT NOT NULL, report_zstd BLOB NOT NULL);
CREATE TABLE IF NOT EXISTS research_sources (url TEXT PRIMARY KEY, label TEXT NOT NULL, state TEXT NOT NULL, detail TEXT NOT NULL, next_attempt_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS research_usage (day TEXT PRIMARY KEY, calls INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS research_actions (action_key TEXT PRIMARY KEY, state TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS research_report_sources (report_id TEXT NOT NULL, portal_key TEXT NOT NULL, document_id BIGINT NOT NULL, source_sha256 TEXT NOT NULL, PRIMARY KEY(report_id, portal_key, document_id));
CREATE TABLE IF NOT EXISTS research_scheduler (singleton BOOLEAN PRIMARY KEY, revision TEXT NOT NULL, observed_at TIMESTAMPTZ NOT NULL);
CREATE TABLE IF NOT EXISTS research_attempts (topic TEXT PRIMARY KEY, revision TEXT NOT NULL, attempted_at TIMESTAMPTZ NOT NULL, retry_at TIMESTAMPTZ NOT NULL);
";

impl DuckDbRepository {
    pub async fn initialize_research(&self) -> Result<(), AppError> {
        self.query_connection("Initialize autonomous research", |conn| {
            conn.execute_batch(SCHEMA).map_err(err)?;
            // Research sends excerpts to a third-party model provider, so a fresh install
            // starts with it off; the user opts in from the research workspace.
            let mut initial_policy = ResearchPolicy {
                enabled: false,
                auto_collect: false,
                ..ResearchPolicy::default()
            };
            if crate::research::model::provider_key("openrouter").is_some() {
                initial_policy.provider = "openrouter".into();
                initial_policy.model = "openai/gpt-5.4".into();
            }
            let policy = serde_json::to_string(&initial_policy).map_err(err)?;
            let first_install = conn.execute("INSERT INTO research_settings VALUES (TRUE, ?1, 'Waiting for records', NULL, NULL, NULL) ON CONFLICT DO NOTHING", params![policy]).map_err(err)? == 1;
            if first_install {
                // Product policy: the autonomous research workspace retains validated derivatives
                // and references, including already-processed managed downloads. A later explicit
                // choice to keep originals remains intact on every subsequent launch.
                let previous: Option<String> = conn.query_row("SELECT policy_json FROM data_maintenance_policy WHERE singleton", [], |row| row.get(0)).optional().map_err(err)?;
                let mut retention = previous.as_deref().map(serde_json::from_str::<DataMaintenancePolicy>).transpose().map_err(err)?.unwrap_or_default();
                retention.retention_mode = "REFERENCE_ONLY".into();
                conn.execute("INSERT INTO data_maintenance_policy VALUES (TRUE, ?1, '1970-01-01T00:00:00.000Z') ON CONFLICT (singleton) DO UPDATE SET policy_json = excluded.policy_json, effective_at = excluded.effective_at", params![serde_json::to_string(&retention).map_err(err)?]).map_err(err)?;
            }
            // A crashed process cannot leave the UI claiming that a model call is still running.
            conn.execute("UPDATE research_settings SET phase = 'Ready to resume'", []).map_err(err)?;
            Ok(())
        }).await
    }

    pub async fn research_policy(&self) -> Result<ResearchPolicy, AppError> {
        self.query_connection("Read research policy", |conn| {
            let raw: String = conn
                .query_row(
                    "SELECT policy_json FROM research_settings WHERE singleton",
                    [],
                    |row| row.get(0),
                )
                .map_err(err)?;
            serde_json::from_str(&raw).map_err(err)
        })
        .await
    }
    pub async fn save_research_policy(&self, policy: ResearchPolicy) -> Result<(), AppError> {
        if policy.enabled && policy.daily_call_limit < policy.minimum_run_calls() {
            return Err(AppError::DataParsing(format!("The daily limit must cover one complete investigation ({} calls), including research passes, draft, independent evidence challenge, final review and public-source discovery when enabled.",policy.minimum_run_calls())));
        }
        if !matches!(policy.provider.as_str(), "openai" | "openrouter" | "ollama")
            || !(1..=168).contains(&policy.interval_hours)
            || !(4..=100).contains(&policy.daily_call_limit)
            || !(2..=6).contains(&policy.research_passes)
            || policy.model.trim().is_empty()
            || policy.model.len() > 120
        {
            return Err(AppError::DataParsing("Choose OpenAI, OpenRouter, or Ollama, a model, a 1–168 hour interval, 2–6 research passes, and 4–100 daily model calls.".into()));
        }
        let raw = serde_json::to_string(&policy).map_err(err)?;
        self.query_connection("Save research policy", move |conn| {
            conn.execute(
                "UPDATE research_settings SET policy_json = ?1 WHERE singleton",
                params![raw],
            )
            .map_err(err)?;
            Ok(())
        })
        .await
    }
    pub async fn research_phase(&self, phase: &str, error: Option<String>) -> Result<(), AppError> {
        let phase = phase.to_string();
        self.query_connection("Research progress", move |conn| {
            conn.execute(
                "UPDATE research_settings SET phase = ?1, last_error = ?2 WHERE singleton",
                params![phase, error],
            )
            .map_err(err)?;
            Ok(())
        })
        .await
    }

    /// New evidence wakes every case after one initial collection window, with a per-case
    /// cooldown and durable failure backoff. Continuous collection must not prevent research.
    pub async fn next_research_topic(
        &self,
        policy: &ResearchPolicy,
    ) -> Result<Option<String>, AppError> {
        let policy = policy.clone();
        self.query_connection("Schedule evidence-driven investigation", move |conn| {
            let day = chrono::Utc::now().format("%Y-%m-%d").to_string();
            let calls: u32 = conn.query_row("SELECT calls FROM research_usage WHERE day = ?1", params![day], |r| r.get(0)).optional().map_err(err)?.unwrap_or(0);
            if !policy.enabled { return Ok(None); }
            let waiting=if policy.daily_call_limit < policy.minimum_run_calls() {
                Some("Daily model limit cannot cover one investigation; update Research settings")
            } else if policy.daily_call_limit.saturating_sub(calls) < policy.minimum_run_calls() {
                Some("Daily model budget used; collection continues until the next UTC day")
            } else if policy.provider != "ollama" && crate::research::model::provider_key(&policy.provider).is_none() {
                Some("Model access needed; collection continues")
            } else { None };
            if let Some(phase)=waiting {
                conn.execute("UPDATE research_settings SET phase=?1 WHERE singleton",params![phase]).map_err(err)?;
                return Ok(None);
            }
            let (documents, digest): (u64, String) = conn.query_row("SELECT COUNT(*), COALESCE(md5(string_agg(d.portal_subdomain || ':' || CAST(d.id AS VARCHAR) || ':' || COALESCE(d.raw_file_sha256,'') || ':' || COALESCE(d.extracted_text_sha256,''), '|' ORDER BY d.portal_subdomain,d.id)), '') FROM documents d JOIN portals p ON p.subdomain=d.portal_subdomain WHERE lower(trim(d.visibility))='public' AND d.extraction_status IN ('SEARCHABLE','SALVAGED') AND p.lifecycle_state <> 'REMOVED'", [], |r| Ok((r.get(0)?,r.get(1)?))).map_err(err)?;
            let (tables, table_digest): (u64, String) = conn.query_row("SELECT COUNT(*), COALESCE(md5(string_agg(snapshot_id || ':' || source_sha256 || ':' || COALESCE(CAST(partial_dataset AS VARCHAR),'unknown') || ':' || COALESCE(CAST(rejected_rows AS VARCHAR),'unknown') || ':' || COALESCE(observation_period_start,'') || ':' || COALESCE(observation_period_end,''), '|' ORDER BY snapshot_id)), '') FROM analytical_snapshots", [], |r| Ok((r.get(0)?,r.get(1)?))).map_err(err)?;
            if documents == 0 && tables == 0 {
                conn.execute("UPDATE research_settings SET phase='Waiting for the first searchable record; collection continues' WHERE singleton",[]).map_err(err)?;
                return Ok(None);
            }
            let case_revision = super::casework::casework_revision(conn)?;
            let monitoring_revision = super::monitoring::monitoring_revision(conn)?;
            let financial_revision = super::financial::financial_mapping_revision(conn)?;
            let review_revision = super::review::review_revision(conn)?;
            let revision = format!("{digest}:{table_digest}:{case_revision}:{monitoring_revision}:{financial_revision}:{review_revision}");
            let tx = conn.transaction().map_err(err)?;
            tx.execute("INSERT INTO research_scheduler VALUES (TRUE, ?1, now()) ON CONFLICT(singleton) DO UPDATE SET revision=excluded.revision WHERE research_scheduler.revision <> excluded.revision", params![revision]).map_err(err)?;
            let settled: bool = tx.query_row("SELECT observed_at <= now()-INTERVAL '60 seconds' FROM research_scheduler WHERE singleton", [], |r| r.get(0)).map_err(err)?;
            if !settled {
                tx.execute("UPDATE research_settings SET phase='Allowing initial records to arrive; collection continues' WHERE singleton",[]).map_err(err)?;
                tx.commit().map_err(err)?; return Ok(None);
            }
            let topic: Option<String> = tx.query_row("WITH topics(topic,ordinal) AS (VALUES ('homelessness',0),('police',1),('abatement',2),('bathrooms',3),('waste',4)) SELECT t.topic FROM topics t LEFT JOIN research_attempts a ON a.topic=t.topic WHERE a.topic IS NULL OR (a.retry_at <= now() AND (a.revision <> ?1 OR a.attempted_at <= now()-CAST(?2 AS INTEGER)*INTERVAL '1 hour')) ORDER BY a.attempted_at ASC NULLS FIRST,t.ordinal LIMIT 1", params![revision,policy.interval_hours], |r| r.get(0)).optional().map_err(err)?;
            if let Some(topic) = &topic {
                // A crash also respects this reservation; a successful pass shortens its cooldown.
                tx.execute("INSERT INTO research_attempts VALUES (?1,?2,now(),now()+INTERVAL '1 hour') ON CONFLICT(topic) DO UPDATE SET revision=excluded.revision,attempted_at=excluded.attempted_at,retry_at=excluded.retry_at", params![topic,revision]).map_err(err)?;
            }
            tx.commit().map_err(err)?;
            Ok(topic)
        }).await
    }

    pub async fn research_attempt_succeeded(&self, topic: &str) -> Result<(), AppError> {
        let topic = topic.to_string();
        self.query_connection("Complete scheduled investigation", move |conn| {
            conn.execute(
                "UPDATE research_attempts SET retry_at=now()+INTERVAL '10 minutes' WHERE topic=?1",
                params![topic],
            )
            .map_err(err)?;
            Ok(())
        })
        .await
    }
    pub async fn research_reserve_call(&self, limit: u32) -> Result<(), AppError> {
        self.query_connection("Reserve research model call", move |conn| {
            let day = chrono::Utc::now().format("%Y-%m-%d").to_string();
            let tx = conn.transaction().map_err(err)?;
            tx.execute("INSERT INTO research_usage VALUES (?1, 0) ON CONFLICT DO NOTHING", params![day]).map_err(err)?;
            let used: u32 = tx.query_row("SELECT calls FROM research_usage WHERE day = ?1", params![day], |row| row.get(0)).map_err(err)?;
            if used >= limit { return Err(AppError::PortalCapacity("Daily research model-call limit reached. Collection continues; research resumes tomorrow or after increasing the limit.".into())); }
            tx.execute("UPDATE research_usage SET calls = calls + 1 WHERE day = ?1", params![day]).map_err(err)?;
            tx.commit().map_err(err)
        }).await
    }
    pub async fn research_source_due(&self, url: &str) -> Result<bool, AppError> {
        let url = url.to_string();
        self.query_connection("Research source schedule", move |conn| conn.query_row("SELECT NOT EXISTS (SELECT 1 FROM research_sources WHERE url = ?1 AND (state = 'REGISTERED' OR next_attempt_at > ?2))", params![url, super::maintenance::now()], |row| row.get(0)).map_err(err)).await
    }
    pub async fn pending_research_sources(&self) -> Result<Vec<ResearchSourceStatus>, AppError> {
        self.query_connection("Pending evidence-gap acquisitions", |conn| {
            conn.prepare("SELECT label, url, state, detail FROM research_sources WHERE state IN ('QUEUED', 'RETRY', 'STARTING') AND next_attempt_at <= ?1 ORDER BY next_attempt_at LIMIT 30").map_err(err)?.query_map(params![super::maintenance::now()], |row| Ok(ResearchSourceStatus { label:row.get(0)?,url:row.get(1)?,state:row.get(2)?,detail:row.get(3)? })).map_err(err)?.collect::<Result<Vec<_>,_>>().map_err(err)
        }).await
    }
    /// Every recorded acquisition's state, and whether its retry/recheck time has passed.
    pub async fn research_source_states(
        &self,
    ) -> Result<std::collections::HashMap<String, (String, bool)>, AppError> {
        self.query_connection("Research source states", |conn| {
            conn.prepare("SELECT url, state, next_attempt_at <= ?1 FROM research_sources")
                .map_err(err)?
                .query_map(params![super::maintenance::now()], |row| {
                    Ok((row.get(0)?, (row.get(1)?, row.get(2)?)))
                })
                .map_err(err)?
                .collect::<Result<_, _>>()
                .map_err(err)
        })
        .await
    }
    /// Changes whenever ingestion adds or revises a document or an analytical snapshot.
    pub async fn corpus_change_marker(&self) -> Result<String, AppError> {
        self.query_connection("Corpus change marker", |conn| {
            conn.query_row(
                "SELECT (SELECT COUNT(*) FROM documents) || ':' || (SELECT COALESCE(MAX(updated_at), '') FROM documents) || ':' || (SELECT COUNT(*) FROM analytical_snapshots)",
                [],
                |row| row.get(0),
            )
            .map_err(err)
        })
        .await
    }
    pub async fn record_research_source(
        &self,
        source: ResearchSourceStatus,
        retry_hours: i64,
    ) -> Result<(), AppError> {
        self.query_connection("Record research acquisition", move |conn| {
            let next = (chrono::Utc::now() + chrono::Duration::hours(retry_hours)).to_rfc3339();
            conn.execute("INSERT INTO research_sources VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT (url) DO UPDATE SET state = excluded.state, detail = excluded.detail, next_attempt_at = excluded.next_attempt_at", params![source.url, source.label, source.state, source.detail, next]).map_err(err)?; Ok(())
        }).await
    }
    pub async fn research_fingerprint_exists(
        &self,
        topic: &str,
        fingerprint: &str,
    ) -> Result<bool, AppError> {
        let (topic, fingerprint) = (topic.to_string(), fingerprint.to_string());
        self.query_connection("Research evidence cache", move |conn| conn.query_row("SELECT EXISTS (SELECT 1 FROM research_reports WHERE topic = ?1 AND fingerprint = ?2 AND created_at > ?3)", params![topic, fingerprint, (chrono::Utc::now() - chrono::Duration::days(7)).to_rfc3339()], |row| row.get(0)).map_err(err)).await
    }
    pub async fn finish_research(&self, topic: &str) -> Result<(), AppError> {
        let topic = topic.to_string();
        self.query_connection("Advance research schedule", move |conn| { conn.execute("UPDATE research_settings SET phase = 'Monitoring for new evidence', last_finished_at = ?1, last_topic = ?2 WHERE singleton", params![super::maintenance::now(), topic]).map_err(err)?; Ok(()) }).await
    }
    pub async fn store_research_report(
        &self,
        report: ResearchReport,
        fingerprint: String,
    ) -> Result<(), AppError> {
        self.query_connection("Save compressed research dossier", move |conn| {
            let json = serde_json::to_vec(&report).map_err(err)?;
            if json.len() > 2_000_000 { return Err(err("Research report exceeded its storage bound")); }
            let compressed = zstd::stream::encode_all(json.as_slice(), 9).map_err(err)?;
            let tx = conn.transaction().map_err(err)?;
            // Reject a report if a document was purged or replaced during a long model call.
            // Bind SQL-only findings to the originating documents as well.
            for evidence in &report.evidence {
                for provenance in &evidence.snapshot_provenance {
                    let Some((table, snapshot)) = provenance.split_once('@') else { return Err(err("Invalid research snapshot provenance")); };
                    let current: bool = tx.query_row("SELECT EXISTS (SELECT 1 FROM analytical_snapshots WHERE snapshot_id = ?1 AND table_name = ?2)", params![snapshot,table], |r| r.get(0)).map_err(err)?;
                    if !current { return Err(err("An analytical source was removed during research. Re-run against current evidence.")); }
                    tx.execute("INSERT INTO research_report_sources SELECT ?1, o.portal_key, o.document_id, s.source_sha256 FROM data_maintenance_origins o JOIN analytical_snapshots s ON s.snapshot_id = o.snapshot_id WHERE o.snapshot_id = ?2 ON CONFLICT DO NOTHING", params![report.id, snapshot]).map_err(err)?;
                }
                if let (Some(portal), Some(document)) = (&evidence.portal_key, evidence.document_id) {
                    let current: bool = tx.query_row("SELECT EXISTS (SELECT 1 FROM documents WHERE portal_subdomain = ?1 AND id = ?2 AND COALESCE(raw_file_sha256, '') = ?3 AND lower(trim(visibility)) = 'public')", params![portal, document, evidence.source_sha256], |row| row.get(0)).map_err(err)?;
                    if !current { return Err(err("A source changed or was removed during research. Re-run against the current evidence.")); }
                    tx.execute("INSERT INTO research_report_sources VALUES (?1, ?2, ?3, ?4) ON CONFLICT DO NOTHING", params![report.id, portal, document, evidence.source_sha256]).map_err(err)?;
                }
            }
            tx.execute("INSERT INTO research_reports VALUES (?1, ?2, ?3, ?4, ?5)", params![report.id, report.topic, report.created_at, fingerprint, compressed]).map_err(err)?;
            tx.commit().map_err(err)?;
            Ok(())
        }).await
    }
    pub async fn research_action(&self, key: String, state: String) -> Result<(), AppError> {
        if key.len() > 200
            || !matches!(
                state.as_str(),
                "OPEN" | "IN_PROGRESS" | "DONE" | "DISMISSED"
            )
        {
            return Err(AppError::DataParsing(
                "Invalid research action state".into(),
            ));
        }
        self.query_connection("Save research follow-through", move |conn| { conn.execute("INSERT INTO research_actions VALUES (?1, ?2) ON CONFLICT (action_key) DO UPDATE SET state = excluded.state", params![key, state]).map_err(err)?; Ok(()) }).await
    }
    pub async fn research_status(&self) -> Result<ResearchStatus, AppError> {
        self.query_connection("Read research dossiers", |conn| {
            let (raw, phase, last_error, last_finished_at): (String, String, Option<String>, Option<String>) = conn.query_row("SELECT policy_json, phase, last_error, last_finished_at FROM research_settings WHERE singleton", [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))).map_err(err)?;
            let policy: ResearchPolicy = serde_json::from_str(&raw).map_err(err)?;
            let credential_available = policy.provider == "ollama" || crate::research::model::provider_key(&policy.provider).is_some();
            let calls_today = conn.query_row("SELECT calls FROM research_usage WHERE day = ?1", params![chrono::Utc::now().format("%Y-%m-%d").to_string()], |row| row.get(0)).optional().map_err(err)?.unwrap_or(0);
            let mut statement = conn.prepare("SELECT report_zstd FROM research_reports ORDER BY created_at DESC LIMIT 10").map_err(err)?;
            let blobs = statement.query_map([], |row| row.get::<_, Vec<u8>>(0)).map_err(err)?.collect::<Result<Vec<_>, _>>().map_err(err)?;
            let mut reports = Vec::new();
            for blob in blobs {
                let mut bytes = Vec::new();
                zstd::stream::read::Decoder::new(blob.as_slice()).map_err(err)?.take(2_000_001).read_to_end(&mut bytes).map_err(err)?;
                if bytes.len() > 2_000_000 { return Err(err("Compressed report exceeds its decoding bound")); }
                reports.push(serde_json::from_slice(&bytes).map_err(err)?);
            }
            let sources = conn.prepare("SELECT label, url, state, detail FROM research_sources ORDER BY label").map_err(err)?.query_map([], |row| Ok(ResearchSourceStatus { label: row.get(0)?, url: row.get(1)?, state: row.get(2)?, detail: row.get(3)? })).map_err(err)?.collect::<Result<Vec<_>, _>>().map_err(err)?;
            let action_states = conn.prepare("SELECT action_key, state FROM research_actions").map_err(err)?.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))).map_err(err)?.collect::<Result<BTreeMap<_, _>, _>>().map_err(err)?;
            Ok(ResearchStatus { policy, credential_available, phase, last_error, last_finished_at, calls_today, reports, sources, action_states })
        }).await
    }
}

pub(crate) fn purge_research_references(
    conn: &duckdb::Connection,
    portal: &str,
    document: Option<i64>,
) -> Result<(), AppError> {
    let exists: bool = conn.query_row("SELECT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_name = 'research_report_sources')", [], |r| r.get(0)).map_err(err)?;
    if !exists {
        return Ok(());
    }
    let ids = conn.prepare("SELECT DISTINCT report_id FROM research_report_sources WHERE portal_key = ?1 AND (?2 IS NULL OR document_id = ?2)").map_err(err)?.query_map(params![portal,document], |r| r.get::<_,String>(0)).map_err(err)?.collect::<Result<Vec<_>,_>>().map_err(err)?;
    for id in ids {
        conn.execute(
            "DELETE FROM research_actions WHERE starts_with(action_key, ?1 || ':')",
            params![id],
        )
        .map_err(err)?;
        conn.execute(
            "DELETE FROM research_report_sources WHERE report_id = ?1",
            params![id],
        )
        .map_err(err)?;
        conn.execute("DELETE FROM research_reports WHERE id = ?1", params![id])
            .map_err(err)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::research::evidence::query_evidence;
    async fn setup() -> (tempfile::TempDir, DuckDbRepository) {
        let root = tempfile::tempdir().unwrap();
        let db = DuckDbRepository::new(root.path().join("core.db")).unwrap();
        db.run_migrations(include_str!("../../migrations/0001_initial_schema.sql"))
            .await
            .unwrap();
        (root, db)
    }
    fn report() -> ResearchReport {
        ResearchReport {
            id: "case-1".into(),
            topic: "homelessness".into(),
            created_at: super::super::maintenance::now(),
            model: "fixture".into(),
            evidence: vec![],
            synthesis: ResearchSynthesis {
                title: "Unreconciled shelter invoices".into(),
                summary: "Evidence-grounded fixture. ".repeat(500),
                findings: vec![],
                questions: vec![],
                requests: vec![],
                meetings: vec![],
                limitations: vec!["Fixture, not a real City finding".into()],
            },
            investigation_log: vec![],
            input_tokens: 123,
            output_tokens: 45,
        }
    }
    #[tokio::test]
    async fn upgrade_enables_compact_research_once_and_preserves_later_operator_choice() {
        let (_root, db) = setup().await;
        db.save_maintenance_policy(DataMaintenancePolicy::default())
            .await
            .unwrap();
        db.initialize_research().await.unwrap();
        let compact = db.maintenance_status().await.unwrap();
        assert_eq!(compact.policy.retention_mode, "REFERENCE_ONLY");
        assert_eq!(compact.effective_at, "1970-01-01T00:00:00.000Z");
        db.save_maintenance_policy(DataMaintenancePolicy {
            retention_mode: "ORIGINALS".into(),
            ..Default::default()
        })
        .await
        .unwrap();
        db.save_research_policy(ResearchPolicy {
            enabled: false,
            auto_collect: false,
            ..Default::default()
        })
        .await
        .unwrap();
        db.initialize_research().await.unwrap();
        assert_eq!(
            db.maintenance_status().await.unwrap().policy.retention_mode,
            "ORIGINALS"
        );
        assert!(!db.research_policy().await.unwrap().enabled);
    }
    #[tokio::test]
    async fn a_fresh_install_does_not_send_anything_to_a_model_provider() {
        let (_root, db) = setup().await;
        db.initialize_research().await.unwrap();
        let policy = db.research_policy().await.unwrap();
        assert!(!policy.enabled && !policy.auto_collect);
    }
    #[tokio::test]
    async fn dossiers_round_trip_compressed_and_call_reservations_are_durable() {
        let (_root, db) = setup().await;
        db.initialize_research().await.unwrap();
        let report = report();
        let original = serde_json::to_vec(&report).unwrap().len();
        db.store_research_report(report, "fp".into()).await.unwrap();
        let conn = db.analytics_connection().await.unwrap();
        let bytes: Vec<u8> = conn
            .query_row("SELECT report_zstd FROM research_reports", [], |r| r.get(0))
            .unwrap();
        assert!(bytes.len() < original / 2);
        let status = db.research_status().await.unwrap();
        assert_eq!(status.reports[0].input_tokens, 123);
        assert!(db
            .research_fingerprint_exists("homelessness", "fp")
            .await
            .unwrap());
        db.research_action("case-1:request:0".into(), "IN_PROGRESS".into())
            .await
            .unwrap();
        assert_eq!(
            db.research_status().await.unwrap().action_states["case-1:request:0"],
            "IN_PROGRESS"
        );
        db.research_reserve_call(1).await.unwrap();
        assert!(db.research_reserve_call(1).await.is_err());
        db.initialize_research().await.unwrap();
        assert_eq!(db.research_status().await.unwrap().calls_today, 1);
    }
    #[tokio::test]
    async fn failed_sources_back_off_and_purged_acquisitions_are_not_recreated() {
        let (_root, db) = setup().await;
        db.initialize_research().await.unwrap();
        let url = "https://example.test/records";
        assert!(db.research_source_due(url).await.unwrap());
        db.record_research_source(
            ResearchSourceStatus {
                url: url.into(),
                label: "Fixture".into(),
                state: "RETRY".into(),
                detail: "failure".into(),
            },
            6,
        )
        .await
        .unwrap();
        assert!(!db.research_source_due(url).await.unwrap());
        db.record_research_source(
            ResearchSourceStatus {
                url: url.into(),
                label: "Fixture".into(),
                state: "REGISTERED".into(),
                detail: "registered".into(),
            },
            -48,
        )
        .await
        .unwrap();
        assert!(!db.research_source_due(url).await.unwrap());
    }
    #[tokio::test]
    async fn research_queries_reconcile_real_amounts_keep_missing_counterparts_and_reject_external_access(
    ) {
        let (_root, db) = setup().await;
        db.initialize_research().await.unwrap();
        let conn = db.analytics_connection().await.unwrap();
        conn.execute_batch("CREATE TABLE analysis_allocations(program VARCHAR, allocation DECIMAL(18,2)); INSERT INTO analysis_allocations VALUES ('shelter',1000000.01),('restrooms',750000.00); CREATE TABLE analysis_payments(program VARCHAR, paid DECIMAL(18,2)); INSERT INTO analysis_payments VALUES ('shelter',600000.01),('shelter',500000.00);").unwrap();
        let sql="WITH actuals AS (SELECT program, SUM(paid) AS paid FROM analysis_payments GROUP BY program) SELECT a.program, a.allocation, p.paid, p.paid-a.allocation AS variance FROM analysis_allocations a LEFT JOIN actuals p ON a.program=p.program ORDER BY a.program";
        let result = query_evidence(
            &db,
            "Do payments exceed allocations?",
            sql,
            "Same fund/year; fixture",
        )
        .await
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&result.text).unwrap();
        assert_eq!(value["columns"][3], "variance");
        assert!(value["rows"][0][2].is_null());
        assert!(value["rows"][1][3].to_string().contains("100000"));
        assert_eq!(result.sql.as_deref(), Some(sql));
        assert!(query_evidence(
            &db,
            "Escape",
            "SELECT * FROM read_csv_auto('/etc/passwd')",
            ""
        )
        .await
        .is_err());
        assert!(
            query_evidence(&db, "Mutate", "DELETE FROM analysis_payments", "")
                .await
                .is_err()
        );
    }
    #[tokio::test]
    async fn scheduler_wakes_all_cases_for_changed_evidence_and_bounds_retries() {
        let (_root, db) = setup().await;
        db.initialize_research().await.unwrap();
        let policy = ResearchPolicy {
            enabled: true,
            auto_collect: true,
            provider: "ollama".into(),
            ..db.research_policy().await.unwrap()
        };
        assert!(db.next_research_topic(&policy).await.unwrap().is_none());
        assert!(db
            .research_status()
            .await
            .unwrap()
            .phase
            .contains("first searchable record"));
        let conn = db.analytics_connection().await.unwrap();
        conn.execute_batch("INSERT INTO analytical_snapshots(snapshot_id,dataset_kind,display_name,table_name,local_path,source_sha256,retrieved_at,validation_summary_json) VALUES ('first','GENERIC','First','analysis_first','/fixture',repeat('a',64),'2026-09-26','{}');").unwrap();
        assert!(
            db.next_research_topic(&policy).await.unwrap().is_none(),
            "Allow a bounded initial acquisition window after even one useful artifact"
        );
        conn.execute(
            "UPDATE research_scheduler SET observed_at=now()-INTERVAL '2 minutes'",
            [],
        )
        .unwrap();
        // Continuous ingestion must not restart the initial window indefinitely.
        conn.execute("UPDATE analytical_snapshots SET source_sha256=repeat('b',64) WHERE snapshot_id='first'", []).unwrap();
        for topic in ["homelessness", "police", "abatement", "bathrooms", "waste"] {
            assert_eq!(
                db.next_research_topic(&policy).await.unwrap().as_deref(),
                Some(topic)
            );
            db.research_attempt_succeeded(topic).await.unwrap();
        }
        assert!(
            db.next_research_topic(&policy).await.unwrap().is_none(),
            "No repeated calls on unchanged evidence"
        );
        conn.execute(
            "UPDATE analytical_snapshots SET partial_dataset=1 WHERE snapshot_id='first'",
            [],
        )
        .unwrap();
        assert!(db.next_research_topic(&policy).await.unwrap().is_none());
        conn.execute_batch("UPDATE research_scheduler SET observed_at=now()-INTERVAL '2 minutes'; UPDATE research_attempts SET retry_at=now()-INTERVAL '1 second';").unwrap();
        assert_eq!(
            db.next_research_topic(&policy).await.unwrap().as_deref(),
            Some("homelessness")
        );
        conn.execute(
            "DELETE FROM research_attempts WHERE topic <> 'homelessness'",
            [],
        )
        .unwrap();
        // Simulate a failing job's reservation: a new revision must not defeat its backoff.
        conn.execute("UPDATE research_attempts SET revision='older'", [])
            .unwrap();
        assert_eq!(
            db.next_research_topic(&policy).await.unwrap().as_deref(),
            Some("police")
        );
        db.research_reserve_call(1).await.unwrap();
        let capped = ResearchPolicy {
            daily_call_limit: 4,
            research_passes: 2,
            ..policy
        };
        assert!(
            db.next_research_topic(&capped).await.unwrap().is_none(),
            "Reserve enough calls for a whole investigation"
        );
        assert!(db
            .research_status()
            .await
            .unwrap()
            .phase
            .contains("cannot cover one investigation"));
        let used = ResearchPolicy {
            daily_call_limit: 5,
            ..capped
        };
        assert!(db.next_research_topic(&used).await.unwrap().is_none());
        let status = db.research_status().await.unwrap();
        assert!(status.phase.contains("next UTC day"));
        assert!(
            status.last_finished_at.is_none(),
            "Scheduling or a failed attempt cannot imply a completed dossier"
        );
    }

    #[tokio::test]
    async fn saved_budget_covers_discovery_only_when_it_can_run() {
        let (_root, db) = setup().await;
        db.initialize_research().await.unwrap();
        let mut policy = ResearchPolicy {
            provider: "openai".into(),
            research_passes: 3,
            daily_call_limit: 6,
            ..Default::default()
        };
        assert_eq!(policy.minimum_run_calls(), 7);
        assert!(db.save_research_policy(policy.clone()).await.is_err());
        policy.auto_collect = false;
        assert_eq!(policy.minimum_run_calls(), 6);
        db.save_research_policy(policy.clone()).await.unwrap();
        policy.auto_collect = true;
        policy.provider = "ollama".into();
        assert_eq!(policy.minimum_run_calls(), 6);
        db.save_research_policy(policy).await.unwrap();
    }

    #[tokio::test]
    async fn query_only_dossiers_are_purged_with_their_source_and_cannot_publish_after_removal() {
        let (_root, db) = setup().await;
        db.initialize_research().await.unwrap();
        let conn = db.analytics_connection().await.unwrap();
        conn.execute_batch("CREATE TABLE analysis_invoice_fixture(amount DECIMAL(18,2)); INSERT INTO analysis_invoice_fixture VALUES (1200.50);
            INSERT INTO analytical_snapshots(snapshot_id,dataset_kind,display_name,table_name,local_path,source_sha256,retrieved_at,validation_summary_json) VALUES ('invoice-snapshot','GENERIC','Invoices','analysis_invoice_fixture','/fixture',repeat('a',64),'2026-09-26','{}');
            INSERT INTO data_maintenance_origins VALUES ('invoice-snapshot','fixture-portal',42);").unwrap();
        let evidence = query_evidence(
            &db,
            "Invoices paid",
            "SELECT SUM(amount) AS paid FROM analysis_invoice_fixture",
            "Fixture only",
        )
        .await
        .unwrap();
        assert_eq!(
            evidence.snapshot_provenance,
            vec!["analysis_invoice_fixture@invoice-snapshot"]
        );
        let mut dossier = report();
        dossier.evidence = vec![evidence];
        db.store_research_report(dossier.clone(), "fp".into())
            .await
            .unwrap();
        db.research_action("case-1:finding:0".into(), "DONE".into())
            .await
            .unwrap();
        purge_research_references(&conn, "fixture-portal", Some(42)).unwrap();
        let status = db.research_status().await.unwrap();
        assert!(status.reports.is_empty());
        assert!(status.action_states.is_empty());
        conn.execute(
            "DELETE FROM analytical_snapshots WHERE snapshot_id = 'invoice-snapshot'",
            [],
        )
        .unwrap();
        assert!(db
            .store_research_report(dossier, "fp".into())
            .await
            .is_err());
    }
}
