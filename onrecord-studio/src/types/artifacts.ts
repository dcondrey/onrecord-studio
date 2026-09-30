/**
 * Types for drafted follow-up requests and appeals generated from Findings.
 *
 * Hand-mirrored from `src-tauri/src/analysis/artifacts/mod.rs`. Enums are structural string unions
 * so they round-trip through the ipc contract check.
 */

import type { FindingCitation, FindingConfidence } from './findings';

export type ArtifactKind = 'FOLLOWUP_REQUEST' | 'APPEAL';

/** What a draft's claims rest on. Only the two counted bases may accompany an appeal. */
export type DraftEvidenceBasis =
  | 'PATTERN_OBSERVATION'
  | 'COUNTED_POLICY_COMPARISON'
  | 'COUNTED_STATUTORY_DEADLINE';

export interface CitedAuthority {
  statute: string;
  quote: string | null;
}

export interface DraftAvailability {
  followup: boolean;
  appeal: boolean;
}

export interface DraftedArtifact {
  id: string;
  kind: ArtifactKind;
  /** A finding id, or `request:<portal>:<id>` for an appeal drafted from a request's dates. */
  subjectFindingId: string;
  jurisdiction: string;
  templateId: string;
  generatedText: string;
  /** The author's declared affiliation, kept apart from the editable body. `null` when none. */
  disclosure: string | null;
  citedAuthorities: CitedAuthority[];
  evidenceBasis: DraftEvidenceBasis;
  confidence: FindingConfidence;
  citations: FindingCitation[];
  /** Facts the draft needs and does not have; each is a bracketed placeholder in the text. */
  unknownFacts: string[];
  generatedAt: string;
}
