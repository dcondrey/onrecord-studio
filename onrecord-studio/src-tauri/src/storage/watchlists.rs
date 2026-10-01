//! Saved watches and their change history.
//!
//! A check observes the local corpus, compares with the stored per-dimension baseline through
//! [`crate::analysis::watchlists::diff`], records only the changes it has not recorded before, and
//! advances the baseline for the dimensions it actually measured. The read, the comparison and the
//! write happen in one transaction on the single writer connection, so two overlapping checks
//! cannot interleave and a repeated check cannot add a second copy of a change.
//!
//! Tables carry no foreign keys to other tables and no integer key columns, so they create
//! identically on a database file written by an older release.
use crate::{
    analysis::{
        topic_packs::{build_topic_packs, TOPIC_PACKS},
        watchlists::{diff, query_terms, FigureSource, Labeled, Slot, SourceHealth, WatchSnapshot},
    },
    error::AppError,
    storage::{
        monitoring::{packed, unpacked},
        sqlite::DuckDbRepository,
    },
    types::*,
};
use chrono::SecondsFormat;
use duckdb::{params, params_from_iter, Connection, OptionalExt};
use std::collections::{BTreeMap, BTreeSet, HashSet};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS watches(id TEXT PRIMARY KEY, kind TEXT NOT NULL, target TEXT NOT NULL, label TEXT NOT NULL, created_at TEXT NOT NULL, enabled BOOLEAN NOT NULL);
CREATE TABLE IF NOT EXISTS watch_baselines(watch_id TEXT NOT NULL, dimension TEXT NOT NULL, observed_at TEXT NOT NULL, payload_zstd BLOB NOT NULL, PRIMARY KEY(watch_id, dimension));
CREATE TABLE IF NOT EXISTS watch_runs(id TEXT PRIMARY KEY, watch_id TEXT NOT NULL, checked_at TEXT NOT NULL, status TEXT NOT NULL, dimensions_json TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS watch_changes(id TEXT PRIMARY KEY, watch_id TEXT NOT NULL, dimension TEXT NOT NULL, detected_at TEXT NOT NULL, portal_key TEXT, payload TEXT NOT NULL);
";

pub const MAX_WATCHES: usize = 50;
#[cfg(not(test))]
const MAX_SCOPE_DOCUMENTS: usize = 20_000;
/// Small under test so the cap's three sides are reachable without twenty thousand rows.
#[cfg(test)]
const MAX_SCOPE_DOCUMENTS: usize = 3;
const MAX_SCOPE_FINDINGS: usize = 50_000;
const MAX_FIGURE_SOURCES: usize = 500;
const MAX_FIGURES_PER_SOURCE: usize = 40;
const MAX_SCOPE_PORTALS: usize = 50;
const RUNS_KEPT: i64 = 50;

fn err(e: impl std::fmt::Display) -> AppError {
    AppError::Storage(e.to_string())
}

fn invalid(message: &str) -> AppError {
    AppError::DataParsing(message.to_string())
}

fn now_stamp() -> String {
    chrono::Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

type Labels = BTreeMap<String, Labeled>;

#[derive(Clone)]
struct ScopeDocument {
    portal_key: String,
    id: i64,
    request_id: i64,
    label: String,
}

/// What must be computed outside the connection closure.
#[derive(Default)]
struct Prefetch {
    topic: Option<TopicScope>,
    /// Why version monitoring could not be brought up to date, when it could not.
    figures_unavailable: Option<String>,
}

struct TopicScope {
    documents: Result<Vec<ScopeDocument>, String>,
    findings: Result<Vec<(String, String, Option<String>)>, String>,
}

impl DuckDbRepository {
    pub async fn initialize_watchlists(&self) -> Result<(), AppError> {
        self.query_connection("Initialize watchlists", |conn| {
            conn.execute_batch(SCHEMA).map_err(err)
        })
        .await
    }

    pub async fn save_watch(&self, watch: Watch) -> Result<Watch, AppError> {
        let target = watch
            .target
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if target.is_empty() || target.chars().count() > 200 {
            return Err(invalid("A watch needs a target of 1 to 200 characters."));
        }
        let target = match watch.kind {
            WatchKind::Query => query_terms(&target)
                .ok_or_else(|| {
                    invalid("A query watch takes 1 to 8 terms of at least 2 characters each.")
                })?
                .join(" "),
            WatchKind::TopicPack => {
                if !TOPIC_PACKS.iter().any(|pack| pack.id == target) {
                    return Err(invalid("That topic pack does not exist."));
                }
                target
            }
            _ => target,
        };
        let label = watch.label.trim();
        let label = if label.is_empty() {
            target.clone()
        } else {
            label.to_string()
        };
        if label.chars().count() > 120 {
            return Err(invalid("A watch label is at most 120 characters."));
        }
        let Watch {
            id, kind, enabled, ..
        } = watch;
        self.query_connection("Save watch", move |conn| {
            conn.execute_batch(SCHEMA).map_err(err)?;
            if kind == WatchKind::Source {
                let known: bool = conn
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM portals WHERE subdomain=?1)",
                        params![target],
                        |r| r.get(0),
                    )
                    .map_err(err)?;
                if !known {
                    return Err(invalid("That source is not one of this workspace's sources."));
                }
            }
            let tx = conn.transaction().map_err(err)?;
            let existing: Option<(String, String, String, String)> = if id.is_empty() {
                None
            } else {
                tx.query_row(
                    "SELECT kind,target,created_at,id FROM watches WHERE id=?1",
                    params![id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .optional()
                .map_err(err)?
            };
            let saved = if let Some((old_kind, old_target, created_at, id)) = existing {
                if old_kind != kind.as_str() || old_target != target {
                    return Err(invalid(
                        "A watch's kind and target cannot change; save a new watch instead.",
                    ));
                }
                tx.execute(
                    "UPDATE watches SET label=?2, enabled=?3 WHERE id=?1",
                    params![id, label, enabled],
                )
                .map_err(err)?;
                Watch { id, kind, target, label, created_at, enabled }
            } else {
                if !id.is_empty() {
                    return Err(invalid("That watch no longer exists."));
                }
                let count: i64 = tx
                    .query_row("SELECT count(*) FROM watches", [], |r| r.get(0))
                    .map_err(err)?;
                if count as usize >= MAX_WATCHES {
                    return Err(invalid("The watchlist is full; delete a watch first."));
                }
                let duplicate: bool = tx
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM watches WHERE kind=?1 AND lower(target)=lower(?2))",
                        params![kind.as_str(), target],
                        |r| r.get(0),
                    )
                    .map_err(err)?;
                if duplicate {
                    return Err(invalid("That target is already on the watchlist."));
                }
                let watch = Watch {
                    id: uuid::Uuid::new_v4().to_string(),
                    kind,
                    target,
                    label,
                    created_at: now_stamp(),
                    enabled,
                };
                tx.execute(
                    "INSERT INTO watches VALUES (?1,?2,?3,?4,?5,?6)",
                    params![watch.id, kind.as_str(), watch.target, watch.label, watch.created_at, watch.enabled],
                )
                .map_err(err)?;
                watch
            };
            tx.commit().map_err(err)?;
            Ok(saved)
        })
        .await
    }

    pub async fn delete_watch(&self, id: String) -> Result<(), AppError> {
        self.query_connection("Delete watch", move |conn| {
            conn.execute_batch(SCHEMA).map_err(err)?;
            let tx = conn.transaction().map_err(err)?;
            for table in ["watch_baselines", "watch_runs", "watch_changes", "watches"] {
                let column = if table == "watches" { "id" } else { "watch_id" };
                tx.execute(
                    &format!("DELETE FROM {table} WHERE {column}=?1"),
                    params![id],
                )
                .map_err(err)?;
            }
            tx.commit().map_err(err)
        })
        .await
    }

    pub async fn list_watches(&self) -> Result<Vec<WatchSummary>, AppError> {
        self.query_connection("List watches", |conn| {
            conn.execute_batch(SCHEMA).map_err(err)?;
            let watches = read_watches(conn, None)?;
            let mut summaries = Vec::new();
            for watch in watches {
                let last: Option<(String, String)> = conn
                    .query_row(
                        "SELECT checked_at,dimensions_json FROM watch_runs WHERE watch_id=?1 ORDER BY checked_at DESC, id DESC LIMIT 1",
                        params![watch.id],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()
                    .map_err(err)?;
                let (last_checked_at, last_dimensions) = match last {
                    Some((at, json)) => (Some(at), serde_json::from_str(&json).map_err(err)?),
                    None => (None, vec![]),
                };
                summaries.push(WatchSummary { watch, last_checked_at, last_dimensions });
            }
            Ok(summaries)
        })
        .await
    }

    pub async fn list_watch_changes(
        &self,
        watch_id: Option<String>,
        limit: u32,
    ) -> Result<Vec<WatchChange>, AppError> {
        let limit = i64::from(limit.clamp(1, 500));
        self.query_connection("List watch changes", move |conn| {
            conn.execute_batch(SCHEMA).map_err(err)?;
            let mut statement = conn
                .prepare(
                    "SELECT payload FROM watch_changes c WHERE (?1 IS NULL OR c.watch_id=?1)
                     AND EXISTS(SELECT 1 FROM watches w WHERE w.id=c.watch_id)
                     AND (c.portal_key IS NULL OR EXISTS(SELECT 1 FROM portals p WHERE p.subdomain=c.portal_key AND p.lifecycle_state<>'REMOVED'))
                     ORDER BY c.detected_at DESC, c.id LIMIT ?2",
                )
                .map_err(err)?;
            let rows = statement
                .query_map(params![watch_id, limit], |r| r.get::<_, String>(0))
                .map_err(err)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(err)?;
            rows.into_iter()
                .map(|json| serde_json::from_str(&json).map_err(err))
                .collect()
        })
        .await
    }

    /// Checks one watch, or every enabled watch when `watch_id` is `None`.
    pub async fn check_watches(
        &self,
        watch_id: Option<String>,
    ) -> Result<Vec<WatchCheck>, AppError> {
        self.check_watches_at(watch_id, now_stamp()).await
    }

    pub(crate) async fn check_watches_at(
        &self,
        watch_id: Option<String>,
        checked_at: String,
    ) -> Result<Vec<WatchCheck>, AppError> {
        let requested = watch_id.clone();
        let watches = self
            .query_connection("Read watches to check", move |conn| {
                conn.execute_batch(SCHEMA).map_err(err)?;
                read_watches(conn, requested.as_deref())
            })
            .await?;
        if watch_id.is_some() && watches.is_empty() {
            return Err(invalid("That watch no longer exists."));
        }
        let watches: Vec<Watch> = watches
            .into_iter()
            .filter(|w| watch_id.is_some() || w.enabled)
            .collect();
        if watches.is_empty() {
            return Ok(vec![]);
        }
        let mut prefetch = Prefetch::default();
        if let Err(error) = self.capture_monitoring_versions().await {
            prefetch.figures_unavailable = Some(format!(
                "Version monitoring could not be brought up to date: {error}"
            ));
        }
        let mut topic_report = None;
        let mut checks = Vec::new();
        for watch in watches {
            let mut watch_prefetch = Prefetch {
                topic: None,
                figures_unavailable: prefetch.figures_unavailable.clone(),
            };
            if watch.kind == WatchKind::TopicPack {
                if topic_report.is_none() {
                    topic_report = Some(build_topic_packs(self).await.map_err(|e| e.to_string()));
                }
                watch_prefetch.topic =
                    Some(topic_scope(topic_report.as_ref().unwrap(), &watch.target));
            }
            let at = checked_at.clone();
            // A watch deleted while this run was in flight is skipped, not recreated.
            if let Some(check) = self
                .query_connection("Check watch", move |conn| {
                    check_one(conn, &watch, watch_prefetch, &at)
                })
                .await?
            {
                checks.push(check);
            }
        }
        if watch_id.is_some() && checks.is_empty() {
            return Err(invalid("That watch no longer exists."));
        }
        Ok(checks)
    }
}

fn topic_scope(
    report: &Result<crate::analysis::topic_packs::TopicPacksReport, String>,
    pack_id: &str,
) -> TopicScope {
    let report = match report {
        Ok(report) => report,
        Err(reason) => {
            let reason = format!("The topic scan failed: {reason}");
            return TopicScope {
                documents: Err(reason.clone()),
                findings: Err(reason),
            };
        }
    };
    let Some(view) = report.topics.iter().find(|t| t.id == pack_id) else {
        let reason = "The topic scan did not return this pack.".to_string();
        return TopicScope {
            documents: Err(reason.clone()),
            findings: Err(reason),
        };
    };
    let documents = if report.document_limit_reached || view.documents.len() < view.document_total {
        Err("The topic scan hit its document limit, so the tagged set is incomplete.".to_string())
    } else {
        Ok(view
            .documents
            .iter()
            .map(|d| ScopeDocument {
                portal_key: d.portal_key.clone(),
                id: d.document_id,
                request_id: d.request_id,
                label: d
                    .file_name
                    .clone()
                    .unwrap_or_else(|| format!("Document {}", d.document_id)),
            })
            .collect())
    };
    let findings = if view.findings.len() < view.finding_total {
        Err("The topic scan listed fewer findings than it counted.".to_string())
    } else {
        Ok(view
            .findings
            .iter()
            .map(|m| {
                (
                    m.finding.id.clone(),
                    m.finding.headline.clone(),
                    m.finding.subject.portal_key.clone(),
                )
            })
            .collect())
    };
    TopicScope {
        documents,
        findings,
    }
}

fn read_watches(conn: &Connection, only: Option<&str>) -> Result<Vec<Watch>, AppError> {
    let mut statement = conn
        .prepare("SELECT id,kind,target,label,created_at,enabled FROM watches WHERE (?1 IS NULL OR id=?1) ORDER BY created_at, id")
        .map_err(err)?;
    let rows = statement
        .query_map(params![only], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, bool>(5)?,
            ))
        })
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;
    rows.into_iter()
        .map(|(id, kind, target, label, created_at, enabled)| {
            Ok(Watch {
                id,
                kind: WatchKind::parse(&kind)
                    .ok_or_else(|| err(format!("Unknown watch kind {kind}")))?,
                target,
                label,
                created_at,
                enabled,
            })
        })
        .collect()
}

