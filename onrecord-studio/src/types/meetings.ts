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

/** Why a document shows no posting time. Each is a different absence, never a time. */
export type PostingAbsence =
  | 'NO_FETCH_RECORD'
  | 'NO_HEADER'
  | 'UNREADABLE_HEADER'
  | 'REQUEST_TIME_HOST'
  | 'HEADER_IS_REQUEST_TIME'
  | 'REQUEST_TIME_NOT_RULED_OUT';

/** The server's `Last-Modified` statement about a file (RFC 3339, UTC), or why there is none. */
export type PostingTime =
  | { state: 'RECORDED'; lastModified: string }
  | { state: 'NOT_RECORDED'; reason: PostingAbsence };

export interface MeetingDocument {
  role: MeetingDocumentRole;
  portalKey: string;
  documentId: number;
  label: string;
  sourceUrl: string | null;
  posting: PostingTime;
  /** Whole minutes from `Last-Modified` to the meeting start; negative when the header is later. `null` unless both are recorded. */
  minutesBeforeMeeting: number | null;
  /** Fetches of this address that returned bytes different from the copy held at the time. */
  differingFetches: number;
}

export type AgendaItemChangeKind = 'ADDED' | 'REMOVED' | 'REWORDED' | 'TEXT_CHANGED';

export interface AgendaItemChange {
  kind: AgendaItemChangeKind;
  item: string;
  before: string | null;
  after: string | null;
}

export type RevisionOrderBasis = 'LAST_MODIFIED' | 'COLLECTION_ORDER';

/** Differences between two consecutive copies of one meeting's agenda. */
export interface AgendaRevision {
  earlierDocumentId: number;
  earlierLabel: string;
  laterDocumentId: number;
  laterLabel: string;
  orderBasis: RevisionOrderBasis;
  changes: AgendaItemChange[];
  /** Items in both copies whose wording could not be compared. */
  titlesNotCompared: string[];
}

/** Counts over agenda files whose posting time and meeting start are both recorded. */
export interface PostingCounts {
  agendaFilesWithBothTimes: number;
  under72HoursBeforeStart: number;
  afterMeetingStart: number;
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
  /** Start time the agenda heading states beside its date (`HH:MM`, local to the publisher), or `null`. */
  startTime: string | null;
  documents: MeetingDocument[];
  revisions: AgendaRevision[];
  commentDeadlines: FindingCitation[];
  commentInstructions: FindingCitation[];
  suggestions: MeetingSuggestion[];
}

export interface MeetingCalendar {
  today: string;
  meetings: CalendarMeeting[];
  postingCounts: PostingCounts;
  limitations: string[];
}
