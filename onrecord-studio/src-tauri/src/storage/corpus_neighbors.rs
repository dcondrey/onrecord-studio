//! Cursor-paged entity neighbourhoods, with no ceiling on how many neighbours are reachable.
//!
//! `entity_neighborhood` answers the strongest N for a drawing. A hub entity has more neighbours
//! than a drawing can hold, and cutting the list silently is a false statement about the corpus,
//! so this pages the whole set in the same order (most shared documents first, then display value,
//! then entity id, which makes the order total and the cursor exact).
//!
//! A child of `corpus` so it reuses that module's SQL fragments and row readers instead of
//! restating them.

use super::{
    entity_kind_from_str, read_entity_node, relationship_basis_from_str,
    relationship_kind_from_str, CorpusIndex, GRADE_RANK_SQL, TYPED_FIRST_SQL,
};
use crate::error::AppError;
use crate::types::{CorpusEntityEdge, CorpusEntityKind, CorpusEntityNode, RelationshipBasis};
use chrono::NaiveDate;
use duckdb::{params, Connection, OptionalExt};
use serde::{Deserialize, Serialize};

/// Bounds the rows one request holds the index's single connection for. Not a result limit.
pub const MAXIMUM_NEIGHBOR_PAGE_SIZE: usize = 1_000;

const CURSOR_VERSION: &str = "v1";
const CURSOR_SEPARATOR: char = '\u{1f}';

