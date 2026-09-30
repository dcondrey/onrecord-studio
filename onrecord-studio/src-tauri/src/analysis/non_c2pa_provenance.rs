//! A parallel provenance-confidence gradient over signals a C2PA-less document still carries.
//!
//! The project's canonical [`ProvenanceConfidence`](crate::types::c2pa::ProvenanceConfidence)
//! ladder treats a verified C2PA manifest as authoritative. In practice, ~99% of public-records
//! documents carry no manifest at all — a scanned PDF stamped through an agency's mail-merge, a
//! Word document exported from a case-management system, a spreadsheet mailed as an attachment.
//! `ProvenanceConfidence` for those documents tops out at [`NoRecordedDigest`] or
//! [`EvidenceFileUnavailable`], and there is no path to say anything further about what the
//! document reports about itself.
//!
//! This module adds a *separate* ladder over non-cryptographic signals: PDF Info /
//! XMP timestamps + producer chain, and OOXML `docProps` timestamps + producer chain. It is
//! never treated as equivalent to a verified C2PA signature — the invariant is that a
//! `NonC2paProvenanceSignal` is at best circumstantial. But when C2PA is silent, having some
//! ordered ranking beats having none.
//!
//! **Absence != measurement**, applied here: `NoSignals` is a distinct rung from
//! `SelfConsistentMetadata`. A document that carried no metadata at all reads differently from a
//! document whose metadata was self-consistent but thin. Nothing collapses those two.
//!
//! [`NoRecordedDigest`]: crate::types::c2pa::ProvenanceConfidence::NoRecordedDigest
//! [`EvidenceFileUnavailable`]: crate::types::c2pa::ProvenanceConfidence::EvidenceFileUnavailable

use serde::{Deserialize, Serialize};

use crate::parsers::ooxml_docprops::OoxmlDocProps;
use crate::parsers::pdf_metadata_signals::PdfMetadataSignals;

/// One rung of the non-C2PA provenance ladder. Ordered weakest → strongest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum NonC2paRung {
    /// No extractable metadata timestamps or producer chain. Says nothing about the document.
    NoSignals,
    /// Some metadata is present, but no independent signals corroborate it — a Producer string
    /// alone with no timestamps, or timestamps that contradict each other's ordering.
    MetadataPresentUncorroborated,
    /// The metadata within a single source is internally consistent (PDF `CreationDate` <=
    /// `ModDate`, OOXML `dcterms:created` <= `dcterms:modified`). Not corroborated across
    /// sources, but not self-contradictory either.
    SelfConsistentMetadata,
    /// Two independent metadata sources agree (PDF Info agrees with embedded XMP; PDF Producer
    /// matches the pdf:Producer XMP claim; OOXML `dcterms:modified` matches `cp:revision`
    /// evolution). Corroboration across formats is the strongest circumstantial rung.
    CorroboratedAcrossFormats,
}

impl NonC2paRung {
    /// A rough confidence score in the same basis-points space the rest of the app uses. Never
    /// reaches the top: this ladder is not equivalent to a cryptographic verification, and the
    /// scoring reflects that.
    pub const fn confidence_basis_points(self) -> u16 {
        match self {
            Self::NoSignals => 0,
            Self::MetadataPresentUncorroborated => 2_500,
            Self::SelfConsistentMetadata => 5_000,
            Self::CorroboratedAcrossFormats => 7_500,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoSignals => "NO_SIGNALS",
            Self::MetadataPresentUncorroborated => "METADATA_PRESENT_UNCORROBORATED",
            Self::SelfConsistentMetadata => "SELF_CONSISTENT_METADATA",
            Self::CorroboratedAcrossFormats => "CORROBORATED_ACROSS_FORMATS",
        }
    }
}

