import type { AnalyticalDatasetKind, AnalyticalImportResult } from './analysisWorkbench';

/**
 * A data pack whose signed manifest verified against a pinned publisher key. From a local folder
 * every listed file verified too; from a published pack only the manifest has, and each file is
 * checked when it is downloaded.
 */
export interface DataPackSummary {
  pack: string;
  /** The pack keeps dated snapshots of each table's file; choosing a table downloads only the latest. */
  snapshots: boolean;
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
  /** Retrievals a snapshot pack recorded as unchanged from the table's latest snapshot. */
  unchangedChecks: number;
  /** The latest of those checks; `null` when there is none. */
  lastCheckedAt: string | null;
  /** Change records between the table's snapshots; listed, never imported. */
  changeRecords: number;
  /** What each change record compares and counts, oldest pair first. */
  changes: DataPackChange[];
}

/** One change record between two retrievals of a table's publisher file. */
export interface DataPackChange {
  previous: string;
  /** `null` when the manifest no longer lists the earlier original. */
  previousRetrievedAt: string | null;
  current: string;
  currentRetrievedAt: string | null;
  added: number;
  removed: number;
  /** `null` when `wholeRecords`: changed records were listed as removed and added, not separated. */
  changed: number | null;
  wholeRecords: boolean;
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
  /** `null`: no source list the app holds publishes this file, so the app does not import it. */
  datasetKind: AnalyticalDatasetKind | null;
  /** The file's own licence, as the signed manifest states it; `null` when the pack's licence covers it. */
  licence: DataPackFileLicence | null;
}

/** The licence one file of a pack is published under, where a pack mixes publishers. */
export interface DataPackFileLicence {
  /** An SPDX id, or a named public-domain basis such as `public-domain-us-government`. */
  id: string;
  url: string;
  /** The credit line the licence requires; `null` when it requires none. */
  attribution: string | null;
  /** The disclaimer the publisher requires; `null` when it requires none. */
  disclaimer: string | null;
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
