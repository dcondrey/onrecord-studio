//! Whether a link, page title, dataset or request is about the collection subject.
//!
//! Keywords alone miss most of what a subject covers: "homelessness" never appears in "Encampment
//! abatement schedule". So a candidate that fails the keyword test is compared with the subject by
//! the on-device embedding model. Measured on 28 labelled titles from a real corpus (see
//! `storage::vector::subject_relevance_calibration`): with the BGE query instruction, similarity
//! at or above `SIMILARITY_THRESHOLD` admitted 9 of 12 relevant titles and 0 of 16 irrelevant
//! ones, where keywords admitted 1 of 12. The set is small; the threshold is a measurement on it,
//! not a guarantee.

use super::guidance::{subject_searches, subject_terms, texts_match_subject};
use crate::storage::vector::VectorIndex;

pub(super) const SIMILARITY_THRESHOLD: f32 = 0.55;
/// BGE retrieval models are trained with this instruction on the query side only.
const QUERY_INSTRUCTION: &str = "Represent this sentence for searching relevant passages: ";

pub(super) struct SubjectMatcher {
    terms: Vec<Vec<String>>,
    /// One query vector per comma-separated search.
    semantic: Option<(VectorIndex, Vec<Vec<f32>>)>,
}

impl SubjectMatcher {
    /// `None` when there is no subject. Without an installed model, or if embedding the subject
    /// fails, matching falls back to keywords alone.
    pub(super) async fn new(subject: Option<&str>, model: Option<&VectorIndex>) -> Option<Self> {
        let subject = subject?;
        let terms = subject_terms(Some(subject));
        let queries = subject_searches(subject)
            .into_iter()
            .map(|search| format!("{QUERY_INSTRUCTION}{search}"))
            .collect();
        let semantic = match model {
            Some(model) => model
                .embed_short_texts(queries)
                .await
                .ok()
                .flatten()
                .filter(|vectors| !vectors.is_empty())
                .map(|vectors| (model.clone(), vectors)),
            None => None,
        };
        Some(Self { terms, semantic })
    }

    /// For each candidate text, whether it is about the subject.
    pub(super) async fn matches_each(&self, candidates: &[String]) -> Vec<bool> {
        let readable: Vec<String> = candidates.iter().map(|text| readable_title(text)).collect();
        let mut matched: Vec<bool> = readable
            .iter()
            .map(|text| texts_match_subject(&self.terms, &[text]))
            .collect();
        let Some((model, searches)) = &self.semantic else {
            return matched;
        };
        let pending: Vec<usize> = (0..readable.len())
            .filter(|&index| !matched[index] && !readable[index].trim().is_empty())
            .collect();
        if pending.is_empty() {
            return matched;
        }
        let texts = pending
            .iter()
            .map(|&index| readable[index].clone())
            .collect();
        if let Ok(Some(vectors)) = model.embed_short_texts(texts).await {
            for (index, vector) in pending.into_iter().zip(vectors) {
                matched[index] = searches.iter().any(|search| {
                    let similarity: f32 = vector.iter().zip(search).map(|(a, b)| a * b).sum();
                    similarity >= SIMILARITY_THRESHOLD
                });
            }
        }
        matched
    }

    /// Keyword test only, for text too long to embed as one title (a page body).
    pub(super) fn mentions(&self, text: &str) -> bool {
        texts_match_subject(&self.terms, &[text])
    }
}

/// For each candidate, whether it is about the subject; every candidate when there is none.
pub(super) async fn admitted(matcher: Option<&SubjectMatcher>, candidates: &[String]) -> Vec<bool> {
    match matcher {
        Some(matcher) => matcher.matches_each(candidates).await,
        None => vec![true; candidates.len()],
    }
}

/// A file name or URL as words: `FY19_HHP%20Grant-Program.pdf` reads "FY19 HHP Grant Program".
pub(super) fn readable_title(text: &str) -> String {
    let decoded = text.replace("%20", " ");
    let name = decoded
        .rsplit('/')
        .find(|segment| !segment.is_empty())
        .unwrap_or(&decoded);
    let stem = match name.rsplit_once('.') {
        Some((stem, extension))
            if !stem.is_empty()
                && (1..=5).contains(&extension.len())
                && extension.chars().all(|c| c.is_ascii_alphanumeric()) =>
        {
            stem
        }
        _ => name,
    };
    stem.split(|c: char| matches!(c, '_' | '-' | '.' | '+') || c.is_whitespace())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_and_addresses_read_as_words() {
        assert_eq!(
            readable_title("FY19_HHP%20Grant-Program.pdf"),
            "FY19 HHP Grant Program"
        );
        assert_eq!(
            readable_title("https://example.gov/files/Encampment_Abatement.xlsx"),
            "Encampment Abatement"
        );
        assert_eq!(
            readable_title("Homelessness Hub at UC San Diego"),
            "Homelessness Hub at UC San Diego"
        );
    }

    #[tokio::test]
    async fn without_a_model_matching_is_by_keyword() {
        let matcher = SubjectMatcher::new(Some("encampments"), None)
            .await
            .unwrap();
        assert_eq!(
            matcher
                .matches_each(&[
                    "Encampment_Abatement.pdf".into(),
                    "Transit_Stops.pdf".into()
                ])
                .await,
            [true, false]
        );
        assert!(SubjectMatcher::new(None, None).await.is_none());
        let either = SubjectMatcher::new(Some("encampment, safe parking"), None)
            .await
            .unwrap();
        assert_eq!(
            either
                .matches_each(&[
                    "Safe_Parking_Program.pdf".into(),
                    "Parking_Revenue.pdf".into(),
                    "Encampment_Abatement.pdf".into(),
                ])
                .await,
            [true, false, true]
        );
    }
}
