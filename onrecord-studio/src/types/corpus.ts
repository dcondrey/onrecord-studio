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
  // TODO(corpus): Add explicit time and confidence bounds when the index can
  // answer those dimensions without post-filtering in the React client.
  portalKey?: string | null;
  minimumEvidenceCount: number;
  resultLimit: number;
}

export interface CorpusIndexStatus {
  indexedDocumentCount: number;
  indexedSegmentCount: number;
  indexedTermCount: number;
  // COMPAT(ipc): The name is inverted relative to the value. This is REMAINING writer-queue
  // capacity, so a high number means idle and 0 means saturated. Do not render it as a backlog
  // gauge. The mirror of this comment is on `pending_write_capacity` in
  // src-tauri/src/types/corpus.rs; both must change together. See docs/ipc-contract.md.
  pendingWriteCapacity: number;
  algorithmVersion: string;
}
