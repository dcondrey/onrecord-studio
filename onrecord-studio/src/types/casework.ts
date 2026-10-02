import type { ResearchEvidence } from "./research";
import type { DeadlineDeterminationBasis } from "./tracker";
export interface CaseworkCitation {
  id: string; reportId: string; evidenceId: string; label: string; quote: string;
  sourceUrl: string | null; sourceSha256: string;
  sourceReferences: ResearchEvidence["sourceReferences"];
  portalKey: string | null; documentId: number | null; characterOffset: number | null;
  verification: string;
}
export interface CaseworkQuestion {
  id: string; question: string; hypothesis: string; nextTest: string; disposition: string;
  owner: string; citationIds: string[]; counterCitationIds: string[];
}
export interface CaseworkProduction {
  id: string; receivedAt: string; description: string; recordTypes: string[]; fields: string[];
  period: string; citationIds: string[]; notes: string;
}
export interface CaseworkRequest {
  id: string; title: string; custodian: string; gap: string; resolves: string; draft: string;
  state: string; expectedRecordTypes: string[]; expectedFields: string[]; expectedPeriod: string;
  submittedAt: string | null; responseDue: string | null; externalReference: string;
  productions: CaseworkProduction[]; unresolvedOmissions: string[]; followUpDraft: string; citationIds: string[];
  compiledDraft: string; filingPlan: RequestFilingPlan | null;
}
export type FollowUpTrigger = 'DETERMINATION_DUE' | 'DETERMINATION_OVERDUE' | 'PRODUCTION_INCOMPLETE';
export interface ScheduledFollowUp { trigger: FollowUpTrigger; scheduledFor: string; rationale: string }
/** The modeled CPRA determination clock for a filed request, as counted on `evaluatedAt`. Absent on an unfiled one: no clock. */
export interface RequestFilingPlan {
  statuteCitation: string; evaluatedAt: string; filedOn: string; determinationDueOn: string;
  calendarDaysElapsed: number; allowedCalendarDays: number;
  determinationBasis: DeadlineDeterminationBasis;
  measuredTo: string | null; deadlinePassedWithoutRecordedResponse: boolean;
  followUps: ScheduledFollowUp[];
}
export interface FilingIdentifyingSource { label: string; sourceSha256: string; verification: string }
export interface RequestFilingRecordResult {
  caseId: string; requestId: string; outputPath: string; packageSha256: string;
  signatureAlgorithm: string; signerKeyId: string; generatedAt: string;
  determinationDueOn: string; remainingOmissions: string[]; verifierScriptPath: string; verifierPagePath: string;
}
export interface CaseworkDecision {
  id: string; body: string; date: string; agendaItem: string; proposedDecision: string;
  action: string; outcome: string; vote: string; state: string;
  citationIds: string[]; outcomeCitationIds: string[];
}
export interface CaseworkCommitment {
  id: string; promise: string; responsibleParty: string; dueDate: string | null;
  completionTest: string; state: string; assessment: string;
  citationIds: string[]; completionCitationIds: string[];
}
export interface CaseworkCase {
  id: string; reportId: string; topic: string; title: string; owner: string; disposition: string;
  revision: number; createdAt: string; updatedAt: string; questions: CaseworkQuestion[];
  requests: CaseworkRequest[]; decisions: CaseworkDecision[]; commitments: CaseworkCommitment[];
  evidence: CaseworkCitation[]; notes: string;
}
export interface CaseworkEvent { caseId: string; revision: number; author: string; at: string; summary: string }
export interface CaseworkRankFactor { label: string; score: number; maximum: number; reason: string }
export interface CaseworkLead {
  reportId: string; findingIndex: number; title: string; nextAction: string; score: number;
  factors: CaseworkRankFactor[]; citations: CaseworkCitation[]; caseId: string | null;
}
export interface CaseworkState { availableEvidence: CaseworkCitation[]; cases: CaseworkCase[]; events: CaseworkEvent[]; leads: CaseworkLead[] }
/** One entry of a case's hash-chained operation log. `author` is a supplied label, not an identity. */
export interface CaseOperation {
  revision: number; sequence: number; author: string; at: string;
  kind: 'ADD' | 'CHANGE' | 'REMOVE' | 'SET'; section: string; target: string; summary: string;
  contentSha256: string | null; previousHash: string | null; hash: string;
}
export interface CaseworkImportPreview {
  mode: string; localRevision: number | null; packetSha256: string; warnings: string[];
  case: CaseworkCase; contributor: string; signerKeyId: string | null;
  /** On a conflict: incoming operations that touch nothing local work touched since the shared base, and those that contend. */
  commuting: CaseOperation[]; contested: CaseOperation[];
}
export interface WithheldCitationRequest { citationId: string; reason: string }
export interface CaseExportLedgerEntry {
  sequence: number; manifestSha256: string; previousManifestSha256: string | null;
  signerKeyId: string; reservedAt: string; outcome: string;
  outcomeDetail: string | null; outcomeRecordedAt: string | null; previousLinkValid: boolean;
}
export interface CompromisedImportWarning { revision: number; signerKeyId: string; revokedAt: string; reason: string }
