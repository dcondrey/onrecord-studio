//! Read-only corpus queries behind entity resolution: the entities to compare, the validated
//! identifiers that link them, and a few example mentions for the evidence a reviewer sees.

use super::corpus::CorpusIndex;
use crate::error::AppError;
use crate::resolution::{IdentifierLink, MentionExample, ResolutionEntity, ResolutionKey};
use duckdb::{params_from_iter, Connection};
use std::collections::BTreeMap;

/// Resource guard on how many name-like entities one candidate scan reads. Past it the scan is
/// reported as incomplete rather than silently cut.
pub const MAXIMUM_SCANNED_ENTITIES: usize = 2_000_000;
/// Example mentions shown per entity. A sample for the reviewer; the entity's document count is
/// carried beside it so a sample is never mistaken for the whole.
pub const EXAMPLES_PER_ENTITY: usize = 3;

pub struct ResolutionScan {
    pub entities: Vec<ResolutionEntity>,
    pub links: Vec<IdentifierLink>,
    pub scan_truncated: bool,
}

fn storage(action: &str, error: duckdb::Error) -> AppError {
    AppError::Storage(format!("Failed to {action}: {error}"))
}

fn placeholders(count: usize) -> String {
    vec!["?"; count].join(", ")
}

fn read_entity(row: &duckdb::Row<'_>) -> duckdb::Result<ResolutionEntity> {
    Ok(ResolutionEntity {
        entity_id: row.get(0)?,
        key: ResolutionKey::new(row.get::<_, String>(1)?, row.get::<_, String>(2)?),
        display_value: row.get(3)?,
        document_count: u64::try_from(row.get::<_, i64>(4)?).unwrap_or(0),
    })
}

const ENTITY_SELECT: &str = "SELECT entity.entity_id, entity.entity_kind, entity.normalized_value,
        entity.display_value, COALESCE(counts.document_count, 0)
     FROM corpus_entities entity
     LEFT JOIN corpus_entity_document_counts counts ON counts.entity_id = entity.entity_id";

