declare const isoDateBrand: unique symbol;
export type IsoDateString = string & { readonly [isoDateBrand]: true };

export function parseIsoDateString(value: string): IsoDateString | null {
  const normalized = value.trim();
  if (!/^\d{4}-\d{2}-\d{2}$/.test(normalized)) return null;
  const date = new Date(`${normalized}T00:00:00Z`);
  return Number.isNaN(date.getTime()) || date.toISOString().slice(0, 10) !== normalized
    ? null
    : normalized as IsoDateString;
}
export type LegalAuthorityKind =
  | 'STATE_STATUTE'
  | 'MUNICIPAL_ORDINANCE'
  | 'AGENCY_POLICY'
  | 'PROCEDURE_MANUAL'
  | 'REGULATION'
  | 'OTHER';

export type LegalMandateSeverity = 'ADVISORY' | 'MODERATE' | 'HIGH' | 'CRITICAL';

export interface LegalMandateInput {
  mandateId: string;
  title: string;
  description: string;
  applicabilityPhrases: string[];
  requiredEvidencePhrases: string[];
  severity: LegalMandateSeverity;
}

export interface LegalAuthorityImport {
  statuteId: string;
  jurisdiction: string;
  statuteCitation: string;
  title: string;
  authorityKind: LegalAuthorityKind;
  rawText: string;
  mandatedDeadlineDays: number | null;
  canonicalSourceUri: string | null;
  effectiveFrom: IsoDateString | null;
  effectiveTo: IsoDateString | null;
  mandates: LegalMandateInput[];
}

export interface LegalAuthoritySummary {
  statuteId: string;
  jurisdiction: string;
  statuteCitation: string;
  title: string;
  authorityKind: LegalAuthorityKind;
  currentVersionId: string;
  contentSha256: string;
  mandatedDeadlineDays: number | null;
  canonicalSourceUri: string | null;
  effectiveFrom: IsoDateString | null;
  effectiveTo: IsoDateString | null;
  provisionCount: number;
  mandateCount: number;
  importedAt: string;
}

export interface LegalAuthorityImportResult {
  authority: LegalAuthoritySummary;
  insertedNewVersion: boolean;
}

export interface LegalAuthorityVersionSummary {
  versionId: string;
  statuteId: string;
  contentSha256: string;
  effectiveFrom: IsoDateString | null;
  effectiveTo: IsoDateString | null;
  sourceUri: string | null;
  importedAt: string;
}

export interface LegalAuthorityVersionDetail {
  version: LegalAuthorityVersionSummary;
  fullText: string;
}

export interface LegalComplianceFinding {
  violationId: string;
  requestId: number;
  portalKey: string;
  matchedStatuteId: string;
  matchedVersionId: string;
  mandateId: string | null;
  statuteCitation: string;
  authorityTitle: string;
  mandateTitle: string | null;
  classification: string;
  calculatedVarianceMetric: string | null;
  varianceDays: number | null;
  confidenceBasisPoints: number;
  evidenceSummary: string;
  supportingDocumentIds: number[];
  missingEvidencePhrases: string[];
  reviewStatus: 'OPEN' | 'CONFIRMED' | 'DISMISSED' | 'RESOLVED_BY_NEW_EVIDENCE';
  firstDetectedAt: string;
  evaluatedAt: string;
  /** The determination clock a timeline finding was measured under; `null` otherwise or when not recorded. */
  clockRuleId: string | null;
  /** Whether the determination date was recorded or a proxy; `null` when not recorded on the finding. */
  determinationBasis: DeterminationBasis | null;
}

/** Mirrors `analysis::statute::StatutoryJurisdiction`. */
export type StatutoryJurisdiction = 'CA_CPRA' | 'US_FOIA';

/** Mirrors `analysis::statute::DeterminationBasis`. */
export type DeterminationBasis =
  | 'SOURCED_DETERMINATION_EVENT'
  | 'EARLIEST_RECORDED_ACTIVITY_PROXY'
  | 'OBSERVATION_WITHOUT_RECORDED_ACTIVITY';

export interface StatuteCitation {
  section: string;
  url: string;
}

export interface StatutoryPeriod {
  days: number;
  kind: 'CALENDAR_CARRIED_PAST_CLOSED_DAYS' | 'WORKING_DAYS';
}

/** Mirrors `analysis::statute::StatutoryClockRule`: one jurisdiction's determination clock. */
export interface StatutoryClockRule {
  id: string;
  jurisdiction: StatutoryJurisdiction;
  title: string;
  actName: string;
  statuteId: string;
  start: { event: string; latestAfterReceipt: StatutoryPeriod | null; citation: StatuteCitation };
  stopEvent: string;
  initial: StatutoryPeriod;
  initialCitation: StatuteCitation;
  calendar: 'CALIFORNIA_JUDICIAL' | 'FEDERAL_LEGAL_PUBLIC';
  calendarCitation: StatuteCitation;
  extension: { maximum: StatutoryPeriod; grounds: string[]; citation: StatuteCitation };
  unrecordedClockStops: StatuteCitation | null;
  notes: string[];
}

export interface LegalEvaluationResult {
  requestId: number;
  portalKey: string;
  evaluatedMandateCount: number;
  applicableMandateCount: number;
  corpusCharacterCount: number;
  evaluationComplete: boolean;
  warnings: string[];
  findings: LegalComplianceFinding[];
}

export type LegalFindingReviewStatus = 'OPEN' | 'CONFIRMED' | 'DISMISSED';
