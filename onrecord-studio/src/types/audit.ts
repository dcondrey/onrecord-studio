// TODO(audit): Model audit transitions as a discriminated event union so
// timeline rendering does not infer semantics from loosely typed strings.
//
// Mirrors `VerificationState` in src-tauri/src/types/audit.rs. Hand-maintained, so
// scripts/check-ipc-contract.mjs fails the build if the two lists drift.
// 'StatutoryDeadlineExceeded' is a legal conclusion and requires a counted statutory violation.
// 'LatencyOutlier' is a statistical one about this agency against its peers, and is never on its
// own a violation.
export type VerificationState =
  | 'Unaudited'
  | 'Compliant'
  | 'PolicyViolationDetected'
  | 'StatutoryDeadlineExceeded'
  | 'LatencyOutlier'
  | 'DataAnomalyFlagged';

export interface CaseAuditRecord {
  auditId: string; // Client-side generated UUID
  targetRequestId: number;
  targetSubdomain: string;
  auditorNotes: string | null;
  verifiedAt: string; // ISO-8601 completion log string
  verificationStatus: VerificationState;
  documentIntegrityFlags: string[]; // e.g., ['REDACTION_HEAVY', 'CORRUPT_METADATA']
}

export interface StateTransitionLog {
  requestId: number;
  transitionTimestamp: string;
  previousState: string;
  nextState: string;
  timeSpentInPreviousStateSeconds: number;
  anomalyMarkers: Array<
    | 'SUSPICIOUSLY_FAST_CLOSURE'
    | 'CLOSE_BEFORE_CREATION'
    | 'OBSERVATION_BEFORE_CREATION'
  >;
}
