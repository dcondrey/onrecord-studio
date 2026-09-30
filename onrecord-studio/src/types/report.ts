/** Hand-mirrored from `src-tauri/src/types/report.rs`. */

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
