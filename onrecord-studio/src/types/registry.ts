/** Mirrors src-tauri/src/types/registry.rs. */
import type { CorpusEntityKind } from './corpus';

export type RegistryKind = 'CSLB_LICENSE_MASTER';

export interface RegistrySnapshotSummary {
  registry: RegistryKind;
  identifierKind: CorpusEntityKind;
  sourcePath: string;
  fileSha256: string;
  importedAt: string;
  recordCount: number;
  skippedRowCount: number;
}

export interface RegistryRecord {
  registrantName: string;
  status: string;
  detail: string;
}

/**
 * Four distinct facts. "No snapshot imported" is not "the snapshot does not list this number",
 * and a registered match says what the number is registered to, never who a party is.
 */
export type RegistryAnchor =
  | { state: 'NO_REGISTRY'; identifierKind: CorpusEntityKind }
  | { state: 'NOT_IMPORTED'; registry: RegistryKind }
  | { state: 'NOT_LISTED'; snapshot: RegistrySnapshotSummary; identifier: string }
  | { state: 'REGISTERED'; snapshot: RegistrySnapshotSummary; identifier: string; record: RegistryRecord };
