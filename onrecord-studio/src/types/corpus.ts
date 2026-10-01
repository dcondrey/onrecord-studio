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
  | 'MONEY'
  | 'PERSON'
  | 'ROLE'
  | 'POSITION'
  | 'DOCUMENT_CLASS'
  | 'EIN'
  | 'CONTRACTOR_LICENSE'
  | 'PARCEL_NUMBER';

/**
 * What an edge claims. `CO_OCCURRENCE` is the derivation this app performs today; the seven
 * others describe an edge the text actually carried. A reader must not draw a `CO_OCCURRENCE`
 * alike to a `SIGNED_BY` — distance in the graph does not equal semantic strength.
 */
export type RelationshipKind =
  | 'CO_OCCURRENCE'
  | 'SIGNED_BY'
  | 'APPROVED'
  | 'CITES'
  | 'PROMISES'
  | 'DELIVERED'
  | 'SUPERSEDES'
  | 'AMENDS';

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
  kind: RelationshipKind;
  sharedDocumentCount: number;
  confidenceBasisPoints: number;
  /** RFC 3339 UTC. `null` when the segment carried no date; absence is not zero. */
  validFrom?: string | null;
  /** Symmetric upper bound. `null` for open-ended / unknown. */
  validUntil?: string | null;
}

export interface CorpusEntityNeighborhood {
  center: CorpusEntityNode;
  edges: CorpusEntityEdge[];
  /** The neighbour list hit its limit. A sparse entity and a cut one are different claims. */
  truncated: boolean;
}

export interface EntityPathStep {
  neighbor: CorpusEntityNode;
  edgeKind: RelationshipKind;
  edgeBasis: RelationshipBasis;
  sharedDocumentCount: number;
}

export interface EntityPath {
  steps: EntityPathStep[];
  hops: number;
  /** Minimum edge confidence across steps > 0. `null` on a zero-hop path. */
  weakestConfidenceBasisPoints?: number | null;
}

/** Mirrors `EntityPathOutcome` (serde tag `outcome`, camelCase). An unknown id and an unconnected pair are different statements and never share a value. */
export type EntityPathOutcome =
  | { outcome: 'found'; path: EntityPath }
  | { outcome: 'notConnected'; maxHops: number }
  | { outcome: 'unknownEntity'; entityId: string };

export interface EntityBroker {
  entity: CorpusEntityNode;
  brokerageScore: number;
  distinctNeighbors: number;
  /** Always true — this ranking is degree-weighted, not computed betweenness. */
  isApproximation: boolean;
}
