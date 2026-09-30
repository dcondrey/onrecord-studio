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

// A universally-applicable template used when no more specific one exists.
static GENERIC_CPRA_FOLLOWUP_AUTHORITIES: [StaticAuthority; 1] = [CPRA_UMBRELLA];

static ABSENCE_FOLLOWUP_AUTHORITIES: [StaticAuthority; 2] = [CPRA_UMBRELLA, CPRA_DISCLOSURE];

static EMAIL_GAP_FOLLOWUP_AUTHORITIES: [StaticAuthority; 2] = [CPRA_UMBRELLA, CPRA_DISCLOSURE];

static METADATA_FOLLOWUP_AUTHORITIES: [StaticAuthority; 1] = [CPRA_UMBRELLA];

static EXTENSION_FOLLOWUP_AUTHORITIES: [StaticAuthority; 2] = [CPRA_UMBRELLA, CPRA_DEADLINE];

static BOILERPLATE_FOLLOWUP_AUTHORITIES: [StaticAuthority; 2] =
    [CPRA_UMBRELLA, CPRA_WRITTEN_JUSTIFICATION];

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
    FindingKind::PolicyViolationDetected,
];

static POLICY_APPEAL_AUTHORITIES: [StaticAuthority; 1] = [CPRA_DEADLINE];

