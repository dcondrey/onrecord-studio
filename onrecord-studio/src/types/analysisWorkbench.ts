export type AnalyticalDatasetKind =
  | 'REGIONAL_DATA'
  | 'RIPA_STOPS'
  | 'RIPA_SEARCH_BASIS'
  | 'CALLS_FOR_SERVICE'
  | 'CALLS_CODEBOOK'
  | 'NIBRS_OFFENSES'
  | 'OPERATING_BUDGET'
  | 'BUDGET_DIMENSION';

export type CityDataset = 'NIBRS_OFFENSES' | 'CALLS_FOR_SERVICE' | 'GET_IT_DONE_ENCAMPMENTS' | 'GET_IT_DONE_OPEN' | 'GET_IT_DONE_OUTCOMES' | 'SCHOOLS' | 'TRANSIT_STOPS' | 'OPERATING_BUDGET'
  | 'RIPA_STOPS' | 'RIPA_STOP_REASONS' | 'RIPA_STOP_RESULTS' | 'RIPA_PROPERTY_SEIZURE_BASIS'
  | 'SHELTER_INVENTORY'
  | 'OPERATING_ACTUALS';

export interface AnalyticalImportRequest {
  datasetKind: AnalyticalDatasetKind;
  displayName: string;
  sourceUri?: string | null;
  localPath: string;
  observationPeriodStart?: string | null;
  observationPeriodEnd?: string | null;
  partialDataset: boolean;
  worksheet?: string | null;
  sourceFileName?: string | null;
  expectedSourceSha256?: string | null;
}

export interface AnalyticalImportCandidate {
  documentId: number;
  portalKey: string;
  fileName: string;
  localPath: string;
  sourceUri: string | null;
  sourceSha256: string | null;
}

export interface AnalyticalImportResult {
  snapshotId: string;
  datasetKind: AnalyticalDatasetKind;
  displayName: string;
  tableName: string;
  importedRows: number;
  rejectedRows: number;
  malformedRows: number;
  duplicateKeyRows: number;
  /** Cells stored as NULL because their text did not convert to the column's type. */
  unconvertedCells: number;
  schemaWarnings: string[];
  sourceSha256: string;
}

export interface AnalyticalDatasetSummary {
  snapshotId: string;
  datasetKind: AnalyticalDatasetKind;
  displayName: string;
  tableName: string;
  importedRows: number;
  rejectedRows: number;
  sourceUri: string | null;
  sourceSha256: string;
  retrievedAt: string;
  observationPeriodStart: string | null;
  observationPeriodEnd: string | null;
  partialDataset: boolean;
  validationSummary: string;
  /** Per column alias; absent columns had none. */
  unconvertedByColumn: Record<string, number>;
}

export interface AnalyticalColumnSummary {
  tableName: string;
  columnName: string;
  logicalType: string;
  originalName: string | null;
  nullable: boolean;
  dictionary: string | null;
}

export interface SqlTemplate {
  templateId: string;
  title: string;
  description: string;
  sql: string;
  denominator: string;
}

export interface AnalyticalSchema {
  datasets: AnalyticalDatasetSummary[];
  columns: AnalyticalColumnSummary[];
  templates: SqlTemplate[];
}

export interface SqlQueryResult {
  columns: string[];
  rows: unknown[][];
  rowCount: number;
  truncated: boolean;
  elapsedMs: number;
  provenance: string[];
}

export interface SqlExportResult {
  outputPath: string;
  rowCount: number;
  sha256: string;
}

export interface ObservationSource {
  snapshotId: string;
  displayName: string;
  sourceSha256: string;
}

export interface NightHoursSectionCount {
  section: string;
  activityType: string;
  activities: number;
  insideWindow: number;
  atWindowBoundary: number;
  outsideWindow: number;
  timeNotRecorded: number;
}

export interface RecordedActivity {
  activityNumber: string;
  activityType: string;
  section: string;
  recordedAt: string;
  atWindowBoundary: boolean;
  location: string;
}

export interface NightHoursObservation {
  sources: ObservationSource[];
  firstActivity: string | null;
  lastActivity: string | null;
  sections: NightHoursSectionCount[];
  listedActivities: RecordedActivity[];
}

export interface LocationSectionCount {
  section: string;
  offenses: number;
  distinctCases: number;
  notGeocoded: number;
  withinNearBand: number | null;
  withinFarBand: number | null;
  beyondFarBand: number | null;
}

export interface NearestFeature {
  name: string;
  distanceFeet: number;
}

export interface EncampmentIncidentDistance {
  caseNumber: string;
  occurredOn: string | null;
  section: string;
  blockAddress: string;
  latitude: number | null;
  longitude: number | null;
  nearestSchool: NearestFeature | null;
  nearestStation: NearestFeature | null;
}

export interface EncampmentLocationObservation {
  incidentSources: ObservationSource[];
  schoolSource: ObservationSource | null;
  stationSource: ObservationSource | null;
  sections: LocationSectionCount[];
  incidents: EncampmentIncidentDistance[];
}

export interface EncampmentOrdinanceReport {
  nightHours: NightHoursObservation | null;
  locations: EncampmentLocationObservation | null;
  notMeasured: string[];
}
