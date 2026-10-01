//! Read-only corpus queries behind entity resolution: the entities to compare, the validated
//! identifiers that link them, and a few example mentions for the evidence a reviewer sees.

use super::corpus::CorpusIndex;
use crate::error::AppError;
use crate::resolution::{
    link_condition, IdentifierLink, MentionExample, ResolutionEntity, ResolutionKey,
    IDENTIFIER_RULES,
};
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
    let identifier_kinds = IDENTIFIER_RULES
        .iter()
        .map(|rule| format!("'{}'", rule.kind))
        .collect::<Vec<_>>()
        .join(", ");
    let link_condition = link_condition("identifier", "named", "linked.span");
    let mut statement = connection
        .prepare(&format!(
            "WITH strong AS (
                        SELECT mention.entity_id
                        FROM corpus_entity_mentions mention
                        JOIN corpus_entities entity ON entity.entity_id = mention.entity_id
                        WHERE entity.entity_kind IN ({identifier_kinds})
                        GROUP BY mention.entity_id
                        HAVING bool_and(mention.validation_grade
                                        IN ('CHECK_DIGIT_PASSED', 'FORMAT_VALIDATED'))
                     ), linked AS (
                        SELECT left_entity_id AS identifier_id, right_entity_id AS entity_id,
                               portal_key, document_id, character_end - character_start AS span
                        FROM corpus_entity_relationships WHERE relationship_kind = 'CO_OCCURRENCE'
                        UNION ALL
                        SELECT right_entity_id, left_entity_id, portal_key, document_id,
                               character_end - character_start
                        FROM corpus_entity_relationships WHERE relationship_kind = 'CO_OCCURRENCE'
                     )
                     SELECT linked.identifier_id, identifier.entity_kind, identifier.display_value,
                            linked.entity_id,
                            COUNT(DISTINCT linked.portal_key || ':' || linked.document_id)
                     FROM linked
                     JOIN strong ON strong.entity_id = linked.identifier_id
                     JOIN corpus_entities identifier ON identifier.entity_id = linked.identifier_id
                     JOIN corpus_entities named ON named.entity_id = linked.entity_id
                          AND named.entity_kind IN ('PERSON', 'ORGANIZATION')
                          AND {link_condition} {restriction}
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn the_scan_links_only_validated_identifiers_and_counts_distinct_documents() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("corpus.db");
        drop(CorpusIndex::open(&path).expect("the index must open"));
        let connection = Connection::open(&path).expect("the fresh index must open");
        let document = |id: i64| {
            format!(
                "('records', 1, {id}, repeat('a', 64), repeat('b', 64), 3, repeat('0', 16),
                  FALSE, 'deterministic-corpus-v3', '2026-09-29T00:00:00Z')"
            )
        };
        let mention = |entity: &str, document: i64, start: i64, grade: &str| {
            format!(
                "('records', {document}, '{entity}', 0, {start}, {}, 5000, 'test', 1, '{grade}')",
                start + 4
            )
        };
        let edge = |email: &str, person: &str, document: i64| {
            format!(
                "('records', {document}, '{email}', '{person}', 'CO_OCCURRENCE', 'COMPUTED', 0, 0, 9, 4000, NULL, NULL)"
            )
        };
        connection
            .execute_batch(&format!(
                "INSERT INTO corpus_documents VALUES {}, {};
                 INSERT INTO corpus_entities VALUES
                    ('e-good', 'EMAIL_ADDRESS', 'jd@x.org', 'jd@x.org', 'EXACT_NORMALIZED_MATCH'),
                    ('e-weak', 'EMAIL_ADDRESS', 'weak@x.org', 'weak@x.org', 'EXACT_NORMALIZED_MATCH'),
                    ('p-1', 'PERSON', 'jane doe', 'Jane Doe', 'EXACT_NORMALIZED_MATCH'),
                    ('p-2', 'PERSON', 'j doe', 'J. Doe', 'EXACT_NORMALIZED_MATCH');
                 INSERT INTO corpus_entity_document_counts VALUES
                    ('e-good', 2), ('e-weak', 2), ('p-1', 2), ('p-2', 2);
                 INSERT INTO corpus_entity_mentions VALUES {}, {}, {}, {}, {}, {}, {}, {};
                 INSERT INTO corpus_entity_relationships VALUES {}, {}, {}, {}, {}, {};",
                document(100),
                document(101),
                mention("e-good", 100, 0, "FORMAT_VALIDATED"),
                mention("e-good", 101, 0, "FORMAT_VALIDATED"),
                mention("e-weak", 100, 10, "FORMAT_VALIDATED"),
                mention("e-weak", 101, 10, "PATTERN_ONLY"),
                mention("p-1", 100, 20, "GAZETTEER_HIT"),
                mention("p-1", 101, 20, "GAZETTEER_HIT"),
                mention("p-2", 100, 30, "GAZETTEER_HIT"),
                mention("p-2", 101, 30, "GAZETTEER_HIT"),
                edge("e-good", "p-1", 100),
                edge("e-good", "p-1", 101),
                edge("e-good", "p-2", 100),
                edge("e-good", "p-2", 101),
                edge("e-weak", "p-1", 100),
                edge("e-weak", "p-2", 100),
            ))
            .expect("fixture rows must insert");
        drop(connection);

        let index = CorpusIndex::open(&path).expect("the index must reopen");
        let scan = index.resolution_scan().await.expect("the scan must read");
        assert!(!scan.scan_truncated);
        let mut people: Vec<_> = scan.entities.iter().map(|e| e.entity_id.as_str()).collect();
        people.sort();
        assert_eq!(people, vec!["p-1", "p-2"]);
        let mut links: Vec<_> = scan
            .links
            .iter()
            .map(|l| (l.identifier_id.as_str(), l.entity_id.as_str(), l.documents))
            .collect();
        links.sort();
        assert_eq!(links, vec![("e-good", "p-1", 2), ("e-good", "p-2", 2)]);

        let only = index
            .resolution_links_for(vec!["p-1".to_string()])
            .await
            .unwrap();
        assert_eq!(only.len(), 1);
        let keyed = index
            .resolution_entities_by_key(vec![ResolutionKey::new("PERSON", "j doe")])
            .await
            .unwrap();
        assert_eq!(keyed.len(), 1);
        assert_eq!(
            (keyed[0].entity_id.as_str(), keyed[0].document_count),
            ("p-2", 2)
        );
        let examples = index
            .resolution_examples(vec!["p-1".to_string(), "p-2".to_string()])
            .await
            .unwrap();
        assert_eq!(examples["p-1"].len(), 2);
        assert_eq!(examples["p-1"][0].document_id, 100);
    }

    #[tokio::test]
    async fn identifiers_link_only_the_entity_kinds_and_distances_their_rule_allows() {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("corpus.db");
        drop(CorpusIndex::open(&path).expect("the index must open"));
        let connection = Connection::open(&path).expect("the fresh index must open");
        let document = |id: i64| {
            format!(
                "('records', 1, {id}, repeat('a', 64), repeat('b', 64), 3, repeat('0', 16),
                  FALSE, 'deterministic-corpus-v3', '2026-09-29T00:00:00Z')"
            )
        };
        let mention = |entity: &str, document: i64| {
            format!(
                "('records', {document}, '{entity}', 0, 0, 4, 5000, 'test', 1, 'FORMAT_VALIDATED')"
            )
        };
        let edge = |identifier: &str, entity: &str, document: i64, span: i64| {
            format!(
                "('records', {document}, '{identifier}', '{entity}', 'CO_OCCURRENCE', 'COMPUTED', 0, 0, {span}, 4000, NULL, NULL)"
            )
        };
        let identifiers = [
            ("i-ein", "EIN"),
            ("i-lic", "CONTRACTOR_LICENSE"),
            ("i-apn", "PARCEL_NUMBER"),
        ];
        let entities = identifiers
            .iter()
            .map(|(id, kind)| {
                format!("('{id}', '{kind}', '{id}', '{id}', 'EXACT_NORMALIZED_MATCH')")
            })
            .chain(
                [
                    ("o-1", "ORGANIZATION"),
                    ("o-2", "ORGANIZATION"),
                    ("o-3", "ORGANIZATION"),
                    ("p-1", "PERSON"),
                ]
                .iter()
                .map(|(id, kind)| {
                    format!("('{id}', '{kind}', '{id}', '{id}', 'EXACT_NORMALIZED_MATCH')")
                }),
            )
            .collect::<Vec<_>>()
            .join(", ");
        let mentions = identifiers
            .iter()
            .flat_map(|(id, _)| [mention(id, 100), mention(id, 101)])
            .collect::<Vec<_>>()
            .join(", ");
        let edges = [
            edge("i-ein", "o-1", 100, 30),
            edge("i-ein", "o-1", 101, 30),
            edge("i-ein", "o-2", 100, 30),
            edge("i-ein", "o-2", 101, 200),
            // One document within reach, one beyond it: the far edge does not count.
            edge("i-ein", "o-3", 100, 30),
            edge("i-ein", "o-3", 101, 201),
            // An EIN does not anchor a person; a parcel anchors neither people nor organizations.
            edge("i-ein", "p-1", 100, 30),
            edge("i-ein", "p-1", 101, 30),
            edge("i-apn", "o-1", 100, 30),
            edge("i-apn", "o-1", 101, 30),
            edge("i-apn", "p-1", 100, 30),
            edge("i-apn", "p-1", 101, 30),
            edge("i-lic", "p-1", 100, 30),
            edge("i-lic", "p-1", 101, 30),
        ]
        .join(", ");
        connection
            .execute_batch(&format!(
                "INSERT INTO corpus_documents VALUES {}, {};
                 INSERT INTO corpus_entities VALUES {entities};
                 INSERT INTO corpus_entity_mentions VALUES {mentions};
                 INSERT INTO corpus_entity_relationships VALUES {edges};",
                document(100),
                document(101),
            ))
            .expect("fixture rows must insert");
        drop(connection);

        let index = CorpusIndex::open(&path).expect("the index must reopen");
        let scan = index.resolution_scan().await.expect("the scan must read");
        let mut links: Vec<_> = scan
            .links
            .iter()
            .map(|l| (l.identifier_id.as_str(), l.entity_id.as_str(), l.documents))
            .collect();
        links.sort();
        assert_eq!(
            links,
            vec![
                ("i-ein", "o-1", 2),
                ("i-ein", "o-2", 2),
                ("i-ein", "o-3", 1),
                ("i-lic", "p-1", 2),
            ]
        );
    }
}
