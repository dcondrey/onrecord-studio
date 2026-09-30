export type WatchKind = 'AGENCY' | 'ENTITY' | 'TOPIC_PACK' | 'QUERY' | 'SOURCE';

export type WatchDimension = 'DOCUMENTS' | 'FINDINGS' | 'FIGURES' | 'WITHHELD' | 'SOURCE_HEALTH';

/** `NOT_MEASURED` (nothing was observed this run) is never the same as `UNCHANGED` (observed and equal). */
export type DimensionOutcome = 'BASELINE_RECORDED' | 'UNCHANGED' | 'CHANGED' | 'NOT_MEASURED';

export type WatchCheckStatus = 'BASELINE' | 'COMPARED' | 'NO_DATA';

export interface Watch {
  id: string;
  kind: WatchKind;
  target: string;
  label: string;
  createdAt: string;
  enabled: boolean;
}

export interface DimensionResult {
  dimension: WatchDimension;
  outcome: DimensionOutcome;
  changeCount: number;
  baselineAsOf: string | null;
  observedAsOf: string | null;
  reason: string | null;
}

export interface WatchChange {
  id: string;
  watchId: string;
  dimension: WatchDimension;
  changeKind: string;
  subject: string;
  /** `null` means the baseline did not contain the subject. */
  oldValue: string | null;
  /** `null` means the current observation does not contain the subject. */
  newValue: string | null;
  oldAsOf: string | null;
  newAsOf: string;
  basis: string;
  detectedAt: string;
  portalKey: string | null;
}

export interface WatchCheck {
  watchId: string;
  checkedAt: string;
  status: WatchCheckStatus;
  dimensions: DimensionResult[];
  newChanges: WatchChange[];
}

export interface WatchSummary {
  watch: Watch;
  lastCheckedAt: string | null;
  lastDimensions: DimensionResult[];
}
