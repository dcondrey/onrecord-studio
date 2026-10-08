//! The local user profile: declared organization affiliations, kept on this device only.
//!
//! Invariant: nothing in this module is read by finding detection, confidence, severity,
//! statutory evaluation or ranking. It filters, highlights and discloses, and the disclosure
//! travels with every outgoing artifact (see [`DisclosureContext`]).

pub mod matching;
pub mod store;
pub mod verified;

use crate::analysis::finding::FindingSubject;
use crate::error::AppError;
use crate::types::profile::*;
use chrono::Utc;
use matching::normalize_name;
use std::collections::BTreeSet;

pub use store::ProfileStore;

fn clean(value: &str, label: &str, maximum: usize) -> Result<String, AppError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(AppError::DataParsing(format!("{label} cannot be empty")));
    }
    if value.chars().any(char::is_control) {
        return Err(AppError::DataParsing(format!(
            "{label} cannot contain control characters"
        )));
    }
    if value.chars().count() > maximum {
        return Err(AppError::DataParsing(format!(
            "{label} is longer than {maximum} characters"
        )));
    }
    Ok(value.to_string())
}

fn normalized_or_error(value: &str, label: &str) -> Result<String, AppError> {
    let normalized = normalize_name(value);
    if normalized.is_empty() {
        return Err(AppError::DataParsing(format!(
            "{label} must contain a letter or digit"
        )));
    }
    Ok(normalized)
}

/// Validates a save request against the current document and returns the next one. A name change
/// drops that affiliation's confirmed and rejected matches and returns its basis to
/// `SelfDeclared`: a decision about "Org A" says nothing about what the entry was renamed to.
pub fn apply_input(
    current: Option<UserProfile>,
    input: ProfileInput,
) -> Result<UserProfile, AppError> {
    let display_name = match input.display_name.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(name) => Some(clean(
            name,
            "Display name",
            MAXIMUM_DISPLAY_NAME_CHARACTERS,
        )?),
    };
    if input.affiliations.len() > MAXIMUM_AFFILIATIONS {
        return Err(AppError::DataParsing(format!(
            "At most {MAXIMUM_AFFILIATIONS} affiliations can be declared"
        )));
    }
    let previous = current.unwrap_or(UserProfile {
        schema_version: PROFILE_SCHEMA_VERSION,
        display_name: None,
        affiliations: Vec::new(),
        match_decisions: Vec::new(),
        verified_domains: Vec::new(),
        updated_at: String::new(),
    });
    let now = Utc::now().to_rfc3339();
    let mut names_seen = BTreeSet::new();
    let mut ids_seen = BTreeSet::new();
    let mut affiliations = Vec::with_capacity(input.affiliations.len());
    let mut kept_decisions_for: BTreeSet<String> = BTreeSet::new();
    for entry in input.affiliations {
        let name = clean(
            &entry.name,
            "Organization name",
            MAXIMUM_AFFILIATION_NAME_CHARACTERS,
        )?;
        let normalized = normalized_or_error(&name, "Organization name")?;
        if !names_seen.insert(normalized.clone()) {
            return Err(AppError::DataParsing(format!(
                "Organization '{name}' is declared more than once"
            )));
        }
        if entry.aliases.len() > MAXIMUM_ALIASES_PER_AFFILIATION {
            return Err(AppError::DataParsing(format!(
                "At most {MAXIMUM_ALIASES_PER_AFFILIATION} aliases per organization"
            )));
        }
        let mut alias_forms = BTreeSet::from([normalized.clone()]);
        let mut aliases = Vec::with_capacity(entry.aliases.len());
        for alias in &entry.aliases {
            let alias = clean(alias, "Alias", MAXIMUM_ALIAS_CHARACTERS)?;
            if !alias_forms.insert(normalized_or_error(&alias, "Alias")?) {
                return Err(AppError::DataParsing(format!(
                    "Alias '{alias}' repeats the organization name or another alias"
                )));
            }
            aliases.push(alias);
        }
        let existing = entry
            .id
            .as_deref()
            .and_then(|id| previous.affiliations.iter().find(|item| item.id == id));
        let (id, created_at, basis, verified_domain) = match existing {
            Some(existing) if normalize_name(&existing.name) == normalized => {
                kept_decisions_for.insert(existing.id.clone());
                (
                    existing.id.clone(),
                    existing.created_at.clone(),
                    existing.basis,
                    existing.verified_domain.clone(),
                )
            }
            Some(existing) => (
                existing.id.clone(),
                existing.created_at.clone(),
                AffiliationBasis::SelfDeclared,
                None,
            ),
            None => (
                uuid::Uuid::new_v4().to_string(),
                now.clone(),
                AffiliationBasis::SelfDeclared,
                None,
            ),
        };
        let (basis, verified_domain) = verified::keep_link_if_still_matching(
            &previous.verified_domains,
            &name,
            &aliases,
            basis,
            verified_domain,
        );
        if !ids_seen.insert(id.clone()) {
            return Err(AppError::DataParsing(
                "The same affiliation was submitted twice".to_string(),
            ));
        }
        affiliations.push(Affiliation {
            id,
            name,
            aliases,
            basis,
            created_at,
            verified_domain,
        });
    }
    let match_decisions = previous
        .match_decisions
        .into_iter()
        .filter(|decision| kept_decisions_for.contains(&decision.affiliation_id))
        .collect();
    Ok(UserProfile {
        schema_version: PROFILE_SCHEMA_VERSION,
        display_name,
        affiliations,
        match_decisions,
        verified_domains: previous.verified_domains,
        updated_at: now,
    })
}

