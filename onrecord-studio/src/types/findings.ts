/**
 * Types for the autonomous findings inbox.
 *
 * Hand-mirrored from `src-tauri/src/analysis/finding.rs` and
 * `src-tauri/src/storage/findings.rs`. Enums are structural string unions so they round-trip
 * cleanly with `#[serde(rename_all = "SCREAMING_SNAKE_CASE")]` on the Rust side.
 *
 * When a new `FindingKind` variant lands on the Rust side, add its SCREAMING_SNAKE_CASE string
 * here to keep the union exhaustive.
 */
import type { LeadFigure } from './analysis';

export type FindingSeverity = 'INFO' | 'NOTABLE' | 'SERIOUS' | 'CRITICAL';

export type FindingKind =
  | 'HELD_BACK_RESPONSE'
  | 'BOILERPLATE_DENIAL'
  | 'JUST_IN_TIME_COMPLIANCE'
  | 'EXTENSION_ABUSE'
  | 'ABSENCE_OF_RECORDS'
  | 'DIFFERENTIAL_TREATMENT'
  | 'METADATA_CONTRADICTS_CONTENT'
  | 'REGENERATED_RESPONSE'
  | 'EMAIL_THREAD_GAP'
  | 'CONTRADICTORY_STATEMENT'
  | 'NO_RECORDS_CONTRADICTED_BY_RELEASE'
  | 'EVASION_PARAPHRASED_DENIALS'
  | 'EVASION_ROUND_NUMBER_THRESHOLDS'
  | 'EVASION_REDACTION_AT_LIMIT'
  | 'EVASION_LATE_ARRIVING_RECORDS'
  | 'REDACTION_CANDIDATE_INFERRED'
  | 'CROSS_CORPUS_SHARED_LANGUAGE'
  | 'CROSS_CORPUS_TEMPORAL_SHIFT'
  | 'CROSS_CORPUS_ORPHAN_TOPIC'
  | 'CROSS_CORPUS_REUSED_DENIAL_GROUNDS'
  | 'POLICY_VIOLATION_DETECTED'
  | 'CLAIM_CONTRADICTS_DATA'
  | 'CORRELATED_FINDINGS'
  | 'PRACTICE_DEPARTURE_COUNTED'
  | 'PRACTICE_DEPARTURE_STATED'
  | 'CONTRACT_CEILING_EXCEEDED'
  | 'PAYMENT_OUTSIDE_CONTRACT_TERM'
  | 'PAYMENT_WITHOUT_CONTRACT_RECORD'
  | 'APPROPRIATION_EXCEEDED'
  | 'RESTRICTED_FUND_OFF_PROGRAM'
  | 'APPROVAL_THRESHOLD_EXCEEDED'
  | 'APPROVAL_RECORD_ABSENT'
  | 'SPLIT_PURCHASE_PATTERN'
  | 'SPLIT_PURCHASE_PROHIBITED_BY_POLICY'
  | 'DEPARTMENT_SPENDING_YEAR_OVER_YEAR'
  | 'OVERTIME_ABOVE_ADOPTED_LINE'
  | 'FUND_ACTUAL_ABOVE_ADOPTED'
  | 'ENCAMPMENT_NOTICE_SHORTFALL'
  | 'ENCAMPMENT_NOTICE_STATED_SHORTFALL'
  | 'REDACTION_INCONSISTENT_ACROSS_RELEASES'
  | 'ROLLING_PRODUCTION'
  | 'SERVICE_REPORT_LEAD'
  | 'POLICE_STOP_LEAD'
  | 'ABATEMENT_LOG_LEAD'
  | 'CHARGE_LOG_LEAD'
  | 'DISCLOSURE_FILING_LEAD'
  | 'CAPACITY_COMPARISON_LEAD'
  | 'SPENDING_RECORD_LEAD'
  | 'POLICE_CALL_LEAD'
  | 'PARKING_CITATION_LEAD'
  | 'HOUSING_APPROVAL_LEAD'
  | 'RELEASE_TIMING_CONCENTRATION';

/**
 * How a finding's confidence was derived. `UNASSESSED` distinguishes an unmeasured rung from a
 * computed-zero rung, matching the project's "absence and measurement never share a
 * representation" invariant.
 */
