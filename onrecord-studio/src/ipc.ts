import { invoke } from '@tauri-apps/api/core';
import type {
  AgencyPerformanceProfile,
  AnalysisConfig,
  AuxiliaryEventInput,
  AuxiliaryImportResult,
  AuxiliaryStreamKind,
  CrossAgencyCorrelation,
  CorpusIndexStatus,
  CorpusPattern,
  CorpusPatternQuery,
  DocumentProvenanceVerification,
  DocumentTextChunk,
  EvidenceBundle,
  LegalAuthorityImport,
  LegalAuthorityImportResult,
  LegalBriefExportResult,
  LegalAuthoritySummary,
  LegalComplianceFinding,
  LegalEvaluationResult,
  LegalFindingReviewStatus,
  OcrCapability,
  PiiMaskResult,
  PortalRuntimeSnapshot,
  PortalSyncStart,
  ProvenanceVerificationRecord,
  RateLimitPolicy,
  RecoveredVersionProfile,
  RedactionIntegrityReport,
  RequestDocumentSummary,
  RequestSearchResult,
  StatutoryExtensionRecord,
  CrawlPolicy,
  SourceDiscoveryPreview,
  SourceGuidance,
  SavedSource,
  SourceIngestionStart,
  SourceRemoval,
  PastedContentInput,
} from './types';

// TODO(ipc): Generate this command/response map from Rust schemas so serialized field drift is
// impossible; the current checker validates command coverage but not every nested response type.

type NoArgs = Record<string, never>;

export interface NativeCommandMap {
  get_ocr_capability: { args: NoArgs; result: OcrCapability };
  start_portal_sync: {
    args: {
      portalUrl: string;
      startPage?: number | null;
      perPage?: number | null;
      rateLimitPolicy?: RateLimitPolicy | null;
    };
    result: PortalSyncStart;
  };
  pause_source_sync: { args: { sourceKey: string }; result: boolean };
  resume_source_sync: { args: { sourceKey: string }; result: SourceIngestionStart };
  remove_source: { args: { sourceKey: string }; result: SourceRemoval };
  list_portal_runtimes: { args: NoArgs; result: PortalRuntimeSnapshot[] };
  list_saved_portals: { args: NoArgs; result: SavedSource[] };
  preview_data_source: {
    args: { url: string };
    result: SourceDiscoveryPreview;
  };
  start_data_source: {
    args: {
      url: string;
      crawlPolicy?: CrawlPolicy | null;
      guidance?: SourceGuidance | null;
      rateLimitPolicy?: RateLimitPolicy | null;
    };
    result: SourceIngestionStart;
  };
  ingest_local_files: {
    args: { paths: string[] };
    result: SourceIngestionStart;
  };
  ingest_pasted_content: {
    args: { input: PastedContentInput };
    result: SourceIngestionStart;
  };
  query_corpus_patterns: {
    args: { query?: CorpusPatternQuery | null };
    result: CorpusPattern[];
  };
  get_corpus_index_status: { args: NoArgs; result: CorpusIndexStatus };
  query_audit_records: {
    args: {
      searchTerm: string;
      portalKey?: string | null;
      agencyId?: number | null;
      status?: string | null;
      policyDeviation?: string | null;
      limit?: number | null;
    };
    result: RequestSearchResult[];
  };
  export_legal_brief: {
    args: {
      requestId: number;
      portalKey: string;
      targetPath?: string | null;
      authorName?: string | null;
    };
    result: LegalBriefExportResult;
  };
  get_agency_performance_profiles: {
    args: { config?: AnalysisConfig | null };
    result: AgencyPerformanceProfile[];
  };
  get_cross_agency_correlations: {
    args: {
      portalKey?: string | null;
      minimumComplaints?: number | null;
      resultLimit?: number | null;
      windowHours?: number | null;
    };
    result: CrossAgencyCorrelation[];
  };
  import_auxiliary_events: {
    args: {
      portalKey?: string | null;
      sourceKind: AuxiliaryStreamKind;
      displayName: string;
      sourceUri?: string | null;
      events: AuxiliaryEventInput[];
    };
    result: AuxiliaryImportResult;
  };
  register_statutory_extension: {
    args: {
      requestId: number;
      portalKey: string;
      extension: StatutoryExtensionRecord;
    };
    result: string[];
  };
  import_legal_authority: {
    args: { input: LegalAuthorityImport };
    result: LegalAuthorityImportResult;
  };
  list_legal_authorities: { args: NoArgs; result: LegalAuthoritySummary[] };
  evaluate_request_legal_compliance: {
    args: { requestId: number; portalKey: string };
    result: LegalEvaluationResult;
  };
  list_request_legal_findings: {
    args: { requestId: number; portalKey: string };
    result: LegalComplianceFinding[];
  };
  review_legal_finding: {
    args: {
      violationId: string;
      requestId: number;
      portalKey: string;
      reviewStatus: LegalFindingReviewStatus;
      reviewerNotes?: string | null;
    };
    result: LegalComplianceFinding[];
  };
  mask_sensitive_text: { args: { text: string }; result: PiiMaskResult };
  verify_document_provenance: {
    args: { documentId: number; portalKey: string };
    result: DocumentProvenanceVerification;
  };
  list_document_provenance_verifications: {
    args: { documentId: number; portalKey: string; limit?: number | null };
    result: ProvenanceVerificationRecord[];
  };
  list_request_documents: {
    args: { requestId: number; portalKey: string };
    result: RequestDocumentSummary[];
  };
  get_document_text_chunk: {
    args: {
      documentId: number;
      portalKey: string;
      characterOffset?: number | null;
      characterLimit?: number | null;
    };
    result: DocumentTextChunk;
  };
  verify_document_redaction_integrity: {
    args: { documentId: number; portalKey: string };
    result: RedactionIntegrityReport;
  };
  carve_document_version_history: {
    args: { documentId: number; portalKey: string };
    result: RecoveredVersionProfile;
  };
  audit_document_residual_evidence: {
    args: { documentId: number; portalKey: string; includeHypotheses?: boolean | null };
    result: EvidenceBundle;
  };
}

export type NativeCommandName = keyof NativeCommandMap;

export class NativeCommandError extends Error {
  readonly command: NativeCommandName;
  readonly details: unknown;

  constructor(command: NativeCommandName, details: unknown) {
    super(formatNativeError(details));
    this.name = 'NativeCommandError';
    this.command = command;
    this.details = details;
  }
}

export async function invokeCommand<Command extends NativeCommandName>(
  command: Command,
  args: NativeCommandMap[Command]['args'],
): Promise<NativeCommandMap[Command]['result']> {
  try {
    return await invoke<NativeCommandMap[Command]['result']>(command, args);
  } catch (error: unknown) {
    throw new NativeCommandError(command, error);
  }
}

export function formatNativeError(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === 'string') return error;
  if (isRecord(error)) {
    const errorType = typeof error.errorType === 'string' ? error.errorType : null;
    const message = formatErrorMessage(error.message);
    if (errorType && message) return `${errorType}: ${message}`;
    if (message) return message;
  }
  try {
    const serialized = JSON.stringify(error);
    if (serialized && serialized !== '{}') return serialized;
  } catch {
    // Fall through to a stable message for non-serializable rejection values.
  }
  return 'The native command failed without a usable error message.';
}

function formatErrorMessage(message: unknown): string | null {
  if (typeof message === 'string') return message;
  if (message === null || message === undefined) return null;
  try {
    return JSON.stringify(message);
  } catch {
    return String(message);
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}
