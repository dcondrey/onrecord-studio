import type { TimelineAnomalyMarker } from './audit';
import type { StatutoryExtensionReason } from './investigation';

/** Mirrors src-tauri/src/types/tracker.rs. The tracker is a local notebook, never a detector input. */

export const TRACKER_LIMITS = {
  claimedRequests: 200,
  eventsPerRequest: 500,
  referenceCharacters: 120,
  titleCharacters: 200,
  jurisdictionCharacters: 80,
  sourceUrlCharacters: 2048,
  importPaths: 20,
} as const;

/** The one jurisdiction value the app has a deadline model for. */
export const CALIFORNIA_CPRA_JURISDICTION = 'CA_CPRA';
export const FEDERAL_FOIA_JURISDICTION = 'US_FOIA';

export type TimelineEventKind =
  | 'SUBMITTED'
  | 'ACKNOWLEDGED'
  | 'EXTENSION_NOTICE'
  | 'DETERMINATION'
  | 'RECORDS_RELEASED'
  | 'CLOSED'
  | 'CLARIFICATION_REQUESTED'
  | 'FEE_NOTICE'
  | 'PARTIAL_PRODUCTION'
  | 'WITHHOLDING_NOTICE'
  | 'FOLLOW_UP_SENT'
  | 'APPEAL_FILED'
  | 'OTHER';

/** A suggestion is not a basis: it lives in `ClaimedRequest.suggestions` and counts toward nothing. */
export type EventBasis = 'USER_ENTERED' | 'EMAIL_CONFIRMED';

export interface EmailEvidence {
  messageIdHash: string;
  subjectExcerpt: string | null;
  /** The email's Date header. Not necessarily the date of the event the sentence describes. */
  emailDate: string | null;
  excerpt: string;
  /** Byte offsets into `excerpt` (UTF-8), not string indices. */
  sentenceStart: number;
  sentenceEnd: number;
  matchedSentence: string;
}

export interface TimelineEvent {
  id: string;
  kind: TimelineEventKind;
  occurredAt: string;
  basis: EventBasis;
  evidence: EmailEvidence | null;
  extensionReason: StatutoryExtensionReason | null;
  expectedDeterminationAt: string | null;
  createdAt: string;
}

export interface SuggestedEvent {
  id: string;
  kind: TimelineEventKind;
  candidateOccurredAt: string | null;
  evidence: EmailEvidence;
  suggestedAt: string;
}

export interface ImportedEmail {
  messageIdHash: string;
  subjectExcerpt: string | null;
  emailDate: string | null;
  importedAt: string;
  suggestionCount: number;
}

export interface ClaimedRequest {
  id: string;
  portalKey: string | null;
  reference: string;
  title: string | null;
  sourceUrl: string | null;
  jurisdiction: string | null;
  claimedAt: string;
  events: TimelineEvent[];
  suggestions: SuggestedEvent[];
  importedEmails: ImportedEmail[];
}

export interface ClaimRequestInput {
  portalKey: string | null;
  reference: string;
  title: string | null;
  sourceUrl: string | null;
  jurisdiction: string | null;
}

export interface UpdateClaimedRequestInput extends ClaimRequestInput {
  claimId: string;
}

export interface AddTimelineEventInput {
  claimId: string;
  kind: TimelineEventKind;
  occurredAt: string;
  extensionReason: StatutoryExtensionReason | null;
  expectedDeterminationAt: string | null;
}

export interface ConfirmSuggestedEventInput {
  claimId: string;
  suggestionId: string;
  kind: TimelineEventKind;
  occurredAt: string;
  extensionReason: StatutoryExtensionReason | null;
  expectedDeterminationAt: string | null;
}

export interface RejectSuggestedEventInput {
  claimId: string;
  suggestionId: string;
}

export interface RemoveTimelineEventInput {
  claimId: string;
  eventId: string;
}

export interface ImportRequestEmailsInput {
  claimId: string;
  paths: string[];
}

export type EmailImportOutcome = 'IMPORTED' | 'ALREADY_IMPORTED' | 'NO_MESSAGE_ID' | 'REJECTED';

export interface EmailImportFileResult {
  fileName: string;
  outcome: EmailImportOutcome;
  suggestionsAdded: number;
  detail: string | null;
}

export interface EmailImportReport {
  claim: ClaimedRequest;
  files: EmailImportFileResult[];
}

export type DeadlineModel = 'CALIFORNIA_CPRA' | 'FEDERAL_FOIA' | 'NO_MODEL';

export type CountdownKind =
  | 'DAYS_REMAINING'
  | 'DEADLINE_PASSED'
  | 'DETERMINATION_RECORDED'
  | 'NO_SUBMISSION_DATE'
  | 'NO_DEADLINE_MODEL'
  | 'UNAVAILABLE';

export interface Countdown {
  kind: CountdownKind;
  days: number | null;
}

export type DeadlineDeterminationBasis =
  | 'SOURCED_DETERMINATION_EVENT'
  | 'EARLIEST_RECORDED_ACTIVITY_PROXY'
  | 'OBSERVATION_WITHOUT_RECORDED_ACTIVITY';

/** The statutory evaluator's own object. Its `isViolation` is not a finding: read `violationAsserted`. */
export interface DeadlineAnalysis {
  requestId: number;
  ruleId: string;
  jurisdiction: 'CA_CPRA' | 'US_FOIA';
  statuteCitation: string;
  createdAt: string;
  clockStartedOn: string;
  determinationAt: string | null;
  determinationSource: string | null;
  determinationBasis: DeadlineDeterminationBasis;
  observedAt: string;
  initialDeadlineAt: string;
  effectiveDeadlineAt: string;
  calendarDaysElapsed: number;
  allowedCalendarDays: number;
  extensionStatus: string;
  extensionReason: string | null;
  isViolation: boolean;
  anomalyMarkers: TimelineAnomalyMarker[];
  evidenceSummary: string;
}

export interface RequestDeadline {
  claimId: string;
  model: DeadlineModel;
  analysis: DeadlineAnalysis | null;
  countdown: Countdown;
  determinationBasis: DeadlineDeterminationBasis | null;
  violationAsserted: boolean;
  basisStatement: string;
  notes: string[];
  pendingSuggestionCount: number;
}