/// Records or replaces the user's answer for one candidate.
pub fn apply_decision(
    mut profile: UserProfile,
    input: AffiliationMatchDecisionInput,
    decision: MatchDecisionKind,
) -> Result<UserProfile, AppError> {
    if !profile
        .affiliations
        .iter()
        .any(|affiliation| affiliation.id == input.affiliation_id)
    {
        return Err(AppError::DataParsing(
            "That organization is not declared in the profile".to_string(),
        ));
    }
    if input.normalized_name.chars().count() > 300 {
        return Err(AppError::DataParsing(
            "Matched name is too long".to_string(),
        ));
    }
    let normalized_name = normalized_or_error(&input.normalized_name, "Matched name")?;
    profile.match_decisions.retain(|existing| {
        !(existing.affiliation_id == input.affiliation_id
            && existing.kind == input.kind
            && existing.normalized_name == normalized_name)
    });
    if profile.match_decisions.len() >= MAXIMUM_MATCH_DECISIONS {
        return Err(AppError::DataParsing(format!(
            "At most {MAXIMUM_MATCH_DECISIONS} match decisions can be kept"
        )));
    }
    let now = Utc::now().to_rfc3339();
    profile.match_decisions.push(MatchDecision {
        affiliation_id: input.affiliation_id,
        kind: input.kind,
        normalized_name,
        decision,
        decided_at: now.clone(),
    });
    profile.updated_at = now;
    Ok(profile)
}

/// The standard sentence, or `None` when there is nothing to disclose.
pub fn disclosure_sentence(profile: Option<&UserProfile>) -> Option<String> {
    let affiliations = &profile?.affiliations;
    if affiliations.is_empty() {
        return None;
    }
    let names = affiliations
        .iter()
        .map(|affiliation| affiliation.name.as_str())
        .collect::<Vec<_>>();
    let joined = match names.as_slice() {
        [only] => (*only).to_string(),
        [first @ .., last] => format!("{} and {last}", first.join(", ")),
        [] => return None,
    };
    let qualifier = if affiliations
        .iter()
        .all(|affiliation| affiliation.basis == AffiliationBasis::SelfDeclared)
    {
        "self-declared, not verified".to_string()
    } else {
        let per_item = affiliations
            .iter()
            .map(|affiliation| match affiliation.basis {
                AffiliationBasis::SelfDeclared => format!("{}: self-declared", affiliation.name),
                AffiliationBasis::EmailDomainVerified => format!(
                    "{}: email domain verified, which does not establish employment",
                    affiliation.name
                ),
            })
            .collect::<Vec<_>>();
        per_item.join("; ")
    };
    Some(format!(
        "Prepared by a user who has declared an affiliation with {joined} ({qualifier})."
    ))
}

