import type { FindingCitation } from './findings';
import type { ClaimCheckView } from './investigation';

/** `DATE_NOT_RECORDED` is not a date: the meeting's documents carry no unambiguous dated heading. */
export type MeetingTiming = 'UPCOMING' | 'PAST' | 'DATE_NOT_RECORDED';
export type MeetingDocumentRole = 'AGENDA' | 'MINUTES' | 'VIDEO';
export type SuggestionBasis = 'COUNTED' | 'STATED' | 'PATTERN';
export type SuggestionReasonKind =
  | 'INVESTIGATION_TOPIC'
  | 'WATCHED_AGENCY'
  | 'WATCHED_ENTITY'
  | 'FINDING_SUBJECT'
  | 'EVIDENCE_GAP'
  | 'CHECKABLE_FIGURE';

export interface MeetingDocument {
  role: MeetingDocumentRole;
  portalKey: string;
  documentId: number;
  label: string;
  sourceUrl: string | null;
}

export interface SuggestionReason {
  kind: SuggestionReasonKind;
  basis: SuggestionBasis;
  reason: string;
  matched: string;
  citation: FindingCitation;
  findingId: string | null;
  gapKey: string | null;
  claimCheck: ClaimCheckView | null;
}

export interface PublicCommentDraft {
  text: string;
  bases: SuggestionBasis[];
  /** `null` means no collected document records a comment deadline. */
  deadline: FindingCitation | null;
  instructions: FindingCitation[];
}

export interface MeetingSuggestion {
  id: string;
  agendaItem: string;
  title: string;
  itemCitation: FindingCitation;
  reasons: SuggestionReason[];
  comment: PublicCommentDraft;
}

export interface CalendarMeeting {
  id: string;
  jurisdiction: string;
  body: string;
  bodyCitation: FindingCitation;
  date: string | null;
  dateCitation: FindingCitation | null;
  dateNote: string | null;
  timing: MeetingTiming;
  documents: MeetingDocument[];
  commentDeadlines: FindingCitation[];
  commentInstructions: FindingCitation[];
  suggestions: MeetingSuggestion[];
}

export interface MeetingCalendar {
  today: string;
  meetings: CalendarMeeting[];
  limitations: string[];
}
