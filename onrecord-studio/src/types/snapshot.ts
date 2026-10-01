export interface ResearchSnapshotManifest {
  schema: string;
  appVersion: string;
  createdAt: string;
  fileName: string;
  fileSha256: string;
  corpusStateDigest: string;
  findingsDigest: string;
  tableRows: Record<string, number>;
}

export interface ResearchSnapshotResult {
  path: string;
  manifestPath: string;
  manifest: ResearchSnapshotManifest;
}
