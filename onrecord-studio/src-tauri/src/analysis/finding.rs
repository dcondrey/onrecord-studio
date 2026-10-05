//! Universal shape every autonomous detector emits into the findings inbox.
//!
//! One `Finding` is one statement about the corpus that a detector believes is worth a user's
//! attention. Detectors emit findings on ingest and on explicit refresh; the inbox ranks them by
//! `(severity descending × confidence descending × last_seen_at descending)`.
//!
//! **Statistical claim vs. legal claim** — a `Finding` whose kind names a legal or policy
//! violation must derive from a counted violation (e.g. `VerificationState::StatutoryDeadlineExceeded`),
//! not a mere statistical anomaly. See `kind_supports_legal_claim` for the guard applied at write
//! time so this cannot silently drift.
//!
//! **Absence vs. measurement** — the confidence field is required on every finding, but its
//! semantics live in `Confidence`, an enum that distinguishes an unknown/unassessed rung from a
//! measured-zero rung. This mirrors the project's rule that a column whose default is a legal
//! value of the measurement it holds cannot say "not computed."

use crate::integrity::sha256_hex;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Bounded severity rungs. Ordered lowest-to-highest so `Ord` sorts a serious finding above an
/// info finding when the inbox is ranked.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FindingSeverity {
    Info,
    Notable,
    Serious,
    Critical,
}

impl FindingSeverity {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Info => "INFO",
            Self::Notable => "NOTABLE",
            Self::Serious => "SERIOUS",
            Self::Critical => "CRITICAL",
        }
    }

    /// Parses the form written to the `findings` table.
    pub fn from_stored(value: &str) -> Option<Self> {
        [Self::Info, Self::Notable, Self::Serious, Self::Critical]
            .into_iter()
            .find(|candidate| candidate.as_str() == value)
    }

    /// Weight for the ranking sort, higher = more prominent.
    pub const fn rank_weight(self) -> u8 {
        match self {
            Self::Info => 1,
            Self::Notable => 2,
            Self::Serious => 3,
            Self::Critical => 4,
        }
    }
}

