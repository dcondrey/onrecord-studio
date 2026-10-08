//! Logic shared across detectors: sentence/unit splitting, the agency-sweep query used by
//! detectors that pair a document with its request's agency, and the passage selection, agency
//! identity, similarity and candidate generation behind the shared-wording detectors.

use crate::{
    analysis::{
        finding::FindingCitation,
        phrase_match::{find_phrases, Phrases, Word},
    },
    error::AppError,
    storage::DuckDbRepository,
};
use duckdb::params;
use std::collections::{BTreeMap, HashMap, HashSet};

/// Byte ranges of sentence-like units: text broken on `\n`, `;`, `!`, `?`, or a `.` that precedes
/// whitespace or end of text, with whitespace-only segments dropped.
///
/// `exclude_abbreviations`: when true, a `.` after a run of three letters or fewer ("Mar.", "St.",
/// "No.") does not end a unit, so an abbreviated month stays with its date
/// (`encampment_notice`'s need). `min_len`: a candidate unit whose trimmed length is below this is
/// dropped (`response_time_claim` filters sub-12-character noise; pass `1` for "non-empty only").
pub(crate) fn split_units(
    text: &str,
    exclude_abbreviations: bool,
    min_len: usize,
) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut start = 0;
    for (index, character) in text.char_indices() {
        let ends = match character {
            '\n' | ';' | '!' | '?' => true,
            '.' => {
                let next_is_break = bytes
                    .get(index + 1)
                    .is_none_or(|next| next.is_ascii_whitespace());
                if exclude_abbreviations {
                    let letters = bytes[..index]
                        .iter()
                        .rev()
                        .take_while(|byte| byte.is_ascii_alphabetic())
                        .count();
                    next_is_break && !(1..=3).contains(&letters)
                } else {
                    next_is_break
                }
            }
            _ => false,
        };
        if ends {
            let end = index + character.len_utf8();
            if text[start..end].trim().len() >= min_len {
                spans.push((start, end));
            }
            start = end;
        }
    }
    if text[start..].trim().len() >= min_len {
        spans.push((start, text.len()));
    }
    spans
}

/// Jaccard similarity between two hashed-shingle/token sets: `|A∩B| / |A∪B|`, `0.0` when either
/// is empty. Generic over whatever 64-bit hash the caller's shingling uses — the ratio depends
/// only on set membership, not on how the hashes were produced.
pub(super) fn jaccard(a: &HashSet<u64>, b: &HashSet<u64>) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let intersection = a.intersection(b).count();
    let union = a.len() + b.len() - intersection;
    if union == 0 {
        return 0.0;
    }
    intersection as f64 / union as f64
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct Agency {
    pub id: i64,
    pub name: String,
}

/// `(portal, request id)` to the agency the request was made to.
pub(super) type AgencyIndex = HashMap<(String, i64), Agency>;

/// Every agency a request in the corpus was made to, keyed by `(portal, request id)`. Bounded by
/// `max_requests` so a huge corpus keeps a detector refresh responsive. `operation` names the
/// query for the calling detector's tracing/error context.
pub(super) async fn load_agencies(
    repository: &DuckDbRepository,
    operation: &'static str,
    max_requests: i64,
) -> Result<AgencyIndex, AppError> {
    repository
        .query_connection(operation, move |conn| {
            let mut statement = conn
                .prepare(
                    "SELECT r.portal_subdomain, r.id, a.id, a.name
                     FROM requests r
                     INNER JOIN agencies a
                        ON a.id = r.agency_id AND a.portal_subdomain = r.portal_subdomain
                     ORDER BY r.created_at DESC
                     LIMIT ?1",
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to prepare agency sweep: {error}"))
                })?;
            let rows = statement
                .query_map(params![max_requests], |row| {
                    Ok((
                        (row.get::<_, String>(0)?, row.get::<_, i64>(1)?),
                        Agency {
                            id: row.get(2)?,
                            name: row.get(3)?,
                        },
                    ))
                })
                .map_err(|error| {
                    AppError::Storage(format!("Failed to run agency sweep: {error}"))
                })?;
            rows.collect::<Result<AgencyIndex, _>>()
                .map_err(|error| AppError::Storage(format!("Failed to decode agency row: {error}")))
        })
        .await
}

/// An agency is one id on one portal. Ids repeat across portals, so the id alone names nothing.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) struct AgencyKey {
    pub portal: String,
    pub id: i64,
}

/// Carried by every shared-wording finding.
pub(super) const PATTERN_CAVEAT: &str = "This is a pattern observation. Shared wording is not \
    evidence that agencies coordinated, and it does not show that any request was not reviewed \
    individually: standard wording, a common template or a quoted statute all produce it.";

