use super::sqlite::DuckDbRepository;
use crate::{
    analysis::review,
    error::AppError,
    integrity::is_sha256_hex,
    types::{review::*, *},
};
use duckdb::params;
use std::io::Read;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS finding_reviews (id TEXT PRIMARY KEY, report_id TEXT NOT NULL, finding_index BIGINT NOT NULL, revision BIGINT NOT NULL, payload TEXT NOT NULL, UNIQUE(report_id,finding_index,revision));
CREATE TABLE IF NOT EXISTS evidence_replays (report_id TEXT NOT NULL, evidence_id TEXT NOT NULL, payload TEXT NOT NULL, PRIMARY KEY(report_id,evidence_id));
CREATE TABLE IF NOT EXISTS public_relationships (id TEXT NOT NULL, revision BIGINT NOT NULL, report_id TEXT NOT NULL, payload TEXT NOT NULL, PRIMARY KEY(id,revision));
CREATE TABLE IF NOT EXISTS pilot_assessments (id TEXT PRIMARY KEY, report_id TEXT NOT NULL, assessed_at TEXT NOT NULL, payload TEXT NOT NULL);
";
fn err(e: impl std::fmt::Display) -> AppError {
    AppError::Storage(e.to_string())
}
pub(crate) fn saved_report(
    conn: &duckdb::Connection,
    id: &str,
) -> Result<ResearchReport, AppError> {
    let blob: Vec<u8> = conn
        .query_row(
            "SELECT report_zstd FROM research_reports WHERE id=?1",
            params![id],
            |r| r.get(0),
        )
        .map_err(err)?;
    let mut bytes = Vec::new();
    zstd::stream::read::Decoder::new(blob.as_slice())
        .map_err(err)?
        .take(2_000_001)
        .read_to_end(&mut bytes)
        .map_err(err)?;
    if bytes.len() > 2_000_000 {
        return Err(err("Research packet exceeds its size limit"));
    }
    serde_json::from_slice(&bytes).map_err(err)
}
fn list_payloads<T: serde::de::DeserializeOwned>(
    conn: &duckdb::Connection,
    sql: &str,
    id: &str,
) -> Result<Vec<T>, AppError> {
    let mut statement = conn.prepare(sql).map_err(err)?;
    let rows = statement
        .query_map(params![id], |r| r.get::<_, String>(0))
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;
    rows.iter()
        .map(|raw| serde_json::from_str(raw).map_err(err))
        .collect()
}
fn current_evidence(conn: &duckdb::Connection, e: &ResearchEvidence) -> Result<bool, AppError> {
    if let (Some(portal), Some(document)) = (&e.portal_key, e.document_id) {
        let current:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM documents d JOIN portals p ON p.subdomain=d.portal_subdomain JOIN documents_fts f ON f.document_id=d.id AND f.portal_subdomain=d.portal_subdomain WHERE d.portal_subdomain=?1 AND d.id=?2 AND d.raw_file_sha256=?3 AND lower(trim(d.visibility))='public' AND d.extraction_status IN ('SEARCHABLE','SALVAGED') AND p.lifecycle_state <> 'REMOVED')",params![portal,document,e.source_sha256],|r|r.get(0)).map_err(err)?;
        if !current || !is_sha256_hex(&e.source_sha256) {
            return Ok(false);
        }
        if e.sql.is_none() {
            let Some(offset) = e.character_offset else {
                return Ok(false);
            };
            if offset > 50_000_000 {
                return Ok(false);
            }
            let retained: String = conn.query_row("SELECT substr(extracted_text,?3+1,10000) FROM documents_fts WHERE portal_subdomain=?1 AND document_id=?2", params![portal, document, offset as i64], |r| r.get(0)).map_err(err)?;
            let current = super::pii::mask_pii(&retained)?.masked_text;
            let normalize = |v: &str| v.split_whitespace().collect::<Vec<_>>().join(" ");
            if !normalize(&current).starts_with(&normalize(&e.text)) {
                return Ok(false);
            }
        }
    } else if e.snapshot_provenance.is_empty() {
        return Ok(false);
    }
    for p in &e.snapshot_provenance {
        let Some((table, snapshot)) = p.split_once('@') else {
            return Ok(false);
        };
        let exists:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM analytical_snapshots WHERE table_name=?1 AND snapshot_id=?2)",params![table,snapshot],|r|r.get(0)).map_err(err)?;
        if !exists {
            return Ok(false);
        }
    }
    for s in &e.source_references {
        let exists:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM analytical_snapshots WHERE snapshot_id=?1 AND source_sha256=?2)",params![s.snapshot,s.sha256],|r|r.get(0)).map_err(err)?;
        if !exists {
            return Ok(false);
        }
    }
    if let Some(sql) = &e.sql {
        let mut statement = conn
            .prepare("SELECT table_name FROM information_schema.tables")
            .map_err(err)?;
        let tables = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(err)?
            .collect::<Result<std::collections::BTreeSet<_>, _>>()
            .map_err(err)?;
        let Ok(referenced) = crate::analysis::sql::validate(sql, &tables) else {
            return Ok(false);
        };
        let current = super::sqlite::analytical_query_provenance(conn, &referenced)?;
        let expected: std::collections::BTreeSet<_> = e.snapshot_provenance.iter().collect();
        if current.iter().collect::<std::collections::BTreeSet<_>>() != expected {
            return Ok(false);
        }
    }
    Ok(true)
}
fn stored_replays(
    conn: &duckdb::Connection,
    report: &ResearchReport,
) -> Result<Vec<EvidenceReplay>, AppError> {
    let mut result: Vec<EvidenceReplay> = list_payloads(
        conn,
        "SELECT payload FROM evidence_replays WHERE report_id=?1",
        &report.id,
    )?;
    for replay in &mut result {
        if let Some(e) = report.evidence.iter().find(|e| e.id == replay.evidence_id) {
            if !current_evidence(conn, e)? {
                replay.state = "SOURCE_CHANGED".into();
                replay.detail="The public retained source or analytical snapshot no longer matches. Review against the current record.".into();
            }
        }
    }
    Ok(result)
}
fn workspace(conn: &duckdb::Connection, id: &str) -> Result<ReviewWorkspace, AppError> {
    let report = saved_report(conn, id)?;
    let reviews=list_payloads(conn,"SELECT payload FROM finding_reviews WHERE report_id=?1 ORDER BY finding_index,revision DESC",id)?;
    let replays = stored_replays(conn, &report)?;
    let mut relationships:Vec<PublicRelationship>=list_payloads(conn,"SELECT payload FROM public_relationships WHERE report_id=?1 QUALIFY row_number() OVER(PARTITION BY id ORDER BY revision DESC)=1",id)?;
    let mut all:Vec<PublicRelationship>=list_payloads(conn,"SELECT payload FROM public_relationships WHERE ?1 <> '' QUALIFY row_number() OVER(PARTITION BY id ORDER BY revision DESC)=1",id)?;
    // An old review is historical, never a promise that a subsequently removed source is current.
    let mut sources = std::collections::HashMap::new();
    for r in &mut all {
        if !sources.contains_key(&r.report_id) {
            sources.insert(r.report_id.clone(), saved_report(conn, &r.report_id)?);
        }
        let source = &sources[&r.report_id];
        let mut stale = false;
        for c in &r.citations {
            match source.evidence.iter().find(|e| e.id == c.evidence_id) {
                Some(e) if current_evidence(conn, e)? => {}
                _ => {
                    stale = true;
                    break;
                }
            }
        }
        if stale {
            r.status = "CANDIDATE".into();
            if let Some(visible) = relationships.iter_mut().find(|item| item.id == r.id) {
                visible.status = "CANDIDATE".into();
                visible.note = format!("Current source identity needs review; the earlier assertion is retained as history. {}", visible.note);
            }
        }
    }
    let (connections, identity_warnings) = review::connections(&all);
    for candidate in review::relationship_candidates(&report) {
        if !relationships.iter().any(|r| r.id == candidate.id) {
            relationships.push(candidate);
        }
    }
    let challenges = review::challenges(&report, &replays);
    Ok(ReviewWorkspace {
        report,
        reviews,
        challenges,
        replays,
        relationships,
        connections,
        identity_warnings,
    })
}

