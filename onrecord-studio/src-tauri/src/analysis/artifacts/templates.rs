//! Jurisdiction-aware artifact templates as a plain data table.
//!
//! One new jurisdiction is one new [`ArtifactTemplate`] entry; no code change to
//! [`crate::analysis::artifacts::followup_request`] or [`crate::analysis::artifacts::appeal`] is
//! required. Statutes are cited by section number (never by name alone) so a reader can verify.
//!
//! The body is a small line-based template with `{PLACEHOLDER}` markers filled at draft time:
//! - `{TODAY}` — today's date in `YYYY-MM-DD`.
//! - `{AGENCY_LABEL}` — human-readable agency identifier, `[AGENCY NAME]` when unknown.
//! - `{REQUEST_LABEL}` — request-number label, `[YOUR REQUEST NUMBER]` when unknown.
//! - `{FINDING_HEADLINE}` — the source finding's own headline (already user-visible).
//! - `{FINDING_EXPLANATION}` — the source finding's own explanation.
//! - `{EVIDENCE_BULLETS}` — bullet list of the finding's citations, one per line.
//! - `{AUTHORITY_CITATIONS}` — the template's `cited_authorities` rendered as "Cal. Gov. Code § X".
//!
//! Any placeholder that resolves to an empty string is left literally so the drafter sees the
//! placeholder and fills it. The alternative (silently omitting) would produce drafts that read as
//! polished but hide the gap.

use super::{ArtifactKind, CitedAuthority};
use crate::analysis::finding::FindingKind;
use std::sync::LazyLock;

/// One template. Multiple templates may apply to the same finding kind; the first one whose
/// `applicable_finding_kinds` includes the finding's kind AND whose jurisdiction matches the caller
/// is used. Templates are ordered from most-specific to fallback.
#[derive(Debug, Clone)]
pub struct ArtifactTemplate {
    pub id: &'static str,
    pub kind: ArtifactKind,
    /// e.g. "US-CA" for California, "US-*" for a federal fallback. Callers pass a specific
    /// jurisdiction; the matcher accepts either an exact match or a "US-*"-style wildcard fallback.
    pub jurisdiction: &'static str,
    pub applicable_finding_kinds: &'static [FindingKind],
    pub subject: &'static str,
    pub body: &'static str,
    pub cited_authorities: &'static [StaticAuthority],
}

/// Statute reference as it sits in the templates table. Rendered into [`CitedAuthority`] at draft
/// time so the wire type doesn't carry `&'static str`.
#[derive(Debug, Clone, Copy)]
pub struct StaticAuthority {
    pub statute: &'static str,
    pub quote: Option<&'static str>,
}

impl StaticAuthority {
    pub fn to_owned_authority(self) -> CitedAuthority {
        CitedAuthority {
            statute: self.statute.to_string(),
            quote: self.quote.map(str::to_string),
        }
    }
}

// California Public Records Act — the top-level statutory home used by every CPRA template.
static CPRA_UMBRELLA: StaticAuthority = StaticAuthority {
    statute: "Cal. Gov. Code § 7920.000 et seq. (California Public Records Act)",
    quote: None,
};

// The deadline statute — used by extension-related and delay-related templates.
static CPRA_DEADLINE: StaticAuthority = StaticAuthority {
    statute: "Cal. Gov. Code § 7922.535 (initial 10-day determination; up to 14-day extension for enumerated causes)",
    quote: None,
};

// The core disclosure obligation — used by "no records" contradiction appeals.
static CPRA_DISCLOSURE: StaticAuthority = StaticAuthority {
    statute: "Cal. Gov. Code § 7922.525 (agency shall make records promptly available)",
    quote: None,
};

// Written justification for withholding — used when denial language is challenged.
static CPRA_WRITTEN_JUSTIFICATION: StaticAuthority = StaticAuthority {
    statute:
        "Cal. Gov. Code § 7922.540 (written response required, exemptions must be cited with basis)",
    quote: None,
};