/// The same basis and as-of rules the graph view applies, evaluated where every neighbour is, so
/// a filtered listing is complete rather than limited to the rows a client happened to load.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EntityNeighborFilter {
    pub bases: Vec<RelationshipBasis>,
    /// `YYYY-MM-DD`.
    pub as_of: Option<String>,
    /// An edge with no recorded dates is unknown at any date, not valid at every date.
    pub include_undated: bool,
    /// Restricts the listing to one kind of neighbour.
    pub kind: Option<CorpusEntityKind>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EntityNeighborPage {
    pub center: CorpusEntityNode,
    pub edges: Vec<CorpusEntityEdge>,
    /// Present exactly when more neighbours follow. Opaque: pass it back unchanged.
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EntityNeighborKindCount {
    pub kind: CorpusEntityKind,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EntityNeighborCounts {
    /// Neighbours matching the filter.
    pub total: u64,
    /// Neighbours before the filter, so "hidden by the filter" is a number and not a guess.
    pub unfiltered_total: u64,
    pub by_kind: Vec<EntityNeighborKindCount>,
}

struct Cursor {
    shared: i64,
    entity_id: String,
    display_value: String,
}

fn encode_cursor(shared: i64, entity_id: &str, display_value: &str) -> String {
    format!(
        "{CURSOR_VERSION}{CURSOR_SEPARATOR}{shared}{CURSOR_SEPARATOR}{entity_id}{CURSOR_SEPARATOR}{display_value}"
    )
}

fn decode_cursor(value: &str) -> Result<Cursor, AppError> {
    let invalid = || AppError::DataParsing("The page cursor is not valid".to_string());
    if value.len() > 2_048 {
        return Err(invalid());
    }
    // The display value is last and may itself contain the separator.
    let mut parts = value.splitn(4, CURSOR_SEPARATOR);
    let (Some(version), Some(shared), Some(entity_id), Some(display_value)) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(invalid());
    };
    if version != CURSOR_VERSION {
        return Err(invalid());
    }
    Ok(Cursor {
        shared: shared.parse().map_err(|_| invalid())?,
        entity_id: entity_id.to_string(),
        display_value: display_value.to_string(),
    })
}

/// The edges of one entity collapsed to one row per neighbour, restricted by `filter`, joined to
/// the neighbour node. `?1` is the centre. Every number a view shows comes from this one text.
fn neighbors_cte(filter: Option<&EntityNeighborFilter>) -> Result<String, AppError> {
    let mut conditions = vec!["TRUE".to_string()];
    if let Some(filter) = filter {
        if filter.bases.is_empty() {
            conditions.push("FALSE".to_string());
        } else {
            let bases = filter
                .bases
                .iter()
                .map(|basis| format!("'{}'", basis.as_str()))
                .collect::<Vec<_>>()
                .join(", ");
            conditions.push(format!("agg.basis IN ({bases})"));
        }
        if let Some(kind) = filter.kind {
            conditions.push(format!("neighbor.entity_kind = '{}'", kind.as_str()));
        }
        if let Some(as_of) = &filter.as_of {
            let day = NaiveDate::parse_from_str(as_of, "%Y-%m-%d").map_err(|_| {
                AppError::DataParsing("The as-of date must be YYYY-MM-DD".to_string())
            })?;
            let day = day.format("%Y-%m-%d");
            let undated = if filter.include_undated {
                "TRUE"
            } else {
                "FALSE"
            };
            conditions.push(format!(
                "CASE WHEN TRY_CAST(agg.valid_from AS TIMESTAMP) IS NULL
                          AND TRY_CAST(agg.valid_until AS TIMESTAMP) IS NULL THEN {undated}
                      ELSE (TRY_CAST(agg.valid_from AS TIMESTAMP) IS NULL
                            OR TRY_CAST(agg.valid_from AS TIMESTAMP)
                               < CAST('{day}' AS TIMESTAMP) + INTERVAL 1 DAY)
                       AND (TRY_CAST(agg.valid_until AS TIMESTAMP) IS NULL
                            OR TRY_CAST(agg.valid_until AS TIMESTAMP)
                               >= CAST('{day}' AS TIMESTAMP))
                 END"
            ));
        }
    }
    // The edge table stores each pair once, ordered by identifier, so a neighbour may sit on
    // either side; `CASE` picks the other end.
    Ok(format!(
        "WITH pair AS (
            SELECT CASE WHEN edge.left_entity_id = ?1 THEN edge.right_entity_id
                        ELSE edge.left_entity_id END AS neighbor_id,
                   edge.portal_key || '|' || edge.document_id AS document_key,
                   edge.confidence_basis_points AS confidence, edge.basis AS basis,
                   {typed_first} AS relationship_kind,
                   edge.valid_from AS valid_from, edge.valid_until AS valid_until
            FROM corpus_entity_relationships edge
            WHERE edge.left_entity_id = ?1 OR edge.right_entity_id = ?1
         ), agg AS (
            SELECT neighbor_id, COUNT(DISTINCT document_key) AS shared,
                   MIN(confidence) AS confidence, MIN(basis) AS basis,
                   MIN(relationship_kind) AS relationship_kind,
                   MIN(valid_from) AS valid_from, MAX(valid_until) AS valid_until
            FROM pair GROUP BY neighbor_id
         ), kept AS (
            SELECT agg.*, neighbor.entity_kind, neighbor.display_value,
                   counts.document_count
            FROM agg
            JOIN corpus_entities neighbor ON neighbor.entity_id = agg.neighbor_id
            JOIN corpus_entity_document_counts counts ON counts.entity_id = agg.neighbor_id
            WHERE EXISTS (SELECT 1 FROM corpus_entity_mentions mention
                          WHERE mention.entity_id = agg.neighbor_id)
              AND {conditions}
         )",
        typed_first = TYPED_FIRST_SQL,
        conditions = conditions.join(" AND ")
    ))
}

fn load_center(
    connection: &Connection,
    entity_id: &str,
) -> Result<Option<CorpusEntityNode>, AppError> {
    connection
        .query_row(
            "SELECT entity.entity_id, entity.entity_kind, entity.display_value,
                    counts.document_count, MAX(GRADE_RANK)
             FROM corpus_entities entity
             JOIN corpus_entity_document_counts counts ON counts.entity_id = entity.entity_id
             JOIN corpus_entity_mentions mention ON mention.entity_id = entity.entity_id
             WHERE entity.entity_id = ?1
             GROUP BY entity.entity_id, entity.entity_kind, entity.display_value,
                      counts.document_count"
                .replace("GRADE_RANK", GRADE_RANK_SQL)
                .as_str(),
            [entity_id],
            read_entity_node,
        )
        .optional()
        .map_err(|error| AppError::Storage(format!("Failed to load the graph centre: {error}")))
}

