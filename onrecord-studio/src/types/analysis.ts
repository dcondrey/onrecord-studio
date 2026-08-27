// TODO(analysis): Generate these shared configuration types from the Rust
// command schema so threshold defaults remain identical across platforms.
export interface AnalysisConfig {
  latency_threshold_days: number;
  min_ocr_character_density: number;
  suspicious_closure_window_minutes: number;
  target_policy_keywords: string[];
}

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
  complaintCount48h: number;
  distinctSourceCount: number;
  firstCorrelatedEventAt: string;
  lastCorrelatedEventAt: string;
  keywordSignalCount: number;
  correlationDensity: number;
}

export interface OutlierCluster {
  clusterId: string;
  coreAnomalyType: 'CIVIC_DISPLACEMENT' | 'PROCUREMENT_ANOMALY' | string;
  dynamicZScore: number;
  matchingRequestIds: number[];
}
