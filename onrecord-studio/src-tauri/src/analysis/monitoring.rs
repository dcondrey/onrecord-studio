//! Deterministic, cited monitoring leads. A changed statement is not proof of a changed outcome.
use crate::types::monitoring::*;
use chrono::NaiveDate;
use regex::Regex;
use std::collections::BTreeSet;
use std::sync::LazyLock;

static MONEY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(\$\s*[0-9]|[0-9][0-9,.]*\s+(million|billion|dollars))").unwrap()
});
static DATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(\b20[0-9]{2}[-/][0-9]{1,2}[-/][0-9]{1,2}\b|\b(january|february|march|april|may|june|july|august|september|october|november|december)\s+[0-9]|\bFY\s*20[0-9]{2})").unwrap()
});
static OBLIGATION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(shall|must|required|committed|deadline|terminate|extend|deliver|obligation)\b",
    )
    .unwrap()
});
static OUTCOME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(placed|placements|permanent housing|exits|served|completed|closed|closure|operating hours|open hours|hours per week)\b").unwrap()
});

pub fn significant_facts(text: &str) -> Vec<MonitoringFact> {
    let mut offset = 0;
    let mut facts = Vec::new();
    for line in text.split_inclusive('\n') {
        let leading = line.chars().take_while(|c| c.is_whitespace()).count();
        let quote: String = line.trim().chars().take(900).collect();
        if quote.len() >= 12 {
            for (category, pattern) in [
                ("MONEY", &*MONEY),
                ("DATE", &*DATE),
                ("OBLIGATION", &*OBLIGATION),
                ("OUTCOME", &*OUTCOME),
            ] {
                if pattern.is_match(&quote) {
                    facts.push(MonitoringFact {
                        category: category.into(),
                        quote: quote.clone(),
                        character_offset: offset + leading,
                    });
                }
            }
        }
        offset += line.chars().count();
        if facts.len() >= 160 {
            break;
        }
    }
    facts.truncate(160);
    facts
}

fn words(text: &str) -> BTreeSet<String> {
    text.split(|c: char| !c.is_alphabetic())
        .filter(|s| s.len() > 2)
        .map(str::to_lowercase)
        .collect()
}

pub fn changed_facts(
    before: &[MonitoringFact],
    after: &[MonitoringFact],
) -> Vec<(String, Option<String>, Option<String>)> {
    let removed = before
        .iter()
        .filter(|old| {
            !after
                .iter()
                .any(|new| new.category == old.category && new.quote == old.quote)
        })
        .collect::<Vec<_>>();
    let added = after
        .iter()
        .filter(|new| {
            !before
                .iter()
                .any(|old| new.category == old.category && new.quote == old.quote)
        })
        .collect::<Vec<_>>();
    let mut paired = BTreeSet::new();
    let mut changes = Vec::new();
    for old in removed {
        let old_words = words(&old.quote);
        let candidate = added
            .iter()
            .enumerate()
            .filter(|(i, new)| !paired.contains(i) && old.category == new.category)
            .filter_map(|(i, new)| {
                let new_words = words(&new.quote);
                let union = old_words.union(&new_words).count().max(1);
                let overlap = old_words.intersection(&new_words).count();
                (overlap * 100 >= union * 60).then_some((overlap * 100 / union, i, *new))
            })
            .max_by_key(|(score, _, _)| *score);
        if let Some((_, index, new)) = candidate {
            paired.insert(index);
            changes.push((
                old.category.clone(),
                Some(old.quote.clone()),
                Some(new.quote.clone()),
            ));
        } else {
            changes.push((old.category.clone(), Some(old.quote.clone()), None));
        }
        if changes.len() >= 24 {
            break;
        }
    }
    for (i, new) in added.into_iter().enumerate() {
        if changes.len() >= 24 {
            break;
        }
        if !paired.contains(&i) {
            changes.push((new.category.clone(), None, Some(new.quote.clone())));
        }
    }
    changes
}

