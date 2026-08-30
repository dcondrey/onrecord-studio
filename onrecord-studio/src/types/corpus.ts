export type CorpusPatternKind =
  | 'EXACT_DUPLICATE'
  | 'NEAR_DUPLICATE'
  | 'SHARED_STATUTE'
  | 'SHARED_ENTITY'
  | 'SHARED_TERM'
  | 'TEMPORAL_CLUSTER'
  | 'ENTITY_RELATIONSHIP';

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
  patternKinds?: CorpusPatternKind[];
  requiredTerms?: string[];
  resultLimit: number;
}

export interface CorpusIndexStatus {
  indexedDocumentCount: number;
  indexedSegmentCount: number;
  indexedTermCount: number;
  indexedRelationshipCount: number;
  /** Remaining bounded writer-queue slots; zero means new writes must wait. */
  availableWriteCapacity: number;
  algorithmVersion: string;
}

export type CorpusEntityKind =
  | 'STATUTE'
  | 'DATE'
  | 'EMAIL_DOMAIN'
  | 'EMAIL_ADDRESS'
  | 'URL_HOST'
  | 'ACRONYM'
  | 'ORGANIZATION'
  | 'MONEY';

/** How hard the extractor that produced a mention verified the string it matched. */
export type EntityValidationGrade =
  | 'CHECK_DIGIT_PASSED'
  | 'FORMAT_VALIDATED'
  | 'PATTERN_ONLY'
  | 'GAZETTEER_HIT';

/** How an edge came to exist. A computed edge must never be drawn as an observed one. */
export type RelationshipBasis = 'OBSERVED' | 'COMPUTED' | 'ASSERTED';

export interface CorpusEntityNode {
  entityId: string;
  kind: CorpusEntityKind;
  displayValue: string;
  documentCount: number;
  /** The weakest grade among this entity's mentions, not the best. */
  validationGrade: EntityValidationGrade;
}

export interface CorpusEntityEdge {
  neighbor: CorpusEntityNode;
  basis: RelationshipBasis;
  sharedDocumentCount: number;
  confidenceBasisPoints: number;
}

export interface CorpusEntityNeighborhood {
  center: CorpusEntityNode;
  edges: CorpusEntityEdge[];
  /** The neighbour list hit its limit. A sparse entity and a cut one are different claims. */
  truncated: boolean;
}