pub(crate) fn purge_review_references(
    conn: &duckdb::Connection,
    portal: &str,
    document: Option<i64>,
) -> Result<(), AppError> {
    let exists:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM information_schema.tables WHERE table_name='finding_reviews')",[],|r|r.get(0)).map_err(err)?;
    if !exists {
        return Ok(());
    }
    let mut statement=conn.prepare("SELECT DISTINCT report_id FROM research_report_sources WHERE portal_key=?1 AND (?2 IS NULL OR document_id=?2)").map_err(err)?;
    let ids = statement
        .query_map(params![portal, document], |r| r.get::<_, String>(0))
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;
    drop(statement);
    for id in ids {
        for table in [
            "finding_reviews",
            "evidence_replays",
            "public_relationships",
            "pilot_assessments",
        ] {
            conn.execute(
                &format!("DELETE FROM {table} WHERE report_id=?1"),
                params![id],
            )
            .map_err(err)?;
        }
    }
    Ok(())
}

pub(crate) fn review_revision(conn: &duckdb::Connection) -> Result<String, AppError> {
    let exists:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM information_schema.tables WHERE table_name='finding_reviews')",[],|r|r.get(0)).map_err(err)?;
    if !exists {
        return Ok(String::new());
    }
    conn.query_row("SELECT COALESCE(md5(string_agg(id||':'||CAST(revision AS VARCHAR),'|' ORDER BY id)),'') FROM finding_reviews",[],|r|r.get(0)).map_err(err)
}

