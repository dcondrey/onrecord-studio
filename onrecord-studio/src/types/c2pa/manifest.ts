export interface C2pAssertion {
  assertion_type: 'c2pa.actions' | 'c2pa.metadata' | string;
  actor_identifier: string;
  is_tamper_evident: boolean;
  validation_errors: string[];
}

export interface ProvenanceManifest {
  documentId: number;
  fileHashSha256: string;
  isCryptographicallySigned: boolean;
  signatureIssuer: string | null;
  signingTimestamp: string | null;
  assertions: C2pAssertion[];
}