fn read_slot<T: serde::de::DeserializeOwned>(
    conn: &Connection,
    watch_id: &str,
    dimension: WatchDimension,
) -> Result<Slot<T>, AppError> {
    let bytes: Option<Vec<u8>> = conn
        .query_row(
            "SELECT payload_zstd FROM watch_baselines WHERE watch_id=?1 AND dimension=?2",
            params![watch_id, dimension.as_str()],
            |r| r.get(0),
        )
        .optional()
        .map_err(err)?;
    match bytes {
        Some(bytes) => unpacked(&bytes),
        None => Ok(Slot::not_measured(
            "No baseline has been recorded for this dimension.",
        )),
    }
}

fn read_baseline(conn: &Connection, watch_id: &str) -> Result<WatchSnapshot, AppError> {
    Ok(WatchSnapshot {
        documents: read_slot(conn, watch_id, WatchDimension::Documents)?,
        findings: read_slot(conn, watch_id, WatchDimension::Findings)?,
        figures: read_slot(conn, watch_id, WatchDimension::Figures)?,
        withheld: read_slot(conn, watch_id, WatchDimension::Withheld)?,
        source_health: read_slot(conn, watch_id, WatchDimension::SourceHealth)?,
    })
}

fn write_slot<T: serde::Serialize>(
    tx: &Connection,
    watch_id: &str,
    dimension: WatchDimension,
    slot: &Slot<T>,
) -> Result<(), AppError> {
    let Slot::Measured { as_of, .. } = slot else {
        return Ok(());
    };
    tx.execute(
        "INSERT INTO watch_baselines VALUES (?1,?2,?3,?4) ON CONFLICT(watch_id,dimension) DO UPDATE SET observed_at=excluded.observed_at, payload_zstd=excluded.payload_zstd",
        params![watch_id, dimension.as_str(), as_of, packed(slot)?],
    )
    .map_err(err)?;
    Ok(())
}