export type FindingConfidence =
  | { kind: 'UNASSESSED' }
  | { kind: 'MEASURED'; basisPoints: number };

export interface FindingSubject {
  portalKey: string | null;
  agencyId: number | null;
  requestId: number | null;
  documentId: number | null;
  entityId: string | null;
}

export interface FindingCitation {
  portalKey: string;
  documentId: number;
  characterStart: number | null;
  characterEnd: number | null;
  quote: string | null;
  label: string | null;
}

export interface Finding {
  id: string;
  kind: FindingKind;
  severity: FindingSeverity;
  confidence: FindingConfidence;
  subject: FindingSubject;
  headline: string;
  explanation: string;
  citations: FindingCitation[];
  discoveredAt: string;
  lastSeenAt: string;
  dismissedAt: string | null;
  sourceDetector: string;
  /**
   * A stored lead's figure. Sent only on a lookup of one finding by id (`FindingsFilter.id`), and
   * absent for a lead whose result was cut short or that predates figures.
   */
  figure?: LeadFigure | null;
}

export interface FindingsFilter {
  portalKey?: string | null;
  agencyId?: number | null;
  requestId?: number | null;
  documentId?: number | null;
  sourceDetector?: string | null;
  includeDismissed?: boolean | null;
  /** RFC 3339 instant; only findings first discovered strictly after it. */
  discoveredAfter?: string | null;
  /** Exactly one finding by id; a lookup, not a search. */
  id?: string | null;
}

export type GapStatus = 'acquiring' | 'exhausted' | 'noAutomaticSource';

/** A route the corpus identified but cannot yet test. Never a finding and never evidence of compliance. */
export interface EvidenceGapView {
  detector: string;
  key: string;
  route: string;
  blocked: string;
  role: string;
  request: string;
  status: GapStatus;
  sources: string[];
}

/** One discovery-feed lead sent to `record_service_leads`. Mirrors `analysis/service_leads.rs`. */
export interface ServiceLeadInput {
  /** Which published records the lead was counted from; decides the finding kind. */
  family: 'SERVICE_REPORTS' | 'POLICE_STOPS' | 'ABATEMENT_LOGS' | 'CHARGE_LOGS' | 'DISCLOSURE_FILINGS' | 'CAPACITY_COMPARISON' | 'SPENDING_RECORDS' | 'POLICE_CALLS' | 'PARKING_CITATIONS' | 'HOUSING_APPROVALS';
  key: string;
  snapshotId: string;
  /** Every table the SQL read, `snapshotId` first; each is cited in the stored finding. */
  snapshotIds: string[];
  /** The statement whose result the lead's figures were read from. */
  sql: string;
  title: string;
  detail: string;
  question: string;
  method: string;
  /** Drawn from the same rows as `detail`; null when the result was cut short or has nothing to draw. */
  figure: LeadFigure | null;
}

export interface ServiceLeadsRecorded {
  findingsRecorded: number;
  staleCleared: number;
}

export interface RefreshFindingsSummary {
  detectorsRun: number;
  findingsRecorded: number;
  staleCleared: number;
}

/**
 * Kinds that name a counted legal or policy violation. Mirrors
 * `FindingKind::is_legal_claim` in `analysis/finding.rs`; every other kind is a pattern or
 * statistical signal and must not be presented as a legal claim.
 */
export const LEGAL_CLAIM_KINDS: readonly FindingKind[] = [
  'POLICY_VIOLATION_DETECTED',
  'PRACTICE_DEPARTURE_COUNTED',
  'CONTRACT_CEILING_EXCEEDED',
  'PAYMENT_OUTSIDE_CONTRACT_TERM',
  'APPROPRIATION_EXCEEDED',
  'RESTRICTED_FUND_OFF_PROGRAM',
  'APPROVAL_THRESHOLD_EXCEEDED',
  'SPLIT_PURCHASE_PROHIBITED_BY_POLICY',
  'ENCAMPMENT_NOTICE_SHORTFALL',
];

export function isLegalClaimKind(kind: FindingKind): boolean {
  return LEGAL_CLAIM_KINDS.includes(kind);
}