/// Phrase cues that select the passages of a document to compare.
pub(super) struct PassageRules {
    /// Phrases that state the outcome of a records search on their own.
    pub outcome: Phrases,
    /// Phrases that statutes, policies and ordinary prose also use. One counts only within
    /// `ADDRESSEE_REACH_WORDS` words of wording that addresses a requester, so a released record
    /// that quotes a statute is not read as a determination.
    pub needs_addressee: Phrases,
    /// Bytes kept on each side of a phrase, before the passage is cut to its paragraph.
    pub half_window: usize,
}

/// Newest public documents read per run by each shared-wording detector.
pub(super) const MAX_DOCUMENTS: i64 = 2_000;
/// Passages compared per document, in reading order.
pub(super) const MAX_PASSAGES_PER_DOCUMENT: usize = 8;
/// A passage shorter than this shares shingles with any other use of its phrase.
pub(super) const MIN_PASSAGE_CHARACTERS: usize = 120;
/// Leading characters of each document that are searched for phrases.
pub(super) const SCANNED_CHARACTERS: i64 = 50_000;
const ADDRESSEE_REACH_WORDS: usize = 120;
/// Citations per finding: one per agency first, then further members.
pub(super) const MAX_CITATIONS: usize = 12;
const MAX_NAMED_AGENCIES: usize = 8;

/// Lowercased words as `phrase_match::words` forms them, with the byte offset each starts at.
fn words_with_starts(text: &str) -> (Vec<Word>, Vec<usize>) {
    let mut words = Vec::new();
    let mut starts = Vec::new();
    let mut current = String::new();
    let mut start = 0;
    let mut finish = |current: &mut String, start: usize, followed_by: Option<char>| {
        let word = std::mem::take(current).replace('’', "'");
        words.push(Word {
            text: word.trim_end_matches('\'').to_string(),
            followed_by,
        });
        starts.push(start);
    };
    for (index, character) in text.char_indices() {
        if character.is_alphanumeric() || (matches!(character, '\'' | '’') && !current.is_empty())
        {
            if current.is_empty() {
                start = index;
            }
            current.extend(character.to_lowercase());
        } else if !current.is_empty() {
            finish(&mut current, start, Some(character));
        }
    }
    if !current.is_empty() {
        finish(&mut current, start, None);
    }
    (words, starts)
}

/// "your … request" within five words, or "you … requested" within two, near the phrase.
fn addresses_a_requester(words: &[Word], hit: usize) -> bool {
    let from = hit.saturating_sub(ADDRESSEE_REACH_WORDS);
    let to = (hit + ADDRESSEE_REACH_WORDS).min(words.len());
    let near = &words[from..to];
    let followed = |index: usize, reach: usize, wanted: &[&str]| {
        near[index + 1..]
            .iter()
            .take(reach)
            .any(|word| wanted.contains(&word.text.as_str()))
    };
    near.iter()
        .enumerate()
        .any(|(index, word)| match word.text.as_str() {
            "your" => followed(index, 5, &["request", "requests"]),
            "you" => followed(index, 2, &["requested"]),
            _ => false,
        })
}

/// Byte ranges of blank-line paragraph breaks: a line break, optional blanks, a line break.
fn paragraph_breaks(segment: &str) -> Vec<(usize, usize)> {
    let bytes = segment.as_bytes();
    let mut breaks = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\n' {
            let mut next = index + 1;
            while next < bytes.len() && matches!(bytes[next], b' ' | b'\t' | b'\r') {
                next += 1;
            }
            if next < bytes.len() && bytes[next] == b'\n' {
                breaks.push((index, next + 1));
                index = next;
                continue;
            }
        }
        index += 1;
    }
    breaks
}

/// The window around the word at byte `at`: `half` bytes each way, widened to whole words, cut to
/// the paragraph holding `at`, and never reaching back before `floor`.
fn window(text: &str, at: usize, half: usize, floor: usize) -> (usize, usize) {
    let mut start = at.saturating_sub(half);
    while !text.is_char_boundary(start) {
        start -= 1;
    }
    let start = text[..start]
        .char_indices()
        .rev()
        .find(|(_, character)| character.is_whitespace())
        .map_or(0, |(index, character)| index + character.len_utf8())
        .max(floor);
    let mut end = (at + half).min(text.len());
    while !text.is_char_boundary(end) {
        end += 1;
    }
    let end = text[end..]
        .find(char::is_whitespace)
        .map_or(text.len(), |index| end + index);
    let start = paragraph_breaks(&text[start..at])
        .last()
        .map_or(start, |(_, after)| start + after);
    let end = paragraph_breaks(&text[at..end])
        .first()
        .map_or(end, |(before, _)| at + before);
    (start, end)
}

