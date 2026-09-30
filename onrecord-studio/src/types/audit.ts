import type { TimelineAnomalyMarker } from '../generated/auditContracts';

export type { TimelineAnomalyMarker, VerificationState } from '../generated/auditContracts';

// 'StatutoryDeadlineExceeded' is a legal conclusion and requires a counted statutory violation.
// 'LatencyOutlier' is a statistical one about this agency against its peers, and is never on its
// own a violation.

export interface StateTransitionLog {
  requestId: number;
  transitionTimestamp: string;
  previousState: string;
  nextState: string;
  timeSpentInPreviousStateSeconds: number;
  anomalyMarkers: TimelineAnomalyMarker[];
}