fn count_neighbors(
    connection: &Connection,
    entity_id: &str,
    filter: Option<&EntityNeighborFilter>,
) -> Result<Vec<(String, i64)>, AppError> {
    let sql = format!(
        "{} SELECT entity_kind, COUNT(*) FROM kept GROUP BY entity_kind ORDER BY entity_kind",
        neighbors_cte(filter)?
    );
    let mut statement = connection.prepare(&sql).map_err(|error| {
        AppError::Storage(format!("Failed to prepare the neighbour count: {error}"))
    })?;
    let rows = statement
        .query_map([entity_id], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|error| AppError::Storage(format!("Failed to count neighbours: {error}")))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| AppError::Storage(format!("Failed to read neighbour counts: {error}")))?;
    Ok(rows)
}

impl CorpusIndex {
    /// One page of an entity's neighbours after `cursor`. `None` when the id names no entity.
    pub async fn entity_neighbors_page(
        &self,
        entity_id: String,
        cursor: Option<String>,
        page_size: usize,
        filter: Option<EntityNeighborFilter>,
    ) -> Result<Option<EntityNeighborPage>, AppError> {
        if !(1..=MAXIMUM_NEIGHBOR_PAGE_SIZE).contains(&page_size) {
            return Err(AppError::DataParsing(format!(
                "A neighbour page must hold between 1 and {MAXIMUM_NEIGHBOR_PAGE_SIZE} rows"
            )));
        }
        let cursor = cursor.as_deref().map(decode_cursor).transpose()?;
        let cte = neighbors_cte(filter.as_ref())?;
        let fetch = i64::try_from(page_size + 1)
            .map_err(|_| AppError::DataParsing("Page size exceeds the supported range".into()))?;
        self.read("Corpus entity neighbour page", move |connection| {
            let Some(center) = load_center(connection, &entity_id)? else {
                return Ok(None);
            };
            // The grade is the weakest across a node's mentions; it is read only for the rows
            // that made the page, never for every neighbour of a hub.
            let sql = format!(
                "{cte},
                 page AS (
                    SELECT kept.* FROM kept
                    WHERE ?2 IS NULL
                       OR kept.shared < ?2
                       OR (kept.shared = ?2
                           AND (kept.display_value > ?3
                                OR (kept.display_value = ?3 AND kept.neighbor_id > ?4)))
                    ORDER BY kept.shared DESC, kept.display_value ASC, kept.neighbor_id ASC
                    LIMIT ?5
                 ), grades AS (
                    SELECT mention.entity_id, MAX(GRADE_RANK) AS grade_rank
                    FROM corpus_entity_mentions mention
                    WHERE mention.entity_id IN (SELECT neighbor_id FROM page)
                    GROUP BY mention.entity_id
                 )
                 SELECT page.neighbor_id, page.entity_kind, page.display_value,
                        page.document_count, grades.grade_rank,
                        page.shared, page.confidence, page.basis, page.relationship_kind,
                        page.valid_from, page.valid_until
                 FROM page JOIN grades ON grades.entity_id = page.neighbor_id
                 ORDER BY page.shared DESC, page.display_value ASC, page.neighbor_id ASC"
            )
            .replace("GRADE_RANK", GRADE_RANK_SQL);
            let mut statement = connection.prepare(&sql).map_err(|error| {
                AppError::Storage(format!("Failed to prepare the neighbour page: {error}"))
            })?;
            let mut edges = statement
                .query_map(
                    params![
                        entity_id,
                        cursor.as_ref().map(|cursor| cursor.shared),
                        cursor.as_ref().map(|cursor| cursor.display_value.clone()),
                        cursor.as_ref().map(|cursor| cursor.entity_id.clone()),
                        fetch
                    ],
                    |row| {
                        Ok(CorpusEntityEdge {
                            neighbor: read_entity_node(row)?,
                            shared_document_count: row.get::<_, i64>(5)?.max(0) as u64,
                            confidence_basis_points: row.get::<_, i64>(6)?.clamp(0, 10_000) as u16,
                            basis: relationship_basis_from_str(&row.get::<_, String>(7)?),
                            kind: relationship_kind_from_str(&row.get::<_, String>(8)?),
                            valid_from: row.get::<_, Option<String>>(9)?,
                            valid_until: row.get::<_, Option<String>>(10)?,
                        })
                    },
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to query the neighbour page: {error}"))
                })?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| {
                    AppError::Storage(format!("Failed to read the neighbour page: {error}"))
                })?;
            let has_more = edges.len() > page_size;
            edges.truncate(page_size);
            let next_cursor = if has_more {
                edges.last().map(|edge| {
                    encode_cursor(
                        i64::try_from(edge.shared_document_count).unwrap_or(i64::MAX),
                        &edge.neighbor.entity_id,
                        &edge.neighbor.display_value,
                    )
                })
            } else {
                None
            };
            Ok(Some(EntityNeighborPage {
                center,
                edges,
                next_cursor,
            }))
        })
        .await
    }

    /// Exact neighbour totals, per kind and overall, with and without the filter. `None` when the
    /// id names no entity.
    pub async fn entity_neighbor_counts(
        &self,
        entity_id: String,
        filter: Option<EntityNeighborFilter>,
    ) -> Result<Option<EntityNeighborCounts>, AppError> {
        self.read("Corpus entity neighbour count", move |connection| {
            if load_center(connection, &entity_id)?.is_none() {
                return Ok(None);
            }
            let kept = count_neighbors(connection, &entity_id, filter.as_ref())?;
            let unfiltered_total = if filter.is_some() {
                count_neighbors(connection, &entity_id, None)?
                    .iter()
                    .map(|(_, count)| *count)
                    .sum::<i64>()
            } else {
                kept.iter().map(|(_, count)| *count).sum::<i64>()
            };
            let by_kind = kept
                .into_iter()
                .filter_map(|(kind, count)| {
                    entity_kind_from_str(&kind).map(|kind| EntityNeighborKindCount {
                        kind,
                        count: u64::try_from(count).unwrap_or(0),
                    })
                })
                .collect::<Vec<_>>();
            Ok(Some(EntityNeighborCounts {
                total: by_kind.iter().map(|entry| entry.count).sum(),
                unfiltered_total: u64::try_from(unfiltered_total).unwrap_or(0),
                by_kind,
            }))
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::time::Instant;

    const CENTRE: &str = "a_center";

    /// One hub joined to `neighbors` entities. Neighbour `i` shares `1 + i % 5` documents with the
    /// hub, so the strength order has large ties that only the display value and id break; every
    /// third edge is OBSERVED, every fourth carries a valid-from date, and display values repeat.
    async fn hub(neighbors: usize) -> (tempfile::TempDir, CorpusIndex) {
        let dir = tempfile::tempdir().expect("tempdir");
        let index = CorpusIndex::open(dir.path().join("corpus.duckdb")).expect("index opens");
        index
            .write("seed hub", move |conn| {
                conn.execute_batch(&format!(
                    "INSERT INTO corpus_documents (portal_key, request_id, document_id,
                        raw_file_sha256, extracted_text_sha256, character_count, simhash_hex,
                        relationships_truncated, algorithm_version, indexed_at)
                     SELECT 'p', 1, d, repeat('a', 64), repeat('b', 64), 10, repeat('0', 16),
                            false, 'v', 'now'
                     FROM range(1, 6) t(d);
                     INSERT INTO corpus_entities VALUES
                        ('{CENTRE}', 'ORGANIZATION', 'centre', 'Centre', 'EXACT_NORMALIZED_MATCH');
                     INSERT INTO corpus_entities
                     SELECT 'n' || lpad(CAST(i AS VARCHAR), 7, '0'),
                            CASE WHEN i % 2 = 0 THEN 'ORGANIZATION' ELSE 'PERSON' END,
                            'v' || i, 'Name ' || (i % 50), 'EXACT_NORMALIZED_MATCH'
                     FROM range(0, {neighbors}) t(i);
                     INSERT INTO corpus_entity_document_counts
                     SELECT entity_id, 1 FROM corpus_entities;
                     INSERT INTO corpus_entity_mentions
                     SELECT 'p', 1, entity_id, 0, 0, 1, 100, 'test', 1,
                            CASE row_number() OVER (ORDER BY entity_id) % 3
                                WHEN 0 THEN 'CHECK_DIGIT_PASSED'
                                WHEN 1 THEN 'PATTERN_ONLY' ELSE 'GAZETTEER_HIT' END
                     FROM corpus_entities;
                     INSERT INTO corpus_entity_relationships
                        (portal_key, document_id, left_entity_id, right_entity_id,
                         relationship_kind, basis, segment_index, character_start, character_end,
                         confidence_basis_points, valid_from, valid_until)
                     SELECT 'p', d, '{CENTRE}', 'n' || lpad(CAST(i AS VARCHAR), 7, '0'),
                            'CO_OCCURRENCE',
                            CASE WHEN i % 3 = 0 THEN 'OBSERVED' ELSE 'COMPUTED' END,
                            d, 0, 1, 5000,
                            CASE WHEN i % 4 = 0 THEN '2024-01-15T00:00:00Z' END, NULL
                     FROM range(0, {neighbors}) a(i), range(1, 6) b(d)
                     WHERE d <= 1 + i % 5;"
                ))
                .map_err(|error| AppError::Storage(error.to_string()))
            })
            .await
            .expect("seed");
        (dir, index)
    }

    async fn walk(
        index: &CorpusIndex,
        page_size: usize,
        filter: Option<EntityNeighborFilter>,
    ) -> Vec<CorpusEntityEdge> {
        let mut edges = Vec::new();
        let mut cursor = None;
        loop {
            let page = index
                .entity_neighbors_page(CENTRE.into(), cursor, page_size, filter.clone())
                .await
                .expect("page")
                .expect("centre exists");
            assert!(page.edges.len() <= page_size);
            edges.extend(page.edges);
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => return edges,
            }
        }
    }

    #[tokio::test]
    async fn pages_reach_every_neighbour_once_in_the_order_the_drawing_uses() {
        let (_dir, index) = hub(150).await;
        let edges = walk(&index, 7, None).await;
        let reference = index
            .entity_neighborhood(CENTRE.into(), 200)
            .await
            .unwrap()
            .unwrap();
        assert!(!reference.truncated);
        // The drawing query leaves ties between equal names unordered; the pages break them by id.
        let mut expected = reference.edges;
        expected.sort_by(|a, b| {
            b.shared_document_count
                .cmp(&a.shared_document_count)
                .then_with(|| a.neighbor.display_value.cmp(&b.neighbor.display_value))
                .then_with(|| a.neighbor.entity_id.cmp(&b.neighbor.entity_id))
        });
        assert_eq!(
            edges, expected,
            "same neighbours and values as the drawing query, in total order"
        );
        let counts = index
            .entity_neighbor_counts(CENTRE.into(), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(counts.total, 150);
        assert_eq!(counts.unfiltered_total, 150);
        assert_eq!(
            counts.by_kind.iter().map(|entry| entry.count).sum::<u64>(),
            150
        );
        assert_eq!(
            edges
                .iter()
                .map(|edge| &edge.neighbor.entity_id)
                .collect::<HashSet<_>>()
                .len(),
            150
        );
    }

    #[tokio::test]
    async fn a_filter_selects_the_same_neighbours_in_the_page_and_the_count() {
        let (_dir, index) = hub(150).await;
        let observed = |as_of: Option<&str>, include_undated: bool| EntityNeighborFilter {
            bases: vec![RelationshipBasis::Observed],
            as_of: as_of.map(str::to_string),
            include_undated,
            kind: None,
        };
        // Observed edges are i % 3 == 0: 50 of 150. Dated ones are also i % 4 == 0: 13 of them.
        for (filter, expected) in [
            (observed(None, true), 50),
            (observed(Some("2024-02-01"), true), 50),
            (observed(Some("2024-02-01"), false), 13),
            (observed(Some("2024-01-10"), false), 0),
            (observed(Some("2024-01-10"), true), 37),
        ] {
            let edges = walk(&index, 9, Some(filter.clone())).await;
            let counts = index
                .entity_neighbor_counts(CENTRE.into(), Some(filter.clone()))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(edges.len(), expected, "{filter:?}");
            assert_eq!(counts.total, expected as u64, "{filter:?}");
            assert_eq!(counts.unfiltered_total, 150);
        }
        let people = EntityNeighborFilter {
            bases: vec![
                RelationshipBasis::Observed,
                RelationshipBasis::Computed,
                RelationshipBasis::Asserted,
            ],
            as_of: None,
            include_undated: true,
            kind: Some(CorpusEntityKind::Person),
        };
        let listed = walk(&index, 9, Some(people.clone())).await;
        assert_eq!(listed.len(), 75, "odd-numbered neighbours are people");
        assert!(listed
            .iter()
            .all(|edge| edge.neighbor.kind == CorpusEntityKind::Person));
        let counts = index
            .entity_neighbor_counts(CENTRE.into(), Some(people))
            .await
            .unwrap()
            .unwrap();
        assert_eq!((counts.total, counts.unfiltered_total), (75, 150));
        let none_shown = EntityNeighborFilter {
            bases: vec![],
            as_of: None,
            include_undated: true,
            kind: None,
        };
        assert!(walk(&index, 9, Some(none_shown)).await.is_empty());
    }

    #[tokio::test]
    async fn unknown_entities_malformed_cursors_and_page_sizes_are_distinct_answers() {
        let (_dir, index) = hub(5).await;
        assert!(index
            .entity_neighbors_page("missing".into(), None, 10, None)
            .await
            .unwrap()
            .is_none());
        assert!(index
            .entity_neighbor_counts("missing".into(), None)
            .await
            .unwrap()
            .is_none());
        assert!(index
            .entity_neighbors_page(CENTRE.into(), Some("bad".into()), 10, None)
            .await
            .is_err());
        for size in [0, MAXIMUM_NEIGHBOR_PAGE_SIZE + 1] {
            assert!(index
                .entity_neighbors_page(CENTRE.into(), None, size, None)
                .await
                .is_err());
        }
    }

    /// Measurement, not a gate: `cargo test --lib scale_proof -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore = "seeds a 20,000-neighbour hub; run explicitly to measure"]
    async fn scale_proof_twenty_thousand_neighbours() {
        const NEIGHBORS: usize = 20_000;
        let started = Instant::now();
        let (_dir, index) = hub(NEIGHBORS).await;
        eprintln!("seed hub of {NEIGHBORS}: {:?}", started.elapsed());
        let started = Instant::now();
        let counts = index
            .entity_neighbor_counts(CENTRE.into(), None)
            .await
            .unwrap()
            .unwrap();
        let count_time = started.elapsed();
        let started = Instant::now();
        let first = index
            .entity_neighbors_page(CENTRE.into(), None, 200, None)
            .await
            .unwrap()
            .unwrap();
        let first_time = started.elapsed();
        let started = Instant::now();
        let second = index
            .entity_neighbors_page(CENTRE.into(), first.next_cursor.clone(), 200, None)
            .await
            .unwrap()
            .unwrap();
        let second_time = started.elapsed();
        let started = Instant::now();
        let all = walk(&index, 200, None).await;
        eprintln!(
            "neighbours {} count {count_time:?} first page {first_time:?} second page \
             {second_time:?} full walk {:?}",
            counts.total,
            started.elapsed()
        );
        assert_eq!(second.edges.len(), 200);
        assert_eq!(all.len() as u64, counts.total);
        assert_eq!(
            all.iter()
                .map(|edge| &edge.neighbor.entity_id)
                .collect::<HashSet<_>>()
                .len(),
            all.len()
        );
        assert!(count_time.as_secs() < 10 && first_time.as_secs() < 10);
    }
}
