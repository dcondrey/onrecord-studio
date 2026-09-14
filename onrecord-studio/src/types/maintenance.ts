export interface DataMaintenancePolicy {
  enabled: boolean;
  autoImport: boolean;
  autoRefresh: boolean;
  refreshHours: number;
  retentionMode: 'ORIGINALS' | 'REFERENCE_ONLY';
  retainedFileBudgetBytes: number;
}
export interface DataMaintenanceEvent { action: string; detail: string; recordedAt: string }
export interface DataMaintenanceStatus {
  policy: DataMaintenancePolicy;
  effectiveAt: string;
  retainedFileBytes: number;
  referencedDocuments: number;
  reclaimedFileBytes: number;
  events: DataMaintenanceEvent[];
}
