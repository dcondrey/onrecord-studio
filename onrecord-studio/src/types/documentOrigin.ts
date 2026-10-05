import type { SourceKind } from './investigation';

/**
 * Mirrors `src-tauri/src/types/document_origin.rs`. A comparison that could not be made
 * (`NOT_COMPARED`) is a different state from one that ran and matched nothing (`COMPARED` with no
 * matches); neither may be rendered as the other.
 */
export type DocumentOrigin = 'ADDED_BY_USER' | 'COLLECTED_FROM_SOURCE';

export interface HeldDocumentMatch {
  portalKey: string;
  sourceName: string;
  requestId: number;
  /** `null`: the source records neither a request reference nor a title. */
  requestLabel: string | null;
  documentId: number;
  fileName: string | null;
}

export interface NearDuplicateMatch {
  document: HeldDocumentMatch;
  simhashDistance: number;
}

export type NotComparedReason =
  | 'FILE_DIGEST_NOT_RECORDED'
  | 'TEXT_NOT_EXTRACTED'
  | 'NOT_IN_SIMILARITY_INDEX';

export type HeldComparison<T> =
  | { state: 'NOT_COMPARED'; reason: NotComparedReason }
  | { state: 'COMPARED'; matches: T[]; truncated: boolean; comparedCount: number };

export interface HeldDocumentRelation {
  exactCopies: HeldComparison<HeldDocumentMatch>;
  sameText: HeldComparison<HeldDocumentMatch>;
  nearDuplicates: HeldComparison<NearDuplicateMatch>;
}

export interface DocumentOriginReport {
  documentId: number;
  portalKey: string;
  origin: DocumentOrigin;
  sourceKind: SourceKind;
  /** `null` for a collected document: the comparison is defined for hand-added files only. */
  heldRelation: HeldDocumentRelation | null;
}

export type InternalMetadataFormat = 'PDF' | 'OOXML';

export type InternalMetadataUnreadReason = 'FILE_UNAVAILABLE' | 'UNSUPPORTED_FORMAT' | 'UNREADABLE';

export interface InternalMetadataField {
  name: string;
  /** `null`: the file records no value for this field. */
  value: string | null;
}

export type DocumentInternalMetadata =
  | { state: 'NOT_READ'; reason: InternalMetadataUnreadReason }
  | { state: 'READ'; format: InternalMetadataFormat; fields: InternalMetadataField[] };
