import type { AnalyticalDatasetKind, AnalyticalImportResult } from './analysisWorkbench';

/** A data pack after its signature, publisher key and every file have verified. */
export interface DataPackSummary {
  pack: string;
  packVersion: string;
  schemaVersion: string;
  /** When the pack builder finished; its rows are the builder's retrieval as of then. */
  builtAt: string;
  signerKeyId: string;
  licenceId: string;
  licenceUrl: string;
  disclaimer: string;
  tables: DataPackTable[];
}

export interface DataPackTable {
  table: string;
  /** `null`: this app has no analytical import for the table. */
  datasetKind: AnalyticalDatasetKind | null;
  files: DataPackFileSummary[];
}

export interface DataPackFileSummary {
  path: string;
  /** `null` for a table published as one file. */
  year: number | null;
  rows: number;
  bytes: number;
  partial: boolean;
  sourceUrl: string;
  retrievedAt: string;
}

/** Exactly one of `result` and `error` is set. */
export interface DataPackImportOutcome {
  path: string;
  table: string;
  year: number | null;
  result: AnalyticalImportResult | null;
  error: string | null;
}
