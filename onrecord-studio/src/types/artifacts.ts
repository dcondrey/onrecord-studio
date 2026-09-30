/**
 * Types for drafted follow-up requests and appeals generated from Findings.
 *
 * Hand-mirrored from `src-tauri/src/analysis/artifacts/mod.rs`. Enums are structural string unions
 * so they round-trip through the ipc contract check.
 */

export type ArtifactKind = 'FOLLOWUP_REQUEST' | 'APPEAL';

export interface CitedAuthority {
  statute: string;
  quote: string | null;
}

export interface DraftedArtifact {
  id: string;
  kind: ArtifactKind;
  subjectFindingId: string;
  jurisdiction: string;
  templateId: string;
  generatedText: string;
  citedAuthorities: CitedAuthority[];
  generatedAt: string;
}
