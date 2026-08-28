export type CorpusPatternKind =
  | 'EXACT_DUPLICATE'
  | 'NEAR_DUPLICATE'
  | 'SHARED_STATUTE'
  | 'SHARED_ENTITY'
  | 'SHARED_TERM'
  | 'TEMPORAL_CLUSTER';

export interface CorpusEvidenceReference {
  portalKey: string;
  requestId: number;
  documentId: number;
  segmentIndex: number | null;
  characterStart: number | null;
  characterEnd: number | null;
  evidenceLabel: string;
  contentSha256: string;
}

export interface CorpusPattern {
  patternId: string;
  kind: CorpusPatternKind;
  title: string;
  explanation: string;
  confidenceBasisPoints: number;
  evidence: CorpusEvidenceReference[];
  algorithmVersion: string;
}

export interface CorpusPatternQuery {
  portalKey?: string | null;
  minimumEvidenceCount: number;
  minimumConfidenceBasisPoints?: number;
  /** Inclusive RFC 3339 lower bound over the time a document entered this derived index. */
  indexedFrom?: string | null;
  /** Inclusive RFC 3339 upper bound over the time a document entered this derived index. */
  indexedTo?: string | null;
  resultLimit: number;
}

export interface CorpusIndexStatus {
  indexedDocumentCount: number;
  indexedSegmentCount: number;
  indexedTermCount: number;
  /** Remaining bounded writer-queue slots; zero means new writes must wait. */
  availableWriteCapacity: number;
  algorithmVersion: string;
}
