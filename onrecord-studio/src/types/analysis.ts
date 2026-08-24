export interface AnalysisConfig {
  latency_threshold_days: number;
  min_ocr_character_density: number;
  suspicious_closure_window_minutes: number;
  target_policy_keywords: string[];
}

export interface AgencyPerformanceProfile {
  agencyId: number;
  agencyName: string;
  totalRequestsProcessed: number;
  totalClosed: number;
  averageClosureDays: number;
  anomalyCount: number;
  totalDocumentsReleased: number;
  keywordFrequencyMap: Record<string, number>;
}

export interface OutlierCluster {
  clusterId: string;
  coreAnomalyType: 'CIVIC_DISPLACEMENT' | 'PROCUREMENT_ANOMALY' | string;
  dynamicZScore: number;
  matchingRequestIds: number[];
}
