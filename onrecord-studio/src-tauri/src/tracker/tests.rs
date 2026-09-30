use super::deadline::request_deadline;
use super::events::{self, ReadEmail};
use super::store::{TrackerStore, TRACKER_FILE_NAME};
use super::*;
use crate::analysis::finding::Finding;
use crate::analysis::statute::DeterminationBasis;
use crate::types::StatutoryExtensionReason;
use chrono::Utc;
use std::io::Write;

const NOW: &str = "2026-06-01T00:00:00Z";

fn at(day: &str) -> chrono::DateTime<Utc> {
    chrono::DateTime::parse_from_rfc3339(&format!("{day}T12:00:00Z"))
        .unwrap()
        .with_timezone(&Utc)
}

fn claim_input(reference: &str) -> ClaimRequestInput {
    ClaimRequestInput {
        portal_key: None,
        reference: reference.to_string(),
        title: None,
        source_url: None,
        jurisdiction: Some(CALIFORNIA_CPRA_JURISDICTION.to_string()),
    }
}

fn tracked(document: &mut TrackerDocument) -> String {
    claim_request(document, claim_input("26-114"), &[], NOW)
        .unwrap()
        .id
}

fn event_input(
    claim_id: &str,
    kind: TimelineEventKind,
    occurred_at: &str,
) -> AddTimelineEventInput {
    AddTimelineEventInput {
        claim_id: claim_id.to_string(),
        kind,
        occurred_at: occurred_at.to_string(),
        extension_reason: None,
        expected_determination_at: None,
    }
}

fn add(document: &mut TrackerDocument, id: &str, kind: TimelineEventKind, when: &str) {
    add_event(document, event_input(id, kind, when), NOW).unwrap();
}

fn email(id: Option<&str>, date: Option<&str>, body: &str) -> ReadEmail {
    ReadEmail {
        file_name: "notice.eml".to_string(),
        message_id: id.map(str::to_string),
        subject: Some("Your request 26-114".to_string()),
        email_date: date.map(str::to_string),
        body_text: body.to_string(),
        notes: Vec::new(),
    }
}

/// Imports `body` as one email and confirms its first suggestion of `kind` at `when`.
#[allow(clippy::too_many_arguments)]
fn confirm_from_email(
    document: &mut TrackerDocument,
    claim_id: &str,
    id: &str,
    body: &str,
    kind: TimelineEventKind,
    when: &str,
    reason: Option<StatutoryExtensionReason>,
    expected: Option<&str>,
) -> Result<TimelineEvent, AppError> {
    import_email(document, claim_id, &email(Some(id), Some(when), body), NOW).unwrap();
    let suggestion = claim_mut(document, claim_id)
        .unwrap()
        .suggestions
        .iter()
        .find(|suggestion| suggestion.kind == kind)
        .expect("a suggestion of that kind")
        .id
        .clone();
    confirm_suggestion(
        document,
        ConfirmSuggestedEventInput {
            claim_id: claim_id.to_string(),
            suggestion_id: suggestion,
            kind,
            occurred_at: when.to_string(),
            extension_reason: reason,
            expected_determination_at: expected.map(str::to_string),
        },
        NOW,
    )
}

// ---- deadline ----

#[test]
fn a_suggested_event_never_affects_the_deadline() {
    let mut document = TrackerDocument::default();
    let id = tracked(&mut document);
    add(
        &mut document,
        &id,
        TimelineEventKind::Submitted,
        "2026-01-01T09:00:00Z",
    );
    let before = request_deadline(&document.claims[0], at("2026-01-20"));

    import_email(
        &mut document,
        &id,
        &email(
            Some("<a@x>"),
            Some("2026-01-05T00:00:00Z"),
            "We have received your request. The determination is that records are available. \
             We need an additional 14 days. This request is closed.",
        ),
        NOW,
    )
    .unwrap();
    let claim = &document.claims[0];
    assert!(claim.suggestions.len() >= 4);
    assert!(claim
        .events
        .iter()
        .all(|event| event.kind == TimelineEventKind::Submitted));

    let after = request_deadline(claim, at("2026-01-20"));
    assert_eq!(before.analysis, after.analysis);
    assert_eq!(before.countdown, after.countdown);
    assert_eq!(before.basis_statement, after.basis_statement);
    assert_eq!(
        after.determination_basis,
        Some(DeterminationBasis::ObservationWithoutRecordedActivity)
    );
    assert_eq!(
        after.pending_suggestion_count as usize,
        claim.suggestions.len()
    );
}

