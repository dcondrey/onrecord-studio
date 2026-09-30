//! Stated commitments that can be checked against something the corpus observed.
//!
//! Only a response-time SLA is extracted: it is the one commitment with a structured observation
//! to count against (a request's recorded turnaround). Fees, retention periods and exemption
//! lists are statements a document makes about itself, so comparing them is a contradiction between
//! two documents and stays in `assertion_index`, never a violation.

use crate::analysis::assertion_index::{extract_assertions, AssertionPredicate, AssertionValue};
use crate::analysis::phrase_match::words;

/// A calendar-day response time. A business-day SLA is dropped: elapsed calendar days cannot be
/// compared to it without a holiday calendar, and a guess would turn a compliant response into a
/// breach.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commitment {
    pub days: u32,
    pub byte_start: usize,
    pub byte_end: usize,
}

pub fn extract_commitments(text: &str) -> Vec<Commitment> {
    extract_assertions(text)
        .into_iter()
        .filter_map(|assertion| {
            let (AssertionPredicate::ResponseTimeCommitment, AssertionValue::Days(days)) =
                (assertion.predicate, assertion.value)
            else {
                return None;
            };
            let matched = text.get(assertion.byte_start..assertion.byte_end)?;
            if words(matched).iter().any(|word| word.text == "business") {
                return None;
            }
            Some(Commitment {
                days,
                byte_start: assertion.byte_start,
                byte_end: assertion.byte_end,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_business_day_sla_is_dropped_and_a_calendar_sla_survives() {
        assert!(
            extract_commitments("Response will be provided within 10 business days").is_empty()
        );
        let days: Vec<_> = extract_commitments("Response will be provided within 10 days")
            .into_iter()
            .map(|commitment| commitment.days)
            .collect();
        assert_eq!(days, [10]);
    }
}
