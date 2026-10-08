//! Ranked, literal-source retrieval. Search syntax is never treated as source text.
use crate::{
    error::AppError,
    integrity::sha256_hex,
    storage::{pii::mask_pii, sqlite::DuckDbRepository},
    types::ResearchEvidence,
};
use duckdb::{params, params_from_iter, OptionalExt};
use regex::{Regex, RegexBuilder};

fn err(error: impl std::fmt::Display) -> AppError {
    AppError::Storage(error.to_string())
}

struct SearchQuery {
    terms: Vec<String>,
    phrases: Vec<String>,
}
impl SearchQuery {
    fn parse(raw: &str) -> Option<Self> {
        if raw.len() > 320 {
            return None;
        }
        let raw = raw.trim().replace(['“', '”'], "\"");
        let quoted = Regex::new(r#""([^"]+)""#).expect("quoted search syntax");
        let mut phrases = quoted
            .captures_iter(&raw)
            .map(|c| {
                c[1].split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .to_lowercase()
            })
            .filter(|p| p.len() >= 3)
            .take(4)
            .collect::<Vec<_>>();
        let mut terms = Vec::new();
        for word in raw.split(|c: char| !c.is_alphanumeric()) {
            let word = word.to_lowercase();
            if word.len() < 3
                || [
                    "the", "and", "for", "with", "from", "that", "this", "what", "which", "where",
                    "when", "are", "was", "were", "has", "have", "how", "does", "did", "can",
                    "into", "than", "provide", "find", "records", "report", "reports",
                ]
                .contains(&word.as_str())
            {
                continue;
            }
            if !terms.contains(&word) {
                terms.push(word);
            }
            if terms.len() == 12 {
                break;
            }
        }
        if terms.is_empty() {
            return None;
        }
        if phrases.is_empty() && terms.len() > 1 {
            phrases.push(terms.join(" "));
        }
        Some(Self { terms, phrases })
    }
    fn term_patterns(&self) -> Vec<String> {
        self.terms
            .iter()
            .map(|t| format!(r"\b{}\b", regex::escape(t)))
            .collect()
    }
    fn phrase_pattern(&self) -> String {
        if self.phrases.is_empty() {
            return self.term_patterns().join("|");
        }
        self.phrases
            .iter()
            .map(|p| {
                p.split_whitespace()
                    .map(regex::escape)
                    .collect::<Vec<_>>()
                    .join(r"\s+")
            })
            .collect::<Vec<_>>()
            .join("|")
    }
}

fn pattern(value: &str) -> Regex {
    RegexBuilder::new(value)
        .case_insensitive(true)
        .build()
        .expect("escaped search expression")
}

impl DuckDbRepository {
    pub async fn research_search(&self, raw: &str) -> Result<Vec<ResearchEvidence>, AppError> {
        let Some(query) = SearchQuery::parse(raw) else {
            return Ok(vec![]);
        };
        self.query_connection("Retrieve ranked investigative passages", move |conn| {
            let _deadline = crate::analysis::sql::QueryDeadline::start(conn, std::time::Duration::from_secs(15))?;
            let term_patterns = query.term_patterns();
            let mut parameters = vec![term_patterns.join("|"), query.phrase_pattern()];
            parameters.extend(term_patterns.clone());
            let scores = (0..term_patterns.len()).map(|i| format!("CASE WHEN regexp_matches(f.extracted_text, ?{}, 'i') THEN 1 ELSE 0 END", i+3)).collect::<Vec<_>>().join(" + ");
            let minimum = query.terms.len().clamp(1, 3);
            // Keep exact matches, but also find terms separated by headings, PDF line wraps or
            // other words.
            let sql = format!("WITH candidates AS (
              SELECT d.portal_subdomain,d.request_id,d.id,COALESCE(d.file_name,'Document') AS label,
                d.download_url,COALESCE(d.raw_file_sha256,'') AS digest,f.extracted_text,p.base_url,d.updated_at,
                ({scores}) AS term_matches,regexp_matches(f.extracted_text,?2,'i') AS phrase_match,
                GREATEST(0,strpos(f.extracted_text,regexp_extract(f.extracted_text,?1,0,'i'))-501) AS start_at
              FROM documents d JOIN documents_fts f ON f.document_id=d.id AND f.portal_subdomain=d.portal_subdomain
              JOIN portals p ON p.subdomain=d.portal_subdomain
              WHERE lower(trim(d.visibility))='public' AND p.lifecycle_state<>'REMOVED'
                AND d.extraction_status IN ('SEARCHABLE','SALVAGED') AND regexp_matches(f.extracted_text,?1,'i')
            ) SELECT portal_subdomain,request_id,id,label,download_url,digest,start_at,
                substr(extracted_text,start_at+1,200000),length(extracted_text),base_url
              FROM candidates WHERE term_matches>={minimum} OR phrase_match
              ORDER BY phrase_match DESC,term_matches DESC,updated_at DESC,portal_subdomain,id LIMIT 24");
            let mut statement = conn.prepare(&sql).map_err(err)?;
            let rows = statement.query_map(params_from_iter(parameters.iter()), |r| Ok((r.get::<_,String>(0)?,r.get::<_,u64>(1)?,r.get::<_,u64>(2)?,r.get::<_,String>(3)?,r.get::<_,Option<String>>(4)?,r.get::<_,String>(5)?,r.get::<_,usize>(6)?,r.get::<_,String>(7)?,r.get::<_,usize>(8)?,r.get::<_,String>(9)?))).map_err(err)?.collect::<Result<Vec<_>,_>>().map_err(err)?;
            let mut candidates = Vec::new();
            for (portal,request,id,label,url,digest,start,text,total,base) in rows {
                for (score,relative,passage) in passages(&text,&query) {
                    let text = mask_pii(&passage)?.masked_text;
                    let offset = start+relative;
                    candidates.push((score,ResearchEvidence { id:format!("doc:{portal}:{id}:{offset}:{}",&sha256_hex(text.as_bytes())[..10]),label:label.clone(),source_url:url.clone().or(Some(base.clone())),portal_key:Some(portal.clone()),request_id:Some(request),document_id:Some(id),character_offset:Some(offset),source_sha256:digest.clone(),text,sql:None,snapshot_provenance:vec![],source_references:vec![],limitations:format!("Exact retained passage at character {offset} of {total}. Ranked lexical retrieval across at most 24 documents and 200,000 characters per document; query terms need not be adjacent. Missing hits do not establish absence. Retrieval order is not publication chronology; text may omit layout/images.") }));
                }
            }
            candidates.sort_by(|a,b| b.0.cmp(&a.0).then(a.1.id.cmp(&b.1.id)));
            // Select one strong passage per original source before allowing a second passage.
            let mut selected = Vec::new();
            let mut sources = std::collections::HashSet::new();
            let mut deferred = Vec::new();
            for (_,item) in candidates {
                let key=if item.source_sha256.is_empty() {item.source_url.clone().unwrap_or_else(||item.id.clone())} else {item.source_sha256.clone()};
                if sources.insert(key) {selected.push(item);} else {deferred.push(item);}
                if selected.len()==6 {break;}
            }
            selected.extend(deferred.into_iter().take(8usize.saturating_sub(selected.len())));
            selected.truncate(8);
            Ok(selected)
        }).await
    }