pub fn distance_meters(a: &ServiceObservation, b: &ServiceObservation) -> Option<f64> {
    let (lat1, lon1, lat2, lon2) = (a.latitude?, a.longitude?, b.latitude?, b.longitude?);
    let delta_lat = (lat2 - lat1).to_radians();
    let delta_lon = (lon2 - lon1).to_radians();
    let h = (delta_lat / 2.0).sin().powi(2)
        + lat1.to_radians().cos() * lat2.to_radians().cos() * (delta_lon / 2.0).sin().powi(2);
    Some(6_371_000.0 * 2.0 * h.sqrt().clamp(0.0, 1.0).asin())
}

pub fn service_comparisons(observations: &[ServiceObservation]) -> Vec<ServiceComparison> {
    let mut output = Vec::new();
    let reviewed = observations
        .iter()
        .filter(|o| o.reviewed)
        .collect::<Vec<_>>();
    for (index, left) in reviewed.iter().enumerate() {
        let Ok(left_date) = NaiveDate::parse_from_str(&left.observed_on, "%Y-%m-%d") else {
            continue;
        };
        for right in reviewed.iter().skip(index + 1) {
            let Ok(right_date) = NaiveDate::parse_from_str(&right.observed_on, "%Y-%m-%d") else {
                continue;
            };
            let days = (right_date - left_date).num_days().abs();
            if days == 0 || days > 90 || left.case_id != right.case_id {
                continue;
            }
            let distance = distance_meters(left, right);
            let same_place = left.place.eq_ignore_ascii_case(&right.place);
            if left.kind == "OPERATION"
                && right.kind == "OPERATION"
                && (distance.is_some_and(|d| d <= 100.0) || (same_place && distance.is_none()))
            {
                output.push(ServiceComparison {
                    kind: "REPEATED_OPERATION".into(), title: format!("Repeated documented operations near {}", left.place), observation_ids: vec![left.id.clone(), right.id.clone()],
                    calculation: format!("{} and {} are {days} days apart; {}.", left.observed_on, right.observed_on, distance.map(|d| format!("locations are {d:.0} meters apart")).unwrap_or_else(|| "same recorded place label; coordinates unavailable".into())),
                    implication: "Obtain operation identifiers, outreach referrals, property records and subsequent service outcomes to test whether these are separate operations at the same location.".into(),
                    limitations: vec!["Proximity and repeated dates do not establish repeated displacement of the same people or program failure.".into()],
                });
            }
            if left.kind == "FACILITY"
                && right.kind == "FACILITY"
                && same_place
                && distance.is_none_or(|d| d <= 50.0)
            {
                let (earlier, later) = if left_date < right_date {
                    (*left, *right)
                } else {
                    (*right, *left)
                };
                if let (Some(before), Some(after)) = (earlier.hours_per_week, later.hours_per_week)
                {
                    if (before - after).abs() > 0.01 {
                        output.push(ServiceComparison { kind: "ACCESS_HOURS_CHANGE".into(), title: format!("Published access changed at {}", left.place), observation_ids: vec![earlier.id.clone(), later.id.clone()], calculation: format!("{after:.1} − {before:.1} = {:+.1} hours/week between {} and {}.", after - before, earlier.observed_on, later.observed_on), implication: "Check staffing, closures, seasonal schedules and replacement facilities against the effective dates before attributing reduced access to a budget decision.".into(), limitations: vec!["Published hours do not establish actual availability, accessibility or reliability.".into()] });
                    }
                }
            }
            if output.len() >= 40 {
                return output;
            }
        }
        if left.kind == "OPERATION" && left.latitude.is_some() {
            let nearest = reviewed
                .iter()
                .filter(|f| f.kind == "FACILITY" && f.case_id == left.case_id)
                .filter_map(|f| {
                    let date = NaiveDate::parse_from_str(&f.observed_on, "%Y-%m-%d").ok()?;
                    let age = (left_date - date).num_days();
                    (0..=90)
                        .contains(&age)
                        .then(|| distance_meters(left, f).map(|d| (d, *f)))
                        .flatten()
                })
                .min_by(|(a, _), (b, _)| a.total_cmp(b));
            if let Some((distance, facility)) = nearest {
                output.push(ServiceComparison { kind: "NEAREST_DOCUMENTED_FACILITY".into(), title: format!("Documented service access near {}", left.place), observation_ids: vec![left.id.clone(), facility.id.clone()], calculation: format!("{} is {distance:.0} meters away in a straight line; facility record dated {}. Weekly hours: {}.", facility.place, facility.observed_on, facility.hours_per_week.map(|h|format!("{h:.1}")).unwrap_or_else(||"not recorded".into())), implication: "Verify walking routes, opening times, eligibility and physical access at the operation time. Seek missing facility inventory records before claiming a service desert.".into(), limitations: vec!["Nearest among reviewed records only. Straight-line distance is not a walking route; inventory coverage may be incomplete.".into()] });
            }
        }
    }
    output.truncate(40);
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pairs_changed_amount_and_obligation_without_treating_reordering_as_change() {
        let before = significant_facts("Grant allocation is $3,000,000.\nThe provider shall deliver 40 placements by September 1, 2026.\n");
        let reordered = significant_facts("The provider shall deliver 40 placements by September 1, 2026.\nGrant allocation is $3,000,000.\n");
        assert!(changed_facts(&before, &reordered).is_empty());
        let after = significant_facts("Grant allocation is $4,000,000.\nThe provider shall deliver 20 placements by September 1, 2027.\n");
        let changes = changed_facts(&before, &after);
        assert!(changes.iter().any(|(kind, old, new)| kind == "MONEY"
            && old.as_ref().unwrap().contains("3,000,000")
            && new.as_ref().unwrap().contains("4,000,000")));
        assert!(changes.iter().any(|(kind, _, _)| kind == "OBLIGATION"));
    }

    fn observation(id: &str, kind: &str, date: &str, hours: Option<f64>) -> ServiceObservation {
        ServiceObservation {
            id: id.into(),
            case_id: Some("riverbed".into()),
            topic: "abatement".into(),
            kind: kind.into(),
            place: "Public river trail".into(),
            observed_on: date.into(),
            latitude: Some(32.75),
            longitude: Some(-117.1),
            hours_per_week: hours,
            status: "RECORDED".into(),
            reviewed: true,
            evidence: MonitoringEvidence {
                portal_key: "city".into(),
                document_id: 1,
                title: "Fixture".into(),
                source_url: "https://www.sandiego.gov/fixture.csv".into(),
                source_sha256: "a".repeat(64),
                extraction_sha256: "b".repeat(64),
                quote: "Public river trail evidence".into(),
                character_offset: 0,
                snapshot_id: None,
                row_number: None,
            },
        }
    }
    #[test]
    fn public_access_comparisons_preserve_unknown_hours_and_do_not_promote_complaints() {
        let first = observation("a", "OPERATION", "2026-09-01", None);
        let second = observation("b", "OPERATION", "2026-09-15", None);
        let facility = observation("c", "FACILITY", "2026-08-31", None);
        let comparisons = service_comparisons(&[first.clone(), second.clone(), facility]);
        assert!(comparisons
            .iter()
            .any(|c| c.kind == "REPEATED_OPERATION" && c.calculation.contains("14 days")));
        assert!(comparisons
            .iter()
            .any(|c| c.kind == "NEAREST_DOCUMENTED_FACILITY"
                && c.calculation.contains("not recorded")));
        let mut complaint = second.clone();
        complaint.kind = "OBSERVATION".into();
        assert!(service_comparisons(&[first.clone(), complaint]).is_empty());
        let mut unreviewed = second.clone();
        unreviewed.reviewed = false;
        assert!(service_comparisons(&[first.clone(), unreviewed]).is_empty());
        let mut other_case = second;
        other_case.case_id = Some("other".into());
        assert!(service_comparisons(&[first, other_case]).is_empty());
        let hours = service_comparisons(&[
            observation("d", "FACILITY", "2026-09-01", Some(56.0)),
            observation("e", "FACILITY", "2026-09-15", Some(28.0)),
        ]);
        assert!(
            hours
                .iter()
                .any(|c| c.kind == "ACCESS_HOURS_CHANGE"
                    && c.calculation.contains("-28.0 hours/week"))
        );
    }
}
