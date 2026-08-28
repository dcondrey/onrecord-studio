export interface C2pAssertion {
  assertion_type: 'c2pa.actions' | 'c2pa.metadata' | string;
  actor_identifier: string;
  is_tamper_evident: boolean;
  validation_errors: string[];
}

export interface ProvenanceClaimNode {
  manifestLabel: string;
  isActive: boolean;
  title: string | null;
  format: string | null;
  instanceId: string | null;
  claimGenerator: string | null;
  assertionLabels: string[];
}

export interface ProvenanceIngredientEdge {
  sourceManifestLabel: string;
  targetManifestLabel: string | null;
  relationship: string;
  title: string | null;
  format: string | null;
  documentId: string | null;
  instanceId: string | null;
  validationStatuses: string[];
}

export interface ProvenanceClaimGraph {
  activeManifestLabel: string | null;
  claims: ProvenanceClaimNode[];
  ingredients: ProvenanceIngredientEdge[];
  validationStatuses: string[];
  certificateChainTrusted: boolean;
  certificateChainDiagnostics: string[];
  truncated: boolean;
}

export interface ProvenanceManifest {
  documentId: number;
  fileHashSha256: string;
  isCryptographicallySigned: boolean;
  signatureIssuer: string | null;
  signingTimestamp: string | null;
  assertions: C2pAssertion[];
}