impl DuckDbRepository {
    pub async fn review_research_context(
        &self,
        topic: &str,
    ) -> Result<serde_json::Value, AppError> {
        let topic = topic.to_string();
        self.query_connection("Read editorial challenges for autonomous research", move |conn| {
            conn.execute_batch(SCHEMA).map_err(err)?;
            let reviews: Vec<FindingReview> = list_payloads(conn, "SELECT f.payload FROM finding_reviews f JOIN research_reports r ON r.id=f.report_id WHERE r.topic=?1 QUALIFY row_number() OVER(PARTITION BY f.report_id,f.finding_index ORDER BY f.revision DESC)=1 ORDER BY f.report_id LIMIT 8", &topic)?;
            let mut items = Vec::new();
            for review in reviews {
                let report = saved_report(conn, &review.input.report_id)?;
                let Some(finding) = report.synthesis.findings.get(review.input.finding_index) else { continue; };
                items.push(serde_json::json!({"finding":finding.title,"verdict":review.input.verdict,"reasoning":review.input.reasoning,"falsificationTest":review.input.falsification_test,"counterSearch":review.input.counter_search,"counterEvidence":review.input.counter_evidence}));
                if serde_json::to_vec(&items).map_err(err)?.len() > 18000 { items.pop(); break; }
            }
            Ok(serde_json::json!({"editorialChallenges":items,"instruction":"Human reviews are leads and workflow decisions, not evidence. Retrieve original sources, seek disconfirmation, and address contested explanations before restating a finding."}))
        }).await
    }
    pub async fn initialize_review(&self) -> Result<(), AppError> {
        self.query_connection("Initialize accountable evidence review", |conn| {
            conn.execute_batch(SCHEMA).map_err(err)
        })
        .await
    }
    pub async fn review_workspace(&self, id: String) -> Result<ReviewWorkspace, AppError> {
        self.query_connection("Review investigation evidence", move |conn| {
            workspace(conn, &id)
        })
        .await
    }
    pub async fn save_finding_review(
        &self,
        input: FindingReviewInput,
    ) -> Result<ReviewWorkspace, AppError> {
        self.query_connection("Append accountable finding review",move|conn|{
            let report=saved_report(conn,&input.report_id)?;review::validate_review(&input,&report)?;
            let tx=conn.transaction().map_err(err)?;
            let revision:u64=tx.query_row("SELECT COALESCE(MAX(revision),0) FROM finding_reviews WHERE report_id=?1 AND finding_index=?2",params![input.report_id,input.finding_index as u64],|r|r.get(0)).map_err(err)?;
            if revision!=input.expected_revision{return Err(err("This finding has a newer review. Reload and compare before saving; your review has not overwritten it."));}
            if input.verdict=="SUPPORTED"{
                let replays=stored_replays(&tx,&report)?;
                if !review::challenges(&report,&replays)[input.finding_index].blockers.is_empty(){return Err(err("Resolve source identity and calculation replay blockers before marking this finding supported."));}
            }
            for cite in &input.counter_evidence{
                let e=report.evidence.iter().find(|e|e.id==cite.evidence_id).unwrap();
                if !current_evidence(&tx,e)?{return Err(err("A counterevidence source was removed or replaced. Reload the evidence."));}
            }
            let record=FindingReview{id:uuid::Uuid::new_v4().to_string(),revision:revision+1,created_at:chrono::Utc::now().to_rfc3339(),input};
            tx.execute("INSERT INTO finding_reviews VALUES (?1,?2,?3,?4,?5)",params![record.id,record.input.report_id,record.input.finding_index as u64,record.revision,serde_json::to_string(&record).map_err(err)?]).map_err(err)?;
            tx.commit().map_err(err)?;workspace(conn,&record.input.report_id)
        }).await
    }
    pub async fn replay_review_evidence(&self, id: String) -> Result<ReviewWorkspace, AppError> {
        let report_id = id.clone();
        let report = self
            .query_connection("Read evidence to replay", move |conn| {
                saved_report(conn, &report_id)
            })
            .await?;
        let mut checks = Vec::new();
        let mut sql_count = 0;
        for e in &report.evidence {
            let mut replay=EvidenceReplay{evidence_id:e.id.clone(),checked_at:chrono::Utc::now().to_rfc3339(),state:"MATCH".into(),detail:"Public retained source identity matches; original bytes may have been released.".into()};
            if let Some(sql) = &e.sql {
                sql_count += 1;
                if sql_count > 12 {
                    replay.state = "NOT_RUN".into();
                    replay.detail =
                        "Twelve-query replay bound reached. Review remaining queries separately."
                            .into();
                } else {
                    match crate::research::evidence::query_evidence(
                        self,
                        &e.label,
                        sql,
                        "Independent saved-query replay",
                    )
                    .await
                    {
                        Ok(current) => {
                            if !review::sql_result_complete(e)
                                || !review::sql_result_complete(&current)
                            {
                                replay.state = "INCOMPLETE".into();
                                replay.detail = "A saved or recomputed result is truncated, or completeness was not recorded. Narrow or aggregate the query; matching prefixes do not verify the full result.".into();
                            } else if normalized_rows(&current.text) != normalized_rows(&e.text)
                                || normalized_rows(&e.text).is_none()
                                || current.snapshot_provenance != e.snapshot_provenance
                            {
                                replay.state = "RESULT_CHANGED".into();
                                replay.detail="Full query result columns or rows differ from the saved evidence.".into();
                            } else {
                                replay.detail="Recomputed saved SQL; columns and complete row values/types match (row order ignored).".into();
                            }
                        }
                        Err(error) => {
                            replay.state = "UNAVAILABLE".into();
                            replay.detail = error.to_string();
                        }
                    }
                }
            }
            checks.push(replay);
        }
        self.query_connection("Validate and save evidence replay",move|conn|{
            let tx=conn.transaction().map_err(err)?;
            for replay in &mut checks{
                let e=report.evidence.iter().find(|e|e.id==replay.evidence_id).unwrap();
                if !current_evidence(&tx,e)?{replay.state="SOURCE_CHANGED".into();replay.detail="Public source identity or retained analytical snapshot no longer matches.".into();}
                tx.execute("INSERT INTO evidence_replays VALUES (?1,?2,?3) ON CONFLICT(report_id,evidence_id) DO UPDATE SET payload=excluded.payload",params![id,replay.evidence_id,serde_json::to_string(replay).map_err(err)?]).map_err(err)?;
            }
            tx.commit().map_err(err)?;workspace(conn,&id)
        }).await
    }
    pub async fn save_public_relationship(
        &self,
        mut value: PublicRelationship,
        expected_revision: u64,
    ) -> Result<ReviewWorkspace, AppError> {
        self.query_connection("Review a public relationship",move|conn|{
            let report=saved_report(conn,&value.report_id)?;review::validate_relationship(&value,&report)?;
            if value.status=="CONFIRMED" && [&value.subject.key,&value.object.key].iter().any(|k|k.starts_with("unresolved:")){return Err(err("Replace unresolved identifiers with cited agency/registry identifiers before linking entities."));}
            let tx=conn.transaction().map_err(err)?;
            for cite in &value.citations{
                let evidence=report.evidence.iter().find(|e|e.id==cite.evidence_id).unwrap();
                if !current_evidence(&tx,evidence)?{return Err(err("This relationship's public source identity has changed or disappeared."));}
            }
            if value.id.is_empty(){value.id=uuid::Uuid::new_v4().to_string();}
            let (revision,original_report):(u64,Option<String>)=tx.query_row("SELECT COALESCE(MAX(revision),0),MIN(report_id) FROM public_relationships WHERE id=?1",params![value.id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(err)?;
            if revision!=expected_revision || original_report.is_some_and(|r|r!=value.report_id){return Err(err("Relationship changed or belongs to another report; reload before saving."));}
            value.revision=revision+1;
            tx.execute("INSERT INTO public_relationships VALUES (?1,?2,?3,?4)",params![value.id,value.revision,value.report_id,serde_json::to_string(&value).map_err(err)?]).map_err(err)?;
            tx.commit().map_err(err)?;workspace(conn,&value.report_id)
        }).await
    }
    pub async fn investigation_evaluations(
        &self,
    ) -> Result<Vec<InvestigationEvaluation>, AppError> {
        self.query_connection("Evaluate retained reference investigations",|conn|{
            let mut result=Vec::new();
            for (id,raw) in [("reference-finance",include_str!("../../../docs/validation/live-finance-dossier.json")),("reference-homelessness",include_str!("../../../docs/validation/live-research-dossier.json"))]{
                let report:ResearchReport=serde_json::from_str(raw).map_err(err)?;let mut evaluation=review::evaluate_report(&report);evaluation.id=id.into();evaluation.title=format!("Reference replay: {}",evaluation.title);if id == "reference-finance" { evaluation.checks.extend(review::reference_finance_arithmetic()?); }result.push(evaluation);
            }
            let mut stmt=conn.prepare("SELECT id FROM research_reports ORDER BY created_at DESC LIMIT 20").map_err(err)?;
            let ids=stmt.query_map([],|r|r.get::<_,String>(0)).map_err(err)?.collect::<Result<Vec<_>,_>>().map_err(err)?;
            for id in ids{let report=saved_report(conn,&id)?;let mut evaluation=review::evaluate_report(&report);evaluation.assessments=list_payloads(conn,"SELECT payload FROM pilot_assessments WHERE report_id=?1 ORDER BY assessed_at DESC",&id)?;result.push(evaluation);}
            Ok(result)
        }).await
    }
    pub async fn save_pilot_assessment(
        &self,
        value: PilotAssessment,
    ) -> Result<Vec<InvestigationEvaluation>, AppError> {
        if value.assessor.trim().is_empty()
            || value.assessor.len() > 120
            || !(1..=5).contains(&value.usefulness)
            || value.minutes_spent > 100_000
            || value.baseline_minutes.is_some_and(|m| m > 100_000)
            || value.result_used.trim().is_empty()
            || value.result_used.len() > 4000
            || value.notes.len() > 8000
        {
            return Err(AppError::DataParsing("Provide an assessor, usefulness 1–5, time spent and a concrete result or reason it was not useful.".into()));
        }
        self.query_connection("Record human pilot assessment", move |conn| {
            let report = saved_report(conn, &value.report_id)?;
            if value.supported_discoveries as usize + value.false_leads as usize
                > report.synthesis.findings.len()
            {
                return Err(err(
                    "Supported discoveries and false leads cannot exceed the findings assessed.",
                ));
            }
            conn.execute(
                "INSERT INTO pilot_assessments VALUES (?1,?2,?3,?4)",
                params![
                    uuid::Uuid::new_v4().to_string(),
                    value.report_id,
                    chrono::Utc::now().to_rfc3339(),
                    serde_json::to_string(&value).map_err(err)?
                ],
            )
            .map_err(err)?;
            Ok(())
        })
        .await?;
        self.investigation_evaluations().await
    }
}
fn normalized_rows(raw: &str) -> Option<(serde_json::Value, Vec<String>)> {
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    let columns = value.get("columns")?.clone();
    let mut rows = value
        .get("rows")?
        .as_array()?
        .iter()
        .map(serde_json::to_string)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    rows.sort();
    Some((columns, rows))
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn setup() -> DuckDbRepository {
        let repo = DuckDbRepository::new(":memory:").unwrap();
        repo.run_migrations(include_str!("../../migrations/0001_initial_schema.sql"))
            .await
            .unwrap();
        repo.initialize_research().await.unwrap();
        repo.initialize_review().await.unwrap();
        let report: ResearchReport = serde_json::from_str(include_str!(
            "../../../docs/validation/live-finance-dossier.json"
        ))
        .unwrap();
        let conn = repo.analytics_connection().await.unwrap();
        conn.execute(
            "INSERT INTO research_reports VALUES (?1,?2,?3,'fixture',?4)",
            params![
                report.id,
                report.topic,
                report.created_at,
                zstd::stream::encode_all(serde_json::to_vec(&report).unwrap().as_slice(), 9)
                    .unwrap()
            ],
        )
        .unwrap();
        repo
    }
    #[tokio::test]
    async fn upgrade_from_pre_review_schema_preserves_reports_and_adds_review_tables() {
        let repo = setup().await;
        repo.initialize_review().await.unwrap();
        assert_eq!(repo.investigation_evaluations().await.unwrap().len(), 3);
    }
    #[tokio::test]
    async fn review_is_append_only_and_concurrent_edits_are_rejected() {
        let repo = setup().await;
        let id = "ea7c13f1-f4f2-47e6-9db1-9dea8862b68b";
        let mut input = FindingReviewInput {
            report_id: id.into(),
            finding_index: 0,
            expected_revision: 0,
            reviewer: "Editor".into(),
            assigned_to: "Reporter".into(),
            verdict: "OPEN".into(),
            reasoning: "Need revised appropriation to separate authorization from variance".into(),
            falsification_test: "Obtain the adopted and revised budget account crosswalk".into(),
            counter_search: "Searched the current actuals and budget packets".into(),
            counter_evidence: vec![],
        };
        assert_eq!(
            repo.save_finding_review(input.clone())
                .await
                .unwrap()
                .reviews[0]
                .revision,
            1
        );
        assert!(repo.save_finding_review(input.clone()).await.is_err());
        input.expected_revision = 1;
        input.verdict = "SUPPORTED".into();
        assert!(repo.save_finding_review(input.clone()).await.is_err());
        input.verdict = "CONTESTED".into();
        let result = repo.save_finding_review(input).await.unwrap();
        assert_eq!(result.reviews.len(), 2);
        assert_eq!(result.reviews[0].revision, 2);
        let context = repo.review_research_context("police").await.unwrap();
        assert_eq!(context["editorialChallenges"][0]["verdict"], "CONTESTED");
        assert!(context.to_string().contains("appropriation"));
        assert!(!context.to_string().contains("Editor"));
    }
    #[tokio::test]
    async fn source_replay_checks_retained_projection_and_public_identity() {
        let repo = setup().await;
        let conn = repo.analytics_connection().await.unwrap();
        conn.execute("INSERT INTO portals(subdomain,display_name,base_url) VALUES ('city','City','https://www.sandiego.gov')",[]).unwrap();
        conn.execute("INSERT INTO requests(id,portal_subdomain,title,status,created_at,updated_at) VALUES (1,'city','Review fixture','closed','2026-09-26','2026-09-26')",[]).unwrap();
        conn.execute("INSERT INTO documents(id,request_id,portal_subdomain,file_name,download_url,visibility,created_at,updated_at,raw_file_sha256,extraction_status) VALUES (1,1,'city','record.txt','https://www.sandiego.gov/record.txt','public','2026-09-26','2026-09-26',repeat('a',64),'SEARCHABLE')",[]).unwrap();
        conn.execute("INSERT INTO documents_fts(document_id,portal_subdomain,file_name,extracted_text) VALUES (1,'city','record.txt','Actual payments are 100.25')",[]).unwrap();
        let e = ResearchEvidence {
            id: "one".into(),
            label: "Actuals".into(),
            source_url: Some("https://www.sandiego.gov/record.txt".into()),
            portal_key: Some("city".into()),
            request_id: Some(1),
            document_id: Some(1),
            character_offset: Some(0),
            source_sha256: "a".repeat(64),
            text: "Actual payments are 100.25".into(),
            sql: None,
            snapshot_provenance: vec![],
            source_references: vec![],
            limitations: String::new(),
        };
        assert!(current_evidence(&conn, &e).unwrap());
        conn.execute("UPDATE documents SET visibility='private' WHERE id=1", [])
            .unwrap();
        assert!(!current_evidence(&conn, &e).unwrap());
        conn.execute("UPDATE documents SET visibility='public' WHERE id=1", [])
            .unwrap();
        conn.execute("UPDATE documents_fts SET extracted_text='Actual payments are 900.25' WHERE document_id=1",[]).unwrap();
        assert!(!current_evidence(&conn, &e).unwrap());
    }
    #[tokio::test]
    async fn source_replay_checks_changed_latest_view_targets() {
        let repo = setup().await;
        let conn = repo.analytics_connection().await.unwrap();
        conn.execute_batch("CREATE TABLE analysis_old(amount DECIMAL(18,2)); INSERT INTO analysis_old VALUES (100.25); CREATE TABLE analysis_new(amount DECIMAL(18,2)); INSERT INTO analysis_new VALUES (900.25); CREATE VIEW analysis_latest AS SELECT * FROM analysis_old;").unwrap();
        for (id, table, hash) in [("old", "analysis_old", "a"), ("new", "analysis_new", "b")] {
            conn.execute("INSERT INTO analytical_snapshots(snapshot_id,dataset_kind,display_name,table_name,source_sha256,retrieved_at,partial_dataset,imported_rows,rejected_rows,validation_summary_json,local_path) VALUES (?1,'REGIONAL_DATA',?1,?2,?3,'2026-09-26',0,1,0,'{}','released-source.csv')",params![id,table,hash.repeat(64)]).unwrap();
        }
        let evidence = ResearchEvidence {
            id: "sql:fixture".into(),
            label: "Payments".into(),
            source_url: None,
            portal_key: None,
            request_id: None,
            document_id: None,
            character_offset: None,
            source_sha256: "query-hash".into(),
            text: r#"{"columns":["amount"],"rows":[["100.25"]],"truncated":false}"#.into(),
            sql: Some("SELECT amount FROM analysis_latest".into()),
            snapshot_provenance: vec!["analysis_old@old".into()],
            source_references: vec![],
            limitations: String::new(),
        };
        assert!(current_evidence(&conn, &evidence).unwrap());
        conn.execute_batch("CREATE OR REPLACE VIEW analysis_latest AS SELECT * FROM analysis_new")
            .unwrap();
        assert!(
            !current_evidence(&conn, &evidence).unwrap(),
            "an old immutable snapshot surviving must not validate a retargeted latest view"
        );
    }
    #[test]
    fn replay_compares_all_rows_types_and_multiplicity() {
        let a = r#"{"columns":["amount"],"rows":[["1.00"],[null],["1.00"]]}"#;
        let b = r#"{"columns":["amount"],"rows":[[null],["1.00"],["1.00"]]}"#;
        assert_eq!(normalized_rows(a), normalized_rows(b));
        assert_ne!(
            normalized_rows(a),
            normalized_rows(r#"{"columns":["amount"],"rows":[[1],[null],[1]]}"#)
        );
        assert_ne!(
            normalized_rows(a),
            normalized_rows(r#"{"columns":["amount"],"rows":[["1.00"],[null]]}"#)
        );
    }
    #[tokio::test]
    async fn human_pilot_scores_require_actual_assessments_and_reject_invented_counts() {
        let repo = setup().await;
        let before = repo.investigation_evaluations().await.unwrap();
        assert!(before.iter().all(|e| e.assessments.is_empty()));
        let mut assessment = PilotAssessment {
            report_id: "ea7c13f1-f4f2-47e6-9db1-9dea8862b68b".into(),
            assessor: "Pilot editor".into(),
            usefulness: 4,
            supported_discoveries: 7,
            false_leads: 0,
            minutes_spent: 20,
            baseline_minutes: None,
            result_used: "Prepared an account crosswalk request".into(),
            notes: "No time-saved estimate without a baseline".into(),
        };
        assert!(repo
            .save_pilot_assessment(assessment.clone())
            .await
            .is_err());
        assessment.supported_discoveries = 1;
        let after = repo.save_pilot_assessment(assessment).await.unwrap();
        assert_eq!(after[2].assessments[0].baseline_minutes, None);
    }
}
