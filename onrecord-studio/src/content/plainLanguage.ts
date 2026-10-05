import type { FindingKind } from '../types/findings';
import type { VerificationState } from '../types/audit';

/**
 * Plain-language readings for a non-specialist: what was observed, what it does not show, and
 * what to do next. This text explains a finding; it never replaces its basis label, confidence,
 * or citations, and it is never a second classification. Whether a kind is a legal claim stays
 * `isLegalClaimKind`, and the component that shows this text derives its basis sentence from
 * that, not from anything written here.
 *
 * Each table is a `Record` over the type that defines the kinds (`FindingKind`,
 * `VerificationState`), so a new variant fails `tsc` until it has text, and
 * `scripts/check-help-coverage.mjs` fails on the same omission without a build.
 * `src/tests/plainLanguage.test.ts` rejects accusatory wording for every kind and legal wording
 * for a kind that is not a legal claim.
 */
export interface PlainText {
  observed: string;
  notShown: string;
  next: string;
}

const READ_SOURCES = 'Open the cited documents and read the passages yourself before relying on this.';

export const PLAIN_FINDING_TEXT: Record<FindingKind, PlainText> = {
  HELD_BACK_RESPONSE: {
    observed: 'A response file was last edited well before it was released, and it was released just before the deadline.',
    notShown: 'It does not show the deadline was missed (it was met) or why the release waited.',
    next: 'Compare the file date with the release date on the cited document, then ask the agency when the response was ready.',
  },
  BOILERPLATE_DENIAL: {
    observed: 'Nearly identical denial wording appears in responses to unrelated requests.',
    notShown: 'It does not show that any denial was wrong or that a request was not reviewed individually. Agencies often use standard wording for standard reasons.',
    next: 'Read two of the cited denials side by side and check whether each explains its own request.',
  },
  JUST_IN_TIME_COMPLIANCE: {
    observed: 'This agency closes a larger share of requests in the last day before the deadline than agencies overall.',
    notShown: 'It does not show any request was late. Every closure was within the deadline.',
    next: 'Open the cited requests and check whether the timing matches the work each one needed.',
  },
  EXTENSION_ABUSE: {
    observed: "One stated reason accounts for most of this agency's deadline extensions.",
    notShown: 'It does not show any single extension was improper. One valid reason can apply often.',
    next: 'Read the extension notices and check whether each reason fits its request.',
  },
  ABSENCE_OF_RECORDS: {
    observed: 'A numbered series of released documents has numbers missing from the middle of the run.',
    notShown: 'It does not show the missing items exist or were owed to you. Numbers can be skipped for ordinary reasons.',
    next: 'List the missing numbers and ask the agency whether those items exist.',
  },
  DIFFERENTIAL_TREATMENT: {
    observed: "Outcomes for one group of requesters differ from the agency's overall pattern.",
    notShown: 'It does not show anyone was treated unfairly. Groups can differ in what they ask for.',
    next: 'Compare the kinds of requests in each group before drawing any conclusion.',
  },
  METADATA_CONTRADICTS_CONTENT: {
    observed: "A document's hidden file dates disagree with what its text says about itself.",
    notShown: 'It does not show the document was changed after the fact. Software can set dates oddly.',
    next: 'Open the document properties and the passage together, and ask the agency for the original file.',
  },
  REGENERATED_RESPONSE: {
    observed: 'A response file carries an edit date weeks or months before it was released.',
    notShown: 'It does not show when the response was decided or sent, or that anything was delayed.',
    next: 'Note the file date and release date, and ask the agency for its processing log.',
  },
  EMAIL_THREAD_GAP: {
    observed: 'A released email refers to an earlier message that was not in the release.',
    notShown: 'It does not show the earlier message is responsive or was left out. It may be outside your request.',
    next: 'Ask the agency whether the referenced message exists and falls within your request.',
  },
  CONTRADICTORY_STATEMENT: {
    observed: 'Two documents state different values for the same fact.',
    notShown: 'It does not show which one is right, or that either is a mistake. They may describe different times.',
    next: 'Check the dates of both documents and ask the agency which one applies.',
  },
  NO_RECORDS_CONTRADICTED_BY_RELEASE: {
    observed: 'A response said no records were found, while other released documents discuss the same topic.',
    notShown: 'It does not show the search was inadequate. The other documents may fall outside what was requested.',
    next: 'Compare the request wording with the other documents, and consider asking the agency to re-check its search.',
  },
  EVASION_PARAPHRASED_DENIALS: {
    observed: 'Denials from one agency on different topics use much the same vocabulary in different wording.',
    notShown: 'It does not show the wording was chosen for any purpose. Similar reasons can produce similar letters.',
    next: 'Read the denial letters together and check whether the reasons fit each request.',
  },
  EVASION_ROUND_NUMBER_THRESHOLDS: {
    observed: 'The number of documents released clusters just under round numbers such as 50 or 100.',
    notShown: 'It does not show releases were sized to a number. Round batch sizes are common.',
    next: 'Look at how the cited releases were batched and ask the agency how it sizes them.',
  },
  EVASION_REDACTION_AT_LIMIT: {
    observed: 'Heavily blacked-out documents show almost none of the technical traces that redaction errors usually leave.',
    notShown: 'It says nothing about what was redacted or why. Clean redaction is the expected good outcome.',
    next: 'No action is needed unless you have another reason to question these redactions.',
  },
  EVASION_LATE_ARRIVING_RECORDS: {
    observed: 'After a no-records answer, the same agency published documents that match the request topic.',
    notShown: 'It does not show the records existed at the time of the answer or that the answer was wrong.',
    next: 'Check the publication dates against the answer date and ask the agency when the records were created.',
  },
  REDACTION_CANDIDATE_INFERRED: {
    observed: 'Surrounding text and other documents point to a possible name behind a redaction.',
    notShown: 'It does not reveal what was redacted. It is a guess from context, and it may be wrong.',
    next: 'Treat it as a lead only. Do not repeat the name as fact, and confirm through a record the agency releases.',
  },
  CROSS_CORPUS_SHARED_LANGUAGE: {
    observed: 'The same paragraph, or a reworded version of it, appears in responses from three or more agencies.',
    notShown: 'It does not show the agencies coordinated or that any request was not reviewed individually. A shared template or common legal text is a likely cause.',
    next: 'Find the source of the paragraph, such as a statute or model policy, before drawing a conclusion.',
  },
  CROSS_CORPUS_TEMPORAL_SHIFT: {
    observed: "One agency's handling changed noticeably around a particular date.",
    notShown: 'It does not show why. Staffing, rules, or the mix of requests can all move these numbers.',
    next: 'Look for what changed at that date, such as a new policy or system, and read requests from before and after.',
  },
  CROSS_CORPUS_ORPHAN_TOPIC: {
    observed: "A topic common in one agency's records is missing from a comparable agency's.",
    notShown: 'It does not show the second agency holds such records. It may never do that work.',
    next: 'Check whether the second agency is responsible for that topic before asking it for records.',
  },
  CROSS_CORPUS_REUSED_DENIAL_GROUNDS: {
    observed: 'One stated ground for delay or denial is used across many different kinds of request.',
    notShown: 'It does not show the ground was misapplied. A broad ground can apply broadly.',
    next: 'Read a few of the cited requests and check whether the ground fits each.',
  },
  POLICY_VIOLATION_DETECTED: {
    observed: "The agency's own policy states a response time or fee, and the recorded requests or responses go past it.",
    notShown: "It is a count against the agency's own words. It is not a court finding, and it does not explain the cause.",
    next: READ_SOURCES,
  },
  CLAIM_CONTRADICTS_DATA: {
    observed: 'A figure stated in a document does not match the imported table for the same year.',
    notShown: 'It does not show which one is wrong, or that anyone meant to mislead.',
    next: 'Check the document sentence and the table row side by side, and check that the year and fund are the same.',
  },
  CORRELATED_FINDINGS: {
    observed: 'Several separate signals point at the same document, request, or agency.',
    notShown: 'It adds no new claim. Several weak signals together are not a stronger conclusion.',
    next: 'Open each listed finding and read it on its own terms.',
  },
  PRACTICE_DEPARTURE_COUNTED: {
    observed: 'A policy states a duty, and a later document from the same agency states how many cases did not follow it.',
    notShown: "The count is the document's, not recounted here. It does not say why, or whether anyone is responsible.",
    next: READ_SOURCES,
  },
  PRACTICE_DEPARTURE_STATED: {
    observed: 'A document says a written practice was not always followed, without giving a number.',
    notShown: 'It does not show how often this happened, or how serious it was.',
    next: 'Ask the agency for the count and the period the statement covers.',
  },
  CONTRACT_CEILING_EXCEEDED: {
    observed: 'Recorded payments on one contract add up to more than its written maximum.',
    notShown: 'It does not show why. A missing amendment or a coding choice can explain it. It says nothing about intent.',
    next: 'Check the contract and amendments on file, then the payment rows, for the same contract.',
  },
  PAYMENT_OUTSIDE_CONTRACT_TERM: {
    observed: "Payments are dated before a contract's recorded start or after its recorded end.",
    notShown: 'It does not show the work was not done or that the payment was improper. Dates may be entered late.',
    next: 'Check the contract dates and any amendments, then the payment dates.',
  },
  PAYMENT_WITHOUT_CONTRACT_RECORD: {
    observed: 'Payments to a vendor do not match any contract in the register provided.',
    notShown: 'It does not show no contract exists. Registers often leave out small purchases.',
    next: 'Ask the agency whether a contract or purchase order covers these payments.',
  },
  APPROPRIATION_EXCEEDED: {
    observed: 'Recorded spending in one fund and account is above the recorded revised budget authority.',
    notShown: 'It does not show the spending was unauthorized. A transfer or a late budget change may explain it.',
    next: 'Check for budget amendments or transfers that apply to this fund and year.',
  },
  RESTRICTED_FUND_OFF_PROGRAM: {
    observed: 'Spending from a restricted fund was charged to a program the restriction register does not name.',
    notShown: 'It does not show the restriction was not met. Program names can differ between systems.',
    next: 'Compare the program identifier with the restriction terms and ask the agency if the names differ.',
  },
  APPROVAL_THRESHOLD_EXCEEDED: {
    observed: "Rows above the approval limit in the agency's own policy record that no approval was obtained.",
    notShown: 'It does not show the spending was improper. It compares recorded rows with the policy text cited.',
    next: READ_SOURCES,
  },
  APPROVAL_RECORD_ABSENT: {
    observed: 'Rows above a stated approval limit have a blank or not-applicable approval entry.',
    notShown: 'It does not show approval was not obtained. It shows a record is missing.',
    next: 'Ask the agency for the approval records for these rows.',
  },
  SPLIT_PURCHASE_PATTERN: {
    observed: 'Several payments to one vendor, each just under a limit, add up to more than the limit.',
    notShown: 'It does not show they were one purchase or that the split was intended. The policy cited does not forbid it.',
    next: 'Check what each payment was for and whether they were separate needs.',
  },
  SPLIT_PURCHASE_PROHIBITED_BY_POLICY: {
    observed: "Payments just under a limit add up to more than it, and the agency's policy bars dividing a purchase to stay under it.",
    notShown: 'The ledger does not say whether it was one purchase, so this is a comparison and not a conclusion about intent.',
    next: 'Check what each payment was for, and ask the agency whether they were separate purchases.',
  },
  DEPARTMENT_SPENDING_YEAR_OVER_YEAR: {
    observed: "A department's spending changed by a large share from one fiscal year to the next.",
    notShown: 'It is a change in totals only. It does not explain the change or show that any rule applies.',
    next: 'Read the budget notes for the year for an explanation.',
  },
  OVERTIME_ABOVE_ADOPTED_LINE: {
    observed: 'Spending on an overtime account is well above the amount adopted in the budget.',
    notShown: 'It does not show the extra spending was unapproved. Adopted is the starting budget and may have been revised.',
    next: 'Look for mid-year budget changes for this account.',
  },
  FUND_ACTUAL_ABOVE_ADOPTED: {
    observed: "A fund's spending is well above its adopted budget for the year.",
    notShown: 'It is not a test of spending authority, because adopted is not the revised amount.',
    next: 'Look for mid-year budget changes for this fund.',
  },
  ENCAMPMENT_NOTICE_SHORTFALL: {
    observed: "The recorded notice date and cleanup date are closer together than the minimum notice the agency's own policy or a stored authority states.",
    notShown: 'It relies on the dates in the documents. It does not show when notice was actually posted or what happened on site.',
    next: READ_SOURCES,
  },
  ENCAMPMENT_NOTICE_STATED_SHORTFALL: {
    observed: 'A notice states a notice period shorter than the stated minimum, and both dates are not recorded.',
    notShown: 'It compares one statement with another. It does not show the actual time given.',
    next: 'If it matters to your work, ask the agency for the posting and cleanup dates.',
  },
  REDACTION_INCONSISTENT_ACROSS_RELEASES: {
    observed: 'Two public copies of the same file, released under different requests, redact different passages.',
    notShown: 'It does not show which release was correct. A later release may apply an exemption the earlier one did not.',
    next: 'Compare the cited passages in both copies, then ask the agency why they differ.',
  },
  ROLLING_PRODUCTION: {
    observed: 'The portal shows records for this request released on several separate dates, spread over months.',
    notShown: 'It does not show that a deadline was missed. The requester may have agreed to installments or changed the request.',
    next: "Read the request's correspondence for a production schedule or a change of scope.",
  },
  RELEASE_TIMING_CONCENTRATION: {
    observed: "A larger share of this agency's releases began late on the last working day before a weekend or court holiday than of the other releases on the same portal.",
    notShown: 'It does not show why. Work is often finished and posted at the end of a week, and the portal records when a file was uploaded, not when it was made public.',
    next: "Read the counts, both shares and the cited files. If it matters to your work, ask the agency for the requests' due dates and the portal's document activity log.",
  },
  SPENDING_RECORD_LEAD: {
    observed: "Sums from published spending tables set beside each other: a department's actual expense against its adopted amount, settlement and claim accounts by year, or overtime paid by department.",
    notShown: 'It does not show why. Adopted is the original appropriation, not the revised budget; an amount not spent may have been transferred or carried forward, and the tables do not say whether overtime was authorised.',
    next: "Ask the department for its revised budget and year-end report for the year named, or its overtime and vacancy reports, and read the Council's settlement resolutions.",
  },
  ABATEMENT_LOG_LEAD: {
    observed: "A count from an agency's released encampment abatement logs: the hours between a notice being posted and the abatement at the same place, and abatements at places with no notice in the log.",
    notShown: 'It does not show that too little notice was given. The logs do not record the notice period that applied, whether anyone was present, or what was done with belongings.',
    next: "Ask the agency for its abatement procedure, the notices as posted and the stored-property log for the places listed.",
  },
  CHARGE_LOG_LEAD: {
    observed: "A count from a police department's released arrest and citation log: arrests and citations citing an encampment, encroachment, lodging or public urination section, and arrests on those sections alone.",
    notShown: 'It does not show that the people charged had no housing, or why they were charged. The log records the section, not whether shelter was offered or what became of the charge.',
    next: "Ask the department for the reports behind the arrests made on these sections alone, and the prosecutor for what became of the charges.",
  },
  CAPACITY_COMPARISON_LEAD: {
    observed: "Three counts set side by side for each fiscal year: the City department's adopted homelessness budget, Encampment reports residents filed, and emergency shelter beds in the January inventory.",
    notShown: "It does not show why any of the three moved. The budget is one department's adopted amount, not all homelessness spending; the beds are emergency shelter only; the reports are complaints filed, not people.",
    next: "Ask the department which programmes each year's adopted amount paid for, and the Housing Commission for its shelter agreements and occupancy in the same years.",
  },
  POLICE_STOP_LEAD: {
    observed: "A count from a police department's published stop records: how often people an officer perceived as unhoused were stopped, arrested, cited or had property seized, and the recorded reasons.",
    notShown: "It does not show unequal treatment. Rates are not adjusted for where stops were made, and perception is the officer's entry.",
    next: 'Ask the department for the stops by reason, beat and warrant status, and for its property log.',
  },
  SERVICE_REPORT_LEAD: {
    observed: "A count from a publisher's table of service reports: reports still open, an area slower than the others, a place reported many times, or reports that left the open file.",
    notShown: 'It does not show that a problem persists or that work was not done. The table records no inspections, repairs or crews.',
    next: 'Ask the responsible department what explains the count, or request the work orders behind it.',
  },
};

