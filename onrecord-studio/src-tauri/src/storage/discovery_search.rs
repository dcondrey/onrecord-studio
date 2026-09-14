//! Search extracted document text without installing or loading an embedding model.
use super::{fts::core_query_tokens, sqlite::DuckDbRepository};
use crate::{
    error::AppError,
    ingestion::regional::source_jurisdiction,
    types::{HybridSearchFilter, HybridSearchResult},
};
use duckdb::params;

impl DuckDbRepository {
    pub async fn search_document_passages(
        &self,
        query: &str,
        filter: HybridSearchFilter,
        limit: usize,
    ) -> Result<Vec<HybridSearchResult>, AppError> {
        let tokens = core_query_tokens(query).map_err(|e| AppError::DataParsing(e.into()))?;
        if tokens.is_empty() {
            return Ok(Vec::new());
        }
        let limit = limit.clamp(1, 100);
        self.query_connection("Document passage search", move |conn| {
            // Resolve publisher attribution before LIMIT, so a busy publisher cannot crowd
            // another jurisdiction out of the candidates. Bound text materialization in SQL.
            let jurisdiction_clause = if let Some(wanted) = &filter.jurisdiction {
                let mut sources = conn.prepare("SELECT subdomain, base_url FROM portals")
                    .map_err(|e| AppError::Storage(e.to_string()))?;
                let rows = sources.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
                    .map_err(|e| AppError::Storage(e.to_string()))?;
                let mut keys = Vec::new();
                for row in rows {
                    let (key, url) = row.map_err(|e| AppError::Storage(e.to_string()))?;
                    if source_jurisdiction(&url).eq_ignore_ascii_case(wanted) {
                        keys.push(format!("'{}'", key.replace('\'', "''")));
                    }
                }
                if keys.is_empty() { return Ok(Vec::new()); }
                format!("AND d.portal_subdomain IN ({})", keys.join(","))
            } else { String::new() };
            let terms = tokens.iter().map(|term| format!("'{}'", term.replace('\'', "''"))).collect::<Vec<_>>().join(",");
            let sql = format!("WITH matches AS (
                SELECT document_id, portal_subdomain, SUM(term_frequency) AS frequency
                FROM document_search_tokens WHERE token IN ({terms})
                GROUP BY document_id, portal_subdomain HAVING COUNT(DISTINCT token) = {}
            ) SELECT d.portal_subdomain, d.request_id, d.id, COALESCE(d.file_name, 'Document'),
                COALESCE(r.title, ''), COALESCE(a.name, 'Unmapped Department'), COALESCE(p.base_url, ''),
                d.content_type, CASE WHEN p.source_kind = 'NEXT_REQUEST' THEN d.created_at ELSE NULL END AS published,
                LEFT(f.extracted_text, 4000000), LENGTH(f.extracted_text)
            FROM matches m JOIN documents d ON d.id = m.document_id AND d.portal_subdomain = m.portal_subdomain
            JOIN documents_fts f ON f.document_id = d.id AND f.portal_subdomain = d.portal_subdomain
            JOIN requests r ON r.id = d.request_id AND r.portal_subdomain = d.portal_subdomain
            LEFT JOIN agencies a ON a.id = r.agency_id AND a.portal_subdomain = r.portal_subdomain
            LEFT JOIN portals p ON p.subdomain = d.portal_subdomain
            WHERE LOWER(TRIM(d.visibility)) = 'public'
              AND (?1 IS NULL OR d.portal_subdomain = ?1)
              AND (?2 IS NULL OR CONTAINS(LOWER(COALESCE(a.name, 'Unmapped Department')), LOWER(?2)))
              AND (?3 IS NULL OR LOWER(d.content_type) = LOWER(?3))
              AND (?4 IS NULL OR LEFT(published, 10) >= ?4)
              AND (?5 IS NULL OR LEFT(published, 10) <= ?5)
              {jurisdiction_clause}
            ORDER BY m.frequency DESC, d.portal_subdomain, d.id LIMIT {limit}", tokens.len());
            let mut statement = conn.prepare(&sql).map_err(|e| AppError::Storage(e.to_string()))?;
            let mut rows = statement.query(params![filter.portal_key, filter.agency_name, filter.document_type, filter.date_from, filter.date_to])
                .map_err(|e| AppError::Storage(e.to_string()))?;
            let mut results = Vec::new();
            while let Some(row) = rows.next().map_err(|e| AppError::Storage(e.to_string()))? {
                let decode = |index| row.get::<_, String>(index).map_err(|e| AppError::Storage(e.to_string()));
                let jurisdiction = source_jurisdiction(&decode(6)?).to_string();
                if filter.jurisdiction.as_ref().is_some_and(|wanted| !wanted.eq_ignore_ascii_case(&jurisdiction)) { continue; }
                let text = decode(9)?;
                let (start, end, excerpt) = best_passage(&text, &tokens);
                let portal_key = decode(0)?;
                let document_id = row.get::<_, u64>(2).map_err(|e| AppError::Storage(e.to_string()))?;
                let length = row.get::<_, usize>(10).map_err(|e| AppError::Storage(e.to_string()))?;
                results.push(HybridSearchResult {
                    stable_id: format!("text:{portal_key}:{document_id}:{start}"), portal_key,
                    request_id: row.get(1).map_err(|e| AppError::Storage(e.to_string()))?, document_id,
                    file_name: decode(3)?, title: decode(4)?, agency_name: decode(5)?, jurisdiction,
                    document_type: row.get(7).map_err(|e| AppError::Storage(e.to_string()))?,
                    publication_date: row.get(8).map_err(|e| AppError::Storage(e.to_string()))?, observation_date: None, effective_date: None,
                    character_start: start, character_end: end, excerpt,
                    lexical_rank: Some(results.len() + 1), semantic_score: None, fused_score: 0.0,
                    retrieval_status: if length > 4_000_000 { "DOCUMENT_TEXT_MATCH_INDEXED_PREFIX" } else { "DOCUMENT_TEXT_MATCH" }.into(),
                });
                if results.len() == limit { break; }
            }
            Ok(results)
        }).await
    }
}

