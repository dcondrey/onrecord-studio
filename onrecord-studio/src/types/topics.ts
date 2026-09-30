/** Hand-mirrored from `src-tauri/src/analysis/topic_packs.rs`. */
import type { Finding } from './findings';

/** Derived from the finding kind on the Rust side; COUNTED is the only legal-claim basis. */
export type TopicFindingBasis = 'COUNTED' | 'STATED' | 'PATTERN';

/** How much of a document the cue match read. */
export type TopicTextCoverage = 'FULL' | 'PREFIX' | 'FILE_NAME_ONLY';

export interface TopicDocumentMatch {
  portalKey: string;
  documentId: number;
  requestId: number;
  fileName: string | null;
  matchedCues: string[];
  textCoverage: TopicTextCoverage;
}

export interface TopicRequestMatch {
  portalKey: string;
  requestId: number;
  title: string | null;
  matchedCues: string[];
}

export interface TopicFindingMatch {
  basis: TopicFindingBasis;
  anchorRequestId: number;
  finding: Finding;
}

export interface TopicPackView {
  id: string;
  title: string;
  cues: string[];
  documentTotal: number;
  documents: TopicDocumentMatch[];
  requestTotal: number;
  requests: TopicRequestMatch[];
  findingTotal: number;
  findings: TopicFindingMatch[];
}

export interface TopicPacksReport {
  topics: TopicPackView[];
  documentsScanned: number;
  documentsPrefixOnly: number;
  documentsFileNameOnly: number;
  documentLimitReached: boolean;
  requestsScanned: number;
  requestLimitReached: boolean;
  textPrefixCharacters: number;
}