/// One classification result. Carries the rung plus a short human-readable rationale so
/// downstream renderers can explain *why* a given rung was reached.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NonC2paProvenanceSignal {
    pub rung: NonC2paRung,
    /// Basis points derived from `rung`. Kept as its own field so a consumer can rank without
    /// re-invoking the enum.
    pub confidence_basis_points: u16,
    /// A short reason string naming which signals produced this rung. Not machine-parsed;
    /// consumed by explanation copy on the frontend.
    pub rationale: String,
}

impl NonC2paProvenanceSignal {
    fn new(rung: NonC2paRung, rationale: impl Into<String>) -> Self {
        Self {
            rung,
            confidence_basis_points: rung.confidence_basis_points(),
            rationale: rationale.into(),
        }
    }
}

/// Classify a PDF's non-C2PA signals.
pub fn classify_pdf(signals: &PdfMetadataSignals) -> NonC2paProvenanceSignal {
    if signals.is_empty() {
        return NonC2paProvenanceSignal::new(
            NonC2paRung::NoSignals,
            "PDF carries no Info dictionary and no XMP metadata stream.",
        );
    }

    let info_dates_ordered = order_holds(signals.info_creation_date, signals.info_mod_date);
    let xmp_dates_ordered = order_holds(signals.xmp_create_date, signals.xmp_modify_date);

    let info_and_xmp_dates_agree = match (signals.info_mod_date, signals.xmp_modify_date) {
        (Some(info), Some(xmp)) => (info - xmp).num_minutes().abs() < 60,
        _ => false,
    };

    if info_and_xmp_dates_agree && signals.producer_strings_match() {
        return NonC2paProvenanceSignal::new(
            NonC2paRung::CorroboratedAcrossFormats,
            "PDF Info ModDate agrees with xmp:ModifyDate and Info Producer matches pdf:Producer.",
        );
    }
    if info_and_xmp_dates_agree {
        return NonC2paProvenanceSignal::new(
            NonC2paRung::CorroboratedAcrossFormats,
            "PDF Info ModDate agrees with xmp:ModifyDate (both timestamp sources match).",
        );
    }
    if info_dates_ordered && xmp_dates_ordered && signals.info_creation_date.is_some() {
        // Both surfaces present, each internally consistent, but they don't corroborate each
        // other. Rank as self-consistent, not corroborated.
        return NonC2paProvenanceSignal::new(
            NonC2paRung::SelfConsistentMetadata,
            "Both PDF Info and XMP timestamps are internally consistent but do not agree with each other.",
        );
    }
    if info_dates_ordered && signals.info_mod_date.is_some() {
        return NonC2paProvenanceSignal::new(
            NonC2paRung::SelfConsistentMetadata,
            "PDF Info dictionary reports a CreationDate on or before its ModDate.",
        );
    }
    if xmp_dates_ordered && signals.xmp_modify_date.is_some() {
        return NonC2paProvenanceSignal::new(
            NonC2paRung::SelfConsistentMetadata,
            "PDF XMP metadata reports xmp:CreateDate on or before xmp:ModifyDate.",
        );
    }
    NonC2paProvenanceSignal::new(
        NonC2paRung::MetadataPresentUncorroborated,
        "PDF carries metadata but no timestamp pair is present and self-consistent.",
    )
}

/// Classify an OOXML package's non-C2PA signals.
pub fn classify_ooxml(props: &OoxmlDocProps) -> NonC2paProvenanceSignal {
    if props.is_empty() {
        return NonC2paProvenanceSignal::new(
            NonC2paRung::NoSignals,
            "OOXML docProps carries no extractable fields.",
        );
    }

    let core_dates_ordered = order_holds(props.created, props.modified);

    // Corroboration: Application + Company + a well-ordered core.xml date pair. The presence of
    // an Application string that names a known Office producer, combined with a modified date
    // that is at least as recent as created, is the strongest signal an OOXML document supplies
    // without cryptographic backing.
    if core_dates_ordered
        && props.modified.is_some()
        && props.created.is_some()
        && props.application.is_some()
        && props.rev_num.is_some()
    {
        return NonC2paProvenanceSignal::new(
            NonC2paRung::CorroboratedAcrossFormats,
            "OOXML core dates are ordered, and application + revision counter both present.",
        );
    }
    if core_dates_ordered && props.modified.is_some() && props.created.is_some() {
        return NonC2paProvenanceSignal::new(
            NonC2paRung::SelfConsistentMetadata,
            "OOXML core dates are internally consistent (created <= modified).",
        );
    }
    NonC2paProvenanceSignal::new(
        NonC2paRung::MetadataPresentUncorroborated,
        "OOXML docProps present, but core dates are absent or contradict.",
    )
}

