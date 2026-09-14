export type AnalyticalDatasetKind =
  | 'REGIONAL_DATA'
  | 'RIPA_STOPS'
  | 'RIPA_SEARCH_BASIS'
  | 'CALLS_FOR_SERVICE'
  | 'CALLS_CODEBOOK'
  | 'NIBRS_OFFENSES'
  | 'OPERATING_BUDGET'
  | 'BUDGET_DIMENSION';

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
