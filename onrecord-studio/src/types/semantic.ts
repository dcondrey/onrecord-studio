export interface EmbeddingModelStatus {
  modelId: string;
  revision: string;
  dimension: number;
  distanceMetric: string;
  normalization: string;
  cacheDir: string;
  installed: boolean;
  ready: boolean;
  artifactCount: number;
  artifactManifestSha256: string | null;
  lastError: string | null;
}

export interface EmbeddingIndexStatus {
  indexedDocumentCount: number;
  indexedChunkCount: number;
  pendingDocumentCount: number;
  failedDocumentCount: number;
  running: boolean;
  cancelled: boolean;
  lastError: string | null;
}

export interface EmbeddingIndexReport {
  indexedDocuments: number;
  indexedChunks: number;
  skippedDocuments: number;
  failedDocuments: number;
  cancelled: boolean;
  firstError: string | null;
}

export interface HybridSearchFilter {
  portalKey?: string | null;
  agencyName?: string | null;
  jurisdiction?: string | null;
  documentType?: string | null;
  dateFrom?: string | null;
  dateTo?: string | null;
}

export interface HybridSearchResult {
  stableId: string;
  portalKey: string;
  requestId: number;
  documentId: number;
  fileName: string;
  title: string;
  agencyName: string;
  jurisdiction: string;
  documentType: string | null;
  publicationDate: string | null;
  observationDate: string | null;
  effectiveDate: string | null;
  characterStart: number;
  characterEnd: number;
  excerpt: string;
  lexicalRank: number | null;
  semanticScore: number | null;
  fusedScore: number;
  retrievalStatus: string;
}

