/** Hand-mirrored from `src-tauri/src/types/report.rs`. */

export interface EvidencePacketRequest {
  portalKey: string | null;
  findingIds: string[] | null;
  targetPath: string | null;
  includeAnnotations: boolean;
}

export interface PacketAnnotationPreviewRequest {
  portalKey: string | null;
  findingIds: string[] | null;
}

export interface AnnotationPreview {
  included: number;
  orphaned: number;
  notPublicOmitted: number;
}

export interface EvidencePacketExportResult {
  markdownPath: string;
  jsonPath: string;
  contentSha256: string;
  claimCount: number;
  gapCount: number;
  withheldCount: number;
  generatedAt: string;
  /** `null`: annotations were not requested. */
  annotationCount: number | null;
}

export interface VerifiableReportRequest {
  portalKey: string | null;
  findingIds: string[] | null;
  targetPath: string | null;
  authorName: string | null;
}

export interface VerifiableReportExportResult {
  outputPath: string;
  fileSha256: string;
  payloadSha256: string;
  signerKeyId: string;
  identityAssurance: string;
  claimCount: number;
  withheldCount: number;
  generatedAt: string;
}