// Segregability — used when a redaction pattern suggests over-withholding.
static CPRA_SEGREGABILITY: StaticAuthority = StaticAuthority {
    statute: "Cal. Gov. Code § 7922.525(b) (agency shall segregate exempt and non-exempt portions)",
    quote: None,
};

// A universally-applicable template used when no more specific one exists.
static GENERIC_CPRA_FOLLOWUP_AUTHORITIES: [StaticAuthority; 1] = [CPRA_UMBRELLA];

static ABSENCE_FOLLOWUP_AUTHORITIES: [StaticAuthority; 2] = [CPRA_UMBRELLA, CPRA_DISCLOSURE];

static METADATA_FOLLOWUP_AUTHORITIES: [StaticAuthority; 1] = [CPRA_UMBRELLA];

static EXTENSION_FOLLOWUP_AUTHORITIES: [StaticAuthority; 2] = [CPRA_UMBRELLA, CPRA_DEADLINE];

static BOILERPLATE_FOLLOWUP_AUTHORITIES: [StaticAuthority; 2] =
    [CPRA_UMBRELLA, CPRA_WRITTEN_JUSTIFICATION];

static NO_RECORDS_APPEAL_AUTHORITIES: [StaticAuthority; 3] =
    [CPRA_UMBRELLA, CPRA_DISCLOSURE, CPRA_WRITTEN_JUSTIFICATION];

static BOILERPLATE_APPEAL_AUTHORITIES: [StaticAuthority; 3] = [
    CPRA_UMBRELLA,
    CPRA_WRITTEN_JUSTIFICATION,
    CPRA_SEGREGABILITY,
];

static EXTENSION_APPEAL_AUTHORITIES: [StaticAuthority; 2] = [CPRA_UMBRELLA, CPRA_DEADLINE];

static CPRA_FOLLOWUPS: &[FindingKind] = &[
    FindingKind::HeldBackResponse,
    FindingKind::BoilerplateDenial,
    FindingKind::JustInTimeCompliance,
    FindingKind::ExtensionAbuse,
    FindingKind::AbsenceOfRecords,
    FindingKind::DifferentialTreatment,
    FindingKind::MetadataContradictsContent,
    FindingKind::RegeneratedResponse,
    FindingKind::EmailThreadGap,
    FindingKind::ContradictoryStatement,
    FindingKind::NoRecordsContradictedByRelease,
];

static CPRA_APPEAL_NO_RECORDS: &[FindingKind] = &[FindingKind::NoRecordsContradictedByRelease];
static CPRA_APPEAL_BOILERPLATE: &[FindingKind] = &[FindingKind::BoilerplateDenial];
static CPRA_APPEAL_EXTENSION: &[FindingKind] = &[FindingKind::ExtensionAbuse];

