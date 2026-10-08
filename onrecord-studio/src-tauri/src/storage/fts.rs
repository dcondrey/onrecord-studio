//! Persistent core tokenization for the DuckDB search projection.
//!
//! The core does not support Boolean search. Every unqualified text token must occur, and each
//! quoted phrase ([`core_query_phrases`]) must occur as consecutive tokens; agency, portal,
//! status, and flag qualifiers travel as separate typed command parameters. The core persists
//! [`token_frequencies`] and searches with [`core_query_tokens`].

pub(crate) use crate::normalization::normalize_index_text as clean_plain_text_for_index;
use std::collections::{BTreeMap, BTreeSet};

const MAXIMUM_INDEX_TOKEN_CHARACTERS: usize = 128;

pub const MAXIMUM_CORE_QUERY_BYTES: usize = 16 * 1024;
pub const MAXIMUM_CORE_QUERY_TOKENS: usize = 512;
const MAXIMUM_CORE_QUERY_PHRASE_TOKENS: usize = 64;

/// The lowercased alphanumeric runs of `text`, each truncated to the indexed length, so the
/// stored projection and a query tokenize identically.
fn index_tokens(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(|token| {
            token
                .to_lowercase()
                .chars()
                .take(MAXIMUM_INDEX_TOKEN_CHARACTERS)
                .collect::<String>()
        })
}

pub fn token_frequencies(text: &str) -> BTreeMap<String, u64> {
    let mut frequencies = BTreeMap::new();
    for token in index_tokens(text) {
        let frequency = frequencies.entry(token).or_insert(0_u64);
        *frequency = frequency.saturating_add(1);
    }
    frequencies
}

/// Extracts the distinct terms the typed frontend sends as full-text content. Structured fields
/// are transported separately. All returned terms are required by the core query.
pub fn core_query_tokens(query: &str) -> Result<Vec<String>, &'static str> {
    if query.len() > MAXIMUM_CORE_QUERY_BYTES {
        return Err("Search text exceeds the 16 KiB query limit");
    }
    let tokens = index_tokens(query).collect::<Vec<_>>();
    if tokens.len() > MAXIMUM_CORE_QUERY_TOKENS {
        return Err("Search text exceeds the 512-token query limit");
    }
    Ok(tokens
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect())
}

/// The quoted phrases of a query, each as its lowercased tokens in order. A phrase must occur as
/// consecutive tokens, so `"body camera"` does not match a record that only says "camera ... body".
/// A one-token phrase adds nothing beyond [`core_query_tokens`] and is omitted, as is an unclosed
/// quote. Tokens are not truncated: the phrase is matched against the full text, not the index.
pub fn core_query_phrases(query: &str) -> Result<Vec<Vec<String>>, &'static str> {
    let segments: Vec<&str> = query.split('"').collect();
    // Odd segments sit inside quotes; the last one is inside only if a closing quote follows it.
    let closed = &segments[..segments.len() - 1];
    let mut phrases: Vec<Vec<String>> = closed
        .iter()
        .skip(1)
        .step_by(2)
        .map(|quoted| {
            quoted
                .split(|character: char| !character.is_alphanumeric())
                .filter(|token| !token.is_empty())
                .map(str::to_lowercase)
                .collect::<Vec<_>>()
        })
        .filter(|tokens| tokens.len() > 1)
        .collect();
    // Each phrase compiles to one regular expression; DuckDB's engine refused one of 512 words as
    // too large, so the limit is stated here instead of surfacing as an engine error.
    if phrases
        .iter()
        .any(|tokens| tokens.len() > MAXIMUM_CORE_QUERY_PHRASE_TOKENS)
    {
        return Err("A quoted phrase can hold at most 64 words");
    }
    phrases.sort();
    phrases.dedup();
    Ok(phrases)
}

#[cfg(test)]
mod tests {
    use super::{core_query_tokens, token_frequencies};

    #[test]
    fn a_query_token_matches_the_indexed_token_for_the_same_word() {
        let long_word = "a".repeat(300);
        let text = format!("Body-Camera footage {long_word}");
        let indexed = token_frequencies(&text);
        let queried = core_query_tokens(&text).expect("query is within limits");
        assert_eq!(queried, indexed.keys().cloned().collect::<Vec<_>>());
    }
}