#[test]
fn a_proxy_determination_never_yields_a_violation() {
    let mut document = TrackerDocument::default();
    let id = tracked(&mut document);
    add(
        &mut document,
        &id,
        TimelineEventKind::Submitted,
        "2026-01-01T09:00:00Z",
    );
    add(
        &mut document,
        &id,
        TimelineEventKind::Determination,
        "2026-03-01T09:00:00Z",
    );
    let deadline = request_deadline(&document.claims[0], at("2026-06-01"));

    assert_eq!(
        deadline.determination_basis,
        Some(DeterminationBasis::EarliestRecordedActivityProxy)
    );
    // The evaluator itself measures 59 days against 10; the gate is what withholds the word.
    assert!(deadline.analysis.as_ref().unwrap().is_violation);
    assert!(!deadline.violation_asserted);
    assert!(!deadline
        .basis_statement
        .to_lowercase()
        .contains("violation is counted"));
    assert!(deadline.basis_statement.contains("proxy"));
    assert_eq!(
        deadline.countdown.kind,
        CountdownKind::DeterminationRecorded
    );
}

#[test]
fn a_confirmed_sourced_determination_beyond_the_allowed_days_is_a_violation() {
    let mut document = TrackerDocument::default();
    let id = tracked(&mut document);
    add(
        &mut document,
        &id,
        TimelineEventKind::Submitted,
        "2026-01-01T09:00:00Z",
    );
    let late = confirm_from_email(
        &mut document,
        &id,
        "<det@x>",
        "The determination on your request is attached.",
        TimelineEventKind::Determination,
        "2026-01-12T09:00:00Z",
        None,
        None,
    )
    .unwrap();
    assert_eq!(late.basis, EventBasis::EmailConfirmed);
    let deadline = request_deadline(&document.claims[0], at("2026-06-01"));
    assert_eq!(
        deadline.determination_basis,
        Some(DeterminationBasis::SourcedDeterminationEvent)
    );
    assert!(deadline.violation_asserted);
    assert!(deadline.basis_statement.contains("violation"));

    // Day 10 is inside the period (limit); day 11 is one past it.
    let mut on_time = TrackerDocument::default();
    let id = tracked(&mut on_time);
    add(
        &mut on_time,
        &id,
        TimelineEventKind::Submitted,
        "2026-01-01T09:00:00Z",
    );
    confirm_from_email(
        &mut on_time,
        &id,
        "<det2@x>",
        "The determination on your request is attached.",
        TimelineEventKind::Determination,
        "2026-01-11T09:00:00Z",
        None,
        None,
    )
    .unwrap();
    assert!(!request_deadline(&on_time.claims[0], at("2026-06-01")).violation_asserted);
}

#[test]
fn an_extension_needs_a_reason_on_every_path_that_can_save_one() {
    let mut document = TrackerDocument::default();
    let id = tracked(&mut document);
    let before = document.clone();
    assert!(add_event(
        &mut document,
        event_input(
            &id,
            TimelineEventKind::ExtensionNotice,
            "2026-01-05T00:00:00Z"
        ),
        NOW
    )
    .is_err());

    import_email(
        &mut document,
        &id,
        &email(
            Some("<ext@x>"),
            Some("2026-01-05T00:00:00Z"),
            "We need an extension of time.",
        ),
        NOW,
    )
    .unwrap();
    let suggestion = document.claims[0]
        .suggestions
        .iter()
        .find(|suggestion| suggestion.kind == TimelineEventKind::ExtensionNotice)
        .unwrap()
        .id
        .clone();
    let confirm = |reason| ConfirmSuggestedEventInput {
        claim_id: id.clone(),
        suggestion_id: suggestion.clone(),
        kind: TimelineEventKind::ExtensionNotice,
        occurred_at: "2026-01-05T00:00:00Z".to_string(),
        extension_reason: reason,
        expected_determination_at: None,
    };
    let held = document.clone();
    assert!(confirm_suggestion(&mut document, confirm(None), NOW).is_err());
    assert_eq!(document, held, "a refused confirmation changes nothing");
    assert!(confirm_suggestion(
        &mut document,
        confirm(Some(StatutoryExtensionReason::VoluminousRecords)),
        NOW
    )
    .is_ok());

    // A reason on some other kind of event is also refused.
    let mut input = event_input(&id, TimelineEventKind::Submitted, "2026-01-01T00:00:00Z");
    input.extension_reason = Some(StatutoryExtensionReason::DataCompilation);
    assert!(add_event(&mut document, input, NOW).is_err());
    assert_eq!(before.claims[0].events.len(), 0);
}