fn check_one(
    conn: &mut Connection,
    watch: &Watch,
    prefetch: Prefetch,
    checked_at: &str,
) -> Result<Option<WatchCheck>, AppError> {
    conn.execute_batch(SCHEMA).map_err(err)?;
    let tx = conn.transaction().map_err(err)?;
    // The watch was read before this transaction. Writing a baseline, run or change for one that
    // was deleted since would leave rows no watch owns, still listed in the all-watches feed.
    let still_saved: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM watches WHERE id=?1)",
            params![watch.id],
            |r| r.get(0),
        )
        .map_err(err)?;
    if !still_saved {
        return Ok(None);
    }
    let baseline = read_baseline(&tx, &watch.id)?;
    let current = observe(&tx, watch, &baseline, &prefetch, checked_at)?;
    let out = diff(&watch.id, &baseline, &current, checked_at);

    let mut new_changes = Vec::new();
    for change in out.changes {
        let inserted = tx
            .execute(
                "INSERT INTO watch_changes VALUES (?1,?2,?3,?4,?5,?6) ON CONFLICT(id) DO NOTHING",
                params![
                    change.id,
                    change.watch_id,
                    change.dimension.as_str(),
                    change.detected_at,
                    change.portal_key,
                    serde_json::to_string(&change).map_err(err)?
                ],
            )
            .map_err(err)?;
        if inserted > 0 {
            new_changes.push(change);
        }
    }
    write_slot(
        &tx,
        &watch.id,
        WatchDimension::Documents,
        &out.next.documents,
    )?;
    write_slot(&tx, &watch.id, WatchDimension::Findings, &out.next.findings)?;
    write_slot(&tx, &watch.id, WatchDimension::Figures, &out.next.figures)?;
    write_slot(&tx, &watch.id, WatchDimension::Withheld, &out.next.withheld)?;
    write_slot(
        &tx,
        &watch.id,
        WatchDimension::SourceHealth,
        &out.next.source_health,
    )?;

    let status = serde_json::to_value(out.status).map_err(err)?;
    tx.execute(
        "INSERT INTO watch_runs VALUES (?1,?2,?3,?4,?5)",
        params![
            uuid::Uuid::new_v4().to_string(),
            watch.id,
            checked_at,
            status.as_str().unwrap_or_default(),
            serde_json::to_string(&out.dimensions).map_err(err)?
        ],
    )
    .map_err(err)?;
    tx.execute(
        "DELETE FROM watch_runs WHERE watch_id=?1 AND id NOT IN (SELECT id FROM watch_runs WHERE watch_id=?1 ORDER BY checked_at DESC, id DESC LIMIT ?2)",
        params![watch.id, RUNS_KEPT],
    )
    .map_err(err)?;
    tx.commit().map_err(err)?;
    Ok(Some(WatchCheck {
        watch_id: watch.id.clone(),
        checked_at: checked_at.to_string(),
        status: out.status,
        dimensions: out.dimensions,
        new_changes,
    }))
}

