export interface MonitoringEvidence {
  portalKey: string; documentId: number; title: string; sourceUrl: string;
  sourceSha256: string; extractionSha256: string; quote: string; characterOffset: number;
  snapshotId: string | null; rowNumber: number | null;
}
export interface CoverageExpectation {
  id: string; caseId: string | null; topic: string; agency: string; recordType: string;
  period: string; queryTerms: string[]; consequence: string; acquisitionAction: string;
  state: string; evidence: MonitoringEvidence[];
}
export interface CoverageAssessment {
  expectation: CoverageExpectation; status: string; explanation: string; candidates: MonitoringEvidence[];
}
export interface RecordChange {
  id: string; sourceUrl: string; title: string; observedAt: string; category: string;
  beforeSha256: string; afterSha256: string; beforeQuote: string | null; afterQuote: string | null;
  interpretation: string; reviewState: string;
}
export interface ServiceObservation {
  id: string; caseId: string | null; topic: string; kind: string; place: string; observedOn: string;
  latitude: number | null; longitude: number | null; hoursPerWeek: number | null;
  status: string; evidence: MonitoringEvidence; reviewed: boolean;
}
export interface ServiceComparison {
  kind: string; title: string; observationIds: string[]; calculation: string; implication: string; limitations: string[];
}
export interface MonitoringStatus {
  topic: string; caseId: string | null; coverage: CoverageAssessment[]; changes: RecordChange[];
  observations: ServiceObservation[]; comparisons: ServiceComparison[];
  sourceCandidates: MonitoringEvidence[]; limitations: string[];
}
