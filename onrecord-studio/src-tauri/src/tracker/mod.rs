//! "My requests": requests the user filed, followed from what the user typed and the notification
//! emails the user dropped in.
//!
//! No portal credential is stored or used, nothing is fetched from a portal, and nothing here is
//! sent anywhere. The tracker is never an input to a detector or a finding: it writes no column
//! the detectors read (it writes no database at all).

pub mod deadline;
pub mod events;
pub mod store;
#[cfg(test)]
mod tests;

pub use store::TrackerStore;

use crate::error::AppError;
use crate::types::tracker::*;
use chrono::DateTime;
use std::collections::HashSet;
use uuid::Uuid;

fn invalid(message: impl Into<String>) -> AppError {
    AppError::DataParsing(format!("TRACKER_INVALID: {}", message.into()))
}

/// Trimmed, single-line, control-character-free text of at most `maximum` characters.
pub(crate) fn clean_line(field: &str, value: &str, maximum: usize) -> Result<String, AppError> {
    let value = value.trim();
    if value.chars().any(char::is_control) {
        return Err(invalid(format!(
            "{field} must not contain control characters"
        )));
    }
    if value.chars().count() > maximum {
        return Err(invalid(format!(
            "{field} must be at most {maximum} characters"
        )));
    }
    Ok(value.to_string())
}

fn required_line(field: &str, value: &str, maximum: usize) -> Result<String, AppError> {
    let value = clean_line(field, value, maximum)?;
    if value.is_empty() {
        return Err(invalid(format!("{field} is required")));
    }
    Ok(value)
}

fn optional_line(
    field: &str,
    value: Option<&str>,
    maximum: usize,
) -> Result<Option<String>, AppError> {
    match value {
        None => Ok(None),
        Some(value) => {
            Ok(Some(clean_line(field, value, maximum)?).filter(|value| !value.is_empty()))
        }
    }
}

/// RFC 3339, normalized to its own offset. Bounded before parsing.
pub(crate) fn instant(field: &str, value: &str) -> Result<String, AppError> {
    let value = clean_line(field, value, 64)?;
    DateTime::parse_from_rfc3339(&value)
        .map(|parsed| parsed.to_rfc3339())
        .map_err(|_| invalid(format!("{field} must be an RFC 3339 date and time")))
}

fn https_url(value: &str) -> Result<String, AppError> {
    let value = clean_line("sourceUrl", value, MAXIMUM_SOURCE_URL_CHARACTERS)?;
    let url = reqwest::Url::parse(&value).map_err(|_| invalid("sourceUrl is not a valid URL"))?;
    if url.scheme() != "https" || url.host_str().is_none() {
        return Err(invalid("sourceUrl must be an https URL with a host"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(invalid(
            "sourceUrl must not contain a user name or password",
        ));
    }
    Ok(url.to_string())
}

struct ClaimFields {
    portal_key: Option<String>,
    reference: String,
    title: Option<String>,
    source_url: Option<String>,
    jurisdiction: Option<String>,
}

fn claim_fields(
    portal_key: Option<&str>,
    reference: &str,
    title: Option<&str>,
    source_url: Option<&str>,
    jurisdiction: Option<&str>,
    registered_portals: &[String],
) -> Result<ClaimFields, AppError> {
    let portal_key = optional_line("portalKey", portal_key, MAXIMUM_PORTAL_KEY_CHARACTERS)?;
    if let Some(key) = &portal_key {
        if !registered_portals.contains(key) {
            return Err(invalid("portalKey is not a registered portal"));
        }
    }
    let source_url = match source_url.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => Some(https_url(value)?),
        None => None,
    };
    Ok(ClaimFields {
        portal_key,
        reference: required_line("reference", reference, MAXIMUM_REFERENCE_CHARACTERS)?,
        title: optional_line("title", title, MAXIMUM_TITLE_CHARACTERS)?,
        source_url,
        jurisdiction: optional_line(
            "jurisdiction",
            jurisdiction,
            MAXIMUM_JURISDICTION_CHARACTERS,
        )?,
    })
}

fn same_request(left: &ClaimFields, other: &ClaimedRequest) -> bool {
    left.portal_key == other.portal_key
        && left.reference.to_lowercase() == other.reference.to_lowercase()
}

pub fn claim_request(
    document: &mut TrackerDocument,
    input: ClaimRequestInput,
    registered_portals: &[String],
    now: &str,
) -> Result<ClaimedRequest, AppError> {
    let fields = claim_fields(
        input.portal_key.as_deref(),
        &input.reference,
        input.title.as_deref(),
        input.source_url.as_deref(),
        input.jurisdiction.as_deref(),
        registered_portals,
    )?;
    if document.claims.len() >= MAXIMUM_CLAIMED_REQUESTS {
        return Err(invalid(format!(
            "at most {MAXIMUM_CLAIMED_REQUESTS} requests can be tracked"
        )));
    }
    if document
        .claims
        .iter()
        .any(|claim| same_request(&fields, claim))
    {
        return Err(invalid("this request is already tracked"));
    }
    let claim = ClaimedRequest {
        id: Uuid::new_v4().to_string(),
        portal_key: fields.portal_key,
        reference: fields.reference,
        title: fields.title,
        source_url: fields.source_url,
        jurisdiction: fields.jurisdiction,
        claimed_at: now.to_string(),
        events: Vec::new(),
        suggestions: Vec::new(),
        imported_emails: Vec::new(),
    };
    document.claims.push(claim.clone());
    Ok(claim)
}

pub(crate) fn claim_mut<'a>(
    document: &'a mut TrackerDocument,
    claim_id: &str,
) -> Result<&'a mut ClaimedRequest, AppError> {
    document
        .claims
        .iter_mut()
        .find(|claim| claim.id == claim_id)
        .ok_or_else(|| invalid("no such tracked request"))
}

