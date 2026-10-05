export interface OrganisationMatch {
  ein: string;
  name: string;
  city: string | null;
  state: string | null;
}

export interface FilingLine {
  label: string;
  value: string;
}

export interface FilingExplanation {
  lineReference: string | null;
  text: string;
}

export interface Form990Person {
  name: string | null;
  title: string | null;
  positions: string[];
  former: boolean;
  reportableCompensation: number | null;
  relatedOrganisationCompensation: number | null;
  otherCompensation: number | null;
}

export interface ScheduleJAmounts {
  base: number | null;
  bonus: number | null;
  other: number | null;
  deferred: number | null;
  nontaxableBenefits: number | null;
  total: number | null;
  reportedInPriorReturn: number | null;
}

export interface ScheduleJPerson {
  name: string | null;
  title: string | null;
  filingOrganisation: ScheduleJAmounts;
  relatedOrganisations: ScheduleJAmounts;
}

export type ScheduleLPart = 'EXCESS_BENEFIT_TRANSACTION' | 'LOAN' | 'GRANT_OR_ASSISTANCE' | 'BUSINESS_TRANSACTION';

export interface ScheduleLTransaction {
  part: ScheduleLPart;
  name: string | null;
  relationship: string | null;
  description: string | null;
  amount: number | null;
  balanceDue: number | null;
  lines: FilingLine[];
}

/** What one e-filed Form 990 states. `null` is an element the return does not carry, never "No" or zero. */
export interface Form990Statement {
  returnVersion: string | null;
  ein: string | null;
  filerName: string | null;
  taxPeriodBegin: string | null;
  taxPeriodEnd: string | null;
  amended: boolean | null;
  totalRevenue: number | null;
  totalExpenses: number | null;
  totalAssetsEndOfYear: number | null;
  totalLiabilitiesEndOfYear: number | null;
  netAssetsEndOfYear: number | null;
  governmentGrants: number | null;
  significantDiversionReported: boolean | null;
  formerListed: boolean | null;
  totalReportableCompensation: number | null;
  people: Form990Person[];
  scheduleJAttached: boolean;
  severancePaymentReported: boolean | null;
  scheduleJPeople: ScheduleJPerson[];
  scheduleJExplanations: FilingExplanation[];
  scheduleLAttached: boolean;
  scheduleLTransactions: ScheduleLTransaction[];
  scheduleLExplanations: FilingExplanation[];
  scheduleOExplanations: FilingExplanation[];
}

export interface OrganisationReturn {
  objectId: string;
  indexYear: number;
  returnType: string;
  taxPeriod: string;
  taxpayerName: string;
  archiveUrl: string | null;
  memberName: string | null;
  xmlSha256: string | null;
  retrievedAt: string | null;
  statement: Form990Statement | null;
  unreadReason: string | null;
}

export interface OrganisationSummaryYear {
  taxYear: number;
  taxPeriod: string | null;
  formType: string | null;
  totalRevenue: number | null;
  totalExpenses: number | null;
  totalAssetsEndOfYear: number | null;
  totalLiabilitiesEndOfYear: number | null;
  officerCompensation: number | null;
}

export interface OrganisationSummary {
  name: string | null;
  city: string | null;
  state: string | null;
  years: OrganisationSummaryYear[];
  sourceUrl: string;
  retrievedAt: string;
}

export interface SingleAuditFinding {
  referenceNumber: string | null;
  complianceRequirement: string | null;
  lines: FilingLine[];
}

export interface SingleAudit {
  reportId: string;
  auditYear: string | null;
  fiscalYearEnd: string | null;
  acceptedDate: string | null;
  auditor: string | null;
  federalAmountExpended: number | null;
  lines: FilingLine[];
  findings: SingleAuditFinding[];
}

export interface SingleAuditHistory {
  audits: SingleAudit[];
  sourceUrl: string;
  retrievedAt: string;
}

export interface PublicMoneyLine {
  role: string;
  datasetName: string;
  sourceUrl: string | null;
  vendorValue: string;
  fiscalYear: string | null;
  rows: number;
  amount: string | null;
}

export interface IndexYearCheck {
  year: number;
  checkedAt: string;
  returnsListed: number;
}

export interface OrganisationFilings {
  ein: string;
  name: string | null;
  checkedAt: string;
  indexYears: IndexYearCheck[];
  returns: OrganisationReturn[];
  summary: OrganisationSummary | null;
  /** `null` when no clearinghouse key was supplied, so no audit lookup ran. */
  singleAudits: SingleAuditHistory | null;
  warnings: string[];
  publicMoney: PublicMoneyLine[];
  publicMoneyNotes: string[];
}

export interface OrganisationCheckSummary {
  ein: string;
  name: string | null;
  checkedAt: string;
}

export interface OrganisationPaymentMapping {
  snapshotId: string;
  vendorColumn: string;
  amountColumn: string;
  fiscalYearColumn: string | null;
}