/// The category a detector reports. New detectors extend this enum in place; the wire encoding is
/// SCREAMING_SNAKE_CASE so it round-trips through the ipc contract check.
///
/// **When adding a variant:** if the new kind reports a legal or policy violation (something a
/// user might cite in a filing), add it to `kind_supports_legal_claim` so writes with a merely
/// statistical basis are rejected at record time. If it's an informational or statistical
/// observation, leave that guard alone.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FindingKind {
    /// A response document whose PDF/OOXML modification timestamp predates its release by more
    /// than 24 hours, released within 48 hours of the statutory deadline — evidence the response
    /// was ready but held back. Not itself a legal violation (the deadline was met); it's a
    /// pattern signal about intent.
    HeldBackResponse,
    /// Near-identical denial language reused across otherwise-unrelated requests. Pattern signal
    /// about whether an agency is doing individualized review vs. running requests through a
    /// stock refusal. Not a legal claim.
    BoilerplateDenial,
    /// An agency whose closures cluster in the final 24 hours before the statutory deadline at a
    /// rate materially above the population base rate. Behavioral pattern about intent, not a
    /// violation — every closure was within the deadline.
    JustInTimeCompliance,
    /// One statutory extension reason accounts for the overwhelming majority of an agency's
    /// invoked extensions. Not itself a legal finding; a single valid extension is legitimate. A
    /// systemic pattern of relying on one ground is a pretext signal worth surfacing.
    ExtensionAbuse,
    /// A released document set carries a self-labelled sequential numbering scheme (invoices,
    /// memos, resolutions, agenda items, etc.) whose numeric span exceeds what was released, and
    /// one or more numbers in the tight sequence are not among the released documents. Pattern
    /// signal that records inside a range were withheld or omitted, not a legal claim on its own.
    AbsenceOfRecords,
    /// Outcomes on requests attributed to one requester class differ materially from an agency's
    /// population baseline (denial rate, latency, or redaction density). Pattern signal about
    /// differential treatment; not a legal claim without a per-request statutory finding.
    DifferentialTreatment,
    /// A document's own metadata contradicts its own content. Examples: content references events
    /// dated after the document's declared creation timestamp; a document labelled "Final"
    /// references a draft dated after itself; a signed contract cites an amendment postdating the
    /// signature. Pattern signal about backdating or silent modification.
    MetadataContradictsContent,
    /// A response document whose non-C2PA metadata (PDF ModDate, OOXML `dcterms:modified` or
    /// `lastPrinted`) predates its release by weeks or months, with the metadata rungs
    /// internally consistent. Distinct from `HeldBackResponse`: this one does not require the
    /// release to land near the deadline; it fires when the record was, on its face, prepared
    /// long before it was issued.
    RegeneratedResponse,
    /// A released `.msg` message references (via `In-Reply-To` or `References` headers) a
    /// Message-ID that was not itself part of the released set. Evidence that correspondence was
    /// pulled from a chain the requester was not shown, and the released set is not the whole
    /// exchange.
    EmailThreadGap,
    /// Two documents in the corpus assert the same `(subject, predicate)` fact with materially
    /// different values — e.g. one policy sets records retention at 3 years and another at 7, or
    /// two response letters state different response-time commitments. Pattern signal about
    /// internal consistency across the released material; not a legal claim on its own.
    ContradictoryStatement,
    /// A response letter denying that responsive records exist (e.g. "no responsive records",
    /// "unable to locate"), while at least three other public documents released to different
    /// requesters mention the same topic. Pattern signal about the denial's factual basis; the
    /// legal characterization of the denial depends on statutes and scope this detector does not
    /// evaluate.
    NoRecordsContradictedByRelease,
    /// An agency's denial language cluster shows HIGH bag-of-words similarity across otherwise
    /// distinct request topics, but LOW shingled similarity (below the threshold Wave 2B's
    /// boilerplate detector would merge them at). Same underlying refusal, distinct surface
    /// phrasing per response — the adaptive-paraphrasing pattern that would defeat direct
    /// boilerplate detection. Adversarial pattern signal, not a legal claim.
    EvasionParaphrasedDenials,
    /// An agency's released-document counts cluster tightly just below a round-number threshold
    /// (10, 25, 50, 100, 500, 1000) with few or no releases exceeding it. A ceiling pattern that
    /// would sit below the sequence-gap detector's radar; potentially responses sized to duck a
    /// heavier review triggered by crossing the number. Adversarial pattern signal, not a legal
    /// claim.
    EvasionRoundNumberThresholds,
    /// An agency's heavy-redaction documents show near-zero structural residual signals (leaks,
    /// invisible-mode text, unapplied Redact annotations, Form-XObject overlays) across a
    /// meaningful sample. Either exemplary redaction practice or redactions tuned to what the
    /// residual audit sees. Adversarial pattern signal, not a legal claim.
    EvasionRedactionAtLimit,
    /// A "no responsive records" denial followed by public releases from the SAME agency, dated
    /// after the denial's determination, whose text matches the denied request's topic vocabulary.
    /// Records surfacing after any plausible appeal window closed. Adversarial pattern signal,
    /// not a legal claim on its own.
    EvasionLateArrivingRecords,
    /// The words on both sides of a redaction mark appear, in the same order, in another
    /// collected document or in a row of an imported table, with text standing where the mark is.
    /// The finding cites both passages and never states what the redaction covers. Not a legal
    /// claim.
    RedactionCandidateInferred,
    /// Same denial paragraph appearing across multiple agencies (not just within one). Distinct
    /// from `BoilerplateDenial`, which stays within one agency. The cross-agency variant is
    /// evidence of a shared template, coordinated legal guidance, or a copy-paste circulating
    /// between offices. Pattern signal, not a legal claim.
    CrossCorpusSharedLanguage,
    /// One agency's behavior stepping over a specific date — heavy-redaction rate jumping by ≥ 40
    /// percentage points, or determination latency doubling with an absolute floor. The
    /// change-point search is a two-sample mean scan bounded by MIN_HISTORY_DAYS and requires a
    /// minimum window on either side. Pattern signal about institutional change, not a legal
    /// claim.
    CrossCorpusTemporalShift,
    /// A topic prominent in Agency A's records but entirely absent from Agency B's records within
    /// the same jurisdiction, when Agency B has enough total requests to trust the absence as
    /// meaningful. Pattern signal about scope asymmetry that warrants investigation. Not a legal
    /// claim.
    CrossCorpusOrphanTopic,
    /// One statutory extension reason invoked across topically diverse requests (Shannon entropy
    /// ≥ 1.5 bits over topic tokens), on top of the concentration gate. Distinct from
    /// `ExtensionAbuse`: the entropy check demands that the reason spans a broad set of subject
    /// matter, not one program area that legitimately keeps hitting the same ground. Pattern
    /// signal for a catch-all rationale, not a legal claim on its own.
    CrossCorpusReusedDenialGrounds,
    /// A commitment an agency's own policy document states (a calendar-day response time or a
    /// per-unit fee) that observed practice in the same portal exceeded: requests that took longer
    /// than the stated days, or responses that state a higher fee. A counted comparison against
    /// the agency's own words, not a rate or an outlier.
    PolicyViolationDetected,
    /// A quantitative statement in a released document (a total, a change, "within budget") that
    /// falls outside the rounding band its wording carries when checked against the imported
    /// budget or actuals table for the same fiscal year, fund and jurisdiction. A comparison of
    /// a sentence with a dataset, not a judgement of which is right.
    ClaimContradictsData,
    /// Two or more findings of different kinds that name exactly the same document, request or
    /// agency. Restates its members without adding a claim; confidence is the weakest member's.
    CorrelatedFindings,
    /// A policy sentence states a duty about a policing practice and another document of the same
    /// agency, in force after the policy's effective date, states `N of M` cases did not follow
    /// it. A counted comparison of two documents' statements; the count is the source's and is
    /// not recounted here.
    PracticeDepartureCounted,
    /// The same pairing, where the departure sentence states noncompliance without a count. A
    /// pattern signal: the extent is unknown.
    PracticeDepartureStated,
    /// Recorded payments on one contract sum to more than its executed ceiling (base amount plus complete executed amendments), with every input recorded. A counted comparison of ledger rows with the contract's own written amount.
    ContractCeilingExceeded,
    /// Posted payments dated before a contract's recorded start or after its recorded end (extended by executed amendments that state their own end date). A counted comparison of two recorded dates.
    PaymentOutsideContractTerm,
    /// Posted payments to a vendor that neither the contract register's vendors nor the payments' contract ids name, while the register is complete. Absence from a table; registers often omit small purchases.
    PaymentWithoutContractRecord,
    /// Recorded actual (or actual plus open encumbrance) spending above the recorded revised appropriation for one fund, year and canonical account. A counted comparison against the ledger's own authority; a conflicting crosswalk blocks it.
    AppropriationExceeded,
    /// Actual spending from a fund recorded as restricted, on a program the restriction register does not name. A counted comparison of exact program identifiers.
    RestrictedFundOffProgram,
    /// Rows above an approval limit the agency's own policy states, whose approval value explicitly records that no approval was obtained, dated while the policy was in force. Rule text, threshold and rows are all cited.
    ApprovalThresholdExceeded,
    /// Rows above a stated approval limit whose approval cell is blank or not applicable, or whose date does not show the policy in force. A missing record, not a statement that approval was not obtained.
    ApprovalRecordAbsent,
    /// Several payments to one vendor within a short window, each just under a stated limit, whose sum exceeds it. A pattern: the policy sentence cited does not forbid splitting.
    SplitPurchasePattern,
    /// The same clustering where the agency's own policy forbids dividing a purchase to stay under a limit and the payments fall in force. Whether it was one purchase is not recorded in the ledger.
    SplitPurchaseProhibitedByPolicy,
    /// A department's imported actual spending moved by at least the threshold share of its
    /// previous fiscal year's, both years read in full. A counted change, not an explanation.
    DepartmentSpendingYearOverYear,
    /// Imported actual spending on an account labelled overtime is at least the threshold share
    /// above the adopted amount for the same account, fund, department and year.
    OvertimeAboveAdoptedLine,
    /// A fund's imported actual spending is at least the threshold share above its adopted amount
    /// for the year. Adopted is not the revised authority, so this is not an appropriation test.
    FundActualAboveAdopted,
    /// An encampment notice whose recorded posting (or notice) date and recorded cleanup date are
    /// fewer calendar days apart than the minimum the agency's own policy, or a stored notice
    /// authority of the same jurisdiction, states. A counted comparison of stated dates.
    EncampmentNoticeShortfall,
    /// An encampment notice that states a calendar-day notice period shorter than that minimum,
    /// without both dates recorded. One statement against another; a pattern signal.
    EncampmentNoticeStatedShortfall,
    /// Two public copies of one file name in a portal, released under different requests, where
    /// a passage marked redacted or withheld in one reads as ordinary text in the other, matched
    /// by the words on either side. A comparison of two releases' redaction decisions; which was
    /// right is not decided.
    RedactionInconsistentAcrossReleases,
    /// A request whose portal timeline records documents released on several separate dates
    /// spread over months. Describes how the release was spread out; it is not a finding that any
    /// deadline was missed, and the release dates are never the agency's determination.
    RollingProduction,
    /// A count read from a publisher's service-report table (311-style reports): open reports
    /// older than a year, an area slower than the others, a place reported repeatedly, referrals
    /// with no outcome, or reports that left an open file between two copies. Counted by the
    /// discovery feed's SQL and recorded as sent; a count of the publisher's rows, not a finding
    /// that a duty was breached.
    ServiceReportLead,
    /// A count read from a police department's published stop records (RIPA): how often people
    /// the officer perceived as unhoused were stopped, arrested, cited or had property seized,
    /// and for what recorded reasons. Counted by the discovery feed's SQL and recorded as sent.
    /// A difference in rates is not adjusted for the reason for the stop and is not a finding of
    /// unequal treatment.
    PoliceStopLead,
    /// A count read from an agency's released encampment abatement logs: the hours between a
    /// posted notice and the abatement at the same place, and abatements at a place with no
    /// notice in the log. Counted by the discovery feed's SQL and recorded as sent. The logs do
    /// not record the notice period that applied, so this is not a finding that notice fell short.
    AbatementLogLead,
    /// A count read from a police department's released arrest and citation log: arrests and
    /// citations citing an encampment, encroachment, lodging or public urination section, and
    /// arrests on those sections alone. Counted by the discovery feed's SQL and recorded as sent.
    /// The log records the charge, not housing status, so this is not a finding about who was
    /// charged or why.
    ChargeLogLead,
    /// Three counted series set side by side per fiscal year: a department's adopted operating
    /// expense, Encampment reports residents filed, and emergency shelter beds in the January
    /// inventory. Counted by the discovery feed's SQL and recorded as sent. A comparison between
    /// sources, not a statement that one series moved another.
    CapacityComparisonLead,
    /// An agency's releases began at or after 3 p.m. on the last working day before a weekend or
    /// California court holiday at a share at least twice, and ten points above, that of the
    /// other releases on the same portal. A count of the portal's recorded upload times. It does
    /// not show why the uploads fall where they do: work is often finished and posted at the end
    /// of a week, and the portal does not record when an uploaded file was made public.
    ReleaseTimingConcentration,
}

