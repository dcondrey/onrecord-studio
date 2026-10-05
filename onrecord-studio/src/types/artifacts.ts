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
  | 'COUNTED_STATUTORY_DEADLINE'
  | 'DEADLINE_PASSED_WITHOUT_RECORDED_RESPONSE';

/** An agency is a portal plus an id; the same id on another portal is another agency. */
export interface AgencyIdentity {
  portalKey: string;
  agencyId: number;
}

export interface AgencyRecordOption extends AgencyIdentity {
  agencyName: string | null;
  closedRequests: number;
}

/** Counts over one agency's closed requests in the library. Mirrors `artifacts/agency_record.rs`. */
export interface AgencyRecord extends AgencyIdentity {
  agencyName: string | null;
  closedRequests: number;
  sourcedDeterminations: number;
  lateSourcedDeterminations: number;
  noRecordsClosures: number;
  noRecordsWithRecordedEarlierRelease: number;
  /** Earliest and latest received date among the closed requests; `null` when there are none. */
  period: [string, string] | null;
}

export interface OverdueFollowUpDraft {
  artifact: DraftedArtifact;
  /** `null` when no agency was chosen. */
  agencyRecord: AgencyRecord | null;
  /** False with a record present: the library holds too few closed requests for the paragraph. */
  agencyParagraphIncluded: boolean;
  agencyOptions: AgencyRecordOption[];
}

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