pub fn update_claim(
    document: &mut TrackerDocument,
    input: UpdateClaimedRequestInput,
    registered_portals: &[String],
) -> Result<ClaimedRequest, AppError> {
    let fields = claim_fields(
        input.portal_key.as_deref(),
        &input.reference,
        input.title.as_deref(),
        input.source_url.as_deref(),
        input.jurisdiction.as_deref(),
        registered_portals,
    )?;
    if document
        .claims
        .iter()
        .any(|claim| claim.id != input.claim_id && same_request(&fields, claim))
    {
        return Err(invalid("this request is already tracked"));
    }
    let claim = claim_mut(document, &input.claim_id)?;
    claim.portal_key = fields.portal_key;
    claim.reference = fields.reference;
    claim.title = fields.title;
    claim.source_url = fields.source_url;
    claim.jurisdiction = fields.jurisdiction;
    Ok(claim.clone())
}

pub fn remove_claim(document: &mut TrackerDocument, claim_id: &str) -> Result<(), AppError> {
    let before = document.claims.len();
    document.claims.retain(|claim| claim.id != claim_id);
    if document.claims.len() == before {
        return Err(invalid("no such tracked request"));
    }
    Ok(())
}

/// The extension rules shared by every way an event can be created, so no path can save an
/// extension notice without its statutory reason.
fn extension_fields(
    kind: TimelineEventKind,
    reason: Option<crate::types::StatutoryExtensionReason>,
    expected_determination_at: Option<&str>,
) -> Result<
    (
        Option<crate::types::StatutoryExtensionReason>,
        Option<String>,
    ),
    AppError,
> {
    if kind == TimelineEventKind::ExtensionNotice {
        let reason = reason.ok_or_else(|| {
            invalid("an extension notice needs the statutory reason for the extension")
        })?;
        let expected = expected_determination_at
            .map(|value| instant("expectedDeterminationAt", value))
            .transpose()?;
        Ok((Some(reason), expected))
    } else if reason.is_some() || expected_determination_at.is_some() {
        Err(invalid(
            "a reason and an expected date apply only to an extension notice",
        ))
    } else {
        Ok((None, None))
    }
}

pub fn add_event(
    document: &mut TrackerDocument,
    input: AddTimelineEventInput,
    now: &str,
) -> Result<TimelineEvent, AppError> {
    let occurred_at = instant("occurredAt", &input.occurred_at)?;
    let (extension_reason, expected_determination_at) = extension_fields(
        input.kind,
        input.extension_reason,
        input.expected_determination_at.as_deref(),
    )?;
    let claim = claim_mut(document, &input.claim_id)?;
    ensure_event_room(claim)?;
    let event = TimelineEvent {
        id: Uuid::new_v4().to_string(),
        kind: input.kind,
        occurred_at,
        basis: EventBasis::UserEntered,
        evidence: None,
        extension_reason,
        expected_determination_at,
        created_at: now.to_string(),
    };
    claim.events.push(event.clone());
    Ok(event)
}