/// Byte ranges of up to `MAX_PASSAGES_PER_DOCUMENT` non-overlapping passages, one around each
/// phrase occurrence that an earlier passage does not already cover.
pub(super) fn template_passages(text: &str, rules: &PassageRules) -> Vec<(usize, usize)> {
    let (words, starts) = words_with_starts(text);
    let mut hits = find_phrases(&words, rules.outcome);
    hits.extend(
        find_phrases(&words, rules.needs_addressee)
            .into_iter()
            .filter(|&hit| addresses_a_requester(&words, hit)),
    );
    hits.sort_unstable();
    hits.dedup();
    let mut passages = Vec::new();
    let mut covered = 0;
    for hit in hits {
        if passages.len() == MAX_PASSAGES_PER_DOCUMENT {
            break;
        }
        let at = starts[hit];
        if at < covered {
            continue;
        }
        let (start, end) = window(text, at, rules.half_window, covered);
        let passage = &text[start..end];
        let trimmed = passage.trim();
        if trimmed.chars().count() < MIN_PASSAGE_CHARACTERS {
            continue;
        }
        let start = start + (passage.len() - passage.trim_start().len());
        let end = start + trimmed.len();
        covered = end;
        passages.push((start, end));
    }
    passages
}

/// One compared passage. `character_start..character_end` are Unicode character offsets into
/// `documents_fts.extracted_text`, the units `SUBSTRING` and the citation readers use.
pub(super) struct PassageCandidate {
    pub document_id: i64,
    pub portal_key: String,
    pub agency: Option<AgencyKey>,
    /// `None` when the agency has no row or a blank name.
    pub agency_name: Option<String>,
    pub request_title: Option<String>,
    pub file_name: Option<String>,
    pub excerpt: String,
    pub character_start: i64,
    pub character_end: i64,
}

/// Passages of the newest `MAX_DOCUMENTS` public documents. Only the first `SCANNED_CHARACTERS`
/// characters of a document are read; a prefix keeps stored offsets valid.
pub(super) async fn load_passages(
    repository: &DuckDbRepository,
    operation: &'static str,
    rules: &'static PassageRules,
) -> Result<Vec<PassageCandidate>, AppError> {
    repository
        .query_connection(operation, move |conn| {
            let failed = |error| AppError::Storage(format!("{operation} failed: {error}"));
            let mut statement = conn
                .prepare(
                    "SELECT d.id, d.portal_subdomain, r.agency_id, a.name, r.title, d.file_name,
                            SUBSTRING(fts.extracted_text, 1, ?1)
                     FROM documents d
                     JOIN documents_fts fts
                        ON fts.document_id = d.id
                       AND fts.portal_subdomain = d.portal_subdomain
                     JOIN requests r
                        ON r.id = d.request_id
                       AND r.portal_subdomain = d.portal_subdomain
                     LEFT JOIN agencies a
                        ON a.id = r.agency_id
                       AND a.portal_subdomain = r.portal_subdomain
                     WHERE LOWER(TRIM(d.visibility)) = 'public'
                       AND d.portal_subdomain NOT IN (SELECT subdomain FROM user_supplied_sources)
                       AND LENGTH(fts.extracted_text) >= ?2
                     ORDER BY d.id DESC, d.portal_subdomain
                     LIMIT ?3",
                )
                .map_err(failed)?;
            let mut rows = statement
                .query(params![
                    SCANNED_CHARACTERS,
                    MIN_PASSAGE_CHARACTERS as i64,
                    MAX_DOCUMENTS
                ])
                .map_err(failed)?;
            let mut out = Vec::new();
            while let Some(row) = rows.next().map_err(failed)? {
                let document_id: i64 = row.get(0).map_err(failed)?;
                let portal_key: String = row.get(1).map_err(failed)?;
                let agency_id: Option<i64> = row.get(2).map_err(failed)?;
                let agency_name: Option<String> = row.get(3).map_err(failed)?;
                let request_title: Option<String> = row.get(4).map_err(failed)?;
                let file_name: Option<String> = row.get(5).map_err(failed)?;
                let text: String = row.get(6).map_err(failed)?;
                let agency = agency_id.map(|id| AgencyKey {
                    portal: portal_key.clone(),
                    id,
                });
                let agency_name = agency_name
                    .map(|name| name.trim().to_string())
                    .filter(|name| !name.is_empty());
                for (start, end) in template_passages(&text, rules) {
                    let character_start = text[..start].chars().count();
                    let character_end = character_start + text[start..end].chars().count();
                    out.push(PassageCandidate {
                        document_id,
                        portal_key: portal_key.clone(),
                        agency: agency.clone(),
                        agency_name: agency_name.clone(),
                        request_title: request_title.clone(),
                        file_name: file_name.clone(),
                        excerpt: text[start..end].to_string(),
                        character_start: character_start as i64,
                        character_end: character_end as i64,
                    });
                }
            }
            Ok(out)
        })
        .await
}