static CPRA_APPEAL_POLICY: &[FindingKind] = &[FindingKind::PolicyViolationDetected];

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
            id: "cpra_followup_email_gap_v1",
            kind: ArtifactKind::FollowupRequest,
            jurisdiction: "US-CA",
            applicable_finding_kinds: &[FindingKind::EmailThreadGap],
            subject: "Follow-up request: email messages referenced by released messages",
            body: "Date: {TODAY}\n\
                To: {AGENCY_LABEL}\n\
                Re: Follow-up to {REQUEST_LABEL}\n\n\
                Under the California Public Records Act, I am writing about records released in \
                response to {REQUEST_LABEL}. A released email names, in its reply headers, a \
                Message-ID that is not among the messages released.\n\n\
                {FINDING_HEADLINE}\n\n\
                {FINDING_EXPLANATION}\n\n\
                The released messages that carry the reference:\n\
                {EVIDENCE_BULLETS}\n\n\
                Under {AUTHORITY_CITATIONS}, I am requesting the message or messages with the \
                Message-ID(s) named above, or, if a message is being withheld in whole or in \
                part, the specific exemption relied on. If no such message exists in the agency's \
                possession, please say so. This request does not assume the message is \
                responsive or non-exempt.\n\n\
                Sincerely,\n\
                [YOUR NAME]",
            cited_authorities: &EMAIL_GAP_FOLLOWUP_AUTHORITIES,
        },
        ArtifactTemplate {
            id: "cpra_followup_held_back_v1",
            kind: ArtifactKind::FollowupRequest,
            jurisdiction: "US-CA",
            applicable_finding_kinds: &[FindingKind::HeldBackResponse],
            subject: "Follow-up request: transmittal and approval records for a response",
            body: "Date: {TODAY}\n\
                To: {AGENCY_LABEL}\n\
                Re: Follow-up to {REQUEST_LABEL}\n\n\
                Under the California Public Records Act, I am writing about the response to \
                {REQUEST_LABEL}. Metadata in the released response records an earlier \
                modification date than its release date:\n\n\
                {FINDING_HEADLINE}\n\n\
                {FINDING_EXPLANATION}\n\n\
                Supporting metadata:\n\
                {EVIDENCE_BULLETS}\n\n\
                Under {AUTHORITY_CITATIONS}, I am requesting the transmittal and approval records \
                for that response: the record showing when it was transmitted to the requester \
                and any internal approval or sign-off records for its release, with their dates. \
                Metadata dates can have ordinary explanations, and this request does not assume \
                otherwise.\n\n\
                Sincerely,\n\
                [YOUR NAME]",
            cited_authorities: &METADATA_FOLLOWUP_AUTHORITIES,
        },
        ArtifactTemplate {
            id: "cpra_followup_contradiction_v1",
            kind: ArtifactKind::FollowupRequest,
            jurisdiction: "US-CA",
            applicable_finding_kinds: &[FindingKind::ContradictoryStatement],
            subject: "Follow-up request: clarification of differing statements in released records",
            body: "Date: {TODAY}\n\
                To: {AGENCY_LABEL}\n\
                Re: Follow-up to {REQUEST_LABEL}\n\n\
                Under the California Public Records Act, I am writing about two released records \
                that state different values for the same item:\n\n\
                {FINDING_HEADLINE}\n\n\
                {FINDING_EXPLANATION}\n\n\
                The statements, with their locations:\n\
                {EVIDENCE_BULLETS}\n\n\
                I am asking the agency to identify which statement is current and to reconcile \
                the two, citing the record, policy, or decision that establishes the current \
                value. Under {AUTHORITY_CITATIONS}, I am also requesting any record that \
                supersedes or amends either document. The records may differ for a legitimate \
                reason, such as a revision; this request only asks which applies.\n\n\
                Sincerely,\n\
                [YOUR NAME]",
            cited_authorities: &GENERIC_CPRA_FOLLOWUP_AUTHORITIES,
        },
        ArtifactTemplate {
            id: "cpra_followup_claim_versus_data_v1",
            kind: ArtifactKind::FollowupRequest,
            jurisdiction: "US-CA",
            applicable_finding_kinds: &[FindingKind::ClaimContradictsData],
            subject: "Follow-up request: figures behind a stated total",
            body: "Date: {TODAY}\n\
                To: {AGENCY_LABEL}\n\
                Re: Follow-up to {REQUEST_LABEL}\n\n\
                Under the California Public Records Act, I am writing about a figure stated in a \
                released record that does not match the published data for the same period and \
                scope:\n\n\
                {FINDING_HEADLINE}\n\n\
                {FINDING_EXPLANATION}\n\n\
                The statement and the data it was compared against, with their locations:\n\
                {EVIDENCE_BULLETS}\n\n\
                I am asking the agency to identify the source of the stated figure and the basis, \
                fund, department and fiscal year it covers, and to provide the underlying report or \
                ledger rows from which it was derived. Under {AUTHORITY_CITATIONS}, I am also \
                requesting any later revision or correction of either the statement or the \
                published data. The two may differ for a legitimate reason, such as a different \
                basis, scope, rounding convention or a later revision; this request only asks \
                which figures apply and how they were reached.\n\n\
                Sincerely,\n\
                [YOUR NAME]",
            cited_authorities: &GENERIC_CPRA_FOLLOWUP_AUTHORITIES,
        },
        ArtifactTemplate {
            id: "cpra_followup_no_records_v1",
            kind: ArtifactKind::FollowupRequest,
            jurisdiction: "US-CA",
            applicable_finding_kinds: &[FindingKind::NoRecordsContradictedByRelease],
            subject: "Follow-up request: released documents that mention the topic of a request",
            body: "Date: {TODAY}\n\
                To: {AGENCY_LABEL}\n\
                Re: Follow-up to {REQUEST_LABEL}\n\n\
                Under the California Public Records Act, I am writing about the response to \
                {REQUEST_LABEL}, which stated that no responsive records were located. Other \
                documents the agency has released publicly appear to mention the same topic:\n\n\
                {FINDING_HEADLINE}\n\n\
                {FINDING_EXPLANATION}\n\n\
                The released documents:\n\
                {EVIDENCE_BULLETS}\n\n\
                Under {AUTHORITY_CITATIONS}, I am asking the agency to identify which of these \
                documents it considers within the scope of {REQUEST_LABEL}, to describe the \
                search it conducted, and to state why any of them was not treated as responsive. \
                A document can mention a topic without being responsive to a specific request, \
                and this request does not assume otherwise.\n\n\
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
        ArtifactTemplate {
            id: "cpra_followup_encampment_notice_v1",
            kind: ArtifactKind::FollowupRequest,
            jurisdiction: "US-CA",
            applicable_finding_kinds: &[
                FindingKind::EncampmentNoticeShortfall,
                FindingKind::EncampmentNoticeStatedShortfall,
            ],
            subject: "Follow-up request: posting and cleanup records for an encampment notice",
            body: "Date: {TODAY}\n\
                To: {AGENCY_LABEL}\n\
                Re: Follow-up to {REQUEST_LABEL}\n\n\
                Under the California Public Records Act, I am writing about a released encampment \
                notice and the minimum notice period that applies to it:\n\n\
                {FINDING_HEADLINE}\n\n\
                {FINDING_EXPLANATION}\n\n\
                The statements, with their locations:\n\
                {EVIDENCE_BULLETS}\n\n\
                Under {AUTHORITY_CITATIONS}, I am requesting the records that show when this \
                notice was posted at the site and when the cleanup took place: posting logs or \
                photographs, the cleanup or abatement schedule, work orders, and any record of a \
                rescheduled date. I am also requesting the notice procedure in force on the \
                posting date. This request only asks for the records.\n\n\
                Sincerely,\n\
                [YOUR NAME]",
            cited_authorities: &GENERIC_CPRA_FOLLOWUP_AUTHORITIES,
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
        // --- CPRA appeal templates: legal-claim kinds only. ---
        ArtifactTemplate {
            id: "cpra_appeal_policy_commitment_v1",
            kind: ArtifactKind::Appeal,
            jurisdiction: "US-CA",
            applicable_finding_kinds: CPRA_APPEAL_POLICY,
            subject: "Complaint: agency practice compared with its own stated response time",
            body: "Date: {TODAY}\n\
                To: {AGENCY_LABEL} (Records Appeals or Custodian of Records)\n\
                Re: Agency response times compared with the agency's own published policy\n\
                Example request on the portal: {REQUEST_LABEL} (the slowest counted request, cited \
                as an example)\n\n\
                I am writing to bring to the agency's attention a difference between the response \
                time its own policy document states and the response times recorded on the public \
                portal.\n\n\
                {FINDING_HEADLINE}\n\n\
                {FINDING_EXPLANATION}\n\n\
                The stated commitment, quoted from the agency's document, and its location:\n\
                {EVIDENCE_BULLETS}\n\n\
                Framework: {AUTHORITY_CITATIONS}. The California Public Records Act sets its own \
                determination period, stated in that section. This complaint does not assert that \
                the statutory period was exceeded on any request; it concerns the agency's own \
                stated commitment. Only requests with a recorded determination event and no \
                recorded extension were counted, as described above.\n\n\
                I ask the agency to review the counted requests against the commitment its policy \
                states, to tell me whether the policy is current, and to explain any request \
                where the stated response time was not met.\n\n\
                Sincerely,\n\
                [YOUR NAME]",
            cited_authorities: &POLICY_APPEAL_AUTHORITIES,
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
    use crate::analysis::artifacts::tests::all_kinds;

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
    fn appeal_templates_exist_exactly_for_the_appeal_family() {
        use crate::analysis::artifacts::DraftFamily;
        for kind in all_kinds() {
            let family = DraftFamily::for_kind(kind);
            let followup = find_template(ArtifactKind::FollowupRequest, "US-CA", kind).is_some();
            let appeal = find_template(ArtifactKind::Appeal, "US-CA", kind).is_some();
            assert_eq!(
                followup,
                family != DraftFamily::NoDraft,
                "{kind:?} follow-up template disagrees with its family"
            );
            assert_eq!(
                appeal,
                family == DraftFamily::RequestAndAppeal,
                "{kind:?} appeal template disagrees with its family"
            );
            if !kind.is_legal_claim() {
                assert!(
                    !appeal,
                    "pattern kind {kind:?} must never have an appeal template"
                );
            }
        }
    }

    #[test]
    fn narrowing_kinds_select_their_specific_followup() {
        for (kind, id) in [
            (FindingKind::EmailThreadGap, "cpra_followup_email_gap_v1"),
            (FindingKind::HeldBackResponse, "cpra_followup_held_back_v1"),
            (
                FindingKind::ContradictoryStatement,
                "cpra_followup_contradiction_v1",
            ),
            (
                FindingKind::NoRecordsContradictedByRelease,
                "cpra_followup_no_records_v1",
            ),
        ] {
            let template = find_template(ArtifactKind::FollowupRequest, "US-CA", kind).unwrap();
            assert_eq!(template.id, id);
        }
    }

    #[test]
    fn policy_appeal_cites_the_statute_the_constant_names() {
        let template = find_template(
            ArtifactKind::Appeal,
            "US-CA",
            FindingKind::PolicyViolationDetected,
        )
        .unwrap();
        let section = crate::analysis::statute::CPRA_STATUTE_CITATION
            .rsplit("§ ")
            .next()
            .unwrap();
        assert!(render_authority_line(template).contains(section));
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
            ArtifactKind::FollowupRequest,
            "US-CA",
            FindingKind::AbsenceOfRecords,
        )
        .unwrap();
        let line = render_authority_line(template);
        assert!(line.contains("§ 7920.000"));
        assert!(line.contains("§ 7922.525"));
        assert!(line.contains("; "));
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
