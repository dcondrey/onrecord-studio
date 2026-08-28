import { create } from 'zustand';
import { readThemePreference, type ThemePreference } from '../utils/theme';
import type {
  AgencyPerformanceProfile,
  CrossAgencyCorrelation,
  LegalAuthoritySummary,
  LegalComplianceFinding,
  PortalRegistration,
  PortalRuntimeSnapshot,
  RequestSearchResult,
  SavedSource,
  SourceLifecycleState,
} from '../types';

export interface IngestionMetrics {
  pagesProcessed: number;
  documentsDownloaded: number;
  anomaliesFound: number;
  bytesTransferred: number;
}

export type WorkspaceView = 'overview' | 'patterns' | 'evidence' | 'activity';
export type AppTheme = ThemePreference;
export type WorkspaceInitializationStatus = 'loading' | 'ready' | 'error';

export interface SourceRepairNotice {
  portalKey: string;
  sourceUrl: string;
  reason: string;
}

interface AppState {
  // Search state mirrors the four filters in the supported native query contract; qualifier text
  // is parsed at the search boundary and unsupported syntax never enters this store.
  workspaceView: WorkspaceView;
  theme: AppTheme;
  isSidebarOpen: boolean;
  isSidebarCollapsed: boolean;
  portals: SavedSource[];
  workspaceInitializationStatus: WorkspaceInitializationStatus;
  workspaceInitializationError: string | null;
  portalRuntimes: PortalRuntimeSnapshot[];
  activePortalKey: string | null;
  portalUrlDraft: string;
  syncingPortalKeys: string[];
  syncRunIdsByPortal: Record<string, string>;
  isSyncing: boolean;
  audioAlertsEnabled: boolean;
  syncLogs: string[];
  metrics: IngestionMetrics;
  agencyProfiles: AgencyPerformanceProfile[];
  crossAgencyCorrelations: CrossAgencyCorrelation[];
  analyticsUpdatedAt: string | null;
  /** Set when the last analytics refresh failed, so panels can say so instead of reporting an empty result as a finding. */
  analyticsError: string | null;
  legalAuthorities: LegalAuthoritySummary[];
  activeLegalFindings: LegalComplianceFinding[];
  legalDataUpdatedAt: string | null;
  searchTerm: string;
  selectedAgencyId: number | null;
  selectedStatus: string | null;
  selectedPolicyDeviation: string | null;
  filteredResults: RequestSearchResult[];
  /** Set when the latest evidence query failed; retained results then belong to the last successful query. */
  searchError: string | null;
  selectedRequestId: number | null;
  selectedRequestPortalKey: string | null;
  isExtractionTerminalOpen: boolean;
  pendingSourceRepair: SourceRepairNotice | null;
  setWorkspaceView: (view: WorkspaceView) => void;
  setTheme: (theme: AppTheme) => void;
  setSidebarOpen: (open: boolean) => void;
  toggleSidebarCollapsed: () => void;
  setPortalUrlDraft: (url: string) => void;
  upsertPortal: (portal: PortalRegistration) => void;
  setPortals: (portals: SavedSource[]) => void;
  setWorkspaceInitialization: (
    status: WorkspaceInitializationStatus,
    error?: string | null,
  ) => void;
  setPortalLifecycleState: (portalKey: string, lifecycleState: SourceLifecycleState) => void;
  removePortal: (portalKey: string) => void;
  setPortalRuntimes: (runtimes: PortalRuntimeSnapshot[]) => void;
  setActivePortal: (portalKey: string | null) => void;
  setPortalSyncState: (portalKey: string, syncing: boolean, syncRunId?: string) => void;
  setAudioAlertsEnabled: (enabled: boolean) => void;
  appendSyncLog: (log: string) => void;
  incrementMetric: (metricKey: 'pagesProcessed' | 'documentsDownloaded' | 'anomaliesFound') => void;
  addBytesTransferred: (bytes: number) => void;
  resetTelemetryMetrics: () => void;
  setAnalyticsData: (
    profiles: AgencyPerformanceProfile[],
    correlations: CrossAgencyCorrelation[],
  ) => void;
  setAnalyticsError: (message: string | null) => void;
  setLegalAuthorities: (authorities: LegalAuthoritySummary[]) => void;
  setActiveLegalFindings: (findings: LegalComplianceFinding[]) => void;
  setSearchFilters: (
    term: string,
    agencyId: number | null,
    status: string | null,
    policyDeviation?: string | null,
  ) => void;
  setFilteredResults: (results: RequestSearchResult[]) => void;
  setSearchError: (message: string | null) => void;
  setSelectedRequest: (requestId: number | null, portalKey?: string | null) => void;
  setTerminalVisibility: (visible: boolean) => void;
  setPendingSourceRepair: (notice: SourceRepairNotice | null) => void;
}

