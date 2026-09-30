//! Change detection for saved watches. Pure: the repository observes, this module compares.
//!
//! Every dimension of a snapshot is a [`Slot`]: either measured at a time (possibly empty) or not
//! measured with a reason. An empty measured slot means "observed, nothing there"; a
//! `NotMeasured` slot means "no data this run". The two never share a representation, so a run
//! that could not observe a dimension cannot be read as "no change", and it never replaces the
//! baseline: the next measured run compares against the last value actually observed, stamped
//! with the time that value was observed.
//!
//! Nothing here is a legal claim. A document missing from a scope, a finding no longer emitted, or
//! a source whose last sync degraded each have several innocent explanations, and the `basis`
//! text on every change says which observation it rests on.
use super::monitoring::changed_facts;
use crate::{integrity::sha256_hex, types::*};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MAX_QUERY_TERMS: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum Slot<T> {
    #[serde(rename_all = "camelCase")]
    Measured { value: T, as_of: String },
    #[serde(rename_all = "camelCase")]
    NotMeasured { reason: String },
}

impl<T> Slot<T> {
    pub fn not_measured(reason: impl Into<String>) -> Self {
        Self::NotMeasured {
            reason: reason.into(),
        }
    }
    pub fn measured(value: T, as_of: impl Into<String>) -> Self {
        Self::Measured {
            value,
            as_of: as_of.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Labeled {
    pub label: String,
    pub portal_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FigureSource {
    pub portal_key: Option<String>,
    pub title: String,
    /// When this version of the source was captured by version monitoring.
    pub observed_at: String,
    pub facts: Vec<MonitoringFact>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceHealth {
    /// The last completed sync ran without errors.
    pub healthy: bool,
    pub detail: String,
    /// When that sync completed. A new sync run is the only thing that can change this.
    pub run_as_of: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WatchSnapshot {
    pub documents: Slot<BTreeMap<String, Labeled>>,
    pub findings: Slot<BTreeMap<String, Labeled>>,
    pub figures: Slot<BTreeMap<String, FigureSource>>,
    pub withheld: Slot<BTreeMap<String, Labeled>>,
    pub source_health: Slot<BTreeMap<String, SourceHealth>>,
}

impl WatchSnapshot {
    pub fn unobserved(reason: &str) -> Self {
        Self {
            documents: Slot::not_measured(reason),
            findings: Slot::not_measured(reason),
            figures: Slot::not_measured(reason),
            withheld: Slot::not_measured(reason),
            source_health: Slot::not_measured(reason),
        }
    }
}

pub struct DiffOutput {
    pub dimensions: Vec<DimensionResult>,
    pub changes: Vec<WatchChange>,
    /// The baseline to store: this run's value for every measured dimension, and the previous
    /// baseline, untouched, for every dimension this run could not observe.
    pub next: WatchSnapshot,
    pub status: WatchCheckStatus,
}

/// Lowercased, de-duplicated search terms. `None` when the text has no usable term or too many.
pub fn query_terms(text: &str) -> Option<Vec<String>> {
    let mut terms: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        let word = word.to_lowercase();
        if word.chars().count() < 2 {
            return None;
        }
        if !terms.contains(&word) {
            terms.push(word);
        }
    }
    (!terms.is_empty() && terms.len() <= MAX_QUERY_TERMS).then_some(terms)
}

struct Ctx<'a> {
    watch_id: &'a str,
    checked_at: &'a str,
}

impl Ctx<'_> {
    #[allow(clippy::too_many_arguments)]
    fn change(
        &self,
        dimension: WatchDimension,
        kind: &str,
        subject: &str,
        old: Option<&str>,
        new: Option<&str>,
        old_as_of: Option<&str>,
        new_as_of: &str,
        basis: &str,
        portal_key: Option<&str>,
    ) -> WatchChange {
        // The id excludes the new as-of time and the detection time, so the same difference read
        // from the same baseline twice is one row.
        let digest = sha256_hex(
            format!(
                "{}|{}|{}|{:?}|{:?}|{:?}",
                self.watch_id, kind, subject, old, new, old_as_of
            )
            .as_bytes(),
        );
        WatchChange {
            id: digest[..32].to_string(),
            watch_id: self.watch_id.to_string(),
            dimension,
            change_kind: kind.to_string(),
            subject: subject.to_string(),
            old_value: old.map(str::to_string),
            new_value: new.map(str::to_string),
            old_as_of: old_as_of.map(str::to_string),
            new_as_of: new_as_of.to_string(),
            basis: basis.to_string(),
            detected_at: self.checked_at.to_string(),
            portal_key: portal_key.map(str::to_string),
        }
    }
}

fn result(
    dimension: WatchDimension,
    outcome: DimensionOutcome,
    count: usize,
    baseline_as_of: Option<&str>,
    observed_as_of: Option<&str>,
    reason: Option<&str>,
) -> DimensionResult {
    DimensionResult {
        dimension,
        outcome,
        change_count: count.min(u32::MAX as usize) as u32,
        baseline_as_of: baseline_as_of.map(str::to_string),
        observed_as_of: observed_as_of.map(str::to_string),
        reason: reason.map(str::to_string),
    }
}

/// Compares the current observation with the stored baseline.
pub fn diff(
    watch_id: &str,
    baseline: &WatchSnapshot,
    current: &WatchSnapshot,
    checked_at: &str,
) -> DiffOutput {
    let ctx = Ctx {
        watch_id,
        checked_at,
    };
    let mut dimensions = Vec::new();
    let mut changes = Vec::new();

    let (r, c, documents) = labeled_dimension(
        &ctx,
        WatchDimension::Documents,
        &baseline.documents,
        &current.documents,
        [
            (
                "DOCUMENT_ADDED",
                "Present in the local corpus now and absent from the baseline observation.",
            ),
            (
                "DOCUMENT_NO_LONGER_LISTED",
                "Present in the baseline observation and absent from the local corpus now. A retention purge, a visibility change or a removed source each produce this; it does not show the agency withdrew the record.",
            ),
        ],
    );
    dimensions.push(r);
    changes.extend(c);

    let (r, c, findings) = labeled_dimension(
        &ctx,
        WatchDimension::Findings,
        &baseline.findings,
        &current.findings,
        [
            (
                "FINDING_ADDED",
                "Recorded in the findings table now and absent from the baseline observation.",
            ),
            (
                "FINDING_RESOLVED",
                "No longer emitted by its detector. The detector's input changed; this does not show the underlying condition was corrected. A dismissed finding is not resolved and stays listed.",
            ),
        ],
    );
    dimensions.push(r);
    changes.extend(c);

    let (r, c, figures) = figures_dimension(&ctx, &baseline.figures, &current.figures);
    dimensions.push(r);
    changes.extend(c);

    let (r, c, withheld) = labeled_dimension(
        &ctx,
        WatchDimension::Withheld,
        &baseline.withheld,
        &current.withheld,
        [
            (
                "NEWLY_REDACTED",
                "A heavy-redaction integrity flag is recorded for this request now and was not in the baseline observation. The flag is a structural signal about the released files, not a finding that redaction was improper.",
            ),
            (
                "REDACTION_FLAG_NO_LONGER_RECORDED",
                "A heavy-redaction flag in the baseline observation is no longer recorded, for example after a source purge or a repeated audit.",
            ),
        ],
    );
    dimensions.push(r);
    changes.extend(c);

    let (r, c, source_health) =
        source_dimension(&ctx, &baseline.source_health, &current.source_health);
    dimensions.push(r);
    changes.extend(c);

    let observed: Vec<_> = dimensions
        .iter()
        .filter(|d| d.outcome != DimensionOutcome::NotMeasured)
        .collect();
    let status = if observed.is_empty() {
        WatchCheckStatus::NoData
    } else if observed
        .iter()
        .all(|d| d.outcome == DimensionOutcome::BaselineRecorded)
    {
        WatchCheckStatus::Baseline
    } else {
        WatchCheckStatus::Compared
    };

    DiffOutput {
        dimensions,
        changes,
        next: WatchSnapshot {
            documents,
            findings,
            figures,
            withheld,
            source_health,
        },
        status,
    }
}

type Labels = BTreeMap<String, Labeled>;

fn labeled_dimension(
    ctx: &Ctx<'_>,
    dimension: WatchDimension,
    base: &Slot<Labels>,
    cur: &Slot<Labels>,
    bases: [(&str, &str); 2],
) -> (DimensionResult, Vec<WatchChange>, Slot<Labels>) {
    let (new_value, new_as_of) = match cur {
        Slot::NotMeasured { reason } => {
            let base_as_of = measured_as_of(base);
            return (
                result(
                    dimension,
                    DimensionOutcome::NotMeasured,
                    0,
                    base_as_of,
                    None,
                    Some(reason),
                ),
                vec![],
                base.clone(),
            );
        }
        Slot::Measured { value, as_of } => (value, as_of),
    };
    let Slot::Measured {
        value: old_value,
        as_of: old_as_of,
    } = base
    else {
        return (
            result(
                dimension,
                DimensionOutcome::BaselineRecorded,
                0,
                None,
                Some(new_as_of),
                None,
            ),
            vec![],
            cur.clone(),
        );
    };
    let mut changes = Vec::new();
    for (key, item) in new_value {
        if !old_value.contains_key(key) {
            changes.push(ctx.change(
                dimension,
                bases[0].0,
                key,
                None,
                Some(&item.label),
                Some(old_as_of),
                new_as_of,
                bases[0].1,
                item.portal_key.as_deref(),
            ));
        }
    }
    for (key, item) in old_value {
        if !new_value.contains_key(key) {
            changes.push(ctx.change(
                dimension,
                bases[1].0,
                key,
                Some(&item.label),
                None,
                Some(old_as_of),
                new_as_of,
                bases[1].1,
                item.portal_key.as_deref(),
            ));
        }
    }
    let outcome = if changes.is_empty() {
        DimensionOutcome::Unchanged
    } else {
        DimensionOutcome::Changed
    };
    (
        result(
            dimension,
            outcome,
            changes.len(),
            Some(old_as_of),
            Some(new_as_of),
            None,
        ),
        changes,
        cur.clone(),
    )
}

fn measured_as_of<T>(slot: &Slot<T>) -> Option<&str> {
    match slot {
        Slot::Measured { as_of, .. } => Some(as_of),
        Slot::NotMeasured { .. } => None,
    }
}

fn figures_dimension(
    ctx: &Ctx<'_>,
    base: &Slot<BTreeMap<String, FigureSource>>,
    cur: &Slot<BTreeMap<String, FigureSource>>,
) -> (
    DimensionResult,
    Vec<WatchChange>,
    Slot<BTreeMap<String, FigureSource>>,
) {
    let dimension = WatchDimension::Figures;
    let (new_value, new_as_of) = match cur {
        Slot::NotMeasured { reason } => {
            return (
                result(
                    dimension,
                    DimensionOutcome::NotMeasured,
                    0,
                    measured_as_of(base),
                    None,
                    Some(reason),
                ),
                vec![],
                base.clone(),
            )
        }
        Slot::Measured { value, as_of } => (value, as_of),
    };
    let Slot::Measured {
        value: old_value,
        as_of: old_as_of,
    } = base
    else {
        return (
            result(
                dimension,
                DimensionOutcome::BaselineRecorded,
                0,
                None,
                Some(new_as_of),
                None,
            ),
            vec![],
            cur.clone(),
        );
    };
    let mut changes = Vec::new();
    // Only a source present in both observations has figures to compare. A source that is new to
    // the scope is reported under documents, and its first captured figures are a baseline.
    for (url, now) in new_value {
        let Some(before) = old_value.get(url) else {
            continue;
        };
        for (category, old, new) in changed_facts(&before.facts, &now.facts) {
            let (kind, basis) = match (&old, &new) {
                (Some(_), Some(_)) => (
                    "FIGURE_CHANGED",
                    "A statement in this source that matched a figure pattern was replaced by a similar statement in a later captured version. Retrieval time is not publication or effective time.",
                ),
                (Some(_), None) => (
                    "FIGURE_NO_LONGER_STATED",
                    "A figure statement in the earlier captured version has no counterpart in the later version.",
                ),
                (None, _) => (
                    "FIGURE_ADDED",
                    "A figure statement appears in the later captured version with no counterpart in the earlier version.",
                ),
            };
            changes.push(ctx.change(
                dimension,
                kind,
                &format!("{url}#{category}"),
                old.as_deref(),
                new.as_deref(),
                Some(&before.observed_at),
                &now.observed_at,
                basis,
                now.portal_key.as_deref(),
            ));
        }
    }
    let outcome = if changes.is_empty() {
        DimensionOutcome::Unchanged
    } else {
        DimensionOutcome::Changed
    };
    (
        result(
            dimension,
            outcome,
            changes.len(),
            Some(old_as_of),
            Some(new_as_of),
            None,
        ),
        changes,
        cur.clone(),
    )
}

fn at(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

fn newer(candidate: &str, than: &str) -> bool {
    match (at(candidate), at(than)) {
        (Some(a), Some(b)) => a > b,
        _ => candidate > than,
    }
}

/// A source's health only changes when a new sync run completes. Without one since the baseline
/// there is no observation, so the dimension is `NotMeasured` rather than "still responding".
fn source_dimension(
    ctx: &Ctx<'_>,
    base: &Slot<BTreeMap<String, SourceHealth>>,
    cur: &Slot<BTreeMap<String, SourceHealth>>,
) -> (
    DimensionResult,
    Vec<WatchChange>,
    Slot<BTreeMap<String, SourceHealth>>,
) {
    let dimension = WatchDimension::SourceHealth;
    let (new_value, new_as_of) = match cur {
        Slot::NotMeasured { reason } => {
            return (
                result(
                    dimension,
                    DimensionOutcome::NotMeasured,
                    0,
                    measured_as_of(base),
                    None,
                    Some(reason),
                ),
                vec![],
                base.clone(),
            )
        }
        Slot::Measured { value, as_of } => (value, as_of),
    };
    let Slot::Measured {
        value: old_value,
        as_of: old_as_of,
    } = base
    else {
        return (
            result(
                dimension,
                DimensionOutcome::BaselineRecorded,
                0,
                None,
                Some(new_as_of),
                None,
            ),
            vec![],
            cur.clone(),
        );
    };
    let mut merged = old_value.clone();
    let mut changes = Vec::new();
    let mut compared = 0_usize;
    let mut first_seen = 0_usize;
    for (portal, now) in new_value {
        match old_value.get(portal) {
            None => {
                first_seen += 1;
                merged.insert(portal.clone(), now.clone());
            }
            Some(before) if newer(&now.run_as_of, &before.run_as_of) => {
                compared += 1;
                if before.healthy != now.healthy {
                    let (kind, basis) = if before.healthy {
                        (
                            "SOURCE_STOPPED_RESPONDING",
                            "The most recent completed sync for this source finished with errors after an earlier sync that finished cleanly. This reports the sync outcome, not why the source failed.",
                        )
                    } else {
                        (
                            "SOURCE_RESPONDING_AGAIN",
                            "The most recent completed sync for this source finished cleanly after an earlier sync that finished with errors.",
                        )
                    };
                    changes.push(ctx.change(
                        dimension,
                        kind,
                        portal,
                        Some(&describe_health(before)),
                        Some(&describe_health(now)),
                        Some(&before.run_as_of),
                        &now.run_as_of,
                        basis,
                        Some(portal),
                    ));
                }
                merged.insert(portal.clone(), now.clone());
            }
            Some(_) => {}
        }
    }
    let next_as_of = merged
        .values()
        .map(|h| h.run_as_of.as_str())
        .max_by(|a, b| {
            if newer(a, b) {
                std::cmp::Ordering::Greater
            } else if newer(b, a) {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .unwrap_or(new_as_of)
        .to_string();
    let next = Slot::measured(merged, next_as_of);
    if compared == 0 && first_seen == 0 {
        return (
            result(
                dimension,
                DimensionOutcome::NotMeasured,
                0,
                Some(old_as_of),
                None,
                Some("No sync run has completed for a source in scope since the baseline."),
            ),
            vec![],
            base.clone(),
        );
    }
    if compared == 0 {
        return (
            result(
                dimension,
                DimensionOutcome::BaselineRecorded,
                0,
                Some(old_as_of),
                Some(new_as_of),
                None,
            ),
            vec![],
            next,
        );
    }
    let outcome = if changes.is_empty() {
        DimensionOutcome::Unchanged
    } else {
        DimensionOutcome::Changed
    };
    (
        result(
            dimension,
            outcome,
            changes.len(),
            Some(old_as_of),
            Some(new_as_of),
            None,
        ),
        changes,
        next,
    )
}

fn describe_health(health: &SourceHealth) -> String {
    format!(
        "{} (sync completed {})",
        if health.healthy {
            "last sync completed cleanly".to_string()
        } else {
            format!("last sync completed with errors: {}", health.detail)
        },
        health.run_as_of
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(items: &[(&str, &str)]) -> Labels {
        items
            .iter()
            .map(|(k, v)| {
                (
                    k.to_string(),
                    Labeled {
                        label: v.to_string(),
                        portal_key: Some("city".into()),
                    },
                )
            })
            .collect()
    }

    fn snapshot_with_documents(slot: Slot<Labels>) -> WatchSnapshot {
        WatchSnapshot {
            documents: slot,
            ..WatchSnapshot::unobserved("test")
        }
    }

    fn documents(out: &DiffOutput) -> &DimensionResult {
        out.dimensions
            .iter()
            .find(|d| d.dimension == WatchDimension::Documents)
            .unwrap()
    }

    #[test]
    fn first_observation_records_a_baseline_and_reports_no_changes() {
        let current = snapshot_with_documents(Slot::measured(
            labels(&[("city:1", "a.pdf")]),
            "2026-10-01T00:00:00Z",
        ));
        let out = diff(
            "w",
            &WatchSnapshot::unobserved("none"),
            &current,
            "2026-10-01T00:00:00Z",
        );
        assert!(out.changes.is_empty());
        assert_eq!(documents(&out).outcome, DimensionOutcome::BaselineRecorded);
        assert_eq!(out.status, WatchCheckStatus::Baseline);
    }

    #[test]
    fn an_unobserved_run_is_not_no_change_and_does_not_advance_the_baseline() {
        let baseline = snapshot_with_documents(Slot::measured(
            labels(&[("city:1", "a.pdf")]),
            "2026-10-01T00:00:00Z",
        ));
        let missed = diff(
            "w",
            &baseline,
            &WatchSnapshot::unobserved("portal unreadable"),
            "2026-10-02T00:00:00Z",
        );
        assert_eq!(documents(&missed).outcome, DimensionOutcome::NotMeasured);
        assert_eq!(
            documents(&missed).reason.as_deref(),
            Some("portal unreadable")
        );
        assert_eq!(missed.status, WatchCheckStatus::NoData);
        assert_eq!(missed.next.documents, baseline.documents);

        let measured = snapshot_with_documents(Slot::measured(
            labels(&[("city:1", "a.pdf"), ("city:2", "b.pdf")]),
            "2026-10-03T00:00:00Z",
        ));
        let out = diff("w", &missed.next, &measured, "2026-10-03T00:00:00Z");
        assert_eq!(out.changes.len(), 1);
        let change = &out.changes[0];
        assert_eq!(change.change_kind, "DOCUMENT_ADDED");
        assert_eq!(change.old_value, None);
        assert_eq!(change.new_value.as_deref(), Some("b.pdf"));
        // Compared with the last observation, not with the missed run.
        assert_eq!(change.old_as_of.as_deref(), Some("2026-10-01T00:00:00Z"));
        assert_eq!(change.new_as_of, "2026-10-03T00:00:00Z");
    }

    #[test]
    fn rechecking_an_unchanged_observation_is_measured_unchanged_and_adds_nothing() {
        let baseline = snapshot_with_documents(Slot::measured(
            labels(&[("city:1", "a.pdf")]),
            "2026-10-01T00:00:00Z",
        ));
        let current = snapshot_with_documents(Slot::measured(
            labels(&[("city:1", "a.pdf"), ("city:2", "b.pdf")]),
            "2026-10-02T00:00:00Z",
        ));
        let first = diff("w", &baseline, &current, "2026-10-02T00:00:00Z");
        assert_eq!(first.changes.len(), 1);
        let again = diff("w", &first.next, &current, "2026-10-02T00:05:00Z");
        assert!(again.changes.is_empty());
        assert_eq!(documents(&again).outcome, DimensionOutcome::Unchanged);
    }

    #[test]
    fn the_same_difference_from_the_same_baseline_has_the_same_id() {
        let baseline = snapshot_with_documents(Slot::measured(labels(&[]), "2026-10-01T00:00:00Z"));
        let current = snapshot_with_documents(Slot::measured(
            labels(&[("city:2", "b.pdf")]),
            "2026-10-02T00:00:00Z",
        ));
        let a = diff("w", &baseline, &current, "2026-10-02T00:00:00Z");
        let b = diff("w", &baseline, &current, "2026-10-09T00:00:00Z");
        assert_eq!(a.changes[0].id, b.changes[0].id);
    }

    fn health(healthy: bool, run: &str) -> SourceHealth {
        SourceHealth {
            healthy,
            detail: "3 failures".into(),
            run_as_of: run.into(),
        }
    }

    fn sources(entries: &[(&str, SourceHealth)], as_of: &str) -> WatchSnapshot {
        WatchSnapshot {
            source_health: Slot::measured(
                entries
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.clone()))
                    .collect(),
                as_of,
            ),
            ..WatchSnapshot::unobserved("test")
        }
    }

    #[test]
    fn a_source_with_no_new_sync_is_not_reported_as_still_responding() {
        let baseline = sources(
            &[("city", health(true, "2026-10-01T00:00:00Z"))],
            "2026-10-01T00:00:00Z",
        );
        let out = diff("w", &baseline, &baseline, "2026-10-05T00:00:00Z");
        let d = out
            .dimensions
            .iter()
            .find(|d| d.dimension == WatchDimension::SourceHealth)
            .unwrap();
        assert_eq!(d.outcome, DimensionOutcome::NotMeasured);
        assert!(out.changes.is_empty());
    }

    #[test]
    fn a_newer_degraded_sync_after_a_clean_one_is_a_stopped_responding_change() {
        let baseline = sources(
            &[("city", health(true, "2026-10-01T00:00:00Z"))],
            "2026-10-01T00:00:00Z",
        );
        let current = sources(
            &[("city", health(false, "2026-10-04T00:00:00Z"))],
            "2026-10-04T00:00:00Z",
        );
        let out = diff("w", &baseline, &current, "2026-10-05T00:00:00Z");
        assert_eq!(out.changes.len(), 1);
        assert_eq!(out.changes[0].change_kind, "SOURCE_STOPPED_RESPONDING");
        assert_eq!(
            out.changes[0].old_as_of.as_deref(),
            Some("2026-10-01T00:00:00Z")
        );
        assert_eq!(out.changes[0].new_as_of, "2026-10-04T00:00:00Z");
        // Re-checking the same run afterwards finds nothing new to compare.
        let again = diff("w", &out.next, &current, "2026-10-05T01:00:00Z");
        assert!(again.changes.is_empty());
    }

    fn fact(category: &str, quote: &str) -> MonitoringFact {
        MonitoringFact {
            category: category.into(),
            quote: quote.into(),
            character_offset: 0,
        }
    }

    fn figures(url: &str, observed_at: &str, facts: Vec<MonitoringFact>) -> WatchSnapshot {
        WatchSnapshot {
            figures: Slot::measured(
                BTreeMap::from([(
                    url.to_string(),
                    FigureSource {
                        portal_key: Some("city".into()),
                        title: "Budget".into(),
                        observed_at: observed_at.into(),
                        facts,
                    },
                )]),
                observed_at,
            ),
            ..WatchSnapshot::unobserved("test")
        }
    }

    #[test]
    fn a_restated_figure_reports_old_and_new_values_with_their_capture_times() {
        let baseline = figures(
            "https://x.gov/b.pdf",
            "2026-10-01T00:00:00Z",
            vec![fact(
                "MONEY",
                "The program budget is $4 million for shelter beds",
            )],
        );
        let current = figures(
            "https://x.gov/b.pdf",
            "2026-10-03T00:00:00Z",
            vec![fact(
                "MONEY",
                "The program budget is $6 million for shelter beds",
            )],
        );
        let out = diff("w", &baseline, &current, "2026-10-04T00:00:00Z");
        assert_eq!(out.changes.len(), 1);
        let c = &out.changes[0];
        assert_eq!(c.change_kind, "FIGURE_CHANGED");
        assert!(c.old_value.as_deref().unwrap().contains("$4 million"));
        assert!(c.new_value.as_deref().unwrap().contains("$6 million"));
        assert_eq!(c.old_as_of.as_deref(), Some("2026-10-01T00:00:00Z"));
        assert_eq!(c.new_as_of, "2026-10-03T00:00:00Z");
    }

    #[test]
    fn query_terms_bound_count_and_length() {
        assert_eq!(query_terms("Sweep  Encampment sweep").unwrap().len(), 2);
        assert!(query_terms("").is_none());
        assert!(query_terms("a b").is_none());
        assert!(query_terms("aa bb cc dd ee ff gg hh").is_some());
        assert!(query_terms("aa bb cc dd ee ff gg hh ii").is_none());
    }
}