impl FindingKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::HeldBackResponse => "HELD_BACK_RESPONSE",
            Self::BoilerplateDenial => "BOILERPLATE_DENIAL",
            Self::JustInTimeCompliance => "JUST_IN_TIME_COMPLIANCE",
            Self::ExtensionAbuse => "EXTENSION_ABUSE",
            Self::AbsenceOfRecords => "ABSENCE_OF_RECORDS",
            Self::DifferentialTreatment => "DIFFERENTIAL_TREATMENT",
            Self::MetadataContradictsContent => "METADATA_CONTRADICTS_CONTENT",
            Self::RegeneratedResponse => "REGENERATED_RESPONSE",
            Self::EmailThreadGap => "EMAIL_THREAD_GAP",
            Self::ContradictoryStatement => "CONTRADICTORY_STATEMENT",
            Self::NoRecordsContradictedByRelease => "NO_RECORDS_CONTRADICTED_BY_RELEASE",
            Self::EvasionParaphrasedDenials => "EVASION_PARAPHRASED_DENIALS",
            Self::EvasionRoundNumberThresholds => "EVASION_ROUND_NUMBER_THRESHOLDS",
            Self::EvasionRedactionAtLimit => "EVASION_REDACTION_AT_LIMIT",
            Self::EvasionLateArrivingRecords => "EVASION_LATE_ARRIVING_RECORDS",
            Self::RedactionCandidateInferred => "REDACTION_CANDIDATE_INFERRED",
            Self::CrossCorpusSharedLanguage => "CROSS_CORPUS_SHARED_LANGUAGE",
            Self::CrossCorpusTemporalShift => "CROSS_CORPUS_TEMPORAL_SHIFT",
            Self::CrossCorpusOrphanTopic => "CROSS_CORPUS_ORPHAN_TOPIC",
            Self::CrossCorpusReusedDenialGrounds => "CROSS_CORPUS_REUSED_DENIAL_GROUNDS",
            Self::PolicyViolationDetected => "POLICY_VIOLATION_DETECTED",
            Self::ClaimContradictsData => "CLAIM_CONTRADICTS_DATA",
            Self::CorrelatedFindings => "CORRELATED_FINDINGS",
            Self::PracticeDepartureCounted => "PRACTICE_DEPARTURE_COUNTED",
            Self::PracticeDepartureStated => "PRACTICE_DEPARTURE_STATED",
            Self::ContractCeilingExceeded => "CONTRACT_CEILING_EXCEEDED",
            Self::PaymentOutsideContractTerm => "PAYMENT_OUTSIDE_CONTRACT_TERM",
            Self::PaymentWithoutContractRecord => "PAYMENT_WITHOUT_CONTRACT_RECORD",
            Self::AppropriationExceeded => "APPROPRIATION_EXCEEDED",
            Self::RestrictedFundOffProgram => "RESTRICTED_FUND_OFF_PROGRAM",
            Self::ApprovalThresholdExceeded => "APPROVAL_THRESHOLD_EXCEEDED",
            Self::ApprovalRecordAbsent => "APPROVAL_RECORD_ABSENT",
            Self::SplitPurchasePattern => "SPLIT_PURCHASE_PATTERN",
            Self::SplitPurchaseProhibitedByPolicy => "SPLIT_PURCHASE_PROHIBITED_BY_POLICY",
            Self::DepartmentSpendingYearOverYear => "DEPARTMENT_SPENDING_YEAR_OVER_YEAR",
            Self::OvertimeAboveAdoptedLine => "OVERTIME_ABOVE_ADOPTED_LINE",
            Self::FundActualAboveAdopted => "FUND_ACTUAL_ABOVE_ADOPTED",
            Self::EncampmentNoticeShortfall => "ENCAMPMENT_NOTICE_SHORTFALL",
            Self::EncampmentNoticeStatedShortfall => "ENCAMPMENT_NOTICE_STATED_SHORTFALL",
            Self::RedactionInconsistentAcrossReleases => "REDACTION_INCONSISTENT_ACROSS_RELEASES",
            Self::RollingProduction => "ROLLING_PRODUCTION",
            Self::ServiceReportLead => "SERVICE_REPORT_LEAD",
            Self::PoliceStopLead => "POLICE_STOP_LEAD",
            Self::AbatementLogLead => "ABATEMENT_LOG_LEAD",
            Self::ChargeLogLead => "CHARGE_LOG_LEAD",
            Self::CapacityComparisonLead => "CAPACITY_COMPARISON_LEAD",
            Self::ReleaseTimingConcentration => "RELEASE_TIMING_CONCENTRATION",
        }
    }

    pub fn from_stored(value: &str) -> Option<Self> {
        [
            Self::HeldBackResponse,
            Self::BoilerplateDenial,
            Self::JustInTimeCompliance,
            Self::ExtensionAbuse,
            Self::AbsenceOfRecords,
            Self::DifferentialTreatment,
            Self::MetadataContradictsContent,
            Self::RegeneratedResponse,
            Self::EmailThreadGap,
            Self::ContradictoryStatement,
            Self::NoRecordsContradictedByRelease,
            Self::EvasionParaphrasedDenials,
            Self::EvasionRoundNumberThresholds,
            Self::EvasionRedactionAtLimit,
            Self::EvasionLateArrivingRecords,
            Self::RedactionCandidateInferred,
            Self::CrossCorpusSharedLanguage,
            Self::CrossCorpusTemporalShift,
            Self::CrossCorpusOrphanTopic,
            Self::CrossCorpusReusedDenialGrounds,
            Self::PolicyViolationDetected,
            Self::ClaimContradictsData,
            Self::CorrelatedFindings,
            Self::PracticeDepartureCounted,
            Self::PracticeDepartureStated,
            Self::ContractCeilingExceeded,
            Self::PaymentOutsideContractTerm,
            Self::PaymentWithoutContractRecord,
            Self::AppropriationExceeded,
            Self::RestrictedFundOffProgram,
            Self::ApprovalThresholdExceeded,
            Self::ApprovalRecordAbsent,
            Self::SplitPurchasePattern,
            Self::SplitPurchaseProhibitedByPolicy,
            Self::DepartmentSpendingYearOverYear,
            Self::OvertimeAboveAdoptedLine,
            Self::FundActualAboveAdopted,
            Self::EncampmentNoticeShortfall,
            Self::EncampmentNoticeStatedShortfall,
            Self::RedactionInconsistentAcrossReleases,
            Self::RollingProduction,
            Self::ServiceReportLead,
            Self::PoliceStopLead,
            Self::AbatementLogLead,
            Self::ChargeLogLead,
            Self::CapacityComparisonLead,
            Self::ReleaseTimingConcentration,
        ]
        .into_iter()
        .find(|candidate| candidate.as_str() == value)
    }

    /// True when the kind names a legal or policy conclusion. Callers building a Finding of a
    /// legal-claim kind must supply `EvidenceBasis::CountedViolation`; the write path enforces
    /// this. See `Finding::record_safe`.
    pub const fn is_legal_claim(self) -> bool {
        match self {
            // HeldBackResponse is a behavioral/pattern signal about intent, not a violation of a
            // stated rule. Kept as a non-legal-claim kind on purpose.
            Self::HeldBackResponse => false,
            // The three Wave 2B detectors are all statistical/pattern signals about agency
            // behavior. None counts a per-request statutory violation and none should be cited in
            // a filing as one — the statutory detector remains the sole source for that.
            Self::BoilerplateDenial => false,
            Self::JustInTimeCompliance => false,
            Self::ExtensionAbuse => false,
            // Wave 2D detectors: also pattern signals, not counted legal claims. A gap in
            // released numbering is evidence records were omitted; it does not itself say the
            // omission was unlawful. Differential treatment across requester classes is a
            // statistical pattern, not a per-request statutory finding. A metadata/content
            // contradiction points at backdating or silent modification; it does not by itself
            // establish a legal violation of any particular rule.
            Self::AbsenceOfRecords => false,
            Self::DifferentialTreatment => false,
            Self::MetadataContradictsContent => false,
            // Wave 2C detectors: both are pattern signals over metadata and released set
            // membership. RegeneratedResponse observes a metadata-vs-release delta; it does not
            // count a statutory-deadline violation. EmailThreadGap observes that a released
            // message names an unpublished Message-ID; whether that gap is a legal violation
            // depends on statutes and exemptions this detector does not evaluate.
            Self::RegeneratedResponse => false,
            Self::EmailThreadGap => false,
            // Wave 2A contradiction detectors: both are pattern signals about text consistency
            // across the released material, not legal claims. Two documents disagreeing is
            // evidence for review, not itself a violation of a stated rule.
            Self::ContradictoryStatement => false,
            Self::NoRecordsContradictedByRelease => false,
            // Wave 6 adversarial-evasion detectors: all pattern signals about adaptation to the
            // direct detectors, not counted statutory or policy violations. A cluster of
            // paraphrased denials, a ceiling below a round-number threshold, a suspiciously clean
            // heavy-redaction stream, and a late-arriving-records pattern are each evidence for
            // human review; none is itself a legal claim.
            Self::EvasionParaphrasedDenials => false,
            Self::EvasionRoundNumberThresholds => false,
            Self::EvasionRedactionAtLimit => false,
            Self::EvasionLateArrivingRecords => false,
            // Wave 9: inference from context is never a legal claim. The engine bounds candidates
            // below a decryption ceiling; users must never mistake an inferred identity for a
            // decoded one.
            Self::RedactionCandidateInferred => false,
            // Wave 8 cross-corpus detectors: all four are pattern signals over jurisdiction-,
            // time-, or topic-spanning aggregates. A shared paragraph, a shifted mean, an
            // asymmetric topic, or a concentrated ground across diverse topics — each is evidence
            // to review, not a per-request statutory finding.
            Self::CrossCorpusSharedLanguage => false,
            Self::CrossCorpusTemporalShift => false,
            Self::CrossCorpusOrphanTopic => false,
            Self::CrossCorpusReusedDenialGrounds => false,
            // The one legal-claim kind: every finding is a counted comparison of an observed
            // request or fee against a commitment the agency's own document states.
            Self::PolicyViolationDetected => true,
            // A sentence disagreeing with a table is a pattern signal for review; which of the
            // two is wrong, and whether either breaks a rule, is not decided here.
            Self::ClaimContradictsData => false,
            // Convergence of separate signals is not a violation; a legal member stays its own row.
            Self::CorrelatedFindings => false,
            // A document's own stated count of cases that did not follow the agency's written
            // rule, set against that rule's sentence. Counted in the document, not recounted
            // here, and never a statement that the conduct was unlawful.
            Self::PracticeDepartureCounted => true,
            // No count: the extent is unknown, so this is a pattern signal.
            Self::PracticeDepartureStated => false,
            // Spending cases: a comparison of recorded rows with a written limit or ceiling with every
            // input recorded is a counted claim; an absence or a clustering is a pattern.
            Self::ContractCeilingExceeded => true,
            Self::PaymentOutsideContractTerm => true,
            Self::PaymentWithoutContractRecord => false,
            Self::AppropriationExceeded => true,
            Self::RestrictedFundOffProgram => true,
            Self::ApprovalThresholdExceeded => true,
            Self::ApprovalRecordAbsent => false,
            Self::SplitPurchasePattern => false,
            Self::SplitPurchaseProhibitedByPolicy => true,
            // Differences between two imported totals. Adopted is not the revised authority and
            // a year-over-year change breaks no rule, so none of these is a legal claim.
            Self::DepartmentSpendingYearOverYear => false,
            Self::OvertimeAboveAdoptedLine => false,
            Self::FundActualAboveAdopted => false,
            // Two recorded dates counted against a stated minimum notice period.
            Self::EncampmentNoticeShortfall => true,
            // A stated period against a stated minimum, with the period actually given unrecorded.
            Self::EncampmentNoticeStatedShortfall => false,
            // Two releases disagreeing about a passage is a pattern signal; neither is judged.
            Self::RedactionInconsistentAcrossReleases => false,
            // A count of recorded release dates. Producing records in installments breaks no
            // rule this kind evaluates.
            Self::RollingProduction => false,
            // Counts of a publisher's rows. The table records no inspections, work or duty.
            Self::ServiceReportLead => false,
            // Counts of the department's own stop records, compared without adjustment.
            Self::PoliceStopLead => false,
            // Two recorded times with no stated minimum to count them against;
            // `EncampmentNoticeShortfall` is the kind that has one.
            Self::AbatementLogLead => false,
            // Counts of charges as the department recorded them.
            Self::ChargeLogLead => false,
            // Counts from three publishers beside each other; no rule is applied to them.
            Self::CapacityComparisonLead => false,
            // Two shares of counted upload times. When work is posted breaks no rule.
            Self::ReleaseTimingConcentration => false,
        }
    }
}

