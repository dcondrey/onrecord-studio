//! Root type orchestration boundary exposing modular data engine shapes.

pub mod analysis;
pub mod audit;
pub mod c2pa;
pub mod corpus;
pub mod investigation;
pub mod legal;
pub mod nextrequest;
pub mod portal;
pub mod source;

// Re-export core structures for convenient root usage
pub use analysis::{AgencyPerformanceProfile, AnalysisConfig, OutlierCluster};
pub use audit::{CaseAuditRecord, StateTransitionLog, TimelineAnomalyMarker, VerificationState};
pub use c2pa::{
    C2pAssertion, ProvenanceConfidence, ProvenanceManifest, RecordedDigestSource,
    SignatureVerificationState,
};
pub use corpus::{
    CorpusEvidenceReference, CorpusIndexStatus, CorpusPattern, CorpusPatternKind,
    CorpusPatternQuery,
};
pub use investigation::{
    CrossAgencyCorrelation, DocumentProvenanceVerification, DocumentTextChunk,
    LegalBriefExportResult, ProvenanceVerificationRecord, RequestDocumentSummary,
    RequestSearchResult, StatutoryExtensionReason, StatutoryExtensionRecord,
};
pub use legal::{
    LegalAuthorityImport, LegalAuthorityImportResult, LegalAuthorityKind, LegalAuthoritySummary,
    LegalComplianceFinding, LegalEvaluationResult, LegalFindingReviewStatus, LegalMandateInput,
    LegalMandateSeverity,
};
pub use nextrequest::{
    DocumentMetadata, EmailMetadata, EmailParticipantMetadata, NextRequestResponse, PaginationMeta,
    PublicRequest,
};
pub use portal::{
    FrontierExclusionCount, PortalApiLayout, PortalRegistration, PortalRuntimeSnapshot,
    PortalSyncStart, RateLimitPolicy, SavedSource, SourceKind, SourceLifecycleState, SourceRemoval,
    StoppedHost,
};
pub use source::{
    CrawlPolicy, DiscoveredSourceLink, PastedContentInput, SourceDiscoveryPreview, SourceGuidance,
    SourceIngestionStart,
};
