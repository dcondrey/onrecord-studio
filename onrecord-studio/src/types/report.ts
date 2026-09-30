/** Hand-mirrored from `src-tauri/src/types/report.rs`. */

export interface EvidencePacketRequest {
  portalKey: string | null;
  findingIds: string[] | null;
  targetPath: string | null;
}

export interface EvidencePacketExportResult {
  markdownPath: string;
  jsonPath: string;
  contentSha256: string;
  claimCount: number;
  gapCount: number;
  withheldCount: number;
  generatedAt: string;
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