/// Distinct `(portal, document)` among `members`.
pub(super) fn distinct_documents(candidates: &[PassageCandidate], members: &[usize]) -> usize {
    members
        .iter()
        .map(|&member| {
            (
                candidates[member].portal_key.as_str(),
                candidates[member].document_id,
            )
        })
        .collect::<HashSet<_>>()
        .len()
}

/// Distinct non-blank request titles among `members`, ignoring case.
pub(super) fn distinct_titles(candidates: &[PassageCandidate], members: &[usize]) -> usize {
    members
        .iter()
        .filter_map(|&member| candidates[member].request_title.as_deref())
        .map(|title| title.trim().to_lowercase())
        .filter(|title| !title.is_empty())
        .collect::<HashSet<_>>()
        .len()
}

/// The agencies `members` were addressed to, each with its recorded name if it has one.
pub(super) fn agencies_of(
    candidates: &[PassageCandidate],
    members: &[usize],
) -> BTreeMap<AgencyKey, Option<String>> {
    members
        .iter()
        .filter_map(|&member| {
            let candidate = &candidates[member];
            Some((candidate.agency.clone()?, candidate.agency_name.clone()))
        })
        .collect()
}

/// An agency as prose. A missing name is said to be missing; the id is never passed off as one.
pub(super) fn agency_label(key: &AgencyKey, name: Option<&str>, show_portal: bool) -> String {
    match (name, show_portal) {
        (Some(name), true) => format!("{name} ({})", key.portal),
        (Some(name), false) => name.to_string(),
        (None, _) => format!(
            "an agency with no recorded name (id {} on {})",
            key.id, key.portal
        ),
    }
}

fn spans_portals(agencies: &BTreeMap<AgencyKey, Option<String>>) -> bool {
    agencies
        .keys()
        .map(|key| key.portal.as_str())
        .collect::<HashSet<_>>()
        .len()
        > 1
}

/// The agencies by name, the first `MAX_NAMED_AGENCIES` spelled out and the rest counted.
pub(super) fn name_agencies(agencies: &BTreeMap<AgencyKey, Option<String>>) -> String {
    let show_portal = spans_portals(agencies);
    let mut names: Vec<String> = agencies
        .iter()
        .take(MAX_NAMED_AGENCIES)
        .map(|(key, name)| agency_label(key, name.as_deref(), show_portal))
        .collect();
    if agencies.len() > MAX_NAMED_AGENCIES {
        names.push(format!("{} more", agencies.len() - MAX_NAMED_AGENCIES));
    }
    names.join("; ")
}

/// Citations for a cluster, each carrying its passage's character range: one member per agency
/// first so every agency that can be cited is, then further members, `MAX_CITATIONS` in all.
pub(super) fn cite_passages(
    candidates: &[PassageCandidate],
    members: &[usize],
) -> Vec<FindingCitation> {
    let show_portal = spans_portals(&agencies_of(candidates, members));
    let mut ordered = members.to_vec();
    ordered.sort_by_key(|&member| {
        let candidate = &candidates[member];
        (
            candidate.portal_key.as_str(),
            candidate.document_id,
            candidate.character_start,
        )
    });
    let mut seen = HashSet::new();
    let (first_of_agency, rest): (Vec<usize>, Vec<usize>) = ordered
        .into_iter()
        .partition(|&member| seen.insert(candidates[member].agency.as_ref()));
    first_of_agency
        .into_iter()
        .chain(rest)
        .take(MAX_CITATIONS)
        .map(|member| {
            let candidate = &candidates[member];
            let source = candidate
                .file_name
                .clone()
                .or_else(|| candidate.request_title.clone())
                .unwrap_or_else(|| format!("document {}", candidate.document_id));
            let agency = match &candidate.agency {
                Some(key) => agency_label(key, candidate.agency_name.as_deref(), show_portal),
                None => "no agency recorded for the request".to_string(),
            };
            FindingCitation {
                portal_key: candidate.portal_key.clone(),
                document_id: candidate.document_id,
                character_start: Some(candidate.character_start),
                character_end: Some(candidate.character_end),
                quote: Some(
                    candidate
                        .excerpt
                        .split_whitespace()
                        .take(24)
                        .collect::<Vec<_>>()
                        .join(" "),
                ),
                label: Some(format!("{agency}: {source}")),
            }
        })
        .collect()
}

/// The member a finding is anchored on: stable across runs over unchanged data.
pub(super) fn exemplar<'a>(
    candidates: &'a [PassageCandidate],
    members: &[usize],
) -> &'a PassageCandidate {
    let index = members
        .iter()
        .copied()
        .min_by_key(|&member| {
            let candidate = &candidates[member];
            (
                candidate.document_id,
                candidate.portal_key.as_str(),
                candidate.character_start,
            )
        })
        .expect("a cluster has members");
    &candidates[index]
}

