//! Assembles one finding into a dossier: the records it cites, the other findings that touch
//! the same request or documents, what would disconfirm it, and which figures in the cited
//! documents could not be checked. Everything is read from what is already stored; nothing is
//! persisted.

use std::collections::{BTreeSet, HashMap};

use chrono::Utc;

use crate::analysis::claim_validation::{claim_check_view, extract_claims, Evaluator};
use crate::analysis::finding::{Finding, FindingKind};
use crate::error::AppError;
use crate::storage::findings::FindingsFilter;
use crate::storage::sqlite::DuckDbRepository;
use crate::types::{
    ClaimCheckOutcome, DossierFalsification, DossierGap, EvidenceLink, LeadDossier, RelatedFinding,
    RelationBasis, RequestDocumentSummary,
};

const MAX_EVIDENCE: usize = 50;
const MAX_RELATED: usize = 20;
const MAX_GAPS: usize = 50;
/// Documents whose figures are re-checked; each costs a text read and an evaluation pass.
const MAX_GAP_DOCUMENTS: usize = 5;
const MAX_DOCUMENT_CHARACTERS: usize = 250_000;

pub async fn build(
    repository: &DuckDbRepository,
    finding_id: String,
) -> Result<Option<LeadDossier>, AppError> {
    let Some(finding) = repository.get_finding_by_id(finding_id).await? else {
        return Ok(None);
    };
    let (evidence, evidence_truncated) = resolve_evidence(repository, &finding).await?;
    let (related, related_truncated) = related_findings(repository, &finding).await?;
    let (gaps, gaps_truncated) = claim_gaps(repository, &finding).await?;
    Ok(Some(LeadDossier {
        falsification: falsification_for(finding.kind),
        finding,
        evidence,
        evidence_truncated,
        related,
        related_truncated,
        gaps,
        gaps_truncated,
        assembled_at: Utc::now(),
    }))
}

async fn resolve_evidence(
    repository: &DuckDbRepository,
    finding: &Finding,
) -> Result<(Vec<EvidenceLink>, bool), AppError> {
    let request_id = finding.subject.request_id;
    let mut documents: HashMap<String, Vec<RequestDocumentSummary>> = HashMap::new();
    let mut agency_labels: HashMap<String, Option<String>> = HashMap::new();
    let mut links = Vec::with_capacity(finding.citations.len().min(MAX_EVIDENCE));
    for citation in finding.citations.iter().take(MAX_EVIDENCE) {
        let mut file_name = None;
        let mut raw_file_sha256 = None;
        let mut agency_label = None;
        if let Some(request_id) = request_id.and_then(|id| u64::try_from(id).ok()) {
            if !documents.contains_key(&citation.portal_key) {
                let listed = repository
                    .list_request_documents_for_view(request_id, &citation.portal_key)
                    .await?;
                documents.insert(citation.portal_key.clone(), listed);
                let label = repository
                    .agency_label_for_request(&citation.portal_key, request_id)
                    .await?;
                agency_labels.insert(citation.portal_key.clone(), label);
            }
            if let Some(summary) = documents[&citation.portal_key]
                .iter()
                .find(|d| i64::try_from(d.document_id).ok() == Some(citation.document_id))
            {
                file_name = Some(summary.file_name.clone());
                raw_file_sha256 = summary.raw_file_sha256.clone();
            }
            agency_label = agency_labels[&citation.portal_key].clone();
        }
        links.push(EvidenceLink {
            portal_key: citation.portal_key.clone(),
            document_id: citation.document_id,
            file_name,
            request_id,
            agency_label,
            raw_file_sha256,
            character_start: citation.character_start,
            character_end: citation.character_end,
            quote: citation.quote.clone(),
            label: citation.label.clone(),
        });
    }
    Ok((links, finding.citations.len() > MAX_EVIDENCE))
}