    pub async fn research_read(
        &self,
        source: &ResearchEvidence,
        offset: usize,
    ) -> Result<Option<ResearchEvidence>, AppError> {
        let (Some(portal), Some(id)) = (&source.portal_key, source.document_id) else {
            return Ok(None);
        };
        if offset > 50_000_000 {
            return Ok(None);
        }
        let mut source = source.clone();
        let portal = portal.clone();
        self.query_connection("Read follow-up evidence",move|conn| {
            // DuckDB substr accepts BIGINT, not Rust usize's UBIGINT. Bind explicitly.
            let row:Option<(String,usize)>=conn.query_row("SELECT substr(f.extracted_text,?3+1,4800),length(f.extracted_text) FROM documents_fts f JOIN documents d ON d.id=f.document_id AND d.portal_subdomain=f.portal_subdomain JOIN portals p ON p.subdomain=d.portal_subdomain WHERE f.portal_subdomain=?1 AND f.document_id=?2 AND lower(trim(d.visibility))='public' AND p.lifecycle_state<>'REMOVED' AND d.extraction_status IN ('SEARCHABLE','SALVAGED') AND COALESCE(d.raw_file_sha256,'')=?4",params![portal,id,offset as i64,source.source_sha256],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(err)?;
            let Some((text,total))=row else{return Ok(None);};
            if text.is_empty(){return Ok(None);}
            source.text=mask_pii(&text)?.masked_text;
            source.id=format!("doc:{portal}:{id}:{offset}:{}",&sha256_hex(source.text.as_bytes())[..10]);
            source.character_offset=Some(offset);
            source.limitations=format!("Exact retained passage at character {offset} of {total}; layout and images may be absent.");
            Ok(Some(source))
        }).await
    }
}

fn passages(text: &str, query: &SearchQuery) -> Vec<(usize, usize, String)> {
    let terms = query
        .term_patterns()
        .iter()
        .map(|p| pattern(p))
        .collect::<Vec<_>>();
    let all = pattern(&query.term_patterns().join("|"));
    let phrase = pattern(&query.phrase_pattern());
    let chars = text.chars().collect::<Vec<_>>();
    let byte_offsets = text
        .char_indices()
        .map(|(offset, _)| offset)
        .collect::<Vec<_>>();
    let mut candidates = Vec::new();
    // Search across the full retained window. Keep later precise matches even when an
    // early section repeats a generic term hundreds of times.
    let mut offsets = std::collections::BTreeSet::new();
    for term in &terms {
        for hit in term.find_iter(text).take(200) {
            offsets.insert(
                byte_offsets
                    .partition_point(|offset| *offset < hit.start())
                    .saturating_sub(450),
            );
        }
    }
    for offset in offsets {
        let end = (offset + 2200).min(chars.len());
        let passage: String = chars[offset..end].iter().collect();
        let matched = terms.iter().filter(|t| t.is_match(&passage)).count();
        if matched < query.terms.len().clamp(1, 3) && !phrase.is_match(&passage) {
            continue;
        }
        let lower = passage.to_lowercase();
        let substance = [
            "$",
            "million",
            "expenditure",
            "contract",
            "outcome",
            "recommend",
            "placement",
            "closure",
            "amendment",
            "adopted",
        ]
        .iter()
        .filter(|t| lower.contains(**t))
        .count();
        let score = matched * 30 + usize::from(phrase.is_match(&passage)) * 80 + substance;
        if all.is_match(&passage) {
            candidates.push((score, offset, passage));
        }
    }
    candidates.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut selected: Vec<(usize, usize, String)> = Vec::new();
    for item in candidates {
        if selected
            .iter()
            .all(|prior| prior.1.abs_diff(item.1) >= 2200)
        {
            selected.push(item);
        }
        if selected.len() == 2 {
            break;
        }
    }
    selected
}

#[cfg(test)]
pub(crate) fn ranked_passages(text: &str, raw: &str) -> Vec<(usize, String)> {
    SearchQuery::parse(raw)
        .map(|q| {
            passages(text, &q)
                .into_iter()
                .map(|(_, o, p)| (o, p))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn fixture() -> (tempfile::TempDir, DuckDbRepository) {
        let temp = tempfile::tempdir().unwrap();
        let db = DuckDbRepository::new(temp.path().join("core.db")).unwrap();
        db.run_migrations(include_str!("../../migrations/0001_initial_schema.sql"))
            .await
            .unwrap();
        let conn = db.analytics_connection().await.unwrap();
        conn.execute("INSERT INTO portals(subdomain,display_name,base_url) VALUES ('city','City','https://www.sandiego.gov')",[]).unwrap();
        conn.execute("INSERT INTO requests(id,portal_subdomain,title,status,created_at,updated_at) VALUES (1,'city','Retrieval fixture','closed','2026-09-27','2026-09-27')",[]).unwrap();
        (temp, db)
    }
    async fn document(db: &DuckDbRepository, id: u64, text: &str, digest: &str) {
        let conn = db.analytics_connection().await.unwrap();
        conn.execute("INSERT INTO documents(id,request_id,portal_subdomain,file_name,download_url,visibility,created_at,updated_at,raw_file_sha256,extraction_status) VALUES (?1,1,'city','record.txt','https://www.sandiego.gov/record.txt','public','2026-09-27','2026-09-27',?2,'SEARCHABLE')",params![id,digest]).unwrap();
        conn.execute("INSERT INTO documents_fts(document_id,portal_subdomain,file_name,extracted_text) VALUES (?1,'city','record.txt',?2)",params![id,text]).unwrap();
    }
    #[tokio::test]
    async fn research_queries_find_distributed_terms_wrapped_quotes_and_original_offsets() {
        let (_temp, db) = fixture().await;
        let text=format!("{} Downtown public\nrestrooms operating contract: $2 million. Homelessness expenditures and closure dates need reconciliation.","é".repeat(1200));
        document(&db, 1, &text, &"a".repeat(64)).await;
        document(
            &db,
            2,
            "Homelessness headline without the specific facility or funding information.",
            &"b".repeat(64),
        )
        .await;
        document(&db, 3, &text, &"a".repeat(64)).await;
        let found = db
            .research_search("\"Downtown public restrooms\" homelessness")
            .await
            .unwrap();
        assert!(
            !found.is_empty(),
            "Quoted compound searches must reach an existing PDF-wrapped passage"
        );
        assert!(found.iter().all(|e| e.document_id != Some(2)));
        let e = &found[0];
        assert!(e.text.contains("$2 million"));
        assert_eq!(
            e.text,
            text.chars()
                .skip(e.character_offset.unwrap())
                .take(e.text.chars().count())
                .collect::<String>()
        );
        let distributed = db
            .research_search("restrooms expenditures closure")
            .await
            .unwrap();
        assert!(!distributed.is_empty());
    }
    #[tokio::test]
    async fn deeper_reads_use_signed_unicode_offsets_and_reject_changed_or_removed_sources() {
        let (_temp, db) = fixture().await;
        let text = format!(
            "Intro shelter {} Later contract payments were recorded separately.",
            "é".repeat(4000)
        );
        document(&db, 1, &text, &"a".repeat(64)).await;
        let source = db.research_search("shelter").await.unwrap().remove(0);
        let read = db.research_read(&source, 4014).await.unwrap().unwrap();
        assert_eq!(read.text, text.chars().skip(4014).collect::<String>());
        assert_eq!(read.character_offset, Some(4014));
        let conn = db.analytics_connection().await.unwrap();
        conn.execute(
            "UPDATE documents SET raw_file_sha256=repeat('b',64) WHERE id=1",
            [],
        )
        .unwrap();
        assert!(db.research_read(&source, 4014).await.unwrap().is_none());
        conn.execute(
            "UPDATE documents SET raw_file_sha256=repeat('a',64) WHERE id=1",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE portals SET lifecycle_state='REMOVED' WHERE subdomain='city'",
            [],
        )
        .unwrap();
        assert!(db.research_read(&source, 4014).await.unwrap().is_none());
    }
    #[tokio::test]
    async fn archived_failed_homelessness_searches_now_retrieve_real_retained_passages() {
        let (_temp, db) = fixture().await;
        let report: crate::types::ResearchReport = serde_json::from_str(include_str!(
            "../../../docs/validation/live-research-dossier.json"
        ))
        .unwrap();
        for (index, evidence) in report
            .evidence
            .iter()
            .filter(|e| e.sql.is_none())
            .enumerate()
        {
            document(
                &db,
                index as u64 + 1,
                &evidence.text,
                &evidence.source_sha256,
            )
            .await;
        }
        let mut recovered = Vec::new();
        for query in [
            "\"Riverbed Encampment Resolution Funding Project\"",
            "\"Kettner Blvd and Vine St\" shelter",
            "\"quarterly budget monitoring reports\" \"homelessness expenditures\"",
            "\"Downtown public restrooms\" homelessness",
            "\"HSSD-funded homelessness expenditures\" memorandum",
            "\"1,000-bed permanent emergency shelter\"",
            "\"26 percent exits to permanent housing\"",
        ] {
            let matches = db.research_search(query).await.unwrap();
            if !matches.is_empty() {
                recovered.push((query, matches.len()));
            }
        }
        assert!(recovered.len()>=5,"Previously zero-hit searches should find their actual archived source passages: {recovered:?}");
        println!("Recovered previously failed real-record searches: {recovered:?}");
    }
}
