//! Reads what an e-filed IRS Form 990 states. Element names were taken from published returns
//! (schema versions 2020v1.3 to 2023v5.1), not from the schema documents; a return that omits an
//! element yields `None` for it.

use crate::error::AppError;
use crate::types::{
    FilingExplanation, FilingLine, Form990Person, Form990Statement, ScheduleJAmounts,
    ScheduleJPerson, ScheduleLPart, ScheduleLTransaction,
};
use roxmltree::{Document, Node};

fn invalid(message: impl Into<String>) -> AppError {
    AppError::DataParsing(message.into())
}

fn child<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Option<Node<'a, 'input>> {
    node.children()
        .find(|c| c.is_element() && c.tag_name().name() == name)
}

fn children<'a, 'input: 'a>(
    node: Node<'a, 'input>,
    name: &'a str,
) -> impl Iterator<Item = Node<'a, 'input>> + 'a {
    node.children()
        .filter(move |c| c.is_element() && c.tag_name().name() == name)
}

fn text(node: Node, name: &str) -> Option<String> {
    child(node, name)
        .and_then(|c| c.text())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn amount(node: Node, name: &str) -> Result<Option<i64>, AppError> {
    text(node, name)
        .map(|value| {
            value.parse::<i64>().map_err(|_| {
                invalid(format!(
                    "Form 990 element {name} holds a non-integer amount"
                ))
            })
        })
        .transpose()
}

/// Returns encode a yes/no answer as `true`/`false` or `1`/`0`, and a checked box as `X`.
fn indicator(node: Node, name: &str) -> Result<Option<bool>, AppError> {
    text(node, name)
        .map(|value| match value.as_str() {
            "true" | "1" | "X" => Ok(true),
            "false" | "0" => Ok(false),
            _ => Err(invalid(format!(
                "Form 990 element {name} holds an unrecognised yes/no value"
            ))),
        })
        .transpose()
}

fn party_name(node: Node) -> Option<String> {
    text(node, "PersonNm").or_else(|| {
        child(node, "BusinessName").and_then(|business| {
            let lines = ["BusinessNameLine1Txt", "BusinessNameLine2Txt"]
                .iter()
                .filter_map(|line| text(business, line))
                .collect::<Vec<_>>();
            (!lines.is_empty()).then(|| lines.join(" "))
        })
    })
}

fn explanations(schedule: Node) -> Vec<FilingExplanation> {
    children(schedule, "SupplementalInformationDetail")
        .filter_map(|detail| {
            Some(FilingExplanation {
                line_reference: text(detail, "FormAndLineReferenceDesc"),
                text: text(detail, "ExplanationTxt")?,
            })
        })
        .collect()
}

fn stated_lines(node: Node, labelled: &[(&str, &str)]) -> Result<Vec<FilingLine>, AppError> {
    let mut lines = Vec::new();
    for (element, label) in labelled {
        if let Some(answer) = indicator(node, element)? {
            lines.push(FilingLine {
                label: (*label).to_string(),
                value: if answer { "Yes" } else { "No" }.to_string(),
            });
        }
    }
    Ok(lines)
}

const POSITIONS: &[(&str, &str)] = &[
    (
        "IndividualTrusteeOrDirectorInd",
        "Individual trustee or director",
    ),
    ("InstitutionalTrusteeInd", "Institutional trustee"),
    ("OfficerInd", "Officer"),
    ("KeyEmployeeInd", "Key employee"),
    (
        "HighestCompensatedEmployeeInd",
        "Highest compensated employee",
    ),
    ("FormerOfcrDirectorTrusteeInd", "Former"),
];

fn person(row: Node) -> Result<Form990Person, AppError> {
    let mut positions = Vec::new();
    for (element, label) in POSITIONS {
        if indicator(row, element)? == Some(true) {
            positions.push((*label).to_string());
        }
    }
    Ok(Form990Person {
        name: party_name(row),
        title: text(row, "TitleTxt"),
        former: indicator(row, "FormerOfcrDirectorTrusteeInd")? == Some(true),
        positions,
        reportable_compensation: amount(row, "ReportableCompFromOrgAmt")?,
        related_organisation_compensation: amount(row, "ReportableCompFromRltdOrgAmt")?,
        other_compensation: amount(row, "OtherCompensationAmt")?,
    })
}

fn schedule_j_person(row: Node) -> Result<ScheduleJPerson, AppError> {
    Ok(ScheduleJPerson {
        name: party_name(row),
        title: text(row, "TitleTxt"),
        filing_organisation: ScheduleJAmounts {
            base: amount(row, "BaseCompensationFilingOrgAmt")?,
            bonus: amount(row, "BonusFilingOrganizationAmount")?,
            other: amount(row, "OtherCompensationFilingOrgAmt")?,
            deferred: amount(row, "DeferredCompensationFlngOrgAmt")?,
            nontaxable_benefits: amount(row, "NontaxableBenefitsFilingOrgAmt")?,
            total: amount(row, "TotalCompensationFilingOrgAmt")?,
            reported_in_prior_return: amount(row, "CompReportPrior990FilingOrgAmt")?,
        },
        related_organisations: ScheduleJAmounts {
            base: amount(row, "CompensationBasedOnRltdOrgsAmt")?,
            bonus: amount(row, "BonusRelatedOrganizationsAmt")?,
            other: amount(row, "OtherCompensationRltdOrgsAmt")?,
            deferred: amount(row, "DeferredCompRltdOrgsAmt")?,
            nontaxable_benefits: amount(row, "NontaxableBenefitsRltdOrgsAmt")?,
            total: amount(row, "TotalCompensationRltdOrgsAmt")?,
            reported_in_prior_return: amount(row, "CompReportPrior990RltdOrgsAmt")?,
        },
    })
}

fn schedule_l(schedule: Node) -> Result<Vec<ScheduleLTransaction>, AppError> {
    let mut transactions = Vec::new();
    for row in children(schedule, "DisqualifiedPersonExBnftTrGrp") {
        transactions.push(ScheduleLTransaction {
            part: ScheduleLPart::ExcessBenefitTransaction,
            name: party_name(row),
            relationship: text(row, "RlnDisqualifiedPersonOrgTxt"),
            description: text(row, "TransactionDesc"),
            amount: None,
            balance_due: None,
            lines: stated_lines(row, &[("TransactionCorrectedInd", "Corrected")])?,
        });
    }
    for row in children(schedule, "LoansBtwnOrgInterestedPrsnGrp") {
        transactions.push(ScheduleLTransaction {
            part: ScheduleLPart::Loan,
            name: party_name(row),
            relationship: text(row, "RelationshipWithOrgTxt"),
            description: text(row, "LoanPurposeTxt"),
            amount: amount(row, "OriginalPrincipalAmt")?,
            balance_due: amount(row, "BalanceDueAmt")?,
            lines: stated_lines(
                row,
                &[
                    ("LoanToOrganizationInd", "Loan to the organisation"),
                    ("LoanFromOrganizationInd", "Loan from the organisation"),
                    ("DefaultInd", "In default"),
                    (
                        "BoardOrCommitteeApprovalInd",
                        "Approved by board or committee",
                    ),
                    ("WrittenAgreementInd", "Written agreement"),
                ],
            )?,
        });
    }
    for row in children(schedule, "GrntAsstBnftInterestedPrsnGrp") {
        let mut lines = Vec::new();
        if let Some(kind) = text(row, "TypeOfAssistanceTxt") {
            lines.push(FilingLine {
                label: "Type of assistance".to_string(),
                value: kind,
            });
        }
        transactions.push(ScheduleLTransaction {
            part: ScheduleLPart::GrantOrAssistance,
            name: party_name(row),
            relationship: text(row, "RelationshipWithOrgTxt"),
            description: text(row, "AssistancePurposeTxt"),
            amount: amount(row, "CashGrantAmt")?,
            balance_due: None,
            lines,
        });
    }
    for row in children(schedule, "BusTrInvolveInterestedPrsnGrp") {
        transactions.push(ScheduleLTransaction {
            part: ScheduleLPart::BusinessTransaction,
            name: child(row, "NameOfInterested").and_then(party_name),
            relationship: text(row, "RelationshipDescriptionTxt"),
            description: text(row, "TransactionDesc"),
            amount: amount(row, "TransactionAmt")?,
            balance_due: None,
            lines: stated_lines(
                row,
                &[("SharingOfRevenuesInd", "Sharing of organisation's revenues")],
            )?,
        });
    }
    // Software emits an empty row for a part the filer left blank; it states nothing.
    transactions.retain(|row| {
        row.name.is_some()
            || row.relationship.is_some()
            || row.description.is_some()
            || row.amount.is_some()
            || row.balance_due.is_some()
    });
    Ok(transactions)
}

/// The return type the return's own header declares, for example `990` or `990EZ`.
pub fn declared_return_type(xml: &str) -> Result<Option<String>, AppError> {
    let document = parse(xml)?;
    Ok(child(document.root_element(), "ReturnHeader")
        .and_then(|header| text(header, "ReturnTypeCd")))
}

fn parse(xml: &str) -> Result<Document<'_>, AppError> {
    Document::parse(xml.trim_start_matches('\u{feff}'))
        .map_err(|error| invalid(format!("The return is not well-formed XML: {error}")))
}