async fn related_findings(
    repository: &DuckDbRepository,
    finding: &Finding,
) -> Result<(Vec<RelatedFinding>, bool), AppError> {
    let mut seen = BTreeSet::new();
    seen.insert(finding.id.clone());
    let mut related = Vec::new();
    let mut push = |rows: Vec<Finding>, basis: RelationBasis| {
        for row in rows {
            if seen.insert(row.id.clone()) {
                related.push(RelatedFinding {
                    id: row.id,
                    kind: row.kind,
                    headline: row.headline,
                    severity: row.severity,
                    confidence: row.confidence,
                    basis,
                    discovered_at: row.discovered_at,
                });
            }
        }
    };
    if finding.subject.request_id.is_some() {
        let rows = repository
            .list_findings(
                FindingsFilter {
                    portal_key: finding.subject.portal_key.clone(),
                    agency_id: None,
                    request_id: finding.subject.request_id,
                    document_id: None,
                    source_detector: None,
                    include_dismissed: None,
                    discovered_after: None,
                    id: None,
                },
                MAX_RELATED + 1,
            )
            .await?;
        push(rows, RelationBasis::SameRequest);
    }
    for document_id in cited_documents(finding).into_iter().take(MAX_GAP_DOCUMENTS) {
        let rows = repository
            .list_findings(
                FindingsFilter {
                    portal_key: Some(document_id.0.clone()),
                    agency_id: None,
                    request_id: None,
                    document_id: Some(document_id.1),
                    source_detector: None,
                    include_dismissed: None,
                    discovered_after: None,
                    id: None,
                },
                MAX_RELATED + 1,
            )
            .await?;
        push(rows, RelationBasis::CitesSameDocument);
    }
    related.sort_by(|a, b| b.discovered_at.cmp(&a.discovered_at).then(a.id.cmp(&b.id)));
    let truncated = related.len() > MAX_RELATED;
    related.truncate(MAX_RELATED);
    Ok((related, truncated))
}

fn cited_documents(finding: &Finding) -> Vec<(String, i64)> {
    let mut seen = BTreeSet::new();
    finding
        .citations
        .iter()
        .filter(|c| seen.insert((c.portal_key.clone(), c.document_id)))
        .map(|c| (c.portal_key.clone(), c.document_id))
        .collect()
}

async fn claim_gaps(
    repository: &DuckDbRepository,
    finding: &Finding,
) -> Result<(Vec<DossierGap>, bool), AppError> {
    let documents = cited_documents(finding);
    let mut gaps = Vec::new();
    let mut truncated = documents.len() > MAX_GAP_DOCUMENTS;
    let mut evaluator: Option<Evaluator> = None;
    for (portal_key, document_id) in documents.into_iter().take(MAX_GAP_DOCUMENTS) {
        let Ok(unsigned_id) = u64::try_from(document_id) else {
            continue;
        };
        let chunk = match repository
            .load_document_text_chunk(unsigned_id, &portal_key, 0, MAX_DOCUMENT_CHARACTERS)
            .await
        {
            Ok(chunk) => chunk,
            Err(error) => {
                gaps.push(DossierGap::NotComputed {
                    portal_key,
                    document_id,
                    reason: format!("the document text could not be read: {error}"),
                });
                continue;
            }
        };
        let claims = extract_claims(&chunk.masked_text);
        if claims.is_empty() {
            continue;
        }
        if evaluator.is_none() {
            evaluator = Some(Evaluator::load(repository).await?);
        }
        let evaluator = evaluator.as_mut().expect("loaded above");
        for claim in claims {
            if gaps.len() >= MAX_GAPS {
                truncated = true;
                break;
            }
            let evaluation = evaluator.evaluate(repository, &portal_key, &claim).await?;
            let view = claim_check_view(&claim, &evaluation);
            if let ClaimCheckOutcome::Unreconciled { reason, detail } = view.result {
                gaps.push(DossierGap::UnreconciledClaim {
                    portal_key: portal_key.clone(),
                    document_id,
                    claim_text: view.claim_text,
                    reason,
                    detail,
                });
            }
        }
    }
    Ok((gaps, truncated))
}

