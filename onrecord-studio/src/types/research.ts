export interface ResearchPolicy {
  enabled: boolean;
  autoCollect: boolean;
  intervalHours: number;
  provider: "openai" | "openrouter" | "ollama";
  model: string;
  dailyCallLimit: number;
  researchPasses: number;
  discoverSources: boolean;
}
export interface ResearchEvidence {
  id: string;
  label: string;
  sourceUrl: string | null;
  portalKey: string | null;
  requestId: number | null;
  documentId: number | null;
  characterOffset: number | null;
  sourceSha256: string;
  text: string;
  sql: string | null;
  snapshotProvenance: string[];
  sourceReferences: { url: string; snapshot: string; sha256: string }[];
  limitations: string;
}
export interface ResearchCitation {
  evidenceId: string;
  quote: string;
}
export interface ResearchFinding {
  title: string;
  interpretation: string;
  whyItMatters: string;
  citations: ResearchCitation[];
  counterEvidence: ResearchCitation[];
  alternativeExplanations: string[];
  nextTest: string;
}
export interface ResearchQuestion {
  question: string;
  why: string;
  evidenceIds: string[];
}
export interface ResearchRequest {
  title: string;
  custodian: string;
  gap: string;
  resolves: string;
  draft: string;
  evidenceIds: string[];
}
export interface ResearchMeeting {
  body: string;
  date: string;
  agendaItem: string;
  relevance: string;
  action: string;
  citations: ResearchCitation[];
}
export interface ResearchSynthesis {
  title: string;
  summary: string;
  findings: ResearchFinding[];
  questions: ResearchQuestion[];
  requests: ResearchRequest[];
  meetings: ResearchMeeting[];
  limitations: string[];
}
export interface ResearchReport {
  id: string;
  topic: string;
  createdAt: string;
  model: string;
  evidence: ResearchEvidence[];
  synthesis: ResearchSynthesis;
  investigationLog: string[];
  inputTokens: number;
  outputTokens: number;
}
export interface ResearchSourceStatus {
  label: string;
  url: string;
  state: string;
  detail: string;
}
export interface ResearchStatus {
  policy: ResearchPolicy;
  credentialAvailable: boolean;
  phase: string;
  lastError: string | null;
  lastFinishedAt: string | null;
  callsToday: number;
  reports: ResearchReport[];
  sources: ResearchSourceStatus[];
  actionStates: Record<string, string>;
}