/// How a finding's confidence was arrived at.
///
/// Kept separate from a raw basis-points integer so an unknown rung is representable distinctly
/// from a computed-zero rung. `Unassessed` means the detector had no measurable basis at hand
/// (e.g. one required signal was absent from the corpus) — the row still records the finding but
/// with no numeric strength. `Measured(bp)` means the value was actively computed; 0 is a real
/// answer meaning "no strength" and reads differently from `Unassessed`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FindingConfidence {
    Unassessed,
    Measured { basis_points: u16 },
}

impl FindingConfidence {
    pub fn measured(basis_points: u16) -> Self {
        Self::Measured {
            basis_points: basis_points.min(10_000),
        }
    }

    /// Weight for ranking. `Unassessed` sorts below any measured value so ranked inbox rows carry
    /// the ones the detector was confident enough to score.
    pub const fn rank_weight(self) -> u16 {
        match self {
            Self::Unassessed => 0,
            Self::Measured { basis_points } => basis_points.saturating_add(1),
        }
    }

    /// Optional numeric form for storage. `None` when unassessed.
    pub const fn stored_basis_points(self) -> Option<u16> {
        match self {
            Self::Unassessed => None,
            Self::Measured { basis_points } => Some(basis_points),
        }
    }

    /// Rebuilds from the stored (nullable) column value.
    pub fn from_stored(basis_points: Option<u16>) -> Self {
        match basis_points {
            None => Self::Unassessed,
            Some(value) => Self::Measured {
                basis_points: value.min(10_000),
            },
        }
    }
}

