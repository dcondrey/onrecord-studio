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
  | 'CROSS_CORPUS_REUSED_DENIAL_GROUNDS';

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
}

export interface FindingsFilter {
  portalKey?: string | null;
  agencyId?: number | null;
  requestId?: number | null;
  documentId?: number | null;
  sourceDetector?: string | null;
  includeDismissed?: boolean | null;
}

export interface RefreshFindingsSummary {
  detectorsRun: number;
  findingsRecorded: number;
  staleCleared: number;
}
