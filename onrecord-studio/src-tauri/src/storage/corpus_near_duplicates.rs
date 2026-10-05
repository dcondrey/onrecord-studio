//! Near-duplicate candidates for one document, from the LSH bands and SimHash fingerprints the
//! pattern query already uses. A child of `corpus` so it shares that index, its distance
//! threshold and its row readers instead of keeping a second similarity structure.

use super::{
    parse_simhash, CorpusIndex, CORPUS_ALGORITHM_VERSION, NEAR_DUPLICATE_MAXIMUM_DISTANCE,
};
use crate::error::AppError;
use duckdb::{params, OptionalExt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NearDuplicateCandidate {
    pub portal_key: String,
    pub document_id: i64,
    pub simhash_distance: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NearDuplicateScan {
    /// The index holds no entry for this text under the current algorithm, so nothing was
    /// compared. Not the same as a scan that found nothing.
    NotIndexed,
    Scanned {
        /// Closest first.
        candidates: Vec<NearDuplicateCandidate>,
        /// Other documents the index held when it was scanned.
        other_indexed_documents: u64,
    },
}

impl CorpusIndex {
    /// Documents whose text differs from this one's and whose fingerprint is within
    /// [`NEAR_DUPLICATE_MAXIMUM_DISTANCE`] bits of it.
    pub async fn near_duplicates_of(
        &self,
        portal_key: String,
        document_id: u64,
        extracted_text_sha256: String,
        limit: usize,
    ) -> Result<NearDuplicateScan, AppError> {
        self.read("Near-duplicate scan for one document", move |connection| {
            let own_simhash = connection
                .query_row(
                    "SELECT simhash_hex FROM corpus_documents
                     WHERE portal_key = ?1 AND document_id = ?2
                       AND extracted_text_sha256 = ?3 AND algorithm_version = ?4",
                    params![
                        portal_key,
                        document_id,
                        extracted_text_sha256,
                        CORPUS_ALGORITHM_VERSION
                    ],
                    |row| parse_simhash(row.get::<_, String>(0)?),
                )
                .optional()
                .map_err(|error| {
                    AppError::Storage(format!("Failed to read the document fingerprint: {error}"))
                })?;
            let Some(own_simhash) = own_simhash else {
                return Ok(NearDuplicateScan::NotIndexed);
            };
            let other_indexed_documents = connection
                .query_row(
                    "SELECT COUNT(*) FROM corpus_documents
                     WHERE NOT (portal_key = ?1 AND document_id = ?2)",
                    params![portal_key, document_id],
                    |row| row.get::<_, u64>(0),
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to count indexed documents: {error}"))
                })?;
            // Two shared bands, as the corpus-wide pattern query requires, most shared first.
            let mut statement = connection
                .prepare(
                    "SELECT other.portal_key, other.document_id, other.simhash_hex
                     FROM corpus_lsh_bands AS own
                     INNER JOIN corpus_lsh_bands AS band
                       ON band.band_index = own.band_index
                      AND band.band_value = own.band_value
                      AND NOT (band.portal_key = own.portal_key
                               AND band.document_id = own.document_id)
                     INNER JOIN corpus_documents AS other
                       ON other.portal_key = band.portal_key
                      AND other.document_id = band.document_id
                     WHERE own.portal_key = ?1 AND own.document_id = ?2
                       AND other.extracted_text_sha256 <> ?3
                     GROUP BY other.portal_key, other.document_id, other.simhash_hex
                     HAVING COUNT(*) >= 2
                     ORDER BY COUNT(*) DESC, other.portal_key, other.document_id
                     LIMIT ?4",
                )
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to prepare the near-duplicate scan: {error}"
                    ))
                })?;
            let rows = statement
                .query_map(
                    params![portal_key, document_id, extracted_text_sha256, limit],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            parse_simhash(row.get::<_, String>(2)?)?,
                        ))
                    },
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to run the near-duplicate scan: {error}"))
                })?;
            let mut candidates = Vec::new();
            for row in rows {
                let (portal_key, document_id, simhash) = row.map_err(|error| {
                    AppError::Storage(format!("Failed to decode a near duplicate: {error}"))
                })?;
                let simhash_distance = (own_simhash ^ simhash).count_ones();
                if simhash_distance <= NEAR_DUPLICATE_MAXIMUM_DISTANCE {
                    candidates.push(NearDuplicateCandidate {
                        portal_key,
                        document_id,
                        simhash_distance,
                    });
                }
            }
            candidates.sort_by_key(|candidate| candidate.simhash_distance);
            Ok(NearDuplicateScan::Scanned {
                candidates,
                other_indexed_documents,
            })
        })
        .await
    }
}
