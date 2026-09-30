//! Verified email domains and the explicit link from an affiliation to one.
//!
//! A confirmed code proves control of one mailbox at a domain. It never proves employment, so the
//! only effect here is the affiliation's `basis` label, and only when the user links the two and
//! the domain's own name equals the affiliation's name or an alias. Provider claims are not used.
//! Nothing in this module reaches finding detection, confidence or statutory evaluation.

use super::matching::normalize_name;
use crate::types::profile::*;
use chrono::Utc;

/// Labels that name a kind of registry or a mail host, not an organization.
const GENERIC_LABELS: &[&str] = &[
    "com", "org", "net", "edu", "gov", "mil", "co", "ac", "www", "mail", "email", "smtp", "mx",
];

fn compact(value: &str) -> String {
    let normalized = normalize_name(value);
    let words = normalized.strip_prefix("the ").unwrap_or(&normalized);
    words.split_whitespace().collect()
}

/// The organization names a domain can stand for: every label except the last (the top-level
/// domain) and generic registry labels, with hyphens read as word breaks. `ci.san-diego.ca.us`
/// yields `ci`, `sandiego` and `ca`; matching requires equality with one of them.
pub fn domain_names(domain: &str) -> Vec<String> {
    let labels: Vec<&str> = domain.split('.').collect();
    let Some((_, rest)) = labels.split_last() else {
        return Vec::new();
    };
    let mut names: Vec<String> = rest
        .iter()
        .filter(|label| !GENERIC_LABELS.contains(label))
        .map(|label| compact(&label.replace('-', " ")))
        .filter(|name| !name.is_empty())
        .collect();
    names.dedup();
    names
}

/// The declared term (name or alias) that equals one of the domain's names, if any.
pub fn matching_term<'a>(name: &'a str, aliases: &'a [String], domain: &str) -> Option<&'a str> {
    let names = domain_names(domain);
    std::iter::once(name)
        .chain(aliases.iter().map(String::as_str))
        .find(|term| {
            let term = compact(term);
            !term.is_empty() && names.contains(&term)
        })
}

fn usable<'a>(domains: &'a [VerifiedDomain], domain: &str) -> Option<&'a VerifiedDomain> {
    domains
        .iter()
        .find(|verified| verified.domain == domain && !verified.is_free_mail)
}

/// Applied on every profile save: a link survives only while its domain is still verified and not
/// free-mail, and the (possibly edited) name or aliases still equal the domain's name.
pub fn keep_link_if_still_matching(
    verified: &[VerifiedDomain],
    name: &str,
    aliases: &[String],
    basis: AffiliationBasis,
    verified_domain: Option<String>,
) -> (AffiliationBasis, Option<String>) {
    match (basis, verified_domain) {
        (AffiliationBasis::EmailDomainVerified, Some(domain))
            if usable(verified, &domain).is_some()
                && matching_term(name, aliases, &domain).is_some() =>
        {
            (AffiliationBasis::EmailDomainVerified, Some(domain))
        }
        _ => (AffiliationBasis::SelfDeclared, None),
    }
}

/// Records a domain a code was confirmed for, replacing an earlier record of the same domain. A
/// free-mail record also severs any link to that domain.
pub fn record_domain(
    current: Option<UserProfile>,
    domain: &str,
    is_free_mail: bool,
) -> UserProfile {
    let now = Utc::now().to_rfc3339();
    let mut profile = current.unwrap_or_else(|| UserProfile {
        schema_version: PROFILE_SCHEMA_VERSION,
        display_name: None,
        affiliations: Vec::new(),
        match_decisions: Vec::new(),
        verified_domains: Vec::new(),
        updated_at: String::new(),
    });
    profile
        .verified_domains
        .retain(|verified| verified.domain != domain);
    if profile.verified_domains.len() >= MAXIMUM_VERIFIED_DOMAINS {
        // Oldest first; a domain an affiliation is linked to is kept.
        let linked: Vec<&str> = profile
            .affiliations
            .iter()
            .filter_map(|affiliation| affiliation.verified_domain.as_deref())
            .collect();
        if let Some(index) = profile
            .verified_domains
            .iter()
            .position(|verified| !linked.contains(&verified.domain.as_str()))
        {
            profile.verified_domains.remove(index);
        }
    }
    profile.verified_domains.push(VerifiedDomain {
        domain: domain.to_string(),
        verified_at: now.clone(),
        is_free_mail,
    });
    if is_free_mail {
        for affiliation in &mut profile.affiliations {
            if affiliation.verified_domain.as_deref() == Some(domain) {
                affiliation.verified_domain = None;
                affiliation.basis = AffiliationBasis::SelfDeclared;
            }
        }
    }
    profile.updated_at = now;
    profile
}

