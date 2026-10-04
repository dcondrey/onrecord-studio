/** Mirrors `startup::StartupPhase` in `src-tauri/src/startup.rs`. */
export type StartupPhase = 'preparing' | 'optimizingStorage' | 'openingRecords' | 'ready' | 'failed';

export type StartupNoticeKind = 'storageRewritten' | 'storageNotRewritten' | 'legacyDatabaseMigrated';

export interface StartupNotice {
  id: string;
  kind: StartupNoticeKind;
  file: string;
  message: string;
  recordedAt: string;
}

export interface StartupStatus {
  /** Monotonic; a reply carrying a lower version than the one held is stale. */
  version: number;
  phase: StartupPhase;
  file: string | null;
  phaseStartedAtMs: number;
  error: string | null;
  notices: StartupNotice[];
}
