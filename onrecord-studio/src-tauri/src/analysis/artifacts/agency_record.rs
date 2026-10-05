//! Counted facts about one agency's closed requests in the sender's library, for a letter.
//!
//! An agency is `(portal, agency id)`: an id is only unique within its portal, so two portals that
//! both number an agency 7 are two agencies. Every count carries its denominator and the period it
//! covers. Below [`MINIMUM_CLOSED_REQUESTS`] no paragraph is produced at all; a zero is printed
//! only where requests were observed and none met the condition.

use serde::{Deserialize, Serialize};

/// Fewer closed requests than this and the paragraph is omitted. With ten, one request moves any
/// share by at most a tenth; below that a count reads as a pattern while describing a handful of
/// individually identifiable requests.
pub const MINIMUM_CLOSED_REQUESTS: u64 = 10;

/// The agency whose record a letter may cite, chosen by the user and never inferred from a name.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgencyIdentity {
    pub portal_key: String,
    pub agency_id: i64,
}

/// An agency the library holds enough closed requests for.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgencyRecordOption {
    pub portal_key: String,
    pub agency_id: i64,
    pub agency_name: Option<String>,
    pub closed_requests: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgencyRecord {
    pub portal_key: String,
    pub agency_id: i64,
    pub agency_name: Option<String>,
    /// Requests assigned to the agency with a recorded closure date: the outer denominator.
    pub closed_requests: u64,
    /// Of those, the ones whose determination date was recorded with a source and whose extension
    /// record is absent or valid. Only these can be counted against the deadline.
    pub sourced_determinations: u64,
    /// Of `sourced_determinations`, those dated after the computed deadline.
    pub late_sourced_determinations: u64,
    /// Of `closed_requests`, those whose stored closure notice states no responsive records.
    pub no_records_closures: u64,
    /// Of `no_records_closures`, those with an undismissed `NoRecordsContradictedByRelease`
    /// finding. That detector records an *earlier* release close in subject and wording.
    pub no_records_with_recorded_earlier_release: u64,
    /// Earliest and latest received date among `closed_requests`; `None` when there are none.
    pub period: Option<(String, String)>,
}

impl AgencyRecord {
    /// The paragraph for a letter, or `None` when too few closed requests were observed.
    pub fn paragraph(&self) -> Option<String> {
        if self.closed_requests < MINIMUM_CLOSED_REQUESTS {
            return None;
        }
        let (from, to) = self.period.as_ref()?;
        let n = self.closed_requests;
        let agency = self
            .agency_name
            .clone()
            .unwrap_or_else(|| format!("agency {}", self.agency_id));
        let determinations = if self.sourced_determinations == 0 {
            format!(
                "None of those {n} carries a determination date recorded with its source, so no \
                 count of determinations against the deadline is stated."
            )
        } else {
            let m = self.sourced_determinations;
            let unsourced = n - m;
            let rest = if unsourced == 0 {
                String::new()
            } else {
                format!(
                    " The other {unsourced} carry no determination date recorded with a source \
                     and are counted neither way."
                )
            };
            format!(
                "{m} of those {n} carry a determination date recorded with its source; {late} of \
                 those {m} are dated after the deadline computed under the rule stated above.{rest}",
                late = self.late_sourced_determinations,
            )
        };
        let no_records = if self.no_records_closures == 0 {
            String::new()
        } else {
            format!(
                " {k} of the {n} have a stored closure notice stating that there were no \
                 responsive records; for {c} of those {k}, my library records an earlier release \
                 by the same agency that is close to the request in subject and wording. That \
                 comparison measures similarity of text and does not establish that the earlier \
                 release was responsive.",
                k = self.no_records_closures,
                c = self.no_records_with_recorded_earlier_release,
            )
        };
        Some(format!(
            "For context, the following counts come only from the requests held in my own library \
             of records published on the {portal} portal, not from all requests the agency \
             received. The library holds {n} closed requests assigned to {agency}, received \
             between {from} and {to}. {determinations}{no_records}",
            portal = self.portal_key,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn record(closed: u64, sourced: u64, late: u64) -> AgencyRecord {
        AgencyRecord {
            portal_key: "sd".into(),
            agency_id: 7,
            agency_name: Some("Purchasing".into()),
            closed_requests: closed,
            sourced_determinations: sourced,
            late_sourced_determinations: late,
            no_records_closures: 0,
            no_records_with_recorded_earlier_release: 0,
            period: (closed > 0).then(|| ("2025-01-02".to_string(), "2026-03-04".to_string())),
        }
    }

    #[test]
    fn the_paragraph_exists_only_from_the_minimum_number_of_closed_requests() {
        assert!(record(MINIMUM_CLOSED_REQUESTS - 1, 9, 9)
            .paragraph()
            .is_none());
        for n in [MINIMUM_CLOSED_REQUESTS, MINIMUM_CLOSED_REQUESTS + 1] {
            let text = record(n, 4, 3)
                .paragraph()
                .expect("at or above the minimum");
            assert!(text.contains(&format!("holds {n} closed requests assigned to Purchasing")));
            assert!(text.contains(&format!("4 of those {n} carry")));
            assert!(text.contains("3 of those 4 are dated after the deadline"));
            assert!(text.contains(&format!("The other {} carry no determination", n - 4)));
            assert!(text.contains("between 2025-01-02 and 2026-03-04"));
            assert!(text.contains("not from all requests the agency received"));
        }
    }

    #[test]
    fn an_observed_zero_is_stated_with_its_denominator_and_no_observation_is_omitted() {
        let observed = record(12, 12, 0).paragraph().expect("twelve observed");
        assert!(observed.contains("0 of those 12 are dated after the deadline"));
        // Twelve closed requests, none with a sourced determination: nothing was measured, so no
        // zero is printed for it.
        let unmeasured = record(12, 0, 0).paragraph().expect("twelve observed");
        assert!(unmeasured.contains("None of those 12 carries a determination date"));
        assert!(!unmeasured.contains("0 of"));
        assert!(record(0, 0, 0).paragraph().is_none());
    }

    #[test]
    fn no_records_closures_are_stated_only_when_some_were_observed() {
        let mut with = record(12, 5, 1);
        assert!(!with.paragraph().unwrap().contains("no responsive records"));
        with.no_records_closures = 3;
        let text = with.paragraph().unwrap();
        assert!(text.contains("3 of the 12 have a stored closure notice"));
        assert!(text.contains("for 0 of those 3"));
        assert!(text.contains("does not establish that the earlier release was responsive"));
    }
}
