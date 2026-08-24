//! Definitions for cryptographic asset provenance and manifest tracking schemas.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvenanceManifest {
    pub document_id: u64,
    pub file_hash_sha256: String,
    pub is_cryptographically_signed: bool,
    pub signature_issuer: Option<String>,
    pub signing_timestamp: Option<String>,
    pub assertions: Vec<C2pAssertion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct C2pAssertion {
    pub assertion_type: String, // e.g., "c2pa.actions", "c2pa.metadata"
    pub actor_identifier: String,
    pub is_tamper_evident: bool,
    pub validation_errors: Vec<String>,
}