fn table_exists(conn: &Connection, name: &str) -> Result<bool, AppError> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM information_schema.tables WHERE table_name=?1)",
        params![name],
        |r| r.get(0),
    )
    .map_err(err)
}

fn public_scope() -> &'static str {
    "lower(trim(d.visibility))='public' AND p.lifecycle_state<>'REMOVED'"
}

fn scope_documents(
    conn: &Connection,
    watch: &Watch,
    prefetch: &Prefetch,
) -> Result<Result<Vec<ScopeDocument>, String>, AppError> {
    let select = "SELECT d.portal_subdomain, d.id, d.request_id, COALESCE(d.file_name, 'Document ' || d.id) FROM documents d JOIN portals p ON p.subdomain=d.portal_subdomain";
    let (sql, binds): (String, Vec<String>) = match watch.kind {
        WatchKind::TopicPack => {
            return Ok(match &prefetch.topic {
                Some(scope) => scope.documents.clone(),
                None => Err("The topic scan did not run.".to_string()),
            })
        }
        WatchKind::Source => (
            format!("{select} WHERE d.portal_subdomain=?1 AND {}", public_scope()),
            vec![watch.target.clone()],
        ),
        WatchKind::Agency => (
            format!(
                "{select} JOIN requests r ON r.id=d.request_id AND r.portal_subdomain=d.portal_subdomain JOIN agencies a ON a.id=r.agency_id AND a.portal_subdomain=r.portal_subdomain WHERE lower(a.name)=lower(?1) AND {}",
                public_scope()
            ),
            vec![watch.target.clone()],
        ),
        WatchKind::Entity | WatchKind::Query => {
            let terms = text_terms(watch);
            let conditions = (1..=terms.len())
                .map(|i| format!("instr(lower(f.extracted_text), ?{i})>0"))
                .collect::<Vec<_>>()
                .join(" AND ");
            (
                format!(
                    "{select} JOIN documents_fts f ON f.document_id=d.id AND f.portal_subdomain=d.portal_subdomain WHERE {} AND {conditions}",
                    public_scope()
                ),
                terms,
            )
        }
    };
    let sql = format!(
        "{sql} ORDER BY d.portal_subdomain, d.id LIMIT {}",
        MAX_SCOPE_DOCUMENTS + 1
    );
    let mut statement = conn.prepare(&sql).map_err(err)?;
    let rows = statement
        .query_map(params_from_iter(binds.iter()), |r| {
            Ok(ScopeDocument {
                portal_key: r.get(0)?,
                id: r.get(1)?,
                request_id: r.get(2)?,
                label: r.get(3)?,
            })
        })
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;
    if rows.len() > MAX_SCOPE_DOCUMENTS {
        return Ok(Err(format!(
            "More than {MAX_SCOPE_DOCUMENTS} documents match this watch; narrow it."
        )));
    }
    Ok(Ok(rows))
}

