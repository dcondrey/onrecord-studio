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