fn ensure_event_room(claim: &ClaimedRequest) -> Result<(), AppError> {
    if claim.events.len() >= MAXIMUM_EVENTS_PER_REQUEST {
        return Err(invalid(format!(
            "a request holds at most {MAXIMUM_EVENTS_PER_REQUEST} events"
        )));
    }
    Ok(())
}

pub fn remove_event(
    document: &mut TrackerDocument,
    claim_id: &str,
    event_id: &str,
) -> Result<(), AppError> {
    let claim = claim_mut(document, claim_id)?;
    let before = claim.events.len();
    claim.events.retain(|event| event.id != event_id);
    if claim.events.len() == before {
        return Err(invalid("no such event"));
    }
    Ok(())
}

/// Accepts a suggestion as the user has now stated it. Only this moves an email-derived event into
/// the list the deadline reads.
pub fn confirm_suggestion(
    document: &mut TrackerDocument,
    input: ConfirmSuggestedEventInput,
    now: &str,
) -> Result<TimelineEvent, AppError> {
    let occurred_at = instant("occurredAt", &input.occurred_at)?;
    let (extension_reason, expected_determination_at) = extension_fields(
        input.kind,
        input.extension_reason,
        input.expected_determination_at.as_deref(),
    )?;
    let claim = claim_mut(document, &input.claim_id)?;
    let position = claim
        .suggestions
        .iter()
        .position(|suggestion| suggestion.id == input.suggestion_id)
        .ok_or_else(|| invalid("no such suggested event"))?;
    ensure_event_room(claim)?;
    let suggestion = claim.suggestions.remove(position);
    let event = TimelineEvent {
        id: Uuid::new_v4().to_string(),
        kind: input.kind,
        occurred_at,
        basis: EventBasis::EmailConfirmed,
        evidence: Some(suggestion.evidence),
        extension_reason,
        expected_determination_at,
        created_at: now.to_string(),
    };
    claim.events.push(event.clone());
    Ok(event)
}

pub fn reject_suggestion(
    document: &mut TrackerDocument,
    claim_id: &str,
    suggestion_id: &str,
) -> Result<(), AppError> {
    let claim = claim_mut(document, claim_id)?;
    let before = claim.suggestions.len();
    claim
        .suggestions
        .retain(|suggestion| suggestion.id != suggestion_id);
    if claim.suggestions.len() == before {
        return Err(invalid("no such suggested event"));
    }
    Ok(())
}

/// Records one email's suggestions. The Message-ID check happens here, inside the store's
/// read-modify-write, so two imports of the same email cannot both pass it.
pub fn import_email(
    document: &mut TrackerDocument,
    claim_id: &str,
    email: &events::ReadEmail,
    now: &str,
) -> Result<EmailImportFileResult, AppError> {
    let claim = claim_mut(document, claim_id)?;
    let file_name = email.file_name.clone();
    let Some(hash) = email.message_id_hash() else {
        return Ok(EmailImportFileResult {
            file_name,
            outcome: EmailImportOutcome::NoMessageId,
            suggestions_added: 0,
            detail: Some(
                "This email has no Message-ID, so it cannot be cited or recognized on a second \
                 import. No event was suggested from it."
                    .to_string(),
            ),
        });
    };
    if claim
        .imported_emails
        .iter()
        .any(|known| known.message_id_hash == hash)
    {
        return Ok(EmailImportFileResult {
            file_name,
            outcome: EmailImportOutcome::AlreadyImported,
            suggestions_added: 0,
            detail: None,
        });
    }
    if claim.imported_emails.len() >= MAXIMUM_IMPORTED_EMAILS_PER_REQUEST {
        return Err(invalid(format!(
            "a request records at most {MAXIMUM_IMPORTED_EMAILS_PER_REQUEST} imported emails"
        )));
    }
    let found = events::suggest_events(email, &hash);
    let room = MAXIMUM_SUGGESTIONS_PER_REQUEST.saturating_sub(claim.suggestions.len());
    let mut details = found.notes;
    if found.suggestions.len() > room {
        details.push(format!(
            "Only {room} more suggestions fit for this request; the rest were not kept."
        ));
    }
    let mut added = 0u32;
    for (kind, evidence) in found.suggestions.into_iter().take(room) {
        claim.suggestions.push(SuggestedEvent {
            id: Uuid::new_v4().to_string(),
            kind,
            candidate_occurred_at: evidence.email_date.clone(),
            evidence,
            suggested_at: now.to_string(),
        });
        added += 1;
    }
    claim.imported_emails.push(ImportedEmail {
        message_id_hash: hash,
        subject_excerpt: email.subject_excerpt(),
        email_date: email.email_date.clone(),
        imported_at: now.to_string(),
        suggestion_count: added,
    });
    details.extend(email.notes.iter().cloned());
    Ok(EmailImportFileResult {
        file_name,
        outcome: EmailImportOutcome::Imported,
        suggestions_added: added,
        detail: (!details.is_empty()).then(|| details.join(" ")),
    })
}