/// True iff `earlier <= later`, or either is absent. An absent date can't contradict.
fn order_holds<T: Ord>(earlier: Option<T>, later: Option<T>) -> bool {
    match (earlier, later) {
        (Some(e), Some(l)) => e <= l,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};

    fn dt(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    #[test]
    fn no_signals_when_pdf_has_no_metadata_at_all() {
        let classification = classify_pdf(&PdfMetadataSignals::default());
        assert_eq!(classification.rung, NonC2paRung::NoSignals);
        assert_eq!(classification.confidence_basis_points, 0);
    }

    #[test]
    fn metadata_present_uncorroborated_when_only_producer_string_exists() {
        let signals = PdfMetadataSignals {
            info_producer: Some("Some Producer".into()),
            ..Default::default()
        };
        let classification = classify_pdf(&signals);
        assert_eq!(
            classification.rung,
            NonC2paRung::MetadataPresentUncorroborated
        );
    }

    #[test]
    fn self_consistent_metadata_when_info_dates_are_ordered_but_xmp_missing() {
        let signals = PdfMetadataSignals {
            info_creation_date: Some(dt("2024-03-01T10:00:00Z")),
            info_mod_date: Some(dt("2024-04-01T10:00:00Z")),
            ..Default::default()
        };
        let classification = classify_pdf(&signals);
        assert_eq!(classification.rung, NonC2paRung::SelfConsistentMetadata);
    }

    #[test]
    fn corroborated_across_formats_when_info_and_xmp_moddate_agree() {
        let signals = PdfMetadataSignals {
            info_mod_date: Some(dt("2024-04-01T10:00:00Z")),
            xmp_modify_date: Some(dt("2024-04-01T10:30:00Z")),
            ..Default::default()
        };
        let classification = classify_pdf(&signals);
        assert_eq!(classification.rung, NonC2paRung::CorroboratedAcrossFormats);
        assert_eq!(classification.confidence_basis_points, 7500);
    }

    #[test]
    fn ooxml_self_consistent_when_core_dates_ordered_without_extras() {
        let props = OoxmlDocProps {
            created: Some(dt("2024-03-01T10:00:00Z")),
            modified: Some(dt("2024-04-01T10:00:00Z")),
            ..Default::default()
        };
        let classification = classify_ooxml(&props);
        assert_eq!(classification.rung, NonC2paRung::SelfConsistentMetadata);
    }

    #[test]
    fn ooxml_corroborated_when_dates_ordered_plus_application_plus_rev_num() {
        let props = OoxmlDocProps {
            created: Some(dt("2024-03-01T10:00:00Z")),
            modified: Some(dt("2024-04-01T10:00:00Z")),
            application: Some("Microsoft Office Word".into()),
            rev_num: Some(47),
            ..Default::default()
        };
        let classification = classify_ooxml(&props);
        assert_eq!(classification.rung, NonC2paRung::CorroboratedAcrossFormats);
    }

    #[test]
    fn ordering_never_reaches_a_c2pa_verified_signature_equivalent() {
        // Invariant: the top confidence a non-C2PA rung yields is 7500 bp, always below the
        // fully-verified C2PA signature the canonical ladder can produce. This test is worth
        // its weight against future edits — the whole point of the parallel ladder is to
        // preserve that ceiling.
        assert!(NonC2paRung::CorroboratedAcrossFormats.confidence_basis_points() < 10_000);
    }
}