// Choose the tightest span containing the greatest number of distinct query terms, then
// include surrounding context. Offsets are Unicode character offsets into preserved text.
fn best_passage(text: &str, terms: &[String]) -> (usize, usize, String) {
    let mut hits = Vec::new();
    let mut word = String::new();
    let mut word_start = 0;
    let mut total = 0;
    for (position, character) in text.chars().chain(std::iter::once(' ')).enumerate() {
        total = position;
        if character.is_alphanumeric() {
            if word.is_empty() {
                word_start = position;
            }
            word.push(character);
        } else if !word.is_empty() {
            let normalized = word.to_lowercase().chars().take(128).collect::<String>();
            if let Some(index) = terms.iter().position(|term| term == &normalized) {
                hits.push((word_start, position, index));
            }
            word.clear();
        }
    }
    let mut counts = vec![0_usize; terms.len()];
    let (mut left, mut distinct, mut best_count, mut best_start, mut best_span) =
        (0, 0, 0, 0, usize::MAX);
    for right in 0..hits.len() {
        counts[hits[right].2] += 1;
        if counts[hits[right].2] == 1 {
            distinct += 1;
        }
        while left < right && (hits[right].1 - hits[left].0 > 1200 || counts[hits[left].2] > 1) {
            counts[hits[left].2] -= 1;
            if counts[hits[left].2] == 0 {
                distinct -= 1;
            }
            left += 1;
        }
        let span = hits[right].1 - hits[left].0;
        if distinct > best_count || (distinct == best_count && span < best_span) {
            best_count = distinct;
            best_start = hits[left].0;
            best_span = span;
        }
    }
    let start = best_start.saturating_sub(160);
    let end = (start + 1600).min(total);
    (
        start,
        end,
        text.chars().skip(start).take(end - start).collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn excerpt_finds_clustered_terms_and_preserves_unicode_offsets() {
        let text = format!(
            "shelter {}é Housing café shelter capacity $12.50 approved.",
            "漢字 ".repeat(1000)
        );
        let (start, end, excerpt) = best_passage(&text, &["shelter".into(), "capacity".into()]);
        assert!(start > 2000);
        assert!(excerpt.contains("shelter capacity $12.50"));
        assert_eq!(
            text.chars()
                .skip(start)
                .take(end - start)
                .collect::<String>(),
            excerpt
        );
    }
}