#[test]
fn only_an_email_sourced_extension_extends_the_deadline() {
    let build = |from_email: bool| {
        let mut document = TrackerDocument::default();
        let id = tracked(&mut document);
        add(
            &mut document,
            &id,
            TimelineEventKind::Submitted,
            "2026-01-01T09:00:00Z",
        );
        if from_email {
            confirm_from_email(
                &mut document,
                &id,
                "<ext@x>",
                "We need an extension of time.",
                TimelineEventKind::ExtensionNotice,
                "2026-01-08T09:00:00Z",
                Some(StatutoryExtensionReason::VoluminousRecords),
                Some("2026-01-20T09:00:00Z"),
            )
            .unwrap();
        } else {
            let mut input = event_input(
                &id,
                TimelineEventKind::ExtensionNotice,
                "2026-01-08T09:00:00Z",
            );
            input.extension_reason = Some(StatutoryExtensionReason::VoluminousRecords);
            input.expected_determination_at = Some("2026-01-20T09:00:00Z".to_string());
            add_event(&mut document, input, NOW).unwrap();
        }
        request_deadline(&document.claims[0], at("2026-01-15"))
    };
    let sourced = build(true);
    assert_eq!(sourced.analysis.as_ref().unwrap().allowed_calendar_days, 19);
    assert_eq!(
        sourced.countdown,
        Countdown {
            kind: CountdownKind::DaysRemaining,
            days: Some(5)
        }
    );
    let entered = build(false);
    assert_eq!(entered.analysis.as_ref().unwrap().allowed_calendar_days, 10);
    assert!(entered
        .notes
        .iter()
        .any(|note| note.contains("no email source")));
}

#[test]
fn the_countdown_crosses_zero_without_calling_the_provisional_period_a_violation() {
    let mut document = TrackerDocument::default();
    let id = tracked(&mut document);
    add(
        &mut document,
        &id,
        TimelineEventKind::Submitted,
        "2026-01-01T00:00:00Z",
    );
    let claim = &document.claims[0];
    let at_day = |day| request_deadline(claim, at(day));
    assert_eq!(
        at_day("2026-01-10").countdown,
        Countdown {
            kind: CountdownKind::DaysRemaining,
            days: Some(1)
        }
    );
    assert_eq!(
        at_day("2026-01-11").countdown,
        Countdown {
            kind: CountdownKind::DaysRemaining,
            days: Some(0)
        }
    );
    let passed = at_day("2026-01-12");
    assert_eq!(
        passed.countdown,
        Countdown {
            kind: CountdownKind::DeadlinePassed,
            days: Some(1)
        }
    );
    assert!(!passed.violation_asserted);
    assert!(passed.basis_statement.contains("provisional"));
}

#[test]
fn absence_is_stated_never_defaulted() {
    let mut document = TrackerDocument::default();
    tracked(&mut document);
    let none = request_deadline(&document.claims[0], at("2026-01-15"));
    assert_eq!(none.countdown.kind, CountdownKind::NoSubmissionDate);
    assert!(none.analysis.is_none());

    for jurisdiction in [None, Some("TX_TPIA".to_string())] {
        let mut document = TrackerDocument::default();
        let id = claim_request(
            &mut document,
            ClaimRequestInput {
                jurisdiction,
                ..claim_input("26-114")
            },
            &[],
            NOW,
        )
        .unwrap()
        .id;
        add(
            &mut document,
            &id,
            TimelineEventKind::Submitted,
            "2026-01-01T00:00:00Z",
        );
        let deadline = request_deadline(&document.claims[0], at("2026-01-15"));
        assert_eq!(deadline.model, DeadlineModel::NoModel);
        assert_eq!(deadline.countdown.kind, CountdownKind::NoDeadlineModel);
        assert!(deadline
            .basis_statement
            .starts_with("No deadline model for this jurisdiction."));
        assert!(deadline.analysis.is_none());
    }
}