/// Reads a full Form 990. A 990-EZ, 990-PF or 990-T has no `IRS990` element and is refused.
pub fn parse_form_990(xml: &str) -> Result<Form990Statement, AppError> {
    let document = parse(xml)?;
    let root = document.root_element();
    let header =
        child(root, "ReturnHeader").ok_or_else(|| invalid("The return has no ReturnHeader"))?;
    let data = child(root, "ReturnData").ok_or_else(|| invalid("The return has no ReturnData"))?;
    let form = child(data, "IRS990")
        .ok_or_else(|| invalid("The return carries no Form 990 (IRS990 element)"))?;
    let filer = child(header, "Filer");
    let part_vii = children(form, "Form990PartVIISectionAGrp")
        .map(person)
        .collect::<Result<Vec<_>, _>>()?;
    let schedule_j_node = child(data, "IRS990ScheduleJ");
    let schedule_l_node = child(data, "IRS990ScheduleL");
    Ok(Form990Statement {
        return_version: root.attribute("returnVersion").map(str::to_string),
        ein: filer.and_then(|filer| text(filer, "EIN")),
        filer_name: filer.and_then(party_name),
        tax_period_begin: text(header, "TaxPeriodBeginDt"),
        tax_period_end: text(header, "TaxPeriodEndDt"),
        amended: indicator(form, "AmendedReturnInd")?,
        total_revenue: amount(form, "CYTotalRevenueAmt")?,
        total_expenses: amount(form, "CYTotalExpensesAmt")?,
        total_assets_end_of_year: amount(form, "TotalAssetsEOYAmt")?,
        total_liabilities_end_of_year: amount(form, "TotalLiabilitiesEOYAmt")?,
        net_assets_end_of_year: amount(form, "NetAssetsOrFundBalancesEOYAmt")?,
        government_grants: amount(form, "GovernmentGrantsAmt")?,
        significant_diversion_reported: indicator(form, "MaterialDiversionOrMisuseInd")?,
        former_listed: indicator(form, "FormerOfcrEmployeesListedInd")?,
        total_reportable_compensation: amount(form, "TotalReportableCompFromOrgAmt")?,
        people: part_vii,
        schedule_j_attached: schedule_j_node.is_some(),
        severance_payment_reported: schedule_j_node
            .map(|schedule| indicator(schedule, "SeverancePaymentInd"))
            .transpose()?
            .flatten(),
        schedule_j_people: schedule_j_node
            .map(|schedule| {
                children(schedule, "RltdOrgOfficerTrstKeyEmplGrp")
                    .map(schedule_j_person)
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?
            .unwrap_or_default(),
        schedule_j_explanations: schedule_j_node.map(explanations).unwrap_or_default(),
        schedule_l_attached: schedule_l_node.is_some(),
        schedule_l_transactions: schedule_l_node
            .map(schedule_l)
            .transpose()?
            .unwrap_or_default(),
        schedule_l_explanations: schedule_l_node.map(explanations).unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FORMER_OFFICER_RETURN: &str =
        include_str!("../../tests/fixtures/form990/202400229349301310_public.xml");
    const LOANS_RETURN: &str =
        include_str!("../../tests/fixtures/form990/202420169349301967_public.xml");

    #[test]
    fn reads_a_published_return_with_a_former_officer_and_a_severance_answer() {
        let statement = parse_form_990(FORMER_OFFICER_RETURN).unwrap();
        assert_eq!(statement.ein.as_deref(), Some("320620949"));
        assert_eq!(statement.tax_period_end.as_deref(), Some("2023-06-30"));
        assert_eq!(statement.total_revenue, Some(2_436_073));
        assert_eq!(statement.total_expenses, Some(1_498_066));
        assert_eq!(statement.total_assets_end_of_year, Some(1_538_384));
        assert_eq!(statement.total_liabilities_end_of_year, Some(191_740));
        assert_eq!(statement.significant_diversion_reported, Some(false));
        assert_eq!(statement.former_listed, Some(true));
        assert_eq!(statement.total_reportable_compensation, Some(306_313));
        assert_eq!(statement.people.len(), 12);
        let former = statement
            .people
            .iter()
            .filter(|p| p.former)
            .collect::<Vec<_>>();
        assert_eq!(former.len(), 1);
        assert_eq!(former[0].name.as_deref(), Some("MATT CARTIER"));
        assert_eq!(former[0].reportable_compensation, Some(191_668));
        assert_eq!(former[0].other_compensation, Some(28_126));
        assert_eq!(former[0].positions, vec!["Former".to_string()]);

        assert!(statement.schedule_j_attached);
        assert_eq!(statement.severance_payment_reported, Some(true));
        let listed = &statement.schedule_j_people[0];
        assert_eq!(listed.filing_organisation.base, Some(86_687));
        assert_eq!(listed.filing_organisation.other, Some(104_981));
        assert_eq!(listed.filing_organisation.total, Some(219_795));
        assert_eq!(listed.related_organisations.total, Some(0));
        assert!(statement
            .schedule_j_explanations
            .iter()
            .any(|e| e.line_reference.as_deref() == Some("PART I, LINE 4A")
                && e.text.contains("SEVERANCE PAYMENT OF $104,981")));

        assert_eq!(statement.schedule_l_transactions.len(), 1);
        let grant = &statement.schedule_l_transactions[0];
        assert_eq!(grant.part, ScheduleLPart::GrantOrAssistance);
        assert_eq!(grant.amount, Some(35_256));
        assert_eq!(grant.name, None);
    }

    #[test]
    fn reads_schedule_l_loans_and_business_transactions_and_keeps_absence_apart_from_no() {
        let statement = parse_form_990(LOANS_RETURN).unwrap();
        let parts = statement
            .schedule_l_transactions
            .iter()
            .map(|t| t.part)
            .collect::<Vec<_>>();
        assert_eq!(
            parts,
            vec![
                ScheduleLPart::Loan,
                ScheduleLPart::Loan,
                ScheduleLPart::BusinessTransaction
            ]
        );
        let from_organisation = &statement.schedule_l_transactions[1];
        assert_eq!(from_organisation.amount, Some(6_325));
        assert_eq!(from_organisation.balance_due, Some(12_914));
        assert!(from_organisation
            .lines
            .iter()
            .any(|l| l.label == "Loan from the organisation" && l.value == "Yes"));
        assert!(!from_organisation
            .lines
            .iter()
            .any(|l| l.label == "Loan to the organisation"));
        let rental = &statement.schedule_l_transactions[2];
        assert_eq!(rental.name.as_deref(), Some("Mark and Marilyn Jones"));
        assert_eq!(rental.amount, Some(58_900));

        // This return attaches no Schedule J: the severance answer is absent, not "No".
        assert!(!statement.schedule_j_attached);
        assert_eq!(statement.severance_payment_reported, None);
        assert_eq!(statement.former_listed, Some(false));
    }

    #[test]
    fn refuses_a_return_without_a_form_990_and_an_unrecognised_answer() {
        let ez = "<Return xmlns=\"http://www.irs.gov/efile\"><ReturnHeader><ReturnTypeCd>990EZ</ReturnTypeCd></ReturnHeader><ReturnData><IRS990EZ/></ReturnData></Return>";
        assert_eq!(declared_return_type(ez).unwrap().as_deref(), Some("990EZ"));
        assert!(parse_form_990(ez).is_err());
        let odd = "\u{feff}<Return><ReturnHeader/><ReturnData><IRS990><MaterialDiversionOrMisuseInd>maybe</MaterialDiversionOrMisuseInd></IRS990></ReturnData></Return>";
        assert!(parse_form_990(odd).is_err());
    }
}