/// Lowercased match terms: the whole phrase for an entity, each term for a query.
fn text_terms(watch: &Watch) -> Vec<String> {
    match watch.kind {
        WatchKind::Entity => vec![watch.target.to_lowercase()],
        _ => query_terms(&watch.target).unwrap_or_default(),
    }
}

fn observe(
    conn: &Connection,
    watch: &Watch,
    baseline: &WatchSnapshot,
    prefetch: &Prefetch,
    now: &str,
) -> Result<WatchSnapshot, AppError> {
    let documents = scope_documents(conn, watch, prefetch)?;
    let doc_keys: Option<HashSet<(String, i64)>> = documents
        .as_ref()
        .ok()
        .map(|docs| docs.iter().map(|d| (d.portal_key.clone(), d.id)).collect());

    let findings = observe_findings(
        conn,
        watch,
        baseline,
        prefetch,
        doc_keys.as_ref(),
        &documents,
        now,
    )?;
    let (withheld, figures, source_health) = match &documents {
        Ok(docs) => (
            observe_withheld(conn, docs, now)?,
            observe_figures(conn, docs, prefetch, now)?,
            observe_sources(conn, watch, docs, now)?,
        ),
        Err(reason) => (
            Slot::not_measured(reason.clone()),
            Slot::not_measured(reason.clone()),
            match watch.kind {
                WatchKind::Source => observe_sources(conn, watch, &[], now)?,
                _ => Slot::not_measured(reason.clone()),
            },
        ),
    };
    let documents = match documents {
        Ok(docs) => Slot::measured(
            docs.into_iter()
                .map(|d| {
                    (
                        format!("{}:{}", d.portal_key, d.id),
                        Labeled {
                            label: d.label,
                            portal_key: Some(d.portal_key),
                        },
                    )
                })
                .collect(),
            now,
        ),
        Err(reason) => Slot::not_measured(reason),
    };
    Ok(WatchSnapshot {
        documents,
        findings,
        figures,
        withheld,
        source_health,
    })
}