#[test]
fn contradictory_dates_are_reported_not_evaluated() {
    let mut document = TrackerDocument::default();
    let id = tracked(&mut document);
    add(
        &mut document,
        &id,
        TimelineEventKind::Submitted,
        "2026-01-10T00:00:00Z",
    );
    add(
        &mut document,
        &id,
        TimelineEventKind::Determination,
        "2026-01-05T00:00:00Z",
    );
    let deadline = request_deadline(&document.claims[0], at("2026-02-01"));
    assert_eq!(deadline.countdown.kind, CountdownKind::Unavailable);
    assert!(deadline.analysis.is_none());
    let future = request_deadline(&document.claims[0], at("2026-01-01"));
    assert_eq!(future.countdown.kind, CountdownKind::Unavailable);
}

// ---- limits and validation ----

#[test]
fn every_limit_accepts_the_limit_and_rejects_one_past_it() {
    let repeat = |count: usize| "é".repeat(count);
    let with = |reference: &str,
                title: Option<String>,
                url: Option<String>,
                jurisdiction: Option<String>| {
        let mut document = TrackerDocument::default();
        claim_request(
            &mut document,
            ClaimRequestInput {
                portal_key: None,
                reference: reference.to_string(),
                title,
                source_url: url,
                jurisdiction,
            },
            &[],
            NOW,
        )
    };
    for (accepted, rejected) in [
        (
            with(&repeat(MAXIMUM_REFERENCE_CHARACTERS), None, None, None),
            with(&repeat(MAXIMUM_REFERENCE_CHARACTERS + 1), None, None, None),
        ),
        (
            with("r", Some(repeat(MAXIMUM_TITLE_CHARACTERS)), None, None),
            with("r", Some(repeat(MAXIMUM_TITLE_CHARACTERS + 1)), None, None),
        ),
        (
            with(
                "r",
                None,
                None,
                Some(repeat(MAXIMUM_JURISDICTION_CHARACTERS)),
            ),
            with(
                "r",
                None,
                None,
                Some(repeat(MAXIMUM_JURISDICTION_CHARACTERS + 1)),
            ),
        ),
    ] {
        assert!(accepted.is_ok());
        assert!(rejected.is_err());
    }
    let host_path = |length: usize| {
        let prefix = "https://portal.example/";
        format!("{prefix}{}", "a".repeat(length - prefix.len()))
    };
    assert!(with(
        "r",
        None,
        Some(host_path(MAXIMUM_SOURCE_URL_CHARACTERS)),
        None
    )
    .is_ok());
    assert!(with(
        "r",
        None,
        Some(host_path(MAXIMUM_SOURCE_URL_CHARACTERS + 1)),
        None
    )
    .is_err());

    // Claims: 199 -> 200 accepted, the 201st refused.
    let mut document = TrackerDocument::default();
    for index in 0..MAXIMUM_CLAIMED_REQUESTS {
        claim_request(&mut document, claim_input(&format!("r{index}")), &[], NOW).unwrap();
    }
    assert!(claim_request(&mut document, claim_input("one-more"), &[], NOW).is_err());

    // Events: the 500th is accepted, the 501st refused.
    let mut document = TrackerDocument::default();
    let id = tracked(&mut document);
    for _ in 0..MAXIMUM_EVENTS_PER_REQUEST - 1 {
        add(
            &mut document,
            &id,
            TimelineEventKind::Other,
            "2026-01-01T00:00:00Z",
        );
    }
    assert!(add_event(
        &mut document,
        event_input(&id, TimelineEventKind::Other, "2026-01-01T00:00:00Z"),
        NOW
    )
    .is_ok());
    assert!(add_event(
        &mut document,
        event_input(&id, TimelineEventKind::Other, "2026-01-01T00:00:00Z"),
        NOW
    )
    .is_err());
}