/// Lowercased alphanumeric runs.
pub(super) fn all_words(text: &str) -> Vec<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// `all_words` without short words and without words every records letter uses.
pub(super) fn content_words(text: &str) -> Vec<String> {
    all_words(text)
        .into_iter()
        .filter(|word| word.len() >= 3 && !STOPWORDS.contains(&word.as_str()))
        .collect()
}

fn fnv1a(words: &[String]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for word in words {
        for byte in word.as_bytes().iter().chain(&[0x1f]) {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
        }
    }
    hash
}

/// Hashes of every run of `size` consecutive words. Empty when there are fewer words than that.
pub(super) fn shingles(words: &[String], size: usize) -> HashSet<u64> {
    words.windows(size).map(fnv1a).collect()
}

pub(super) fn word_set(words: &[String]) -> HashSet<u64> {
    words
        .iter()
        .map(|word| fnv1a(std::slice::from_ref(word)))
        .collect()
}

/// MinHash banding: `rows` minimum hashes per band, `bands` bands. Two sets with Jaccard `s` share
/// at least one band with probability `1 - (1 - s^rows)^bands`.
#[derive(Clone, Copy)]
pub(super) struct Banding {
    pub rows: usize,
    pub bands: usize,
}

/// For thresholds of 0.60 and above: a pair at 0.60 is missed with probability 5.9e-5, a pair at
/// 0.85 with probability below 1e-16.
pub(super) const BANDING_FROM_0_60: Banding = Banding { rows: 3, bands: 40 };
/// For a threshold of 0.35: a pair at 0.35 is missed with probability 9.0e-4, at 0.45 2.2e-7.
pub(super) const BANDING_FROM_0_35: Banding = Banding {
    rows: 3,
    bands: 160,
};