fn observe_findings(
    conn: &Connection,
    watch: &Watch,
    baseline: &WatchSnapshot,
    prefetch: &Prefetch,
    doc_keys: Option<&HashSet<(String, i64)>>,
    documents: &Result<Vec<ScopeDocument>, String>,
    now: &str,
) -> Result<Slot<Labels>, AppError> {
    // A finding that stops being listed while still in the table was dismissed, not resolved. The
    // topic scan omits dismissed findings, so ids the baseline held that are still stored stay.
    let stored: BTreeMap<String, (String, Option<String>)> = {
        let mut statement = conn
            .prepare("SELECT id, headline, explanation, subject_portal_key, subject_document_id, subject_agency_id FROM findings LIMIT ?1")
            .map_err(err)?;
        let rows = statement
            .query_map(params![(MAX_SCOPE_FINDINGS + 1) as i64], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<i64>>(4)?,
                    r.get::<_, Option<i64>>(5)?,
                ))
            })
            .map_err(err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(err)?;
        if rows.len() > MAX_SCOPE_FINDINGS {
            return Ok(Slot::not_measured(format!(
                "More than {MAX_SCOPE_FINDINGS} findings are stored; the watch cannot be scoped."
            )));
        }
        let agencies: HashSet<(String, i64)> = if watch.kind == WatchKind::Agency {
            let mut statement = conn
                .prepare("SELECT portal_subdomain, id FROM agencies WHERE lower(name)=lower(?1)")
                .map_err(err)?;
            let found = statement
                .query_map(params![watch.target], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
                })
                .map_err(err)?
                .collect::<Result<HashSet<_>, _>>()
                .map_err(err)?;
            found
        } else {
            HashSet::new()
        };
        let terms = text_terms(watch);
        let mut scoped = BTreeMap::new();
        for (id, headline, explanation, portal, document, agency) in rows {
            let by_document = match (&portal, document, doc_keys) {
                (Some(portal), Some(document), Some(keys)) => {
                    keys.contains(&(portal.clone(), document))
                }
                _ => false,
            };
            let matched = match watch.kind {
                WatchKind::Source => {
                    portal.as_deref() == Some(watch.target.as_str()) || by_document
                }
                WatchKind::Agency => {
                    by_document
                        || matches!((&portal, agency), (Some(p), Some(a)) if agencies.contains(&(p.clone(), a)))
                }
                WatchKind::Entity | WatchKind::Query => {
                    let text = format!("{headline} {explanation}").to_lowercase();
                    by_document || terms.iter().all(|t| text.contains(t))
                }
                WatchKind::TopicPack => by_document,
            };
            // A topic watch takes its listed ids from the topic scan; every stored row is kept
            // here only so a baseline id that was dismissed can be recognized as still stored.
            if matched || watch.kind == WatchKind::TopicPack {
                scoped.insert(id, (headline, portal));
            }
        }
        scoped
    };

    if watch.kind == WatchKind::TopicPack {
        let topic = match &prefetch.topic {
            Some(scope) => scope,
            None => return Ok(Slot::not_measured("The topic scan did not run.")),
        };
        let listed = match &topic.findings {
            Ok(listed) => listed,
            Err(reason) => return Ok(Slot::not_measured(reason.clone())),
        };
        if let Err(reason) = documents {
            return Ok(Slot::not_measured(reason.clone()));
        }
        let mut found: Labels = listed
            .iter()
            .map(|(id, headline, portal)| {
                (
                    id.clone(),
                    Labeled {
                        label: headline.clone(),
                        portal_key: portal.clone(),
                    },
                )
            })
            .collect();
        if let Slot::Measured { value, .. } = &baseline.findings {
            for id in value.keys() {
                if !found.contains_key(id) {
                    if let Some((headline, portal)) = stored.get(id) {
                        found.insert(
                            id.clone(),
                            Labeled {
                                label: headline.clone(),
                                portal_key: portal.clone(),
                            },
                        );
                    }
                }
            }
        }
        return Ok(Slot::measured(found, now));
    }
    // Without the document scope an agency or text watch cannot tell a finding that left from one
    // it could no longer reach through a document, so the dimension is not measured.
    if matches!(
        watch.kind,
        WatchKind::Entity | WatchKind::Query | WatchKind::Agency
    ) {
        if let Err(reason) = documents {
            return Ok(Slot::not_measured(reason.clone()));
        }
    }
    Ok(Slot::measured(
        stored
            .into_iter()
            .map(|(id, (headline, portal))| {
                (
                    id,
                    Labeled {
                        label: headline,
                        portal_key: portal,
                    },
                )
            })
            .collect(),
        now,
    ))
}