static TEMPLATES: LazyLock<Vec<ArtifactTemplate>> = LazyLock::new(|| {
    vec![
        // --- CPRA follow-up templates, ordered most-specific to fallback. ---
        ArtifactTemplate {
            id: "cpra_followup_absence_v1",
            kind: ArtifactKind::FollowupRequest,
            jurisdiction: "US-CA",
            applicable_finding_kinds: &[FindingKind::AbsenceOfRecords],
            subject: "Follow-up request: missing items from a self-labelled sequence",
            body: "Date: {TODAY}\n\
                To: {AGENCY_LABEL}\n\
                Re: Follow-up to {REQUEST_LABEL}\n\n\
                Under the California Public Records Act (Cal. Gov. Code § 7920.000 et seq.), I am \
                writing to follow up on records released in response to {REQUEST_LABEL}. The \
                released material contains a self-labelled sequential numbering scheme, and one \
                or more numbers within the tight span of that sequence are not among the \
                documents released. Details of the observed sequence:\n\n\
                {FINDING_HEADLINE}\n\n\
                {FINDING_EXPLANATION}\n\n\
                Supporting evidence from the released set:\n\
                {EVIDENCE_BULLETS}\n\n\
                Under {AUTHORITY_CITATIONS}, I am requesting the specific items missing from the \
                sequence, or a written explanation identifying the exemption relied on to withhold \
                each missing item.\n\n\
                Please respond within the statutory 10-day determination window (Cal. Gov. Code § \
                7922.535). If any missing item is redacted rather than withheld in full, please \
                release the segregable portions per § 7922.525(b).\n\n\
                Sincerely,\n\
                [YOUR NAME]",
            cited_authorities: &ABSENCE_FOLLOWUP_AUTHORITIES,
        },
        ArtifactTemplate {
            id: "cpra_followup_metadata_v1",
            kind: ArtifactKind::FollowupRequest,
            jurisdiction: "US-CA",
            applicable_finding_kinds: &[
                FindingKind::HeldBackResponse,
                FindingKind::MetadataContradictsContent,
                FindingKind::RegeneratedResponse,
            ],
            subject: "Follow-up request: records concerning the preparation timing of the response",
            body: "Date: {TODAY}\n\
                To: {AGENCY_LABEL}\n\
                Re: Follow-up to {REQUEST_LABEL}\n\n\
                Under the California Public Records Act, I am writing to follow up on records \
                released in response to {REQUEST_LABEL}. Metadata carried by the released \
                material raises a specific question about when the response was prepared \
                relative to when it was released:\n\n\
                {FINDING_HEADLINE}\n\n\
                {FINDING_EXPLANATION}\n\n\
                Supporting metadata evidence:\n\
                {EVIDENCE_BULLETS}\n\n\
                I am requesting records reflecting the internal preparation and review of the \
                response to {REQUEST_LABEL}, including but not limited to: staff assignment \
                records for the request, drafts of the response prior to its release, and any \
                internal correspondence discussing the timing of release.\n\n\
                Under {AUTHORITY_CITATIONS}, please respond within the statutory 10-day \
                determination window. Segregable portions of any responsive record should be \
                released per Cal. Gov. Code § 7922.525(b).\n\n\
                Sincerely,\n\
                [YOUR NAME]",
            cited_authorities: &METADATA_FOLLOWUP_AUTHORITIES,
        },
        ArtifactTemplate {
            id: "cpra_followup_extension_v1",
            kind: ArtifactKind::FollowupRequest,
            jurisdiction: "US-CA",
            applicable_finding_kinds: &[FindingKind::ExtensionAbuse],
            subject: "Follow-up request: records demonstrating basis for repeated extensions",
            body: "Date: {TODAY}\n\
                To: {AGENCY_LABEL}\n\
                Re: Follow-up regarding repeated statutory-extension invocations\n\n\
                Under the California Public Records Act, I am writing to request records that \
                would demonstrate the actual basis for the agency's repeated reliance on one \
                statutory extension ground. The pattern the released record set shows:\n\n\
                {FINDING_HEADLINE}\n\n\
                {FINDING_EXPLANATION}\n\n\
                Cal. Gov. Code § 7922.535(b) permits extensions of the initial 10-day \
                determination window only when specific enumerated conditions are actually \
                present. Systemic reliance on a single ground is a pattern worth verifying.\n\n\
                Evidence supporting this observation:\n\
                {EVIDENCE_BULLETS}\n\n\
                I am requesting, under {AUTHORITY_CITATIONS}, records showing the specific factual \
                basis for each extension invocation over the past 12 months, including any \
                internal template or standard-operating-procedure documents that shape how the \
                agency selects an extension ground.\n\n\
                Please respond within the statutory determination window.\n\n\
                Sincerely,\n\
                [YOUR NAME]",
            cited_authorities: &EXTENSION_FOLLOWUP_AUTHORITIES,
        },
        ArtifactTemplate {
            id: "cpra_followup_boilerplate_v1",
            kind: ArtifactKind::FollowupRequest,
            jurisdiction: "US-CA",
            applicable_finding_kinds: &[FindingKind::BoilerplateDenial],
            subject: "Follow-up request: policy authorizing repeated denial language",
            body: "Date: {TODAY}\n\
                To: {AGENCY_LABEL}\n\
                Re: Follow-up on denial pattern\n\n\
                Under the California Public Records Act, I am writing to request records that \
                would demonstrate whether the identical or near-identical denial language the \
                agency has used across multiple distinct requests reflects individualized review \
                of each request or a standing template applied without one.\n\n\
                Pattern observed in the released material:\n\
                {FINDING_HEADLINE}\n\n\
                {FINDING_EXPLANATION}\n\n\
                Supporting evidence:\n\
                {EVIDENCE_BULLETS}\n\n\
                Under {AUTHORITY_CITATIONS}, please provide any internal template, form letter, \
                or standard-operating-procedure document authorizing the repeated denial \
                language, plus any records reflecting the individualized review conducted for \
                each identified request.\n\n\
                Please respond within the statutory 10-day determination window.\n\n\
                Sincerely,\n\
                [YOUR NAME]",
            cited_authorities: &BOILERPLATE_FOLLOWUP_AUTHORITIES,
        },
        // --- Generic CPRA fallback follow-up: applies to any finding kind not caught above. ---
        ArtifactTemplate {
            id: "cpra_followup_generic_v1",
            kind: ArtifactKind::FollowupRequest,
            jurisdiction: "US-CA",
            applicable_finding_kinds: CPRA_FOLLOWUPS,
            subject: "Follow-up request",
            body: "Date: {TODAY}\n\
                To: {AGENCY_LABEL}\n\
                Re: Follow-up to {REQUEST_LABEL}\n\n\
                Under the California Public Records Act (Cal. Gov. Code § 7920.000 et seq.), I am \
                writing to follow up on the response to {REQUEST_LABEL}. The following observation \
                warrants clarification or additional records:\n\n\
                {FINDING_HEADLINE}\n\n\
                {FINDING_EXPLANATION}\n\n\
                Supporting evidence:\n\
                {EVIDENCE_BULLETS}\n\n\
                Under {AUTHORITY_CITATIONS}, please respond within the statutory 10-day \
                determination window (Cal. Gov. Code § 7922.535). If responsive records exist and \
                any portion is being withheld, please cite the specific statutory exemption \
                relied on for each withheld portion, per Cal. Gov. Code § 7922.540.\n\n\
                Sincerely,\n\
                [YOUR NAME]",
            cited_authorities: &GENERIC_CPRA_FOLLOWUP_AUTHORITIES,
        },
        // --- CPRA appeal templates. ---
        ArtifactTemplate {
            id: "cpra_appeal_no_records_v1",
            kind: ArtifactKind::Appeal,
            jurisdiction: "US-CA",
            applicable_finding_kinds: CPRA_APPEAL_NO_RECORDS,
            subject: "Appeal: denial contradicted by publicly-released records",
            body: "Date: {TODAY}\n\
                To: {AGENCY_LABEL} (Records Appeals)\n\
                Re: Appeal of denial to {REQUEST_LABEL}\n\n\
                I am writing to appeal the denial of my public records request {REQUEST_LABEL}, in \
                which the agency stated that no responsive records exist.\n\n\
                {FINDING_HEADLINE}\n\n\
                {FINDING_EXPLANATION}\n\n\
                The following documents released by the agency to other requesters contain \
                content responsive to the topic of my request, contradicting the denial's factual \
                basis:\n\
                {EVIDENCE_BULLETS}\n\n\
                Under {AUTHORITY_CITATIONS}, the California Public Records Act requires an agency \
                to make responsive records promptly available (Cal. Gov. Code § 7922.525). A \
                denial resting on the assertion that no responsive records exist cannot stand \
                when the agency's own prior productions contain such records.\n\n\
                I request that this appeal be reviewed and that the responsive records be \
                released, with any withheld portions accompanied by the specific statutory \
                exemption relied on and the basis for its application (Cal. Gov. Code § 7922.540).\n\n\
                Sincerely,\n\
                [YOUR NAME]",
            cited_authorities: &NO_RECORDS_APPEAL_AUTHORITIES,
        },
        ArtifactTemplate {
            id: "cpra_appeal_boilerplate_v1",
            kind: ArtifactKind::Appeal,
            jurisdiction: "US-CA",
            applicable_finding_kinds: CPRA_APPEAL_BOILERPLATE,
            subject: "Appeal: denial appears to reflect a template rather than individualized review",
            body: "Date: {TODAY}\n\
                To: {AGENCY_LABEL} (Records Appeals)\n\
                Re: Appeal of denial to {REQUEST_LABEL}\n\n\
                I am appealing the denial of my public records request {REQUEST_LABEL}. The \
                denial's language appears verbatim or near-verbatim across multiple unrelated \
                requests the agency has responded to:\n\n\
                {FINDING_HEADLINE}\n\n\
                {FINDING_EXPLANATION}\n\n\
                Supporting instances of the repeated language:\n\
                {EVIDENCE_BULLETS}\n\n\
                Under {AUTHORITY_CITATIONS}, the California Public Records Act requires an agency \
                to state the reasons for withholding a record with reference to the specific \
                exemption relied on (Cal. Gov. Code § 7922.540) and to segregate exempt from \
                non-exempt portions (Cal. Gov. Code § 7922.525(b)). A response that consists \
                only of stock language, without an individualized statement of how the cited \
                exemption applies to the records at issue in this request, does not satisfy the \
                statutory obligation.\n\n\
                I request that the denial be reviewed, that any responsive segregable portions be \
                released, and that the agency provide an individualized statement of the basis \
                for any continued withholding.\n\n\
                Sincerely,\n\
                [YOUR NAME]",
            cited_authorities: &BOILERPLATE_APPEAL_AUTHORITIES,
        },
        ArtifactTemplate {
            id: "cpra_appeal_extension_v1",
            kind: ArtifactKind::Appeal,
            jurisdiction: "US-CA",
            applicable_finding_kinds: CPRA_APPEAL_EXTENSION,
            subject: "Appeal: pattern of repeated extension invocations",
            body: "Date: {TODAY}\n\
                To: {AGENCY_LABEL} (Records Appeals)\n\
                Re: Appeal regarding pattern of statutory extensions\n\n\
                I am appealing the agency's pattern of relying on a single statutory extension \
                ground under Cal. Gov. Code § 7922.535(b) to defer determinations on public \
                records requests:\n\n\
                {FINDING_HEADLINE}\n\n\
                {FINDING_EXPLANATION}\n\n\
                Supporting evidence:\n\
                {EVIDENCE_BULLETS}\n\n\
                Under {AUTHORITY_CITATIONS}, an extension of the 10-day initial determination \
                window is permitted only when the specific enumerated conditions in § 7922.535(b) \
                are actually present. A systemic pattern of invoking one ground on a large \
                fraction of requests raises the question whether the ground is being asserted \
                pretextually rather than because the enumerated condition is present in each \
                specific request.\n\n\
                I request that the agency's use of this ground be reviewed, that the specific \
                factual basis for each invocation be provided, and that any requests where the \
                condition was not actually present receive an immediate determination.\n\n\
                Sincerely,\n\
                [YOUR NAME]",
            cited_authorities: &EXTENSION_APPEAL_AUTHORITIES,
        },
    ]
});