#[test]
fn text_is_trimmed_and_controls_urls_portals_and_dates_are_refused() {
    let mut document = TrackerDocument::default();
    let mut input = claim_input("  26-114  ");
    let claim = claim_request(&mut document, input.clone(), &[], NOW).unwrap();
    assert_eq!(claim.reference, "26-114");
    assert!(
        claim_request(&mut document, claim_input("26-114"), &[], NOW).is_err(),
        "duplicate"
    );

    for bad in ["a\nb", "a\u{0007}b", "a\tb", "   "] {
        input.reference = bad.to_string();
        assert!(
            claim_request(&mut document, input.clone(), &[], NOW).is_err(),
            "{bad:?}"
        );
    }
    input.reference = "ok".to_string();
    for url in [
        "http://portal.example/x",
        "ftp://portal.example",
        "https://user:pw@portal.example/",
        "not a url",
        "javascript:alert(1)",
    ] {
        input.source_url = Some(url.to_string());
        assert!(
            claim_request(&mut document, input.clone(), &[], NOW).is_err(),
            "{url}"
        );
    }
    input.source_url = Some("https://portal.example/requests/26-114".to_string());
    input.portal_key = Some("unregistered".to_string());
    assert!(claim_request(&mut document, input.clone(), &["known".to_string()], NOW).is_err());
    input.portal_key = Some("known".to_string());
    assert!(claim_request(&mut document, input, &["known".to_string()], NOW).is_ok());

    let id = document.claims[0].id.clone();
    for bad in ["2026-01-01", "yesterday", "2026-13-01T00:00:00Z", ""] {
        assert!(
            add_event(
                &mut document,
                event_input(&id, TimelineEventKind::Submitted, bad),
                NOW
            )
            .is_err(),
            "{bad}"
        );
    }
}

#[test]
fn a_document_at_every_count_limit_survives_a_save_and_a_load() {
    let directory = tempfile::tempdir().unwrap();
    let store = TrackerStore::new(directory.path());
    store
        .update(|document| {
            for index in 0..MAXIMUM_CLAIMED_REQUESTS {
                let id = claim_request(document, claim_input(&format!("r{index}")), &[], NOW)?.id;
                for _ in 0..MAXIMUM_EVENTS_PER_REQUEST {
                    add_event(
                        document,
                        event_input(&id, TimelineEventKind::Other, "2026-01-01T00:00:00Z"),
                        NOW,
                    )?;
                }
            }
            Ok(())
        })
        .unwrap();
    let loaded = store.load().unwrap();
    assert_eq!(loaded.claims.len(), MAXIMUM_CLAIMED_REQUESTS);
    assert!(loaded
        .claims
        .iter()
        .all(|claim| claim.events.len() == MAXIMUM_EVENTS_PER_REQUEST));
}

// ---- storage ----

#[test]
fn a_failed_write_leaves_the_previous_file_intact_and_no_temp_file_behind() {
    let directory = tempfile::tempdir().unwrap();
    let store = TrackerStore::new(directory.path());
    store
        .update(|document| claim_request(document, claim_input("keep"), &[], NOW))
        .unwrap();
    let before = std::fs::read(store.path()).unwrap();

    let failure =
        crate::atomic_write::write_atomic_with(store.path(), "the request tracker", |file| {
            file.write_all(b"half of the new")?;
            Err(std::io::Error::other("disk full"))
        });
    assert!(failure.is_err());
    assert_eq!(std::fs::read(store.path()).unwrap(), before);
    let names: Vec<_> = std::fs::read_dir(directory.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, [std::ffi::OsString::from(TRACKER_FILE_NAME)]);
    assert_eq!(store.load().unwrap().claims[0].reference, "keep");
}