export const PLAIN_VERIFICATION_TEXT: Record<VerificationState, PlainText> = {
  Unaudited: {
    observed: 'No check has been run on this yet.',
    notShown: 'This is not a result. It does not mean the record is fine or that it has a problem.',
    next: 'Run the analysis to get a result.',
  },
  Compliant: {
    observed: 'The checks that were run did not count a deadline overrun.',
    notShown: 'It covers only the requests and dates that were recorded. It does not vouch for anything unrecorded.',
    next: 'Check how many requests were included before relying on it.',
  },
  PolicyViolationDetected: {
    observed: "Recorded practice went past a commitment the agency's own policy states.",
    notShown: "It is a count against the agency's words. It is not a court finding and does not explain why.",
    next: 'Open the cited policy sentence and the cited requests and compare them.',
  },
  StatutoryDeadlineExceeded: {
    observed: 'At least one request took longer than the calendar-day limit the statute sets, counting only recorded dates.',
    notShown: 'It is a count, not a court finding. Extensions or missing dates can change the count.',
    next: 'Open the counted requests and check the dates and any extension notices.',
  },
  LatencyOutlier: {
    observed: "This agency's closing times are much slower than other agencies'.",
    notShown: 'It is a comparison with peers only. No statute is involved, and it does not mean any deadline was missed.',
    next: 'Look at which requests were slow and what they involved.',
  },
  DataAnomalyFlagged: {
    observed: 'Some recorded dates or values look unusual, such as a closing date before the opening date.',
    notShown: 'It does not show the agency did anything. Data entry or export errors are a common cause.',
    next: 'Check the flagged rows against the source records.',
  },
};

/** Verification states that name a counted comparison. Every other state is a statistical or procedural label. */
export const LEGAL_VERIFICATION_STATES: readonly VerificationState[] = ['PolicyViolationDetected', 'StatutoryDeadlineExceeded'];