pub fn build_disclosure(
    profile: Option<&UserProfile>,
    own: matching::ConfirmedSubjects,
) -> AffiliationDisclosure {
    let sentence = disclosure_sentence(profile);
    AffiliationDisclosure {
        status: if sentence.is_some() {
            DisclosureStatus::Declared
        } else {
            DisclosureStatus::Unset
        },
        affiliations: profile
            .map(|profile| {
                profile
                    .affiliations
                    .iter()
                    .map(|affiliation| DisclosedAffiliation {
                        name: affiliation.name.clone(),
                        basis: affiliation.basis,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        sentence,
        own_portal_keys: own.portal_keys.into_iter().collect(),
        own_agencies: own
            .agencies
            .into_iter()
            .map(|(portal_key, agency_id)| OwnAgency {
                portal_key,
                agency_id,
            })
            .collect(),
        own_entity_ids: own.entity_ids.into_iter().collect(),
    }
}

pub const OWN_SUBJECT_SENTENCE: &str =
    "The subject of this document is an organization the author \
     has declared an affiliation with (self-declared, not verified).";

/// What an outgoing artifact must carry about its author. Constructing drafts and reports requires
/// one, so leaving the disclosure out is a compile error rather than a forgotten call.
#[derive(Debug, Clone, Default)]
pub struct DisclosureContext {
    sentence: Option<String>,
    own_portal_keys: BTreeSet<String>,
    own_agencies: BTreeSet<(String, i64)>,
    own_entity_ids: BTreeSet<String>,
}

impl DisclosureContext {
    /// No profile, or one that declares nothing. Adds no text.
    pub fn unset() -> Self {
        Self::default()
    }

    pub fn from_disclosure(disclosure: &AffiliationDisclosure) -> Self {
        Self {
            sentence: disclosure.sentence.clone(),
            own_portal_keys: disclosure.own_portal_keys.iter().cloned().collect(),
            own_agencies: disclosure
                .own_agencies
                .iter()
                .map(|agency| (agency.portal_key.clone(), agency.agency_id))
                .collect(),
            own_entity_ids: disclosure.own_entity_ids.iter().cloned().collect(),
        }
    }

    pub fn sentence(&self) -> Option<&str> {
        self.sentence.as_deref()
    }

    pub fn subject_is_own(&self, subject: &FindingSubject) -> bool {
        let portal_key = subject.portal_key.as_deref();
        let entity_id = subject.entity_id.as_deref();
        portal_key.is_some_and(|key| self.own_portal_keys.contains(key))
            || portal_key
                .zip(subject.agency_id)
                .is_some_and(|(key, id)| self.own_agencies.contains(&(key.to_string(), id)))
            || entity_id.is_some_and(|id| self.own_entity_ids.contains(id))
    }

    /// The own-organization sentence when the subject matches, and only when the author declared
    /// something at all.
    pub fn own_subject_sentence(&self, subject: &FindingSubject) -> Option<&'static str> {
        (self.sentence.is_some() && self.subject_is_own(subject)).then_some(OWN_SUBJECT_SENTENCE)
    }

    /// The disclosure an outgoing artifact carries as its own field, apart from any editable
    /// body. `None` when nothing is declared.
    pub fn text(&self, subject: &FindingSubject) -> Option<String> {
        let mut text = self.sentence()?.to_string();
        if let Some(own) = self.own_subject_sentence(subject) {
            text.push(' ');
            text.push_str(own);
        }
        Some(text)
    }
}

#[cfg(test)]
pub(crate) mod tests;
