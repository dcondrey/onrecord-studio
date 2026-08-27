// TODO(legal): Replace free-form ISO date strings with branded date types and
// validate effective-range ordering before authority imports reach Rust.
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
  effectiveFrom: string | null;
  effectiveTo: string | null;
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
  effectiveFrom: string | null;
  effectiveTo: string | null;
  provisionCount: number;
  mandateCount: number;
  importedAt: string;
}

export interface LegalAuthorityImportResult {
  authority: LegalAuthoritySummary;
  insertedNewVersion: boolean;
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
