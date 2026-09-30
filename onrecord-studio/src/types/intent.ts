/**
 * Types for the deterministic investigative intent router.
 *
 * Hand-mirrored from `src-tauri/src/types/intent.rs`.
 */

export type CrossCorpusSignal =
  | 'SHARED_DENIAL_LANGUAGE'
  | 'REUSED_EXTENSION_GROUNDS'
  | 'BEHAVIOR_SHIFT'
  | 'ORPHAN_TOPIC';

export type QueryPlan =
  | { kind: 'CONTRADICTIONS_IN_AGENCY'; agency: string | null }
  | { kind: 'WHY_REQUEST_MISSED_DEADLINE'; requestId: number; portalKey: string | null }
  | { kind: 'SIGNATORIES_FOR'; subject: string }
  | { kind: 'APPEARANCE_ACROSS_AGENCIES'; subject: string }
  | { kind: 'CROSS_CORPUS_SIGNAL'; signal: CrossCorpusSignal }
  | { kind: 'CONNECTION_BETWEEN'; from: string; to: string }
  | { kind: 'FINDINGS_FOR_AGENCY'; agency: string | null };

/** What a row's claim rests on. A co-occurrence is proximity in text, never a relationship. */
export type EvidenceBasis =
  | 'STORED_FINDING'
  | 'TYPED_RELATIONSHIP'
  | 'CO_OCCURRENCE_ONLY'
  | 'FULL_TEXT_TERM_MATCH'
  | 'STATUTORY_SOURCED_DETERMINATION'
  | 'STATUTORY_ACTIVITY_PROXY'
  | 'STATUTORY_PROVISIONAL_OBSERVATION';

export type IntentStatus =
  | 'ANSWERED'
  | 'NO_MATCH'
  | 'NOT_UNDERSTOOD'
  | 'NEEDS_SUBJECT'
  | 'AMBIGUOUS'
  | 'UNAVAILABLE';

export interface IntentCitation {
  portalKey: string;
  documentId: number | null;
  characterStart: number | null;
  characterEnd: number | null;
  quote: string | null;
  label: string | null;
}

export interface IntentRow {
  id: string;
  headline: string;
  detail: string | null;
  basis: EvidenceBasis;
  basisStatement: string;
  relationshipKind: string | null;
  /** `null` means no confidence was assessed; it is never zero. */
  confidenceBasisPoints: number | null;
  agencyName: string | null;
  portalKeys: string[];
  citations: IntentCitation[];
  omittedCitationCount: number;
  /** Present exactly when `citations` is empty. */
  citationsUnavailableReason: string | null;
}

export interface IntentOption {
  label: string;
  description: string;
  example: string;
}

export interface IntentAnswer {
  utterance: string;
  status: IntentStatus;
  plan: QueryPlan | null;
  summary: string;
  rows: IntentRow[];
  truncated: boolean;
  rowLimit: number;
  notes: string[];
  options: IntentOption[];
}