#[test]
fn a_write_past_the_size_limit_is_refused_and_leaves_the_file_readable() {
    let directory = tempfile::tempdir().unwrap();
    let probe = TrackerStore::new(directory.path());
    probe
        .update(|document| claim_request(document, claim_input("first"), &[], NOW))
        .unwrap();
    let size = std::fs::metadata(probe.path()).unwrap().len();
    let before = std::fs::read(probe.path()).unwrap();

    let tight = TrackerStore::with_maximum_bytes(directory.path(), size);
    assert!(tight
        .update(|document| claim_request(document, claim_input("second"), &[], NOW))
        .is_err());
    assert_eq!(std::fs::read(tight.path()).unwrap(), before);
    assert_eq!(tight.load().unwrap().claims.len(), 1);
    // At exactly the limit a same-size rewrite is accepted; one byte under refuses a read.
    assert!(tight
        .update(|document| update_claim(
            document,
            UpdateClaimedRequestInput {
                claim_id: document.claims[0].id.clone(),
                portal_key: None,
                reference: "first".to_string(),
                title: None,
                source_url: None,
                jurisdiction: Some(CALIFORNIA_CPRA_JURISDICTION.to_string()),
            },
            &[]
        ))
        .is_ok());
    assert!(TrackerStore::with_maximum_bytes(directory.path(), size - 1)
        .load()
        .is_err());
}

#[test]
fn a_corrupt_or_foreign_file_is_an_error_not_an_empty_tracker() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(TRACKER_FILE_NAME);
    let store = TrackerStore::new(directory.path());
    assert!(store.load().unwrap().claims.is_empty());
    for content in [&b"{ not json"[..], br#"{"schemaVersion":99,"claims":[]}"#] {
        std::fs::write(&path, content).unwrap();
        assert!(store.load().is_err());
        assert!(
            store.update(|_| Ok(())).is_err(),
            "an unreadable file is never overwritten"
        );
        assert_eq!(std::fs::read(&path).unwrap(), content);
    }
}

// ---- email import ----

#[test]
fn an_email_is_imported_once_and_a_rejected_suggestion_does_not_return() {
    let mut document = TrackerDocument::default();
    let id = tracked(&mut document);
    let first = email(
        Some("<Same@X>"),
        Some("2026-01-05T00:00:00Z"),
        "This request is closed.",
    );
    let outcome = import_email(&mut document, &id, &first, NOW).unwrap();
    assert_eq!(outcome.outcome, EmailImportOutcome::Imported);
    assert_eq!(outcome.suggestions_added, 1);

    // The same message under different brackets and case is the same message.
    let again = email(
        Some("same@x"),
        Some("2026-01-05T00:00:00Z"),
        "This request is closed.",
    );
    assert_eq!(
        import_email(&mut document, &id, &again, NOW)
            .unwrap()
            .outcome,
        EmailImportOutcome::AlreadyImported
    );
    assert_eq!(document.claims[0].suggestions.len(), 1);

    let suggestion = document.claims[0].suggestions[0].id.clone();
    reject_suggestion(&mut document, &id, &suggestion).unwrap();
    assert_eq!(
        import_email(&mut document, &id, &first, NOW)
            .unwrap()
            .outcome,
        EmailImportOutcome::AlreadyImported
    );
    assert!(document.claims[0].suggestions.is_empty());
}

#[test]
fn an_email_with_no_message_id_is_reported_and_yields_nothing() {
    let mut document = TrackerDocument::default();
    let id = tracked(&mut document);
    let outcome = import_email(
        &mut document,
        &id,
        &email(None, None, "This request is closed."),
        NOW,
    )
    .unwrap();
    assert_eq!(outcome.outcome, EmailImportOutcome::NoMessageId);
    assert!(document.claims[0].suggestions.is_empty());
    assert!(document.claims[0].imported_emails.is_empty());
}

#[test]
fn suggestions_carry_the_email_date_as_a_candidate_and_cite_the_exact_sentence() {
    let body = "Dear requester,\n\
        > We have received your earlier message.\n\
        Café Ünïcode: we have received your request and will respond. Thank you.\n\
        Records are available at the link below!";
    let read = email(Some("<c@x>"), Some("2026-01-05T09:30:00-08:00"), body);
    let found = events::suggest_events(&read, &read.message_id_hash().unwrap());
    let kinds: Vec<_> = found.suggestions.iter().map(|(kind, _)| *kind).collect();
    assert_eq!(
        kinds,
        [
            TimelineEventKind::Acknowledged,
            TimelineEventKind::Determination
        ]
    );
    for (_, evidence) in &found.suggestions {
        assert_eq!(
            &evidence.excerpt[evidence.sentence_start..evidence.sentence_end],
            evidence.matched_sentence
        );
        assert_eq!(
            evidence.email_date.as_deref(),
            Some("2026-01-05T09:30:00-08:00")
        );
        assert_eq!(evidence.message_id_hash.len(), 64);
        assert!(evidence.excerpt.len() <= MAXIMUM_EXCERPT_BYTES);
    }
    assert_eq!(
        found.suggestions[0].1.matched_sentence,
        "Café Ünïcode: we have received your request and will respond."
    );

    let mut document = TrackerDocument::default();
    let id = tracked(&mut document);
    import_email(&mut document, &id, &read, NOW).unwrap();
    assert!(document.claims[0]
        .suggestions
        .iter()
        .all(|s| s.candidate_occurred_at.as_deref() == Some("2026-01-05T09:30:00-08:00")));
    assert!(validate_document(&document).is_ok());
}

