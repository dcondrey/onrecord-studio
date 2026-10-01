/** Mirrors src-tauri/src/resolution and commands/resolution.rs. Judgments are keyed by (kind, value), never by entity id. */

export type ResolutionVerdict = 'SAME' | 'DIFFERENT' | 'UNSURE';
export type ResolutionSuggestionBasis = 'STRONG_IDENTIFIER' | 'NAME_ONLY' | 'MANUAL';

export const RESOLUTION_REASON_MAX = 1000;

export interface ResolutionKey {
  kind: string;
  value: string;
}

export interface MentionExample {
  portalKey: string;
  documentId: number;
  characterStart: number;
  characterEnd: number;
}

export interface ResolutionEntitySnapshot {
  key: ResolutionKey;
  displayValue: string;
  documentCount: number;
  /** A sample of where the entity was read, not all of it; compare with `documentCount`. */
  examples: MentionExample[];
}

export interface SharedIdentifier {
  kind: string;
  displayValue: string;
  leftDocuments: number;
  rightDocuments: number;
}

export interface ResolutionEvidence {
  basis: ResolutionSuggestionBasis;
  left: ResolutionEntitySnapshot;
  right: ResolutionEntitySnapshot;
  sharedIdentifier: SharedIdentifier | null;
  nameVariants: string[];
}

export interface ResolutionCandidate {
  pairId: string;
  leftEntityId: string;
  rightEntityId: string;
  evidence: ResolutionEvidence;
}

export interface CandidateGuards {
  skippedNameGroups: number;
  skippedIdentifiers: number;
}

export interface ResolutionCandidatePage {
  candidates: ResolutionCandidate[];
  nextCursor: string | null;
  remaining: number;
  scanTruncated: boolean;
  guards: CandidateGuards;
  ledgerHead: string;
}

export interface RecordResolutionInput {
  leftEntityId: string;
  rightEntityId: string;
  verdict: ResolutionVerdict;
  author: string;
  reason: string;
  supersedes: string | null;
}

export interface LedgerEntry {
  id: string;
  left: ResolutionKey;
  right: ResolutionKey;
  verdict: ResolutionVerdict;
  author: string;
  decidedAt: string;
  reason: string;
  supersedes: string | null;
  evidence: ResolutionEvidence;
}

export interface ResolutionLedgerItem {
  entry: LedgerEntry;
  importedFrom: string | null;
  hash: string;
  isLatest: boolean;
}

export interface BasisDecisions {
  basis: ResolutionSuggestionBasis;
  same: number;
  different: number;
  unsure: number;
}

export interface DecisionSummary {
  byBasis: BasisDecisions[];
}

export interface Contradiction {
  keys: ResolutionKey[];
  sameEntryIds: string[];
  differentEntryIds: string[];
}

export interface ResolutionLedgerPage {
  items: ResolutionLedgerItem[];
  nextCursor: string | null;
  total: number;
  entryCount: number;
  head: string;
  summary: DecisionSummary;
  contradictions: Contradiction[];
}

export interface ClusterMember {
  entityId: string;
  key: ResolutionKey;
  displayValue: string;
}

export interface ClusterReason {
  entryId: string;
  leftDisplayValue: string;
  rightDisplayValue: string;
  author: string;
  decidedAt: string;
  reason: string;
}

export interface ResolutionClusterView {
  entityId: string;
  key: ResolutionKey;
  members: ClusterMember[];
  reasons: ClusterReason[];
  contradictions: Contradiction[];
  resolutionId: string;
  ledgerHead: string;
  corpusStateDigest: string;
}

export interface ImportConflict {
  entryId: string;
  reason: string;
}

export interface ResolutionImportPreview {
  signerKeyId: string;
  packetSha256: string;
  exportedAt: string;
  entryCount: number;
  additions: number;
  alreadyPresent: number;
  conflicts: ImportConflict[];
  contradictionsBefore: number;
  contradictionsAfter: number;
  warnings: string[];
}

export interface ResolutionImportResult {
  added: number;
  alreadyPresent: number;
  conflicts: ImportConflict[];
  head: string;
}
