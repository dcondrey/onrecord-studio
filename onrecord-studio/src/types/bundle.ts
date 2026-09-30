/** Hand-mirrored from `src-tauri/src/bundle/mod.rs` and `src-tauri/src/commands/bundle.rs`. */

export interface ShareableBundleRequest {
  findingIds: string[];
}

export interface ShareableBundleExportRequest {
  findingIds: string[];
  /** The `contentSha256` of the preview that was confirmed. */
  confirmedContentSha256: string;
  targetPath: string | null;
}

export type BundleExclusionReason =
  | 'NOT_FOUND'
  | 'INFERRED_REDACTION_IDENTITY'
  | 'DISMISSED_BY_REVIEWER'
  | 'LEGAL_CLAIM_WITHOUT_MEASURED_CONFIDENCE'
  | 'NO_CITATION'
  | 'CITES_NON_PUBLIC_RECORD'
  | 'CITES_REMOVED_SOURCE';

export interface BundleExclusion {
  findingId: string;
  /** Masked. Null when the finding no longer exists or is a hypothesis. */
  headline: string | null;
  reason: BundleExclusionReason;
  explanation: string;
}

export interface BundlePreviewItem {
  findingId: string;
  headline: string;
  severity: string;
  basisLabel: string;
  sourceCount: number;
  excerptCount: number;
  maskedValueCount: number;
}

export interface BundlePreview {
  schemaVersion: number;
  included: BundlePreviewItem[];
  excluded: BundleExclusion[];
  sourceNames: string[];
  contentSha256: string;
}

export interface ShareableBundleResult {
  outputPath: string;
  fileSha256: string;
  contentSha256: string;
  generatedAt: string;
  findingCount: number;
  excludedCount: number;
}
