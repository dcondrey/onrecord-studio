export type AnnotationKind = 'NOTE' | 'HIGHLIGHT' | 'ISSUE';
export type AnnotationStatus = 'OPEN' | 'CONFIRMED' | 'DISPUTED';
export type AnchorState = 'ANCHORED' | 'RE_ANCHORED' | 'ORPHANED';

export interface DocumentAnnotation {
  id: string;
  portalKey: string;
  documentId: number;
  kind: AnnotationKind;
  /** Position in the document's stored text when marked; null for a mark with no passage. */
  characterOffset: number | null;
  quote: string;
  prefix: string;
  suffix: string;
  /** Digest of the stored text when marked; null when it was not recorded. */
  textSha256: string | null;
  /** No surrounding text was recorded, so the anchor is the quote and offset alone. */
  contextUnanchored: boolean;
  note: string;
  /** The issue code, or free text when it is not in the codebook. */
  tag: string | null;
  /** Null for a mark made before authors were recorded. */
  author: string | null;
  status: AnnotationStatus;
  findingId: string | null;
  /** The signing key of the packet this mark arrived in; null for a mark made here. */
  importedFromKey: string | null;
  createdAt: string;
}

export interface ResolvedAnchor {
  state: AnchorState;
  sourceOffset: number | null;
  sourceLength: number | null;
  reason: string | null;
}

/** Null anchor: the mark has no passage, or the document's text could not be read to check it. */
export interface ResolvedAnnotation extends DocumentAnnotation {
  anchor: ResolvedAnchor | null;
}

export interface DocumentAnnotationList {
  textAvailable: boolean;
  currentTextSha256: string | null;
  annotations: ResolvedAnnotation[];
}

export interface AddAnnotationInput {
  portalKey: string;
  documentId: number;
  kind: AnnotationKind;
  quote: string;
  prefix: string;
  suffix: string;
  note: string;
  tag: string | null;
  findingId: string | null;
}

export interface CodebookEntry {
  code: string;
  label: string;
  definition: string;
}

export interface AnnotationSettings {
  reviewer: string | null;
  codebook: CodebookEntry[];
}

export interface CodebookConflict {
  code: string;
  local: CodebookEntry;
  incoming: CodebookEntry;
}

export interface CodebookImportPreview {
  added: CodebookEntry[];
  identical: number;
  conflicts: CodebookConflict[];
}

export interface AnnotationSearchQuery {
  code: string | null;
  author: string | null;
  status: AnnotationStatus | null;
  text: string | null;
  orphanedOnly: boolean;
  cursor: string | null;
  limit: number | null;
}

export interface AnnotationSearchRow extends ResolvedAnnotation {
  requestId: number | null;
  fileName: string | null;
}

export interface AnnotationSearchPage {
  rows: AnnotationSearchRow[];
  nextCursor: string | null;
}

export interface AnnotationImportConflict {
  local: DocumentAnnotation;
  incoming: DocumentAnnotation;
}

export interface AnnotationImportPreview {
  signerKeyId: string;
  packetSha256: string;
  exportedAt: string;
  appVersion: string;
  exporter: string | null;
  portalKey: string;
  documentId: number;
  packetTextSha256: string | null;
  localTextSha256: string | null;
  toAdd: DocumentAnnotation[];
  alreadyPresent: number;
  conflicts: AnnotationImportConflict[];
}

export interface AnnotationImportResult {
  added: number;
  alreadyPresent: number;
  conflicts: number;
}

export const ANNOTATION_LIMITS = {
  noteCharacters: 4_000,
  quoteCharacters: 2_000,
  tagCharacters: 60,
  authorCharacters: 80,
  codeLabelCharacters: 120,
  codeDefinitionCharacters: 2_000,
  contextCharacters: 32,
  searchPage: 50,
} as const;