/// The user's explicit choice to tie one affiliation to one verified domain. Anything other than
/// `Linked` leaves the profile unchanged and the affiliation `SelfDeclared`.
pub fn link_affiliation(
    mut profile: UserProfile,
    input: &AffiliationDomainLinkInput,
) -> (UserProfile, AffiliationLinkStatus, String) {
    let refuse = |profile, status, message: String| (profile, status, message);
    let Some(index) = profile
        .affiliations
        .iter()
        .position(|affiliation| affiliation.id == input.affiliation_id)
    else {
        return refuse(
            profile,
            AffiliationLinkStatus::AffiliationNotFound,
            "That organization is not declared in the profile.".to_string(),
        );
    };
    let Some(verified) = profile
        .verified_domains
        .iter()
        .find(|verified| verified.domain == input.domain)
    else {
        return refuse(
            profile,
            AffiliationLinkStatus::DomainNotVerified,
            "No code has been confirmed for that domain.".to_string(),
        );
    };
    if verified.is_free_mail {
        return refuse(
            profile,
            AffiliationLinkStatus::FreeMailDomain,
            format!(
                "{} is a consumer mailbox provider. It was recorded but says nothing about an \
                 organization, so it is not used as proof.",
                input.domain
            ),
        );
    }
    let affiliation = &profile.affiliations[index];
    if matching_term(&affiliation.name, &affiliation.aliases, &input.domain).is_none() {
        let names = domain_names(&input.domain).join(", ");
        return refuse(
            profile,
            AffiliationLinkStatus::NotMatching,
            format!(
                "Neither the name nor an alias of this organization equals what {} names ({names}). \
                 It stays self-declared. If it is the same organization, add that name as an alias \
                 and try again.",
                input.domain
            ),
        );
    }
    let affiliation = &mut profile.affiliations[index];
    affiliation.basis = AffiliationBasis::EmailDomainVerified;
    affiliation.verified_domain = Some(input.domain.clone());
    profile.updated_at = Utc::now().to_rfc3339();
    (
        profile,
        AffiliationLinkStatus::Linked,
        "Linked. This records control of a mailbox at that domain, not employment.".to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::apply_input;

    fn declared(name: &str, aliases: &[&str]) -> UserProfile {
        apply_input(
            None,
            ProfileInput {
                display_name: None,
                affiliations: vec![AffiliationInput {
                    id: None,
                    name: name.to_string(),
                    aliases: aliases.iter().map(|a| a.to_string()).collect(),
                }],
            },
        )
        .unwrap()
    }

    fn link(profile: UserProfile, domain: &str) -> (UserProfile, AffiliationLinkStatus, String) {
        let input = AffiliationDomainLinkInput {
            affiliation_id: profile.affiliations[0].id.clone(),
            domain: domain.to_string(),
        };
        link_affiliation(profile, &input)
    }

    #[test]
    fn a_matching_domain_upgrades_only_when_linked() {
        let profile = record_domain(
            Some(declared("City of San Diego", &["SanDiego"])),
            "sandiego.gov",
            false,
        );
        assert_eq!(
            profile.affiliations[0].basis,
            AffiliationBasis::SelfDeclared
        );
        let (linked, status, _) = link(profile, "sandiego.gov");
        assert_eq!(status, AffiliationLinkStatus::Linked);
        assert_eq!(
            linked.affiliations[0].basis,
            AffiliationBasis::EmailDomainVerified
        );
        assert_eq!(
            linked.affiliations[0].verified_domain.as_deref(),
            Some("sandiego.gov")
        );
    }

    #[test]
    fn hyphenated_and_subdomain_forms_match_the_compact_name() {
        assert_eq!(
            domain_names("ci.san-diego.ca.us"),
            vec!["ci", "sandiego", "ca"]
        );
        let profile = record_domain(
            Some(declared("The San Diego", &[])),
            "mail.san-diego.org",
            false,
        );
        assert_eq!(
            link(profile, "mail.san-diego.org").1,
            AffiliationLinkStatus::Linked
        );
    }

    #[test]
    fn a_non_matching_domain_stays_self_declared_and_says_why() {
        let profile = record_domain(Some(declared("City of San Diego", &[])), "acme.com", false);
        let (after, status, message) = link(profile.clone(), "acme.com");
        assert_eq!(status, AffiliationLinkStatus::NotMatching);
        assert_eq!(after, profile);
        assert!(message.contains("acme") && message.contains("self-declared"));
    }

    #[test]
    fn a_free_mail_domain_never_upgrades_even_when_the_name_matches() {
        let profile = record_domain(Some(declared("Gmail", &[])), "gmail.com", true);
        assert_eq!(profile.verified_domains[0].domain, "gmail.com");
        let (after, status, _) = link(profile.clone(), "gmail.com");
        assert_eq!(status, AffiliationLinkStatus::FreeMailDomain);
        assert_eq!(after, profile);
        assert_eq!(after.affiliations[0].basis, AffiliationBasis::SelfDeclared);
    }

    #[test]
    fn an_unverified_domain_cannot_be_linked() {
        let (_, status, _) = link(declared("Acme", &[]), "acme.com");
        assert_eq!(status, AffiliationLinkStatus::DomainNotVerified);
    }

    #[test]
    fn a_later_free_mail_record_severs_an_existing_link() {
        let profile = record_domain(Some(declared("Acme", &[])), "acme.com", false);
        let (linked, _, _) = link(profile, "acme.com");
        let after = record_domain(Some(linked), "acme.com", true);
        assert_eq!(after.affiliations[0].basis, AffiliationBasis::SelfDeclared);
        assert_eq!(after.affiliations[0].verified_domain, None);
    }

    fn save(profile: UserProfile, name: &str, aliases: &[&str]) -> UserProfile {
        apply_input(
            Some(profile.clone()),
            ProfileInput {
                display_name: None,
                affiliations: vec![AffiliationInput {
                    id: Some(profile.affiliations[0].id.clone()),
                    name: name.to_string(),
                    aliases: aliases.iter().map(|a| a.to_string()).collect(),
                }],
            },
        )
        .unwrap()
    }

    #[test]
    fn saving_keeps_the_link_and_the_domains_only_while_the_match_holds() {
        let profile = record_domain(Some(declared("Acme", &[])), "acme.com", false);
        let (linked, _, _) = link(profile, "acme.com");
        let kept = save(linked.clone(), "Acme", &["Acme Corp"]);
        assert_eq!(
            kept.affiliations[0].basis,
            AffiliationBasis::EmailDomainVerified
        );
        assert_eq!(kept.verified_domains, linked.verified_domains);
        let renamed = save(linked.clone(), "Globex", &[]);
        assert_eq!(
            renamed.affiliations[0].basis,
            AffiliationBasis::SelfDeclared
        );
        assert_eq!(renamed.affiliations[0].verified_domain, None);
        assert_eq!(renamed.verified_domains.len(), 1);
        // Same name, but the alias that carried the match was removed: no longer equal.
        let via_alias = record_domain(
            Some(declared("Acme Corporation", &["Acme"])),
            "acme.com",
            false,
        );
        let (linked, _, _) = link(via_alias, "acme.com");
        let dropped = save(linked, "Acme Corporation", &[]);
        assert_eq!(
            dropped.affiliations[0].basis,
            AffiliationBasis::SelfDeclared
        );
    }

    #[test]
    fn the_domain_list_is_bounded_and_keeps_the_linked_domain() {
        let profile = record_domain(Some(declared("Acme", &[])), "acme.com", false);
        let (mut profile, _, _) = link(profile, "acme.com");
        for index in 0..(MAXIMUM_VERIFIED_DOMAINS + 3) {
            profile = record_domain(Some(profile), &format!("d{index}.example"), false);
            assert!(profile.verified_domains.len() <= MAXIMUM_VERIFIED_DOMAINS);
        }
        assert!(profile
            .verified_domains
            .iter()
            .any(|v| v.domain == "acme.com"));
    }

    #[test]
    fn the_stored_file_holds_the_domain_and_never_an_address_or_code() {
        let profile = record_domain(None, "acme.com", false);
        let json = serde_json::to_string(&profile).unwrap();
        assert!(json.contains("acme.com"));
        assert!(!json.contains('@') && !json.contains("code"));
    }

    #[test]
    fn a_profile_written_before_this_field_still_reads() {
        let old = r#"{"schemaVersion":1,"displayName":null,"affiliations":[{"id":"a","name":"X","aliases":[],"basis":"SELF_DECLARED","createdAt":"t"}],"matchDecisions":[],"updatedAt":"t"}"#;
        let profile: UserProfile = serde_json::from_str(old).unwrap();
        assert!(profile.verified_domains.is_empty());
        assert_eq!(profile.affiliations[0].verified_domain, None);
    }
}