const emptyMetrics = (): IngestionMetrics => ({
  pagesProcessed: 0,
  documentsDownloaded: 0,
  anomaliesFound: 0,
  bytesTransferred: 0,
});

export const useAppStore = create<AppState>((set) => ({
  workspaceView: 'overview',
  theme: readThemePreference(),
  isSidebarOpen: false,
  isSidebarCollapsed: false,
  portals: [],
  workspaceInitializationStatus: 'loading',
  workspaceInitializationError: null,
  portalRuntimes: [],
  activePortalKey: null,
  portalUrlDraft: '',
  syncingPortalKeys: [],
  syncRunIdsByPortal: {},
  isSyncing: false,
  audioAlertsEnabled: readAudioAlertsPreference(),
  syncLogs: [],
  metrics: emptyMetrics(),
  agencyProfiles: [],
  crossAgencyCorrelations: [],
  analyticsUpdatedAt: null,
  analyticsError: null,
  legalAuthorities: [],
  activeLegalFindings: [],
  legalDataUpdatedAt: null,
  searchTerm: '',
  selectedAgencyId: null,
  selectedStatus: null,
  selectedPolicyDeviation: null,
  filteredResults: [],
  searchError: null,
  selectedRequestId: null,
  selectedRequestPortalKey: null,
  isExtractionTerminalOpen: false,
  pendingSourceRepair: null,
  setWorkspaceView: (workspaceView) => set({ workspaceView, isSidebarOpen: false }),
  setTheme: (theme) => set({ theme }),
  setSidebarOpen: (isSidebarOpen) => set({ isSidebarOpen }),
  toggleSidebarCollapsed: () => set((state) => ({ isSidebarCollapsed: !state.isSidebarCollapsed })),
  setPortalUrlDraft: (portalUrlDraft) => set({ portalUrlDraft }),
  // Verifying or provisioning a source is the act that makes it active, on the native side too:
  // the registration upsert resets `lifecycle_state`. So an upsert here carries ACTIVE rather
  // than preserving a stale PAUSED that the native write has already cleared.
  upsertPortal: (portal) =>
    set((state) => {
      const portals = state.portals.filter(
        (entry) => entry.registration.portalKey !== portal.portalKey,
      );
      portals.push({
        registration: portal,
        lifecycleState: 'ACTIVE',
        healthStatus: 'verified',
        lastError: null,
      });
      portals.sort((left, right) =>
        left.registration.displayName.localeCompare(right.registration.displayName),
      );
      return {
        portals,
        activePortalKey: state.activePortalKey ?? portal.portalKey,
      };
    }),
  setPortals: (portals) =>
    set((state) => ({
      portals: [...portals].sort((left, right) =>
        left.registration.displayName.localeCompare(right.registration.displayName),
      ),
      activePortalKey:
        state.activePortalKey &&
        portals.some((portal) => portal.registration.portalKey === state.activePortalKey)
          ? state.activePortalKey
          : portals[0]?.registration.portalKey ?? null,
    })),
  setWorkspaceInitialization: (
    workspaceInitializationStatus,
    workspaceInitializationError = null,
  ) => set({ workspaceInitializationStatus, workspaceInitializationError }),
  setPortalLifecycleState: (portalKey, lifecycleState) =>
    set((state) => ({
      portals: state.portals.map((entry) =>
        entry.registration.portalKey === portalKey ? { ...entry, lifecycleState } : entry,
      ),
    })),
  removePortal: (portalKey) =>
    set((state) => {
      const portals = state.portals.filter((entry) => entry.registration.portalKey !== portalKey);
      return {
        portals,
        activePortalKey:
          state.activePortalKey === portalKey
            ? portals[0]?.registration.portalKey ?? null
            : state.activePortalKey,
      };
    }),
  // A runtime snapshot is authoritative and replaces event-derived run state atomically. This is
  // the recovery path when telemetry was missed or a process restarted between start/stop events.
  setPortalRuntimes: (portalRuntimes) => set(() => {
    const running = portalRuntimes.filter((runtime) => runtime.isRunning);
    const syncingPortalKeys = running
      .map((runtime) => runtime.portal.portalKey)
      .sort((left, right) => left.localeCompare(right));
    return {
      portalRuntimes,
      syncingPortalKeys,
      syncRunIdsByPortal: Object.fromEntries(
        running.map((runtime) => [runtime.portal.portalKey, runtime.syncRunId]),
      ),
      isSyncing: syncingPortalKeys.length > 0,
    };
  }),
  setActivePortal: (activePortalKey) =>
    set({
      activePortalKey,
      selectedRequestId: null,
      selectedRequestPortalKey: null,
      filteredResults: [],
      searchError: null,
      searchTerm: '',
      selectedAgencyId: null,
      selectedStatus: null,
      selectedPolicyDeviation: null,
      activeLegalFindings: [],
    }),
  setPortalSyncState: (portalKey, syncing, syncRunId) =>
    set((state) => {
      const currentRunId = state.syncRunIdsByPortal[portalKey];
      // Event-derived updates must identify their run. A stale start cannot replace the active
      // run, and an unscoped or stale stop cannot clear it. `setPortalRuntimes` is the deliberate
      // authoritative escape hatch for missed telemetry and process-restart reconciliation.
      if (syncing && !syncRunId) return state;
      if (currentRunId && syncRunId !== currentRunId) {
        return state;
      }
      const keys = new Set(state.syncingPortalKeys);
      const syncRunIdsByPortal = { ...state.syncRunIdsByPortal };
      if (syncing) keys.add(portalKey);
      else keys.delete(portalKey);
      if (syncing && syncRunId) syncRunIdsByPortal[portalKey] = syncRunId;
      if (!syncing) delete syncRunIdsByPortal[portalKey];
      const syncingPortalKeys = [...keys].sort();
      return {
        syncingPortalKeys,
        syncRunIdsByPortal,
        isSyncing: syncingPortalKeys.length > 0,
      };
    }),
  setAudioAlertsEnabled: (audioAlertsEnabled) => set(() => {
    if (typeof window !== 'undefined') {
      window.localStorage.setItem('onrecord-audio-alerts', audioAlertsEnabled ? 'enabled' : 'disabled');
    }
    return { audioAlertsEnabled };
  }),
  appendSyncLog: (log) =>
    set((state) => ({
      syncLogs: [`[${new Date().toLocaleTimeString()}] ${log}`, ...state.syncLogs].slice(0, 250),
    })),
  incrementMetric: (metricKey) =>
    set((state) => ({ metrics: { ...state.metrics, [metricKey]: state.metrics[metricKey] + 1 } })),
  addBytesTransferred: (bytes) =>
    set((state) => ({
      metrics: {
        ...state.metrics,
        bytesTransferred: state.metrics.bytesTransferred + Math.max(0, bytes),
      },
    })),
  resetTelemetryMetrics: () => set({ syncLogs: [], metrics: emptyMetrics() }),
  setAnalyticsData: (agencyProfiles, crossAgencyCorrelations) =>
    set({
      agencyProfiles,
      crossAgencyCorrelations,
      analyticsUpdatedAt: new Date().toISOString(),
      analyticsError: null,
    }),
  setAnalyticsError: (analyticsError) => set({ analyticsError }),
  setLegalAuthorities: (legalAuthorities) =>
    set({
      legalAuthorities: [...legalAuthorities].sort((left, right) =>
        left.statuteCitation.localeCompare(right.statuteCitation),
      ),
      legalDataUpdatedAt: new Date().toISOString(),
    }),
  setActiveLegalFindings: (activeLegalFindings) => set({ activeLegalFindings }),
  setSearchFilters: (
    searchTerm,
    selectedAgencyId,
    selectedStatus,
    selectedPolicyDeviation = null,
  ) =>
    set({ searchTerm, selectedAgencyId, selectedStatus, selectedPolicyDeviation }),
  setFilteredResults: (filteredResults) => set({ filteredResults, searchError: null }),
  setSearchError: (searchError) => set({ searchError }),
  setSelectedRequest: (selectedRequestId, selectedRequestPortalKey = null) =>
    set({ selectedRequestId, selectedRequestPortalKey, activeLegalFindings: [] }),
  setTerminalVisibility: (isExtractionTerminalOpen) => set({ isExtractionTerminalOpen }),
  setPendingSourceRepair: (pendingSourceRepair) => set({ pendingSourceRepair }),
}));

function readAudioAlertsPreference() {
  return typeof window !== 'undefined'
    && window.localStorage.getItem('onrecord-audio-alerts') === 'enabled';
}
