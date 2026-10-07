import type { AnalyticalDatasetKind, AnalyticalImportResult } from './analysisWorkbench';

/**
 * A data pack whose signed manifest verified against a pinned publisher key. From a local folder
 * every listed file verified too; from a published pack only the manifest has, and each file is
 * checked when it is downloaded.
 */
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
  /** SHA-256 of the verified manifest; an import names it, so it imports only this manifest. */
  manifestSha256: string;
  tables: DataPackTable[];
}

export interface DataPackTable {
  table: string;
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
  /** When the pack builder retrieved the file from its publisher. */
  retrievedAt: string;
  /** `null`: the City source list does not publish this file, so the app does not import it. */
  datasetKind: AnalyticalDatasetKind | null;
}

/** A pack OnRecord publishes, and the public repository the app fetches it from. */
export interface PublishedPack {
  pack: string;
  repositoryUrl: string;
}

/** One table of a published pack to download, and its year for a table split by year. */
export interface PublishedPackSelection {
  table: string;
  year: number | null;
}

/** Exactly one of `result` and `error` is set. */
export interface DataPackImportOutcome {
  path: string;
  table: string;
  year: number | null;
  /** The manifest's retrieval time for this file. */
  retrievedAt: string;
  result: AnalyticalImportResult | null;
  /** Set with `result`: the bytes were already a stored snapshot; the pack was recorded beside it. */
  alreadyStored: boolean | null;
  error: string | null;
}