/// What the finding is about. Every field is optional because different detectors anchor at
/// different granularities: some at an agency, some at a specific document, some at a request.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct FindingSubject {
    pub portal_key: Option<String>,
    pub agency_id: Option<i64>,
    pub request_id: Option<i64>,
    pub document_id: Option<i64>,
    pub entity_id: Option<String>,
}

impl FindingSubject {
    pub fn is_empty(&self) -> bool {
        self.portal_key.is_none()
            && self.agency_id.is_none()
            && self.request_id.is_none()
            && self.document_id.is_none()
            && self.entity_id.is_none()
    }
}

/// One piece of evidence the finding rests on. `quote` is an optional short excerpt so the inbox
/// can render "why did the detector say this" without a round-trip.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct FindingCitation {
    pub portal_key: String,
    pub document_id: i64,
    pub character_start: Option<i64>,
    pub character_end: Option<i64>,
    pub quote: Option<String>,
    pub label: Option<String>,
}

/// One autonomous finding.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    /// Deterministic 20-hex hash over `(kind, source_detector, subject fields, salient body
    /// bits)` so re-runs upsert cleanly without duplicating.
    pub id: String,
    pub kind: FindingKind,
    pub severity: FindingSeverity,
    pub confidence: FindingConfidence,
    pub subject: FindingSubject,
    pub headline: String,
    pub explanation: String,
    pub citations: Vec<FindingCitation>,
    pub discovered_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    pub dismissed_at: Option<DateTime<Utc>>,
    /// The stable slug of the detector that produced this row, so `clear_stale_findings` can
    /// target one detector's output without touching others.
    pub source_detector: String,
}

