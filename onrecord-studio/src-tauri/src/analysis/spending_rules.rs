//! Spending and procurement rules an agency states about itself in a policy document.
//!
//! Extraction is word-token phrase matching (`phrase_match`), never patterns over raw text. A rule
//! is taken only when one sentence states one dollar figure, a comparator, an approval or limit cue
//! and a spending noun. A sentence carrying an exception ("unless", "except", "not", "no",
//! "without", "exempt") is dropped: its limit cannot be applied to a row without knowing the
//! exception, so it is reported as no rule rather than as a broader one.
//!
//! Money is read only as a `$` figure, in whole or cents, scaled to the financial module's fixed
//! point. A figure written in words ("twenty-five thousand dollars") is not extracted.

use super::financial::decimal;
use super::phrase_match::{has_phrase, words, Phrases, Word};

const STRICT_ABOVE: Phrases = &[
    &["over"],
    &["exceed"],
    &["exceeds"],
    &["exceeding"],
    &["above"],
    &["more", "than"],
    &["greater", "than"],
    &["in", "excess", "of"],
];
const INCLUSIVE_ABOVE: Phrases = &[
    &["or", "more"],
    &["or", "greater"],
    &["or", "above"],
    &["and", "above"],
    &["at", "least"],
];
const REQUIREMENT: Phrases = &[
    &["require"],
    &["requires"],
    &["required"],
    &["requiring"],
    &["must"],
    &["shall"],
];
const APPROVAL: Phrases = &[
    &["approval"],
    &["approved"],
    &["approve"],
    &["authorization"],
    &["authorized"],
    &["authorize"],
];
const EXCEPTION: Phrases = &[
    &["unless"],
    &["except"],
    &["not"],
    &["no"],
    &["without"],
    &["exempt"],
    &["exempted"],
    &["exemption"],
];
const CONTRACT_NOUNS: Phrases = &[
    &["contract"],
    &["contracts"],
    &["award"],
    &["awards"],
    &["agreement"],
    &["agreements"],
    &["procurement"],
    &["procurements"],
];
const PAYMENT_NOUNS: Phrases = &[
    &["payment"],
    &["payments"],
    &["invoice"],
    &["invoices"],
    &["disbursement"],
    &["disbursements"],
    &["expenditure"],
    &["expenditures"],
    &["purchase"],
    &["purchases"],
    &["order"],
    &["orders"],
];
const CARD: Phrases = &[
    &["purchasing", "card"],
    &["procurement", "card"],
    &["credit", "card"],
    &["corporate", "card"],
    &["pcard"],
];
const LIMIT: Phrases = &[&["limit"], &["limits"]];
const OVERTIME: Phrases = &[&["overtime"]];
const SPLITTING: Phrases = &[
    &["split"],
    &["splitting"],
    &["fragment"],
    &["fragmenting"],
    &["artificially", "divide"],
    &["breaking", "up"],
];
const PROHIBITION: Phrases = &[
    &["shall", "not"],
    &["may", "not"],
    &["must", "not"],
    &["cannot"],
    &["prohibited"],
    &["forbidden"],
    &["not", "permitted"],
    &["not", "allowed"],
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleKind {
    /// Amounts over a limit require an approval.
    ApprovalThreshold,
    /// A purchasing-card per-transaction ceiling.
    CardLimit,
    /// Overtime needs prior authorization; it states no amount.
    OvertimeAuthorization,
    /// The policy forbids dividing a purchase to stay under a limit.
    SplittingProhibited,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedRule {
    pub kind: RuleKind,
    /// Scaled like `financial::decimal`. `None` for rules that state no amount.
    pub limit: Option<i128>,
    /// True when the stated amount itself is covered ("$5,000 or more").
    pub inclusive: bool,
    /// Applies to contract award amounts.
    pub contracts: bool,
    /// Applies to payment and purchase amounts.
    pub payments: bool,
    pub byte_start: usize,
    pub byte_end: usize,
}

impl ExtractedRule {
    /// True when `amount` is past the stated limit under the stated comparator.
    pub fn exceeded_by(&self, amount: i128) -> bool {
        self.limit
            .is_some_and(|limit| amount > limit || (self.inclusive && amount == limit))
    }
}

/// Sentence spans over byte offsets. A period between digits or directly before one is part of a
/// number or a section reference, not a sentence end.
fn sentences(text: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut start = 0;
    let characters: Vec<(usize, char)> = text.char_indices().collect();
    for (position, &(offset, character)) in characters.iter().enumerate() {
        let ends = match character {
            '!' | '?' | ';' | '\n' => true,
            '.' => !characters
                .get(position + 1)
                .is_some_and(|&(_, next)| next.is_ascii_digit()),
            _ => false,
        };
        if ends {
            let end = offset;
            if text[start..end].trim().len() > 1 {
                spans.push((start, end));
            }
            start = offset + character.len_utf8();
        }
    }
    if text[start..].trim().len() > 1 {
        spans.push((start, text.len()));
    }
    spans
}

/// Every `$` figure in the sentence, scaled. Thousands separators are dropped; a trailing comma
/// or period is punctuation.
fn dollar_amounts(sentence: &str) -> Vec<i128> {
    let characters: Vec<char> = sentence.chars().collect();
    let mut amounts = Vec::new();
    let mut index = 0;
    while index < characters.len() {
        if characters[index] != '$' {
            index += 1;
            continue;
        }
        let mut digits = String::new();
        let mut cursor = index + 1;
        while cursor < characters.len() {
            let character = characters[cursor];
            let next_is_digit = characters
                .get(cursor + 1)
                .is_some_and(|next| next.is_ascii_digit());
            if character.is_ascii_digit() {
                digits.push(character);
            } else if (character == ',' || character == '.') && next_is_digit {
                if character == '.' {
                    digits.push('.');
                }
            } else {
                break;
            }
            cursor += 1;
        }
        if let Some(amount) = decimal(&digits) {
            amounts.push(amount);
        }
        index = cursor.max(index + 1);
    }
    amounts
}

fn single_amount(sentence: &str) -> Option<i128> {
    let amounts = dollar_amounts(sentence);
    let first = *amounts.first()?;
    amounts
        .iter()
        .all(|amount| *amount == first)
        .then_some(first)
}

fn classify(sentence: &str, tokens: &[Word]) -> Option<ExtractedRule> {
    let overtime = has_phrase(tokens, OVERTIME);
    let needs_approval = has_phrase(tokens, REQUIREMENT) && has_phrase(tokens, APPROVAL);
    let excepted = has_phrase(tokens, EXCEPTION);
    let rule = |kind, limit, inclusive, contracts, payments| ExtractedRule {
        kind,
        limit,
        inclusive,
        contracts,
        payments,
        byte_start: 0,
        byte_end: 0,
    };
    if has_phrase(tokens, SPLITTING) && has_phrase(tokens, PROHIBITION) {
        let noun = has_phrase(tokens, PAYMENT_NOUNS) || has_phrase(tokens, CONTRACT_NOUNS);
        return noun.then(|| rule(RuleKind::SplittingProhibited, None, false, false, true));
    }
    if excepted {
        return None;
    }
    if overtime && needs_approval {
        return Some(rule(
            RuleKind::OvertimeAuthorization,
            None,
            false,
            false,
            false,
        ));
    }
    if has_phrase(tokens, CARD) && has_phrase(tokens, LIMIT) {
        let limit = single_amount(sentence)?;
        return Some(rule(RuleKind::CardLimit, Some(limit), false, false, true));
    }
    if !needs_approval {
        return None;
    }
    let contracts = has_phrase(tokens, CONTRACT_NOUNS);
    let payments = has_phrase(tokens, PAYMENT_NOUNS);
    if !contracts && !payments {
        return None;
    }
    let inclusive = has_phrase(tokens, INCLUSIVE_ABOVE);
    if !inclusive && !has_phrase(tokens, STRICT_ABOVE) {
        return None;
    }
    let limit = single_amount(sentence)?;
    Some(rule(
        RuleKind::ApprovalThreshold,
        Some(limit),
        inclusive,
        contracts,
        payments,
    ))
}

pub fn extract_rules(text: &str) -> Vec<ExtractedRule> {
    sentences(text)
        .into_iter()
        .filter_map(|(start, end)| {
            let sentence = &text[start..end];
            let mut rule = classify(sentence, &words(sentence))?;
            rule.byte_start = start + (sentence.len() - sentence.trim_start().len());
            rule.byte_end = start + sentence.trim_end().len();
            Some(rule)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn only(text: &str) -> ExtractedRule {
        let mut rules = extract_rules(text);
        assert_eq!(rules.len(), 1, "{text}: {rules:?}");
        rules.remove(0)
    }

    #[test]
    fn a_purchase_threshold_keeps_its_comparator() {
        let over = only("Purchases over $25,000 require council approval.");
        assert_eq!(over.kind, RuleKind::ApprovalThreshold);
        assert_eq!(over.limit, decimal("25000"));
        assert!(!over.inclusive && over.payments && !over.contracts);
        assert!(!over.exceeded_by(decimal("25000").unwrap()));
        assert!(over.exceeded_by(decimal("25000.01").unwrap()));
        let at_least = only("Contracts of $50,000 or more must be approved by the Board.");
        assert!(at_least.inclusive && at_least.contracts);
        assert!(at_least.exceeded_by(decimal("50000").unwrap()));
        assert!(!at_least.exceeded_by(decimal("49999.99").unwrap()));
    }

    #[test]
    fn card_overtime_and_splitting_statements_are_their_own_kinds() {
        let card = only("The purchasing card limit is $5,000 per transaction.");
        assert_eq!(card.kind, RuleKind::CardLimit);
        assert_eq!(card.limit, decimal("5000"));
        assert_eq!(
            only("Overtime requires prior authorization.").kind,
            RuleKind::OvertimeAuthorization
        );
        assert_eq!(
            only("Splitting a purchase to avoid the limit is prohibited.").kind,
            RuleKind::SplittingProhibited
        );
    }

    #[test]
    fn a_sentence_with_an_exception_or_two_figures_or_no_spending_noun_is_no_rule() {
        for text in [
            "Purchases over $25,000 require council approval unless an emergency is declared.",
            "Purchases over $25,000 do not require approval.",
            "Purchases between $5,000 and $25,000 require approval.",
            "Visitors over $25,000 require approval.",
            "Purchases require approval.",
            "Purchases over twenty-five thousand dollars require council approval.",
        ] {
            assert!(extract_rules(text).is_empty(), "{text}");
        }
    }

    #[test]
    fn a_decimal_point_does_not_end_the_sentence_and_offsets_cover_the_rule() {
        let text = "Section 5.2 applies. Payments over $1,250.50 require approval. Other text.";
        let rule = only(text);
        assert_eq!(rule.limit, decimal("1250.50"));
        assert_eq!(
            &text[rule.byte_start..rule.byte_end],
            "Payments over $1,250.50 require approval"
        );
    }
}