#[test]
fn excerpts_stay_within_bounds_for_multibyte_text_and_overlong_sentences_are_skipped() {
    let filler = "ü".repeat(400);
    let body = format!(
        "{filler} closed {filler}. {filler}. Also closed {}.",
        "x ".repeat(400)
    );
    let read = email(Some("<m@x>"), None, &body);
    let found = events::suggest_events(&read, "h");
    // The first sentence is long (800 bytes of filler) and the last is too; none may be cut.
    assert!(found.suggestions.is_empty());
    assert!(!found.notes.is_empty());

    let near = format!(
        "{} closed.",
        "ü".repeat((MAXIMUM_MATCHED_SENTENCE_BYTES - 8) / 2)
    );
    let read = email(Some("<m2@x>"), None, &format!("{near} {filler}"));
    let found = events::suggest_events(&read, "h");
    let evidence = &found.suggestions[0].1;
    assert!(evidence.excerpt.len() <= MAXIMUM_EXCERPT_BYTES);
    assert_eq!(
        &evidence.excerpt[evidence.sentence_start..evidence.sentence_end],
        evidence.matched_sentence
    );
}

#[test]
fn a_document_whose_evidence_offsets_lie_is_refused() {
    let mut document = TrackerDocument::default();
    let id = tracked(&mut document);
    import_email(
        &mut document,
        &id,
        &email(Some("<o@x>"), None, "This request is closed."),
        NOW,
    )
    .unwrap();
    document.claims[0].suggestions[0].evidence.sentence_end -= 1;
    assert!(validate_document(&document).is_err());
}

#[test]
fn email_paths_must_be_regular_eml_or_msg_files() {
    let directory = tempfile::tempdir().unwrap();
    let eml = directory.path().join("a.EML");
    std::fs::write(&eml, b"Subject: x\r\n\r\nbody").unwrap();
    assert!(events::validate_email_path(eml.to_str().unwrap()).is_ok());

    let text = directory.path().join("a.txt");
    std::fs::write(&text, b"x").unwrap();
    assert!(events::validate_email_path(text.to_str().unwrap()).is_err());
    assert!(events::validate_email_path(directory.path().to_str().unwrap()).is_err());
    assert!(events::validate_email_path("").is_err());
    assert!(events::validate_email_path("a\n.eml").is_err());
    assert!(
        events::validate_email_path(directory.path().join("missing.eml").to_str().unwrap())
            .is_err()
    );

    #[cfg(unix)]
    {
        let link = directory.path().join("link.eml");
        std::os::unix::fs::symlink(&eml, &link).unwrap();
        assert!(events::validate_email_path(link.to_str().unwrap()).is_err());
    }

    // Size: the cap is accepted, one byte past it is refused, empty is refused.
    let sized = |length: u64| {
        let path = directory.path().join(format!("s{length}.eml"));
        std::fs::File::create(&path)
            .unwrap()
            .set_len(length)
            .unwrap();
        events::validate_email_path(path.to_str().unwrap())
    };
    assert!(sized(events::MAXIMUM_EMAIL_FILE_BYTES).is_ok());
    assert!(sized(events::MAXIMUM_EMAIL_FILE_BYTES + 1).is_err());
    assert!(sized(0).is_err());
}

