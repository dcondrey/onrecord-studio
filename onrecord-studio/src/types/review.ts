import type { ResearchCitation, ResearchReport } from './research';
export interface FindingReviewInput { reportId: string; findingIndex: number; expectedRevision: number; reviewer: string; assignedTo: string; verdict: 'OPEN'|'SUPPORTED'|'CONTESTED'|'REJECTED'; reasoning: string; falsificationTest: string; counterSearch: string; counterEvidence: ResearchCitation[] }
export interface FindingReview { id: string; revision: number; createdAt: string; input: FindingReviewInput }
export interface EvidenceReplay { evidenceId: string; checkedAt: string; state: string; detail: string }
export interface FindingChallenge { findingIndex: number; title: string; independentSources: number; checks: string[]; blockers: string[]; nextTest: string }
export interface PublicEntityRef { key: string; name: string; kind: string; jurisdiction: string }
export interface PublicRelationship { id: string; reportId: string; revision: number; subject: PublicEntityRef; predicate: string; object: PublicEntityRef; validFrom: string|null; validTo: string|null; citations: ResearchCitation[]; status: string; reviewer: string; note: string }
export interface PublicConnection { subject: string; via: string; object: string; relationshipIds: string[]; reportIds: string[]; explanation: string; limitations: string[] }
export interface ReviewWorkspace { report: ResearchReport; reviews: FindingReview[]; challenges: FindingChallenge[]; replays: EvidenceReplay[]; relationships: PublicRelationship[]; connections: PublicConnection[]; identityWarnings: string[] }
export interface PilotAssessment { reportId: string; assessor: string; usefulness: number; supportedDiscoveries: number; falseLeads: number; minutesSpent: number; baselineMinutes: number|null; resultUsed: string; notes: string }
export interface EvaluationCheck { name: string; passed: boolean; detail: string }
export interface InvestigationEvaluation { id: string; title: string; checkedAt: string; checks: EvaluationCheck[]; assessments: PilotAssessment[]; limitations: string[] }
