export interface ReconstructionCitation {
  id: string;
  documentId: number;
  portalKey: string;
  title: string;
  sourceUrl: string | null;
  sourceSha256: string;
  extractionSha256: string;
  characterOffset: number;
  quote: string;
}

export interface ReconstructionCandidate {
  value: string;
  evidence: ReconstructionCitation[];
  supports: string[];
  limitations: string[];
  contradicted: boolean;
}

export interface ReconstructionFinding {
  id: string;
  method: string;
  status: string;
  subject: string;
  observation: ReconstructionCitation;
  candidates: ReconstructionCandidate[];
  explanation: string;
  nextStep: string;
  requestDraft: string;
}

export interface ReconstructionReport {
  analysisVersion: string;
  documentId: number;
  portalKey: string;
  sourceSha256: string;
  corpusFingerprint: string;
  documentsCompared: number;
  passagesCompared: number;
  findings: ReconstructionFinding[];
  limitations: string[];
  searchTerms: string[];
}

export interface ReconstructionOptions {
  contextQuote?: string | null;
  fixedWidthMasks?: boolean;
}