#[test]
fn a_real_eml_file_is_read_and_a_malformed_one_is_refused() {
    let directory = tempfile::tempdir().unwrap();
    let good = directory.path().join("good.eml");
    std::fs::write(
        &good,
        "Message-ID: <real@portal.example>\r\nDate: Mon, 05 Jan 2026 09:30:00 -0800\r\nSubject: Request 26-114\r\n\
         Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\n\
         Your request has been received. Caf=C3=A9.\r\n",
    )
    .unwrap();
    let bad = directory.path().join("bad.eml");
    std::fs::write(&bad, b"this is not an email").unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let read = events::read_email_file(good.to_str().unwrap())
            .await
            .unwrap();
        assert_eq!(read.message_id.as_deref(), Some("real@portal.example"));
        assert_eq!(
            read.email_date.as_deref(),
            Some("2026-01-05T09:30:00-08:00")
        );
        assert!(read.body_text.contains("Café"));
        assert!(events::read_email_file(bad.to_str().unwrap())
            .await
            .is_err());
    });
}

// ---- isolation from detectors ----

#[test]
fn findings_are_identical_with_and_without_tracked_requests() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let repository = crate::profile::tests::seeded_repository().await;
        let first = crate::analysis::detectors::refresh_all_findings(&repository)
            .await
            .unwrap();
        assert!(
            first.findings_recorded >= 1,
            "the fixture must make a detector fire"
        );
        let before = crate::profile::tests::snapshot(&repository).await;
        assert!(!before.is_empty());

        // A tracked request for one of the fixture's own requests, with a determination that would
        // change a statutory finding if anything read it.
        let directory = tempfile::tempdir().unwrap();
        let store = TrackerStore::new(directory.path());
        store
            .update(|document| {
                let id = claim_request(
                    document,
                    ClaimRequestInput {
                        portal_key: Some("clerk-portal".to_string()),
                        ..claim_input("Request 1")
                    },
                    &["clerk-portal".to_string()],
                    NOW,
                )?
                .id;
                add_event(
                    document,
                    event_input(&id, TimelineEventKind::Submitted, "2026-01-01T00:00:00Z"),
                    NOW,
                )?;
                add_event(
                    document,
                    event_input(
                        &id,
                        TimelineEventKind::Determination,
                        "2026-05-01T00:00:00Z",
                    ),
                    NOW,
                )?;
                Ok(())
            })
            .unwrap();
        assert_eq!(store.load().unwrap().claims.len(), 1);

        crate::analysis::detectors::refresh_all_findings(&repository)
            .await
            .unwrap();
        let after = crate::profile::tests::snapshot(&repository).await;
        let key = |finding: &Finding| {
            (
                finding.id.clone(),
                finding.kind,
                finding.severity,
                finding.confidence,
                finding.headline.clone(),
                finding.explanation.clone(),
                finding.subject.clone(),
                finding.citations.clone(),
            )
        };
        assert_eq!(
            before.iter().map(key).collect::<Vec<_>>(),
            after.iter().map(key).collect::<Vec<_>>()
        );
    });
}

#[test]
fn nothing_that_produces_findings_reads_the_tracker_and_the_tracker_touches_no_database() {
    fn production_sources(directory: &std::path::Path, files: &mut Vec<(String, String)>) {
        for entry in std::fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                production_sources(&path, files);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                let source = std::fs::read_to_string(&path).unwrap();
                let production = source
                    .split("#[cfg(test)]")
                    .next()
                    .unwrap_or("")
                    .to_string();
                files.push((path.display().to_string(), production));
            }
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut readers = Vec::new();
    for directory in ["analysis", "storage"] {
        production_sources(&root.join(directory), &mut readers);
    }
    let hits: Vec<_> = readers
        .iter()
        .filter(|(_, source)| {
            source.contains("crate::tracker") || source.contains("types::tracker")
        })
        .map(|(path, _)| path.clone())
        .collect();
    assert!(hits.is_empty(), "these read the tracker: {hits:?}");

    let mut own = Vec::new();
    production_sources(&root.join("tracker"), &mut own);
    let writers: Vec<_> = own
        .iter()
        .filter(|(_, source)| {
            [
                "crate::storage",
                "DuckDb",
                "duckdb",
                "requester_attribution",
                "reqwest::Client",
                "reqwest::get",
            ]
            .iter()
            .any(|needle| source.contains(needle))
        })
        .map(|(path, _)| path.clone())
        .collect();
    assert!(
        writers.is_empty(),
        "the tracker must not touch storage or the network: {writers:?}"
    );
}