fn mix(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

/// Sets bucketed by MinHash band, so the sets worth comparing with one are those sharing a bucket
/// with it instead of every other set.
pub(super) struct BandIndex {
    keys: Vec<Vec<u64>>,
    buckets: HashMap<u64, Vec<usize>>,
}

impl BandIndex {
    pub(super) fn build(sets: &[HashSet<u64>], banding: Banding) -> Self {
        let seeds: Vec<u64> = (0..banding.rows * banding.bands)
            .map(|index| mix(index as u64))
            .collect();
        let mut keys = Vec::with_capacity(sets.len());
        let mut buckets: HashMap<u64, Vec<usize>> = HashMap::new();
        let mut minimums = vec![u64::MAX; seeds.len()];
        for (index, set) in sets.iter().enumerate() {
            if set.is_empty() {
                keys.push(Vec::new());
                continue;
            }
            minimums.fill(u64::MAX);
            for element in set {
                for (minimum, seed) in minimums.iter_mut().zip(&seeds) {
                    *minimum = (*minimum).min(mix(element ^ seed));
                }
            }
            let band_keys: Vec<u64> = minimums
                .chunks(banding.rows)
                .enumerate()
                .map(|(band, rows)| rows.iter().fold(band as u64, |key, row| mix(key ^ row)))
                .collect();
            for key in &band_keys {
                buckets.entry(*key).or_default().push(index);
            }
            keys.push(band_keys);
        }
        Self { keys, buckets }
    }

    /// Indexes above `index` that share a band with it, ascending.
    pub(super) fn later(&self, index: usize) -> Vec<usize> {
        let mut found: Vec<usize> = self.keys[index]
            .iter()
            .flat_map(|key| {
                let bucket = &self.buckets[key];
                bucket[bucket.partition_point(|&other| other <= index)..]
                    .iter()
                    .copied()
            })
            .collect();
        found.sort_unstable();
        found.dedup();
        found
    }
}

pub(super) struct SeedCluster {
    pub members: Vec<usize>,
    /// Mean Jaccard of each other member against the first.
    pub average_similarity: f64,
}

/// Greedy clustering in index order: each set not yet in a cluster seeds one and takes every later
/// unclustered set at or above `threshold` against it. `later(i)` supplies the indexes above `i`
/// to compare; clusters of one are dropped.
pub(super) fn seed_clusters(
    sets: &[HashSet<u64>],
    threshold: f64,
    later: impl Fn(usize) -> Vec<usize>,
) -> Vec<SeedCluster> {
    let mut clusters = Vec::new();
    let mut assigned = vec![false; sets.len()];
    for seed in 0..sets.len() {
        if assigned[seed] {
            continue;
        }
        assigned[seed] = true;
        let mut members = vec![seed];
        let mut total = 0.0;
        for other in later(seed) {
            if assigned[other] {
                continue;
            }
            let similarity = jaccard(&sets[seed], &sets[other]);
            if similarity >= threshold {
                assigned[other] = true;
                members.push(other);
                total += similarity;
            }
        }
        if members.len() > 1 {
            clusters.push(SeedCluster {
                average_similarity: total / (members.len() - 1) as f64,
                members,
            });
        }
    }
    clusters
}

/// Connected groups of two or more under `linked`, tried on each index and the `later` ones.
/// Members ascend within a group and groups are ordered by first member.
pub(super) fn linked_groups(
    count: usize,
    later: impl Fn(usize) -> Vec<usize>,
    linked: impl Fn(usize, usize) -> bool,
) -> Vec<Vec<usize>> {
    fn root(parent: &mut [usize], mut index: usize) -> usize {
        while parent[index] != index {
            parent[index] = parent[parent[index]];
            index = parent[index];
        }
        index
    }
    let mut parent: Vec<usize> = (0..count).collect();
    for index in 0..count {
        for other in later(index) {
            if linked(index, other) {
                let (a, b) = (root(&mut parent, index), root(&mut parent, other));
                parent[a.max(b)] = a.min(b);
            }
        }
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for index in 0..count {
        let group = root(&mut parent, index);
        groups.entry(group).or_default().push(index);
    }
    groups
        .into_values()
        .filter(|group| group.len() > 1)
        .collect()
}

const STOPWORDS: &[&str] = &[
    "the",
    "and",
    "for",
    "any",
    "all",
    "with",
    "from",
    "this",
    "that",
    "have",
    "has",
    "was",
    "were",
    "are",
    "not",
    "but",
    "into",
    "onto",
    "about",
    "please",
    "provide",
    "records",
    "record",
    "document",
    "documents",
    "information",
    "request",
    "requests",
    "public",
    "responsive",
    "you",
    "your",
    "our",
    "their",
    "will",
    "may",
    "can",
    "would",
    "shall",
    "such",
    "under",
    "over",
    "than",
    "then",
    "which",
    "when",
    "how",
    "who",
    "what",
    "why",
    "does",
    "did",
    "them",
    "these",
    "those",
    "some",
    "there",
    "here",
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    struct Draw(u64);

    impl Draw {
        fn below(&mut self, limit: usize) -> usize {
            self.0 = mix(self.0);
            (self.0 % limit as u64) as usize
        }
    }

    fn word(index: usize) -> String {
        format!("w{index}")
    }

    /// Replaced words in each copy of a template, so similarities fall on both sides of every
    /// threshold in use.
    const REPLACED: [usize; 14] = [0, 0, 1, 1, 2, 3, 4, 6, 9, 13, 18, 24, 30, 36];

    /// `families` templates of sixty words, each in one copy per entry of `REPLACED`, then `loose`
    /// unrelated passages. Loose passages draw half their words from a small common stock, as
    /// letters do.
    fn synthetic_passages(families: usize, loose: usize) -> Vec<Vec<String>> {
        let mut draw = Draw(7);
        let mut passages = Vec::new();
        for family in 0..families {
            let template: Vec<String> = (0..60)
                .map(|_| word(1_000 + family * 500 + draw.below(400)))
                .collect();
            for replaced in REPLACED {
                let mut copy = template.clone();
                for _ in 0..replaced {
                    let position = draw.below(copy.len());
                    copy[position] = word(10_000_000 + draw.below(1_000_000));
                }
                passages.push(copy);
            }
        }
        for _ in 0..loose {
            passages.push(
                (0..60)
                    .map(|position| {
                        if position % 2 == 0 {
                            word(draw.below(300))
                        } else {
                            word(20_000_000 + draw.below(5_000_000))
                        }
                    })
                    .collect(),
            );
        }
        passages
    }

    fn everything_later(count: usize) -> impl Fn(usize) -> Vec<usize> {
        move |index| (index + 1..count).collect()
    }

    fn pairs_at(sets: &[HashSet<u64>], threshold: f64) -> Vec<(usize, usize)> {
        let mut pairs = Vec::new();
        for a in 0..sets.len() {
            for b in a + 1..sets.len() {
                if jaccard(&sets[a], &sets[b]) >= threshold {
                    pairs.push((a, b));
                }
            }
        }
        pairs
    }

    #[test]
    fn banded_candidates_reproduce_the_exhaustive_comparison() {
        let passages = synthetic_passages(20, 200);
        let shingle_sets: Vec<_> = passages.iter().map(|words| shingles(words, 5)).collect();
        let word_sets: Vec<_> = passages.iter().map(|words| word_set(words)).collect();
        let count = passages.len();
        for (sets, threshold, banding) in [
            (&shingle_sets, 0.60, BANDING_FROM_0_60),
            (&shingle_sets, 0.85, BANDING_FROM_0_60),
            (&word_sets, 0.60, BANDING_FROM_0_60),
            (&word_sets, 0.35, BANDING_FROM_0_35),
        ] {
            let index = BandIndex::build(sets, banding);
            let wanted = pairs_at(sets, threshold);
            assert!(wanted.len() >= 30, "{} pairs at {threshold}", wanted.len());
            for (a, b) in &wanted {
                assert!(index.later(*a).contains(b), "{a}-{b} at {threshold}");
            }
            let members = |clusters: Vec<SeedCluster>| -> Vec<Vec<usize>> {
                clusters
                    .into_iter()
                    .map(|cluster| cluster.members)
                    .collect()
            };
            assert_eq!(
                members(seed_clusters(sets, threshold, |seed| index.later(seed))),
                members(seed_clusters(sets, threshold, everything_later(count))),
            );
            let linked = |a: usize, b: usize| jaccard(&sets[a], &sets[b]) >= threshold;
            assert_eq!(
                linked_groups(count, |member| index.later(member), linked),
                linked_groups(count, everything_later(count), linked),
            );
            let compared: usize = (0..count).map(|member| index.later(member).len()).sum();
            assert!(
                compared < count * (count - 1) / 4,
                "{compared} pairs compared"
            );
        }
    }

    /// Measurement, not a check: `cargo test --release --lib banded_pass_cost -- --ignored
    /// --nocapture`. Figures are recorded in `docs/analysis_methods.md`.
    #[test]
    #[ignore = "timing run"]
    fn banded_pass_cost() {
        for count in [2_000_usize, 16_000, 40_000] {
            let families = count / 200;
            let passages = synthetic_passages(families, count - families * REPLACED.len());
            for (label, threshold, banding, sets) in [
                (
                    "five-word shingles at 0.60",
                    0.60,
                    BANDING_FROM_0_60,
                    passages
                        .iter()
                        .map(|words| shingles(words, 5))
                        .collect::<Vec<_>>(),
                ),
                (
                    "word sets at 0.35",
                    0.35,
                    BANDING_FROM_0_35,
                    passages.iter().map(|words| word_set(words)).collect(),
                ),
            ] {
                let started = Instant::now();
                let index = BandIndex::build(&sets, banding);
                let built = started.elapsed();
                let compared: usize = (0..count).map(|member| index.later(member).len()).sum();
                let started = Instant::now();
                let banded = linked_groups(
                    count,
                    |member| index.later(member),
                    |a, b| jaccard(&sets[a], &sets[b]) >= threshold,
                );
                let banded_time = started.elapsed();
                println!(
                    "{count} passages, {label}: index {built:?}, banded pass {banded_time:?}, \
                     {compared} of {} pairs compared, {} groups",
                    count * (count - 1) / 2,
                    banded.len()
                );
                if count <= 16_000 {
                    let started = Instant::now();
                    let exhaustive = linked_groups(count, everything_later(count), |a, b| {
                        jaccard(&sets[a], &sets[b]) >= threshold
                    });
                    println!(
                        "  exhaustive pass {:?}, same groups: {}",
                        started.elapsed(),
                        exhaustive == banded
                    );
                }
            }
        }
    }

    #[test]
    fn a_phrase_split_across_lines_is_found_and_the_passage_stops_at_its_paragraph() {
        static RULES: PassageRules = PassageRules {
            outcome: &[&["no", "responsive", "records"]],
            needs_addressee: &[],
            half_window: 300,
        };
        let paragraph = "After a search of its files the office has no responsive\nrecords for \
            the period named, and it closes the matter on that basis without further action \
            today.";
        let text = format!("Opening paragraph about something else.\n \n{paragraph}\n\nClosing.");
        let passages = template_passages(&text, &RULES);
        assert_eq!(passages.len(), 1);
        assert_eq!(&text[passages[0].0..passages[0].1], paragraph);
    }

    #[test]
    fn passages_per_document_are_capped() {
        static RULES: PassageRules = PassageRules {
            outcome: &[&["no", "responsive", "records"]],
            needs_addressee: &[],
            half_window: 300,
        };
        let paragraph = "The office searched its files for item N and found no responsive records \
            for the period named, and it closes that item on that basis without further action.";
        let text = [paragraph; MAX_PASSAGES_PER_DOCUMENT + 1].join("\n\n");
        assert_eq!(
            template_passages(&text, &RULES).len(),
            MAX_PASSAGES_PER_DOCUMENT
        );
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    use crate::{error::AppError, storage::DuckDbRepository};
    use duckdb::params;
    use std::collections::HashSet;

    /// One public document on its own request. `agency` is `(id, name)` within `portal`.
    pub(crate) struct Letter {
        pub portal: &'static str,
        pub agency: Option<(i64, &'static str)>,
        pub document_id: i64,
        pub title: String,
        pub text: String,
    }

    pub(crate) fn letter(
        portal: &'static str,
        agency: (i64, &'static str),
        document_id: i64,
        text: &str,
    ) -> Letter {
        Letter {
            portal,
            agency: Some(agency),
            document_id,
            title: format!("Request {portal}-{document_id}"),
            text: text.to_string(),
        }
    }

    pub(crate) async fn corpus(letters: Vec<Letter>) -> DuckDbRepository {
        let repository = DuckDbRepository::new(":memory:").expect("open");
        repository
            .run_migrations(include_str!("../../../migrations/0001_initial_schema.sql"))
            .await
            .expect("migrate");
        repository
            .query_connection("shared-language fixture", move |connection| {
                let fixture = |error| AppError::Storage(format!("fixture: {error}"));
                for letter in &letters {
                    connection
                        .execute(
                            "INSERT INTO portals (subdomain, display_name, base_url)
                             VALUES (?1, ?1, 'https://example.invalid')
                             ON CONFLICT DO NOTHING",
                            params![letter.portal],
                        )
                        .map_err(fixture)?;
                    if let Some((id, name)) = letter.agency {
                        connection
                            .execute(
                                "INSERT INTO agencies (id, portal_subdomain, name)
                                 VALUES (?1, ?2, ?3) ON CONFLICT DO NOTHING",
                                params![id, letter.portal, name],
                            )
                            .map_err(fixture)?;
                    }
                    let file_name = format!("letter-{}.pdf", letter.document_id);
                    connection
                        .execute(
                            "INSERT INTO requests (id, portal_subdomain, title, status, created_at,
                                                   updated_at, agency_id)
                             VALUES (?1, ?2, ?3, 'closed', '2025-01-01T00:00:00Z',
                                     '2025-01-02T00:00:00Z', ?4)",
                            params![
                                letter.document_id,
                                letter.portal,
                                letter.title,
                                letter.agency.map(|(id, _)| id)
                            ],
                        )
                        .map_err(fixture)?;
                    connection
                        .execute(
                            "INSERT INTO documents (id, request_id, portal_subdomain, file_name,
                                                    visibility, created_at, updated_at,
                                                    extracted_text_sha256)
                             VALUES (?1, ?1, ?2, ?3, 'public', '2025-01-01T00:00:00Z',
                                     '2025-01-01T00:00:00Z', ?4)",
                            params![
                                letter.document_id,
                                letter.portal,
                                file_name,
                                format!("{}-{}", letter.portal, letter.document_id)
                            ],
                        )
                        .map_err(fixture)?;
                    connection
                        .execute(
                            "INSERT INTO documents_fts (document_id, portal_subdomain, file_name,
                                                        extracted_text)
                             VALUES (?1, ?2, ?3, ?4)",
                            params![letter.document_id, letter.portal, file_name, letter.text],
                        )
                        .map_err(fixture)?;
                }
                Ok(())
            })
            .await
            .expect("seed");
        repository
    }

    /// The stored text a citation's character range covers, read the way citation readers do.
    pub(crate) async fn stored_slice(
        repository: &DuckDbRepository,
        citation: &crate::analysis::finding::FindingCitation,
    ) -> String {
        let (portal, document) = (citation.portal_key.clone(), citation.document_id);
        let start = citation.character_start.expect("range start");
        let length = citation.character_end.expect("range end") - start;
        repository
            .query_connection("shared-language slice", move |connection| {
                connection
                    .query_row(
                        "SELECT SUBSTRING(extracted_text, ?1 + 1, ?2) FROM documents_fts
                         WHERE portal_subdomain = ?3 AND document_id = ?4",
                        params![start, length, portal, document],
                        |row| row.get(0),
                    )
                    .map_err(|error| AppError::Storage(format!("slice: {error}")))
            })
            .await
            .expect("slice")
    }

    /// Two sets of integers whose Jaccard is exactly `shared / total`.
    pub(crate) fn overlapping(shared: u64, total: u64) -> (HashSet<u64>, HashSet<u64>) {
        let only_first = (total - shared) / 2;
        let only_second = total - shared - only_first;
        let first = (0..shared + only_first).collect();
        let second = (0..shared)
            .chain(1_000_000..1_000_000 + only_second)
            .collect();
        (first, second)
    }

    pub(crate) const NO_RECORDS_LETTER: &str = "This letter responds to your request for public \
        records. The Department has reviewed its files and has determined that it has no \
        responsive records within its custody or control at this time. If you have questions \
        about this determination you may contact the records coordinator.";

    /// Four passages with one vocabulary in four word orders.
    pub(crate) const REWORDED_LETTERS: [&str; 4] = [
        "After staff searched archives and files, the department found no responsive records \
         concerning downtown redevelopment housing contract materials identified by the requester.",
        "Concerning housing contract materials for downtown redevelopment identified by the \
         requester, no responsive records were found when department staff searched files and \
         archives.",
        "The requester identified downtown redevelopment housing contract materials; department \
         staff searched archives, then files, and found no responsive records.",
        "Department staff searched files and archives for the contract materials on downtown \
         housing redevelopment the requester identified, and no responsive records were found.",
    ];
}
