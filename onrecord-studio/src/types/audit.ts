import type { TimelineAnomalyMarker } from '../generated/auditContracts';

export type { TimelineAnomalyMarker, VerificationState } from '../generated/auditContracts';

import type { VerificationState } from '../generated/auditContracts';

// 'StatutoryDeadlineExceeded' is a legal conclusion and requires a counted statutory violation.
// 'LatencyOutlier' is a statistical one about this agency against its peers, and is never on its
// own a violation.

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
  anomalyMarkers: TimelineAnomalyMarker[];
}

export type AuditTransitionEvent =
  | { type: 'stateTransition'; transition: StateTransitionLog }
  | {
      type: 'anomalyDetected';
      marker: TimelineAnomalyMarker;
      transition: StateTransitionLog;
    };

/** Expands one persisted transition into explicit, exhaustively typed timeline events. */
export function auditTransitionEvents(transition: StateTransitionLog): AuditTransitionEvent[] {
  return [
    { type: 'stateTransition', transition },
    ...transition.anomalyMarkers.map(
      (marker): AuditTransitionEvent => ({ type: 'anomalyDetected', marker, transition }),
    ),
  ];
}
