//! Root type orchestration boundary exposing modular data engine shapes.

pub mod nextrequest;
pub mod analysis;
pub mod audit;
pub mod c2pa;

// Re-export core structures for convenient root usage
pub use nextrequest::{NextRequestResponse, PublicRequest, DocumentMetadata, PaginationMeta};
pub use analysis::{AnalysisConfig, AgencyPerformanceProfile, OutlierCluster};
pub use audit::{CaseAuditRecord, VerificationState, StateTransitionLog};
pub use c2pa::{ProvenanceManifest, C2pAssertion};