fn observe_withheld(
    conn: &Connection,
    docs: &[ScopeDocument],
    now: &str,
) -> Result<Slot<Labels>, AppError> {
    if !table_exists(conn, "case_audits")? || !table_exists(conn, "document_integrity_flags")? {
        return Ok(Slot::not_measured(
            "No redaction audit table exists in this database.",
        ));
    }
    let requests: HashSet<(String, i64)> = docs
        .iter()
        .map(|d| (d.portal_key.clone(), d.request_id))
        .collect();
    let audited: HashSet<(String, i64)> = {
        let mut statement = conn
            .prepare("SELECT DISTINCT target_subdomain, target_request_id FROM case_audits")
            .map_err(err)?;
        let rows = statement
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
            .map_err(err)?
            .collect::<Result<HashSet<_>, _>>()
            .map_err(err)?;
        rows
    };
    if requests.is_disjoint(&audited) {
        return Ok(Slot::not_measured(
            "No redaction audit is recorded for any request in this watch's scope.",
        ));
    }
    let mut statement = conn
        .prepare("SELECT DISTINCT a.target_subdomain, a.target_request_id FROM case_audits a JOIN document_integrity_flags f ON f.audit_id=a.audit_id WHERE f.flag_type='REDACTION_HEAVY'")
        .map_err(err)?;
    let flagged = statement
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;
    drop(statement);
    let mut value = BTreeMap::new();
    for (portal, request) in flagged {
        if !requests.contains(&(portal.clone(), request)) {
            continue;
        }
        let title: Option<String> = conn
            .query_row(
                "SELECT title FROM requests WHERE portal_subdomain=?1 AND id=?2",
                params![portal, request],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)?
            .flatten();
        value.insert(
            format!("{portal}:request:{request}"),
            Labeled {
                label: format!(
                    "Request {request}: {}",
                    title.unwrap_or_else(|| "untitled".into())
                ),
                portal_key: Some(portal),
            },
        );
    }
    Ok(Slot::measured(value, now))
}

fn observe_figures(
    conn: &Connection,
    docs: &[ScopeDocument],
    prefetch: &Prefetch,
    now: &str,
) -> Result<Slot<BTreeMap<String, FigureSource>>, AppError> {
    if let Some(reason) = &prefetch.figures_unavailable {
        return Ok(Slot::not_measured(reason.clone()));
    }
    if !table_exists(conn, "monitor_heads")? {
        return Ok(Slot::not_measured(
            "Version monitoring has no captured versions yet.",
        ));
    }
    let keys: HashSet<(&str, i64)> = docs.iter().map(|d| (d.portal_key.as_str(), d.id)).collect();
    let mut statement = conn
        .prepare("SELECT h.source_url, h.observed_at, h.facts_zstd, d.portal_subdomain, d.id, COALESCE(d.file_name,'Document') FROM monitor_heads h JOIN documents d ON d.download_url=h.source_url ORDER BY h.source_url, d.portal_subdomain, d.id")
        .map_err(err)?;
    let rows = statement
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Vec<u8>>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, String>(5)?,
            ))
        })
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;
    let mut value = BTreeMap::new();
    for (url, observed_at, bytes, portal, id, title) in rows {
        if !keys.contains(&(portal.as_str(), id)) || value.contains_key(&url) {
            continue;
        }
        if value.len() >= MAX_FIGURE_SOURCES {
            return Ok(Slot::not_measured(format!(
                "More than {MAX_FIGURE_SOURCES} captured sources are in scope; narrow the watch."
            )));
        }
        let facts: Vec<MonitoringFact> = unpacked(&bytes)?;
        let facts = facts
            .into_iter()
            .filter(|f| matches!(f.category.as_str(), "MONEY" | "OUTCOME"))
            .take(MAX_FIGURES_PER_SOURCE)
            .collect();
        value.insert(
            url,
            FigureSource {
                portal_key: Some(portal),
                title,
                observed_at,
                facts,
            },
        );
    }
    if value.is_empty() {
        return Ok(Slot::not_measured(
            "No document in this watch's scope has a captured version to compare.",
        ));
    }
    Ok(Slot::measured(value, now))
}

