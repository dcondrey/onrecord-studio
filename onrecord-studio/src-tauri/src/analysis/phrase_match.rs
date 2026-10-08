//! Word-token phrase matching shared by the cue-based extractors.
//!
//! Cues are phrase lists over lowercased word tokens, not patterns over raw text: a phrase can only
//! match on word boundaries, and each list is data that can be read and tested one entry at a time.

pub(crate) type Phrases = &'static [&'static [&'static str]];

pub(crate) struct Word {
    pub(crate) text: String,
    pub(crate) followed_by: Option<char>,
}

impl Word {
    /// `No. 5`, `No #5`, `No 5`: the number sign, not a negation.
    pub(crate) fn abbreviation_like(&self) -> bool {
        matches!(self.followed_by, Some('.' | '#' | ':'))
    }
}

/// Lowercased words, keeping internal apostrophes so `didn't` stays one word.
pub(crate) fn words(connector: &str) -> Vec<Word> {
    let mut words = Vec::new();
    let mut current = String::new();
    for character in connector.chars() {
        if character.is_alphanumeric() || (matches!(character, '\'' | '’') && !current.is_empty())
        {
            if character == '’' {
                current.push('\'');
            } else {
                current.extend(character.to_lowercase());
            }
        } else if !current.is_empty() {
            words.push(Word {
                text: std::mem::take(&mut current),
                followed_by: Some(character),
            });
        }
    }
    if !current.is_empty() {
        words.push(Word {
            text: current,
            followed_by: None,
        });
    }
    words
}

fn matches_at(window: &[Word], phrase: &[&str]) -> bool {
    window
        .iter()
        .zip(phrase)
        .all(|(word, cue)| word.text == *cue)
}

fn contains_phrase(words: &[Word], phrase: &[&str]) -> bool {
    words
        .windows(phrase.len())
        .any(|window| matches_at(window, phrase))
}

pub(crate) fn has_phrase(words: &[Word], phrases: Phrases) -> bool {
    phrases.iter().any(|phrase| contains_phrase(words, phrase))
}

/// How many distinct phrases of the list occur.
pub(crate) fn count_phrases(words: &[Word], phrases: Phrases) -> usize {
    phrases
        .iter()
        .filter(|phrase| contains_phrase(words, phrase))
        .count()
}

/// Word index of every occurrence of any phrase of the list, in ascending order.
pub(crate) fn find_phrases(words: &[Word], phrases: Phrases) -> Vec<usize> {
    let mut found: Vec<usize> = phrases
        .iter()
        .flat_map(|phrase| {
            words
                .windows(phrase.len())
                .enumerate()
                .filter(|(_, window)| matches_at(window, phrase))
                .map(|(index, _)| index)
                .collect::<Vec<_>>()
        })
        .collect();
    found.sort_unstable();
    found.dedup();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curly_apostrophe_in_final_word_matches_straight_cue() {
        let tokens = words("they didn’t");
        let last = tokens.last().expect("final word");
        assert_eq!(last.text, "didn't");
        assert_eq!(last.followed_by, None);
        assert!(has_phrase(&tokens, &[&["didn't"]]));
    }
}
