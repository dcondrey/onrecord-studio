export type VerificationState = 
  | 'Unaudited' 
  | 'Compliant' 
  | 'PolicyViolationDetected' 
  | 'StatutoryDeadlineExceeded' 
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
}
