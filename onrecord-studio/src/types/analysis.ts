export type { AnalysisConfig } from '../generated/analysisConfig';

export interface AgencyPerformanceProfile {
  agencyId: number;
  portalSubdomain: string;
  agencyName: string;
  totalRequestsProcessed: number;
  totalClosed: number;
  validLatencySampleSize: number;
  excludedInvalidLifetimeCount: number;
  /**
   * `null` when the agency has closed nothing measurable. Never 0.0 for that case: 0.0 days is
   * the fastest possible answer, and an agency that has never closed a request is not fast.
   * `validLatencySampleSize` says how many closures the value rests on.
   */
  averageClosureDays: number | null;
  globalMeanClosureDays: number | null;
  globalStddevClosureDays: number | null;
  /** `null` when there is no closure sample, or the population has no spread. Never 0.0. */
  performanceZScore: number | null;
  // Narrower than VerificationState: `convert_raw_profile` produces only these three.
  // 'LatencyOutlier' means slower than peers by a wide margin; it is not a statutory finding.
  complianceIndicator: 'Compliant' | 'LatencyOutlier' | 'StatutoryDeadlineExceeded';
  anomalyCount: number;
  statutoryViolationCount: number;
  statutoryViolationRate: number;
  suspiciousFastClosureCount: number;
  totalDocumentsReleased: number;
  keywordFrequencyMap: Record<string, number>;
}

export interface CrossAgencyCorrelation {
  portalKey: string;
  requestId: number;
  agencyId: number | null;
  agencyName: string;
  requestTitle: string;
  requestTimestamp: string;
  neighborhoodBlock: string;
  complaintCount: number;
  correlationWindowHours: number;
  distinctSourceCount: number;
  firstCorrelatedEventAt: string;
  lastCorrelatedEventAt: string;
  keywordSignalCount: number;
  correlationDensity: number;
}

/**
 * One value in a lead figure: a measurement, or why there is none. Absence is never sent as 0 or
 * null. Mirrors `LeadFigureValue` in `analysis/service_leads.rs`.
 */
export type LeadFigureValue =
  | { state: 'value'; value: number }
  | { state: 'not-recorded' | 'not-covered' | 'excluded'; reason: string };

export type LeadFigureUnit = 'count' | 'currency' | 'percent' | 'hours' | 'days';

/**
 * What a lead figure may claim. A lead is never a counted legal finding, so the crimson 'counted'
 * claim cannot be expressed here at all.
 */
export type LeadFigureClaim = 'descriptive' | 'statistical';

/** Real records outside the figure's denominator (blank, conflicting, cancelled), drawn beside it. */
export interface LeadFigureExcluded {
  key: string;
  label: string;
  count: number;
}

interface LeadFigureBase {
  /** Names the measure; the figure's heading. */
  measure: string;
  claim: LeadFigureClaim;
  /** A standing qualification printed under the heading. */
  caption: string | null;
  /** Footnotes: undated or unplaced counts, rows below a minimum. */
  notes: string[];
}

/** One series per panel on a shared x (years); a panel's absent point is a gap with its reason. */
export interface LeadSeriesFigure extends LeadFigureBase {
  kind: 'series';
  points: { key: string; label: string; partial: boolean }[];
  panels: {
    key: string;
    label: string;
    unit: LeadFigureUnit;
    /** Aligned with `points`. */
    values: LeadFigureValue[];
    /** A lighter comparison line in the same panel, on the same scale. */
    context: { label: string; values: LeadFigureValue[] } | null;
  }[];
}

export type LeadSegmentTone = 'lead' | 'ord-1' | 'ord-2' | 'ord-3' | 'ord-4';

/**
 * Stacked rows: each row's values (aligned with `segments`) stack from zero, negatives the other
 * way. `normalise` draws each row as shares of its own total. Excluded counts are stubs beside the
 * bar; an overlay is a tick above it, never a segment (its records are already in a segment).
 */
export interface LeadStackedFigure extends LeadFigureBase {
  kind: 'stacked';
  unit: LeadFigureUnit;
  normalise: boolean;
  segments: { key: string; label: string; tone: LeadSegmentTone }[];
  rows: {
    key: string;
    /** Rows with the same group form one small multiple. */
    group: string;
    label: string;
    partial: boolean;
    values: number[];
    excluded: LeadFigureExcluded[];
    overlay: { label: string; value: number } | null;
    /** Annotation beside the row ('12 jobs with no tier recorded'). */
    note: string | null;
  }[];
}

/**
 * Two values per row on one axis: the lead's subject and a comparison. 'dots' is a dumbbell;
 * 'bullet' draws the comparison as a target tick and the subject as a bar, with an optional
 * yardstick band around the comparison (an app reference, not a rule).
 */
export interface LeadPairedFigure extends LeadFigureBase {
  kind: 'paired';
  unit: LeadFigureUnit;
  display: 'dots' | 'bullet';
  subjectLabel: string;
  comparisonLabel: string;
  band: { percent: number; minimum: number; label: string } | null;
  rows: {
    key: string;
    label: string;
    subject: LeadFigureValue;
    comparison: LeadFigureValue;
    /** Numerator and denominator in words ('12 of 40'). */
    subjectBasis: string | null;
    comparisonBasis: string | null;
  }[];
}

/** A distribution in bins on a scaled axis; records left of zero are an excluded column. */
export interface LeadBinsFigure extends LeadFigureBase {
  kind: 'bins';
  unit: 'hours';
  bins: { key: string; label: string; from: number; to: number | null; count: number }[];
  before: LeadFigureExcluded | null;
  median: number | null;
  reference: { at: number; label: string } | null;
  /** The denominator strip: matched, not matchable and unmatched records as one 100% bar. */
  denominator: LeadFigureExcluded[];
}

/** A ranked list; the lead's own subject row is marked. */
export interface LeadRankedFigure extends LeadFigureBase {
  kind: 'ranked';
  unit: LeadFigureUnit;
  rows: { key: string; label: string; value: LeadFigureValue; subject: boolean; detail: string | null }[];
  excluded: LeadFigureExcluded[];
}

/**
 * The figure a lead draws, built from the same rows as its detail text and stored with it. Bounded
 * and schema-checked by `analysis/service_leads.rs`.
 */
export type LeadFigure = LeadSeriesFigure | LeadStackedFigure | LeadPairedFigure | LeadBinsFigure | LeadRankedFigure;

export type LeadFigureKind = LeadFigure['kind'];

/** The kinds `analysis/service_leads.rs` accepts; compared by scripts/check-ipc-contract.mjs. */
export const LEAD_FIGURE_KINDS: readonly LeadFigureKind[] = ['series', 'stacked', 'paired', 'bins', 'ranked'];

/** Bounds shared with `analysis/service_leads.rs`; compared by scripts/check-ipc-contract.mjs. */
export const LEAD_FIGURE_LIMITS = {
  maxRows: 60,
  maxSegments: 8,
  maxPanels: 4,
  maxPoints: 40,
  maxNotes: 6,
  maxLabelChars: 300,
  maxNoteChars: 600,
  maxBytes: 24_000,
} as const;