fn validate_evidence(evidence: &EmailEvidence) -> Result<(), AppError> {
    if evidence.message_id_hash.is_empty() {
        return Err(invalid("evidence has no message-id hash"));
    }
    if let Some(subject) = &evidence.subject_excerpt {
        clean_line(
            "subjectExcerpt",
            subject,
            MAXIMUM_SUBJECT_EXCERPT_CHARACTERS,
        )?;
    }
    if let Some(date) = &evidence.email_date {
        instant("emailDate", date)?;
    }
    if evidence.excerpt.len() > MAXIMUM_EXCERPT_BYTES
        || evidence.matched_sentence.len() > MAXIMUM_MATCHED_SENTENCE_BYTES
        || evidence.matched_sentence.is_empty()
    {
        return Err(invalid("evidence text is out of bounds"));
    }
    if evidence.excerpt.chars().any(char::is_control)
        || evidence.matched_sentence.chars().any(char::is_control)
    {
        return Err(invalid("evidence text must not contain control characters"));
    }
    match evidence
        .excerpt
        .get(evidence.sentence_start..evidence.sentence_end)
    {
        Some(slice) if slice == evidence.matched_sentence => Ok(()),
        _ => Err(invalid("the matched sentence is not where its offsets say")),
    }
}

/// Every invariant a saved document must hold. Run on each read and before each write, so a
/// hand-edited or corrupted file cannot smuggle an unbounded or inconsistent entry into the UI or
/// the deadline.
pub fn validate_document(document: &TrackerDocument) -> Result<(), AppError> {
    if document.claims.len() > MAXIMUM_CLAIMED_REQUESTS {
        return Err(invalid("too many tracked requests"));
    }
    let mut seen_claims = HashSet::new();
    for claim in &document.claims {
        if !seen_claims.insert(claim.id.as_str()) {
            return Err(invalid("duplicate request id"));
        }
        required_line("reference", &claim.reference, MAXIMUM_REFERENCE_CHARACTERS)?;
        optional_line("title", claim.title.as_deref(), MAXIMUM_TITLE_CHARACTERS)?;
        optional_line(
            "jurisdiction",
            claim.jurisdiction.as_deref(),
            MAXIMUM_JURISDICTION_CHARACTERS,
        )?;
        if let Some(url) = &claim.source_url {
            https_url(url)?;
        }
        if claim.events.len() > MAXIMUM_EVENTS_PER_REQUEST
            || claim.suggestions.len() > MAXIMUM_SUGGESTIONS_PER_REQUEST
            || claim.imported_emails.len() > MAXIMUM_IMPORTED_EMAILS_PER_REQUEST
        {
            return Err(invalid("a request holds more entries than the limit"));
        }
        for event in &claim.events {
            instant("occurredAt", &event.occurred_at)?;
            let (reason, expected) = extension_fields(
                event.kind,
                event.extension_reason,
                event.expected_determination_at.as_deref(),
            )?;
            if reason != event.extension_reason
                || expected.is_none() != event.expected_determination_at.is_none()
            {
                return Err(invalid("an event's extension fields are inconsistent"));
            }
            match (&event.basis, &event.evidence) {
                (EventBasis::EmailConfirmed, Some(evidence)) => validate_evidence(evidence)?,
                (EventBasis::UserEntered, None) => {}
                _ => return Err(invalid("an event's basis and evidence disagree")),
            }
        }
        for suggestion in &claim.suggestions {
            validate_evidence(&suggestion.evidence)?;
            if let Some(date) = &suggestion.candidate_occurred_at {
                instant("candidateOccurredAt", date)?;
            }
        }
    }
    Ok(())
}
