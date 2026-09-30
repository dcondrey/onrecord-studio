/**
 * Types for the on-demand redaction-inference IPC command.
 *
 * Hand-mirrored from `src-tauri/src/analysis/redaction_inference.rs`. This is the interactive
 * companion to the autonomous `RedactionCandidateInferred` finding: a specific redaction span the
 * user is looking at, with candidate identities ranked by likelihood and traceable evidence.
 *
 * **Ceilings.** No candidate ever exceeds the semantic ceiling (9500 bp) even when the fastembed
 * model corroborates it, and lexical fallback is bounded lower (6000 bp). Inference is not
 * decryption; the UI must never render a candidate as a decoded value.
 */

import type { FindingConfidence } from './findings';

export type CandidateKind = 'PERSON' | 'ORGANIZATION' | 'AMOUNT' | 'DATE' | 'OTHER';

export interface Evidence {
  portalKey: string;
  documentId: number;
  quote: string;
  matchScore: number;
}

export interface Candidate {
  kind: CandidateKind;
  value: string;
  likelihoodBasisPoints: number;
  supportingEvidence: Evidence[];
}

export interface DocumentRef {
  portalKey: string;
  documentId: number;
}

export interface InferenceReport {
  candidates: Candidate[];
  corroboratingDocs: DocumentRef[];
  confidenceCeiling: FindingConfidence;
  semanticRankingUsed: boolean;
  contextSnippet: string;
}