/// Returns the first template whose kind + jurisdiction + applicable finding kinds match, if any.
/// The registry is ordered most-specific first so this simply takes the first hit.
pub fn find_template(
    kind: ArtifactKind,
    jurisdiction: &str,
    finding_kind: FindingKind,
) -> Option<&'static ArtifactTemplate> {
    TEMPLATES.iter().find(|template| {
        template.kind == kind
            && jurisdiction_matches(template.jurisdiction, jurisdiction)
            && template.applicable_finding_kinds.contains(&finding_kind)
    })
}

/// Simple matcher. Exact match, or a "US-*" wildcard that matches any US-prefixed jurisdiction.
fn jurisdiction_matches(template_jurisdiction: &str, requested: &str) -> bool {
    if template_jurisdiction == requested {
        return true;
    }
    if let Some(prefix) = template_jurisdiction.strip_suffix('*') {
        return requested.starts_with(prefix);
    }
    false
}

/// Renders the template's `cited_authorities` into the string form templates use in body text.
/// Joined with "; " so multiple authorities in one sentence read naturally.
pub fn render_authority_line(template: &ArtifactTemplate) -> String {
    template
        .cited_authorities
        .iter()
        .map(|authority| authority.statute)
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_template_id_is_unique() {
        let mut ids: Vec<&str> = TEMPLATES.iter().map(|t| t.id).collect();
        ids.sort_unstable();
        let count_before = ids.len();
        ids.dedup();
        assert_eq!(count_before, ids.len(), "duplicate template id detected");
    }

    #[test]
    fn every_cpra_finding_kind_has_a_matching_followup_template() {
        for kind in CPRA_FOLLOWUPS {
            let hit = find_template(ArtifactKind::FollowupRequest, "US-CA", *kind);
            assert!(
                hit.is_some(),
                "no CPRA follow-up template applies to {kind:?}"
            );
        }
    }

    #[test]
    fn absence_kind_selects_the_specific_template_not_the_generic() {
        let template = find_template(
            ArtifactKind::FollowupRequest,
            "US-CA",
            FindingKind::AbsenceOfRecords,
        )
        .expect("should find a template");
        assert_eq!(template.id, "cpra_followup_absence_v1");
    }

    #[test]
    fn no_records_kind_selects_no_records_appeal_when_kind_is_appeal() {
        let template = find_template(
            ArtifactKind::Appeal,
            "US-CA",
            FindingKind::NoRecordsContradictedByRelease,
        )
        .expect("should find an appeal template");
        assert_eq!(template.id, "cpra_appeal_no_records_v1");
    }

    #[test]
    fn a_kind_with_no_appeal_template_returns_none() {
        // JustInTimeCompliance has a follow-up template but no appeal template.
        let hit = find_template(
            ArtifactKind::Appeal,
            "US-CA",
            FindingKind::JustInTimeCompliance,
        );
        assert!(
            hit.is_none(),
            "no appeal template should exist for this kind"
        );
    }

    #[test]
    fn jurisdiction_wildcard_matches_us_prefix() {
        assert!(jurisdiction_matches("US-*", "US-CA"));
        assert!(jurisdiction_matches("US-*", "US-NY"));
        assert!(!jurisdiction_matches("US-*", "EU-DE"));
    }

    #[test]
    fn render_authority_line_joins_statutes() {
        let template = find_template(
            ArtifactKind::Appeal,
            "US-CA",
            FindingKind::NoRecordsContradictedByRelease,
        )
        .unwrap();
        let line = render_authority_line(template);
        assert!(line.contains("§ 7920.000"));
        assert!(line.contains("§ 7922.525"));
        assert!(line.contains("§ 7922.540"));
    }

    #[test]
    fn every_template_cites_a_statute_by_section_number() {
        for template in TEMPLATES.iter() {
            assert!(
                !template.cited_authorities.is_empty(),
                "template {} cites no authorities",
                template.id
            );
            for authority in template.cited_authorities {
                assert!(
                    authority.statute.contains('§'),
                    "template {} statute lacks a section marker: {}",
                    template.id,
                    authority.statute
                );
            }
        }
    }
}
