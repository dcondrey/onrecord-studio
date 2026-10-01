export type AnnotationKind = 'NOTE' | 'HIGHLIGHT' | 'ISSUE';

export interface DocumentAnnotation {
  id: string;
  portalKey: string;
  documentId: number;
  kind: AnnotationKind;
  characterOffset: number;
  quote: string;
  note: string;
  tag: string | null;
  createdAt: string;
}

export interface AddAnnotationInput {
  portalKey: string;
  documentId: number;
  kind: AnnotationKind;
  characterOffset: number;
  quote: string;
  note: string;
  tag: string | null;
}

export const ANNOTATION_LIMITS = {
  noteCharacters: 4_000,
  quoteCharacters: 2_000,
  tagCharacters: 60,
} as const;
