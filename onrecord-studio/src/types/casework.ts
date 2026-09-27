import type { ResearchEvidence } from "./research";
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
export interface CaseworkImportPreview {
  mode: string; localRevision: number | null; packetSha256: string; warnings: string[];
  case: CaseworkCase; contributor: string;
}