impl Finding {
    /// The write-time guard behind `FindingKind::is_legal_claim`. A finding that names a legal
    /// conclusion must say how strongly it is believed, where it comes from, and whom it is about;
    /// a statistical signal never reaches this branch.
    pub fn check_recordable(&self) -> Result<(), String> {
        if !self.kind.is_legal_claim() {
            return Ok(());
        }
        if !matches!(self.confidence, FindingConfidence::Measured { .. }) {
            return Err("a legal-claim finding needs a measured confidence".to_string());
        }
        if self.citations.is_empty() {
            return Err("a legal-claim finding needs at least one citation".to_string());
        }
        if self.subject.portal_key.is_none() {
            return Err("a legal-claim finding needs a portal subject".to_string());
        }
        Ok(())
    }

    /// Composite ranking key: severity, then confidence, then freshness. Higher tuple = higher
    /// position in the inbox. The inbox itself sorts descending on this key.
    pub fn rank_key(&self) -> (u8, u16, i64) {
        (
            self.severity.rank_weight(),
            self.confidence.rank_weight(),
            self.last_seen_at.timestamp(),
        )
    }
}

/// Deterministic finding id. Detectors call this to fill `Finding.id`, so a re-run over unchanged
/// data produces the same id (upserted) and a re-run over changed evidence produces a new one
/// (recorded fresh, prior id still in the table until `clear_stale_findings` drops it).
///
/// Salient bits are inputs whose change should invalidate the prior finding (e.g. the specific
/// day-count of a held-back response). Everything the detector wants to be irrelevant to
/// deduplication (rendering copy, prose that may be edited) should be excluded.
pub fn finding_id(
    kind: FindingKind,
    source_detector: &str,
    subject: &FindingSubject,
    salient_bits: &[&str],
) -> String {
    // Field labels included so an added/removed subject field can't collide silently. The unit
    // separator (0x1f) is not a legal character in any field we hash, so no field value can
    // impersonate another by embedding the separator.
    let mut parts: Vec<String> = Vec::with_capacity(8 + salient_bits.len());
    parts.push(format!("kind={}", kind.as_str()));
    parts.push(format!("detector={source_detector}"));
    parts.push(format!(
        "portal_key={}",
        subject.portal_key.as_deref().unwrap_or("")
    ));
    parts.push(format!(
        "agency_id={}",
        subject.agency_id.map_or(String::new(), |v| v.to_string())
    ));
    parts.push(format!(
        "request_id={}",
        subject.request_id.map_or(String::new(), |v| v.to_string())
    ));
    parts.push(format!(
        "document_id={}",
        subject.document_id.map_or(String::new(), |v| v.to_string())
    ));
    parts.push(format!(
        "entity_id={}",
        subject.entity_id.as_deref().unwrap_or("")
    ));
    for salient in salient_bits {
        parts.push((*salient).to_string());
    }
    sha256_hex(parts.join("\u{1f}").as_bytes())[..20].into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_ranks_serious_above_notable() {
        assert!(FindingSeverity::Critical.rank_weight() > FindingSeverity::Serious.rank_weight());
        assert!(FindingSeverity::Serious.rank_weight() > FindingSeverity::Notable.rank_weight());
        assert!(FindingSeverity::Notable.rank_weight() > FindingSeverity::Info.rank_weight());
    }

    #[test]
    fn confidence_unassessed_is_distinct_from_measured_zero() {
        let unassessed = FindingConfidence::Unassessed;
        let measured_zero = FindingConfidence::measured(0);
        assert_ne!(unassessed, measured_zero);
        assert_eq!(unassessed.stored_basis_points(), None);
        assert_eq!(measured_zero.stored_basis_points(), Some(0));
        assert!(measured_zero.rank_weight() > unassessed.rank_weight());
    }

    #[test]
    fn confidence_measured_bounds_basis_points_to_10000() {
        assert_eq!(
            FindingConfidence::measured(25_000).stored_basis_points(),
            Some(10_000)
        );
        assert_eq!(
            FindingConfidence::measured(10_000).stored_basis_points(),
            Some(10_000)
        );
        assert_eq!(
            FindingConfidence::measured(0).stored_basis_points(),
            Some(0)
        );
    }

    #[test]
    fn finding_id_is_stable_for_equivalent_inputs() {
        let subject = FindingSubject {
            portal_key: Some("sd-city".into()),
            document_id: Some(42),
            ..Default::default()
        };
        let id_one = finding_id(
            FindingKind::HeldBackResponse,
            "held_back_response",
            &subject,
            &["days=7"],
        );
        let id_two = finding_id(
            FindingKind::HeldBackResponse,
            "held_back_response",
            &subject,
            &["days=7"],
        );
        assert_eq!(id_one, id_two);
        assert_eq!(id_one.len(), 20);
    }

    #[test]
    fn finding_id_changes_when_salient_bits_change() {
        let subject = FindingSubject {
            portal_key: Some("sd-city".into()),
            document_id: Some(42),
            ..Default::default()
        };
        let with_seven = finding_id(
            FindingKind::HeldBackResponse,
            "held_back_response",
            &subject,
            &["days=7"],
        );
        let with_eight = finding_id(
            FindingKind::HeldBackResponse,
            "held_back_response",
            &subject,
            &["days=8"],
        );
        assert_ne!(with_seven, with_eight);
    }

    #[test]
    fn a_legal_claim_without_citations_is_not_recordable() {
        let now = Utc::now();
        let claim = Finding {
            id: "a".into(),
            kind: FindingKind::PolicyViolationDetected,
            severity: FindingSeverity::Serious,
            confidence: FindingConfidence::measured(8000),
            subject: FindingSubject {
                portal_key: Some("sd-city".into()),
                ..Default::default()
            },
            headline: String::new(),
            explanation: String::new(),
            citations: vec![],
            discovered_at: now,
            last_seen_at: now,
            dismissed_at: None,
            source_detector: "test".into(),
        };
        assert!(claim.check_recordable().is_err());
        let pattern = Finding {
            kind: FindingKind::HeldBackResponse,
            ..claim
        };
        assert!(pattern.check_recordable().is_ok());
    }

    #[test]
    fn rank_key_sorts_critical_above_serious() {
        let now = Utc::now();
        let base_subject = FindingSubject::default();
        let critical = Finding {
            id: "a".into(),
            kind: FindingKind::HeldBackResponse,
            severity: FindingSeverity::Critical,
            confidence: FindingConfidence::measured(5000),
            subject: base_subject.clone(),
            headline: "critical".into(),
            explanation: "critical".into(),
            citations: vec![],
            discovered_at: now,
            last_seen_at: now,
            dismissed_at: None,
            source_detector: "test".into(),
        };
        let serious = Finding {
            id: "b".into(),
            severity: FindingSeverity::Serious,
            confidence: FindingConfidence::measured(9000),
            ..critical.clone()
        };
        // Critical outranks serious even when serious has higher confidence.
        assert!(critical.rank_key() > serious.rank_key());
    }
}