fn observe_sources(
    conn: &Connection,
    watch: &Watch,
    docs: &[ScopeDocument],
    now: &str,
) -> Result<Slot<BTreeMap<String, SourceHealth>>, AppError> {
    if !table_exists(conn, "portal_sync_runs")? {
        return Ok(Slot::not_measured(
            "No sync history exists in this database.",
        ));
    }
    let mut portals: BTreeSet<String> = docs.iter().map(|d| d.portal_key.clone()).collect();
    if watch.kind == WatchKind::Source {
        portals.insert(watch.target.clone());
    }
    if portals.len() > MAX_SCOPE_PORTALS {
        return Ok(Slot::not_measured(
            "More sources are in scope than a watch can follow.",
        ));
    }
    let mut value = BTreeMap::new();
    for portal in portals {
        let run: Option<(String, String, i64, Option<String>)> = conn
            .query_row(
                "SELECT r.status, r.completed_at, r.failures, p.last_error FROM portal_sync_runs r JOIN portals p ON p.subdomain=r.portal_subdomain WHERE r.portal_subdomain=?1 AND r.completed_at IS NOT NULL AND r.status IN ('COMPLETED','COMPLETED_WITH_ERRORS') ORDER BY r.completed_at DESC LIMIT 1",
                params![portal],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()
            .map_err(err)?;
        if let Some((status, completed_at, failures, last_error)) = run {
            value.insert(
                portal,
                SourceHealth {
                    healthy: status == "COMPLETED",
                    detail: match last_error {
                        Some(error) if !error.is_empty() => format!("{failures} failures; {error}"),
                        _ => format!("{failures} failures"),
                    },
                    run_as_of: completed_at,
                },
            );
        }
    }
    if value.is_empty() {
        return Ok(Slot::not_measured(
            "No completed sync run is recorded for a source in this watch's scope.",
        ));
    }
    Ok(Slot::measured(value, now))
}

/// Watch baselines and changes hold labels and quotes derived from source documents, so a purge
/// removes what they recorded from the purged records. Entries are removed, not whole baselines:
/// a wiped baseline would report every remaining record as new on the next check, and the purged
/// records are absent from the next observation too, so neither is reported as a change.
///
/// A source purge (`document` is `None`) removes that source's entries from every baseline
/// dimension and its recorded changes. A document purge removes the document's entry from the
/// documents baseline and its source's entry from the figures baseline, with their changes.
pub(crate) fn purge_watchlist_references(
    conn: &Connection,
    portal: &str,
    document: Option<i64>,
) -> Result<(), AppError> {
    if !table_exists(conn, "watch_baselines")? {
        return Ok(());
    }
    let document_key = document.map(|id| format!("{portal}:{id}"));
    let urls: Vec<String> = match document {
        Some(id) => {
            let mut statement = conn
                .prepare("SELECT download_url FROM documents WHERE portal_subdomain=?1 AND id=?2 AND download_url IS NOT NULL")
                .map_err(err)?;
            let rows = statement
                .query_map(params![portal, id], |r| r.get::<_, String>(0))
                .map_err(err)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(err)?;
            rows
        }
        None => vec![],
    };
    let mut statement = conn
        .prepare("SELECT watch_id, dimension, payload_zstd FROM watch_baselines")
        .map_err(err)?;
    let rows = statement
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Vec<u8>>(2)?,
            ))
        })
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;
    drop(statement);
    for (watch_id, dimension, bytes) in rows {
        let Some(dimension) = WatchDimension::parse(&dimension) else {
            continue;
        };
        let mut slot: serde_json::Value = unpacked(&bytes)?;
        let Some(entries) = slot.get_mut("value").and_then(|v| v.as_object_mut()) else {
            continue;
        };
        let before = entries.len();
        entries.retain(|key, entry| {
            let from_portal = entry.get("portalKey").and_then(|p| p.as_str()) == Some(portal);
            !match (document, dimension) {
                (None, WatchDimension::SourceHealth) => key == portal,
                (None, _) => from_portal,
                (Some(_), WatchDimension::Documents) => Some(key) == document_key.as_ref(),
                (Some(_), WatchDimension::Figures) => urls.contains(key),
                (Some(_), _) => false,
            }
        });
        if entries.len() != before {
            conn.execute(
                "UPDATE watch_baselines SET payload_zstd=?3 WHERE watch_id=?1 AND dimension=?2",
                params![watch_id, dimension.as_str(), packed(&slot)?],
            )
            .map_err(err)?;
        }
    }
    match document {
        None => {
            conn.execute(
                "DELETE FROM watch_changes WHERE portal_key=?1",
                params![portal],
            )
            .map_err(err)?;
        }
        Some(_) => {
            let mut statement = conn
                .prepare("SELECT id, payload FROM watch_changes WHERE portal_key=?1")
                .map_err(err)?;
            let rows = statement
                .query_map(params![portal], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })
                .map_err(err)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(err)?;
            drop(statement);
            for (id, payload) in rows {
                let change: WatchChange = serde_json::from_str(&payload).map_err(err)?;
                let from_document = Some(&change.subject) == document_key.as_ref()
                    || urls
                        .iter()
                        .any(|url| change.subject.starts_with(&format!("{url}#")));
                if from_document {
                    conn.execute("DELETE FROM watch_changes WHERE id=?1", params![id])
                        .map_err(err)?;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