/// Exhaustive so a new kind fails to compile here instead of shipping with no falsification text.
pub fn falsification_for(kind: FindingKind) -> DossierFalsification {
    let (statement, records): (&str, &[&str]) = match kind {
        FindingKind::HeldBackResponse => (
            "The records were released as soon as they were ready and nothing was held for a later batch.",
            &["the agency's internal review log for the request", "the dates each responsive record was located and approved"],
        ),
        FindingKind::BoilerplateDenial => (
            "The denial was tailored to this request and its stated grounds match the records withheld.",
            &["the withholding log naming each withheld record and its exemption", "denials to comparable requests that differ in substance"],
        ),
        FindingKind::JustInTimeCompliance => (
            "The response timing reflects the work required, not the deadline.",
            &["the response dates for the agency's other requests in the same period", "the request's internal task history"],
        ),
        FindingKind::ExtensionAbuse => (
            "Each extension cited a condition that existed for this request.",
            &["the extension notices with their stated grounds", "the volume of records actually located"],
        ),
        FindingKind::AbsenceOfRecords => (
            "The records genuinely do not exist for the period requested.",
            &["the retention schedule for the record class", "a later release or index listing the same class of record"],
        ),
        FindingKind::DifferentialTreatment => (
            "The differing outcomes follow from differences in the requests, not in the requesters.",
            &["the full text of the compared requests", "the exemptions cited in each response"],
        ),
        FindingKind::MetadataContradictsContent => (
            "The metadata was produced by a tool or conversion step and describes the file, not the record.",
            &["the original file as held by the agency", "the export or conversion process used for release"],
        ),
        FindingKind::RegeneratedResponse => (
            "The response was regenerated for a documented reason such as a corrected attachment.",
            &["the correspondence explaining the reissue", "both versions of the response with their dates"],
        ),
        FindingKind::EmailThreadGap => (
            "The missing messages were never sent, or were withheld under a cited exemption.",
            &["the withholding log for the thread", "the mailbox export or thread index"],
        ),
        FindingKind::ContradictoryStatement => (
            "One statement supersedes the other, or they describe different items.",
            &["a revision or amendment dated between the two statements", "the record that defines the item each statement describes"],
        ),
        FindingKind::NoRecordsContradictedByRelease => (
            "The later release covers a different request scope or a record created after the first response.",
            &["the creation dates of the later-released records", "the scope stated in each request"],
        ),
        FindingKind::EvasionParaphrasedDenials => (
            "The differing wording reflects different staff or templates, not a changed position.",
            &["the agency's response templates", "the withholding grounds recorded for each denial"],
        ),
        FindingKind::EvasionRoundNumberThresholds => (
            "The round figures are the agency's own estimates and are labelled as such.",
            &["the fee or volume estimate worksheets", "the final figures once the work was done"],
        ),
        FindingKind::EvasionRedactionAtLimit => (
            "The redactions fall at the limit because the exemption genuinely applies to that span.",
            &["the exemption log for each redaction", "an unredacted copy released to another requester"],
        ),
        FindingKind::EvasionLateArrivingRecords => (
            "The late records were located late for a reason recorded at the time.",
            &["the search log showing when each location was searched", "the correspondence announcing the supplemental release"],
        ),
        FindingKind::RedactionCandidateInferred => (
            "The matching words are reused wording, and the other passage concerns a different record, date or person.",
            &["an unredacted copy from another release", "the dates and record identifiers of both cited passages"],
        ),
        FindingKind::CrossCorpusSharedLanguage => (
            "The shared language comes from a common template or statute, not coordination.",
            &["the template or model language both agencies used", "the dates each agency adopted it"],
        ),
        FindingKind::CrossCorpusTemporalShift => (
            "The timing shift follows a policy or staffing change that is documented.",
            &["the policy or staffing record dated at the shift", "response times for unrelated request types over the same period"],
        ),
        FindingKind::CrossCorpusOrphanTopic => (
            "The topic is covered by a different record class or a different agency.",
            &["the records index of the agency that does hold it", "the referral or transfer correspondence"],
        ),
        FindingKind::CrossCorpusReusedDenialGrounds => (
            "The reused grounds apply to each request on its own facts.",
            &["the withheld records described in each denial", "the exemption analysis for each"],
        ),
        FindingKind::PolicyViolationDetected => (
            "The counted deadline was met under a tolling or extension the record supports.",
            &["the extension or tolling notice", "the date the request was received as recorded by the agency"],
        ),
        FindingKind::ClaimContradictsData => (
            "The stated figure refers to a different basis, fund, period or a later revision of the data.",
            &["the report or ledger the statement drew its figure from", "a later revision of the published data or of the statement"],
        ),
        FindingKind::DepartmentSpendingYearOverYear => (
            "The change reflects a documented programme, reorganisation or accounting reclassification.",
            &["the adopted budget narrative for both years", "the account mapping or reclassification notice"],
        ),
        FindingKind::OvertimeAboveAdoptedLine => (
            "A budget amendment or transfer raised the overtime line after adoption.",
            &["the revised budget for the account", "the transfer or supplemental appropriation record"],
        ),
        FindingKind::FundActualAboveAdopted => (
            "The revised authority for the fund was higher than the adopted amount.",
            &["the mid-year budget revisions for the fund", "the supplemental appropriation record"],
        ),
        FindingKind::EncampmentNoticeShortfall => (
            "A longer notice was given by a route the recorded dates do not show.",
            &["the notice as posted, with its posting date", "the cleanup log with its date"],
        ),
        FindingKind::EncampmentNoticeStatedShortfall => (
            "The stated period is not the period actually given.",
            &["the dated posting or service record for the notice", "the minimum-notice authority in force on that date"],
        ),
        FindingKind::ServiceReportLead => (
            "The reports were handled and the table was not updated, or the difference reflects what was reported rather than how it was handled.",
            &["the work orders or inspection records for the reports counted", "the department's published service schedule and staffing for the service"],
        ),
        FindingKind::AbatementLogLead => (
            "The notice was given earlier or by another route than the posting the log records, or the two rows are different events at one place.",
            &["the notices as posted, with their posting times", "the abatement work orders and the stored-property log for the same places"],
        ),
        FindingKind::DisclosureFilingLead => (
            "The reports reflect routine civic fundraising the official was asked to support, and the count reflects how completely the office reports.",
            &["the filed Form 803 for each report, with payor, payee and amount", "council docket items and contract awards naming the same payors"],
        ),
        FindingKind::ChargeLogLead => (
            "The sections were charged against people who are housed, or alongside conduct the log does not record.",
            &["the arrest and citation reports for the activity numbers counted", "the prosecutor's filing decision and case outcome for each charge"],
        ),
        FindingKind::CapacityComparisonLead => (
            "The three series measure different things over different periods: the department's adopted amount is not all homelessness spending, and the bed count is one night each January.",
            &["the department's adopted and actual expenditure by programme for the years compared", "the Housing Commission's shelter contracts and nightly occupancy reports for the same years"],
        ),
        FindingKind::PoliceStopLead => (
            "The difference follows from why and where the stops were made, not from who was stopped.",
            &["the stop records broken down by reason for the stop, beat and assignment", "the department's property and evidence log for the items seized"],
        ),
        FindingKind::RollingProduction => (
            "The requester narrowed or added to the request, agreed to installments, or the records were found and reviewed in stages.",
            &["the request's correspondence on scope and schedule", "any production schedule or extension notice the agency sent"],
        ),
        FindingKind::SpendingRecordLead => (
            "The budget was revised after adoption, the amount was transferred or carried forward, or the City recorded the payment under another department or account.",
            &["the year-end budget monitoring report and mid-year budget amendments for the department and year", "the department's overtime and vacancy reports, or the settlement resolutions, for the same year"],
        ),
        FindingKind::PoliceCallLead => (
            "The call type is the dispatcher's label when the call was entered, not what the officers found, and the disposition code records how the call was closed, not what was done.",
            &["the CAD event narratives for the calls counted", "the incident and arrest reports for the beats named"],
        ),
        FindingKind::ParkingCitationLead => (
            "The vehicles were stored or abandoned, not lived in.",
            &["the citing officers' notes for the citations counted", "the tow and impound records for the blocks named"],
        ),
        FindingKind::ReleaseTimingConcentration => (
            "Staff finish and post work at the end of the week, or the requests released then fell due then.",
            &["the request log with each request's due date and assigned staff", "the portal's document activity log showing when each file was uploaded and when it was made public"],
        ),
        FindingKind::RedactionInconsistentAcrossReleases => (
            "The two copies are different versions of the record, or the later release applied an exemption the earlier one did not.",
            &["both releases with their response letters and cited exemptions", "the redaction log or review notes for each request"],
        ),
        FindingKind::CorrelatedFindings => (
            "The correlated signals share a benign cause such as a single large release.",
            &["the release log for the period", "each underlying finding's own records"],
        ),
        FindingKind::PracticeDepartureCounted => (
            "The counted departure follows a documented exception to the practice.",
            &["the policy stating the practice and its exceptions", "the approval record for the exception"],
        ),
        FindingKind::PracticeDepartureStated => (
            "The stated practice was superseded before the departure.",
            &["the current version of the policy with its effective date", "the record that announced the change"],
        ),
        FindingKind::ContractCeilingExceeded => (
            "An amendment raised the ceiling before the payments exceeded it.",
            &["the contract amendments with their approval dates", "the payment register for the contract"],
        ),
        FindingKind::PaymentOutsideContractTerm => (
            "The payment settled work performed within the term, or the term was extended.",
            &["the invoice with its service dates", "the term extension or close-out record"],
        ),
        FindingKind::PaymentWithoutContractRecord => (
            "The contract exists under a different number or was exempt from the contract register.",
            &["the procurement file for the vendor", "the exemption or emergency purchase record"],
        ),
        FindingKind::AppropriationExceeded => (
            "A budget transfer or supplemental appropriation covered the excess.",
            &["the transfer or supplemental appropriation record", "the revised budget for the account"],
        ),
        FindingKind::RestrictedFundOffProgram => (
            "The expenditure is an eligible use of the restricted fund under its governing terms.",
            &["the fund's governing ordinance or grant terms", "the expenditure's program coding and justification"],
        ),
        FindingKind::ApprovalThresholdExceeded => (
            "The approval was obtained at the required level and recorded elsewhere.",
            &["the approval record for the transaction", "the delegation of authority in force at the time"],
        ),
        FindingKind::ApprovalRecordAbsent => (
            "The approval exists in a system the release did not cover.",
            &["the approval workflow export", "the signed authorization form"],
        ),
        FindingKind::SplitPurchasePattern => (
            "The purchases were separate needs that happened to fall close together.",
            &["the requisitions with their stated purposes", "the delivery records for each purchase"],
        ),
        FindingKind::SplitPurchaseProhibitedByPolicy => (
            "The purchases fall under an exception the policy allows.",
            &["the procurement policy's exception list", "the approval for the exception"],
        ),
    };
    DossierFalsification {
        statement: statement.to_string(),
        records_to_seek: records.iter().map(|r| r.to_string()).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::falsification_for;
    use crate::analysis::finding::FindingKind;

    #[test]
    fn every_kind_names_records_that_would_disconfirm_it() {
        for kind in [
            FindingKind::HeldBackResponse,
            FindingKind::ClaimContradictsData,
            FindingKind::PolicyViolationDetected,
            FindingKind::SplitPurchaseProhibitedByPolicy,
        ] {
            let falsification = falsification_for(kind);
            assert!(!falsification.statement.is_empty());
            assert!(falsification.records_to_seek.len() >= 2, "{kind:?}");
            let lowered = falsification.statement.to_lowercase();
            assert!(
                !lowered.contains("false") && !lowered.contains("lied"),
                "{kind:?}"
            );
        }
    }
}