fn entities_where(
    connection: &Connection,
    condition: &str,
    values: &[String],
) -> Result<Vec<ResolutionEntity>, AppError> {
    let mut statement = connection
        .prepare(&format!("{ENTITY_SELECT} WHERE {condition}"))
        .map_err(|error| storage("prepare the entity lookup", error))?;
    let rows = statement
        .query_map(params_from_iter(values.iter()), read_entity)
        .map_err(|error| storage("read entities", error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| storage("decode entities", error))?;
    Ok(rows)
}

/// Validated identifiers co-occurring with name-like entities, optionally only for `only`.
fn identifier_links(
    connection: &Connection,
    only: Option<&[String]>,
) -> Result<Vec<IdentifierLink>, AppError> {
    let restriction = match only {
        Some(ids) => format!("AND named.entity_id IN ({})", placeholders(ids.len())),
        None => String::new(),
    };
    let mut statement = connection
        .prepare(&format!(
            "WITH strong AS (
                        SELECT mention.entity_id
                        FROM corpus_entity_mentions mention
                        JOIN corpus_entities entity ON entity.entity_id = mention.entity_id
                        WHERE entity.entity_kind = 'EMAIL_ADDRESS'
                        GROUP BY mention.entity_id
                        HAVING bool_and(mention.validation_grade
                                        IN ('CHECK_DIGIT_PASSED', 'FORMAT_VALIDATED'))
                     ), linked AS (
                        SELECT left_entity_id AS identifier_id, right_entity_id AS entity_id,
                               portal_key, document_id
                        FROM corpus_entity_relationships WHERE relationship_kind = 'CO_OCCURRENCE'
                        UNION ALL
                        SELECT right_entity_id, left_entity_id, portal_key, document_id
                        FROM corpus_entity_relationships WHERE relationship_kind = 'CO_OCCURRENCE'
                     )
                     SELECT linked.identifier_id, identifier.entity_kind, identifier.display_value,
                            linked.entity_id,
                            COUNT(DISTINCT linked.portal_key || ':' || linked.document_id)
                     FROM linked
                     JOIN strong ON strong.entity_id = linked.identifier_id
                     JOIN corpus_entities identifier ON identifier.entity_id = linked.identifier_id
                     JOIN corpus_entities named ON named.entity_id = linked.entity_id
                          AND named.entity_kind IN ('PERSON', 'ORGANIZATION') {restriction}
                     GROUP BY linked.identifier_id, identifier.entity_kind,
                              identifier.display_value, linked.entity_id"
        ))
        .map_err(|error| storage("prepare the identifier links", error))?;
    let values: &[String] = only.unwrap_or(&[]);
    let links = statement
        .query_map(params_from_iter(values.iter()), |row| {
            Ok(IdentifierLink {
                identifier_id: row.get(0)?,
                identifier_kind: row.get(1)?,
                identifier_display: row.get(2)?,
                entity_id: row.get(3)?,
                documents: u64::try_from(row.get::<_, i64>(4)?).unwrap_or(0),
            })
        })
        .map_err(|error| storage("read identifier links", error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| storage("decode identifier links", error))?;
    Ok(links)
}

impl CorpusIndex {
    /// Name-like entities and the validated identifiers that co-occur with them. An identifier is
    /// strong only when every mention of it passed format or check-digit validation.
    pub async fn resolution_scan(&self) -> Result<ResolutionScan, AppError> {
        self.read("Entity resolution scan", |connection| {
            let mut statement = connection
                .prepare(&format!(
                    "{ENTITY_SELECT} WHERE entity.entity_kind IN ('PERSON', 'ORGANIZATION')
                     ORDER BY entity.entity_id LIMIT {}",
                    MAXIMUM_SCANNED_ENTITIES + 1
                ))
                .map_err(|error| storage("prepare the entity scan", error))?;
            let mut entities = statement
                .query_map([], read_entity)
                .map_err(|error| storage("scan entities", error))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| storage("decode scanned entities", error))?;
            let scan_truncated = entities.len() > MAXIMUM_SCANNED_ENTITIES;
            entities.truncate(MAXIMUM_SCANNED_ENTITIES);

            let links = identifier_links(connection, None)?;
            Ok(ResolutionScan {
                entities,
                links,
                scan_truncated,
            })
        })
        .await
    }

    pub async fn resolution_entities_by_id(
        &self,
        ids: Vec<String>,
    ) -> Result<Vec<ResolutionEntity>, AppError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        self.read("Entity lookup", move |connection| {
            entities_where(
                connection,
                &format!("entity.entity_id IN ({})", placeholders(ids.len())),
                &ids,
            )
        })
        .await
    }

    /// Identifier links for just these entities, for the evidence of one pair.
    pub async fn resolution_links_for(
        &self,
        ids: Vec<String>,
    ) -> Result<Vec<IdentifierLink>, AppError> {
        self.read("Identifier lookup", move |connection| {
            identifier_links(connection, Some(&ids))
        })
        .await
    }

    /// The entities that currently carry these stable keys.
    pub async fn resolution_entities_by_key(
        &self,
        keys: Vec<ResolutionKey>,
    ) -> Result<Vec<ResolutionEntity>, AppError> {
        self.read("Entity key lookup", move |connection| {
            let mut found = Vec::new();
            for chunk in keys.chunks(200) {
                let condition =
                    vec!["(entity.entity_kind = ? AND entity.normalized_value = ?)"; chunk.len()]
                        .join(" OR ");
                let values: Vec<String> = chunk
                    .iter()
                    .flat_map(|key| [key.kind.clone(), key.value.clone()])
                    .collect();
                found.extend(entities_where(connection, &condition, &values)?);
            }
            Ok(found)
        })
        .await
    }

    /// The first few mentions of each entity, in a fixed order, for the evidence snapshot.
    pub async fn resolution_examples(
        &self,
        ids: Vec<String>,
    ) -> Result<BTreeMap<String, Vec<MentionExample>>, AppError> {
        if ids.is_empty() {
            return Ok(BTreeMap::new());
        }
        self.read("Entity examples", move |connection| {
            let mut statement = connection
                .prepare(&format!(
                    "SELECT entity_id, portal_key, document_id, character_start, character_end
                     FROM (
                        SELECT entity_id, portal_key, document_id, character_start, character_end,
                               row_number() OVER (
                                   PARTITION BY entity_id
                                   ORDER BY portal_key, document_id, character_start
                               ) AS position
                        FROM corpus_entity_mentions WHERE entity_id IN ({})
                     ) WHERE position <= {EXAMPLES_PER_ENTITY}",
                    placeholders(ids.len())
                ))
                .map_err(|error| storage("prepare the example lookup", error))?;
            let rows = statement
                .query_map(params_from_iter(ids.iter()), |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        MentionExample {
                            portal_key: row.get(1)?,
                            document_id: u64::try_from(row.get::<_, i64>(2)?).unwrap_or(0),
                            character_start: u64::try_from(row.get::<_, i64>(3)?).unwrap_or(0),
                            character_end: u64::try_from(row.get::<_, i64>(4)?).unwrap_or(0),
                        },
                    ))
                })
                .map_err(|error| storage("read examples", error))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| storage("decode examples", error))?;
            let mut grouped: BTreeMap<String, Vec<MentionExample>> = BTreeMap::new();
            for (id, example) in rows {
                grouped.entry(id).or_default().push(example);
            }
            for examples in grouped.values_mut() {
                examples.sort_by(|a, b| {
                    (&a.portal_key, a.document_id, a.character_start).cmp(&(
                        &b.portal_key,
                        b.document_id,
                        b.character_start,
                    ))
                });
            }
            Ok(grouped)
        })
        .await
    }
}
