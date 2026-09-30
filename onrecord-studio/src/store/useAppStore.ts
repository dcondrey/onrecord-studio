import { create } from 'zustand';
import { readThemePreference, type ThemePreference } from '../utils/theme';
import {
  readContrastPreference,
  readMotionPreference,
  type ContrastPreference,
  type MotionPreference,
} from '../utils/accessibility';
import type {
  AgencyPerformanceProfile,
  CrossAgencyCorrelation,
  LegalAuthoritySummary,
  LegalComplianceFinding,
  OcrCapability,
  PortalRegistration,
  PortalRuntimeSnapshot,
  RequestSearchResult,
  SavedSource,
  SourceLifecycleState,
} from '../types';
import {
  loadBriefingProject,
  persistBriefingProject,
  type BriefingEvidence,
  type BriefingProject,
} from '../utils/briefing';

export interface IngestionMetrics {
  pagesProcessed: number;
  documentsDownloaded: number;
  anomaliesFound: number;
  bytesTransferred: number;
}

export type WorkspaceView = 'overview' | 'patterns' | 'analysis' | 'evidence' | 'activity' | 'briefing';
export type AppTheme = ThemePreference;
export type AppMotion = MotionPreference;
export type AppContrast = ContrastPreference;
export type WorkspaceInitializationStatus = 'loading' | 'ready' | 'error';

export interface SourceRepairNotice {
  portalKey: string;
  sourceUrl: string;
  reason: string;
}

export interface PatternEvidenceSelection {
  patternId: string;
  portalKey: string | null;
  documentId: number | null;
  characterStart: number | null;
}

interface AppState {
  // Search state mirrors the four filters in the supported native query contract; qualifier text
  // is parsed at the search boundary and unsupported syntax never enters this store.
  workspaceView: WorkspaceView;
  theme: AppTheme;
  motion: AppMotion;
  contrast: AppContrast;
  isSidebarOpen: boolean;
  isSidebarCollapsed: boolean;
  portals: SavedSource[];
  workspaceInitializationStatus: WorkspaceInitializationStatus;
  workspaceInitializationError: string | null;
  ocrCapability: OcrCapability | null;
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
  /**
   * Incremented by any surface that writes data the analytics aggregates read, so the single
   * refresh that owns those queries re-runs. A panel left on a pre-write aggregate reports a
   * coverage answer the database no longer supports.
   */
  analyticsRefreshRequests: number;
  evidenceRefreshRequests: number;
  /**
   * Incremented after each ingest completion and by the explicit Refresh button so the findings
   * inbox picks up the latest sweep without a race against the ingest hook.
   */
  findingsRefreshRequests: number;
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
  patternEvidenceSelection: PatternEvidenceSelection | null;
  briefingProject: BriefingProject;
  /**
   * When set, the OverviewDashboard renders the SubjectDossier for this target. `entity` is a
   * resolved entity id (safe to fetch directly). `name` is an unresolved free-text query that the
   * dossier resolves via `list_top_corpus_entities` and disambiguates when it matches more than
   * one entity.
   */
  subjectDossierTarget: { kind: 'entity' | 'name'; value: string } | null;
  /**
   * True when the post-ingest brief overlay should be visible. Set by `syncStopped` in App.tsx
   * after `refresh_findings` completes; cleared when the user dismisses it or opens a specific
   * finding.
   */
  postIngestBriefOpen: boolean;
  /**
   * ISO-8601 timestamp of the last moment the user visibly saw the findings inbox (its
   * FindingsInbox mount, or a Refresh press, or a dismiss). Used by ChangeFeed to report "N new
   * findings since you last opened." `null` means the user has never opened the inbox in this
   * session — treat every finding as new.
   */
  lastFindingsSeenAt: string | null;
  setWorkspaceView: (view: WorkspaceView) => void;
  setTheme: (theme: AppTheme) => void;
  setMotion: (motion: AppMotion) => void;
  setContrast: (contrast: AppContrast) => void;
  setSidebarOpen: (open: boolean) => void;
  toggleSidebarCollapsed: () => void;
  setPortalUrlDraft: (url: string) => void;
  upsertPortal: (portal: PortalRegistration) => void;
  setPortals: (portals: SavedSource[]) => void;
  setWorkspaceInitialization: (
    status: WorkspaceInitializationStatus,
    error?: string | null,
  ) => void;
  setOcrCapability: (capability: OcrCapability | null) => void;
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
  requestAnalyticsRefresh: () => void;
  requestEvidenceRefresh: () => void;
  requestFindingsRefresh: () => void;
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
  setPatternEvidenceSelection: (selection: PatternEvidenceSelection | null) => void;
  updateBriefingProject: (patch: Partial<Omit<BriefingProject, 'version' | 'evidence'>>) => void;
  addBriefingEvidence: (evidence: BriefingEvidence) => void;
  updateBriefingEvidence: (id: string, patch: Pick<Partial<BriefingEvidence>, 'claim' | 'note' | 'readiness'>) => void;
  removeBriefingEvidence: (id: string) => void;
  openSubjectDossier: (target: { kind: 'entity' | 'name'; value: string }) => void;
  clearSubjectDossier: () => void;
  setPostIngestBriefOpen: (open: boolean) => void;
  markFindingsSeen: () => void;
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
  motion: readMotionPreference(),
  contrast: readContrastPreference(),
  isSidebarOpen: false,
  isSidebarCollapsed: false,
  portals: [],
  workspaceInitializationStatus: 'loading',
  workspaceInitializationError: null,
  ocrCapability: null,
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
  analyticsRefreshRequests: 0,
  evidenceRefreshRequests: 0,
  findingsRefreshRequests: 0,
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
  patternEvidenceSelection: null,
  briefingProject: loadBriefingProject(
    typeof window === 'undefined' ? undefined : window.localStorage,
  ),
  subjectDossierTarget: null,
  postIngestBriefOpen: false,
  lastFindingsSeenAt: null,
  setWorkspaceView: (workspaceView) => set({ workspaceView, isSidebarOpen: false }),
  setTheme: (theme) => set({ theme }),
  setMotion: (motion) => set({ motion }),
  setContrast: (contrast) => set({ contrast }),
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
  setOcrCapability: (ocrCapability) => set({ ocrCapability }),
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
  requestAnalyticsRefresh: () =>
    set((state) => ({ analyticsRefreshRequests: state.analyticsRefreshRequests + 1 })),
  requestEvidenceRefresh: () =>
    set((state) => ({ evidenceRefreshRequests: state.evidenceRefreshRequests + 1 })),
  requestFindingsRefresh: () =>
    set((state) => ({ findingsRefreshRequests: state.findingsRefreshRequests + 1 })),
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
  setPatternEvidenceSelection: (patternEvidenceSelection) => set({ patternEvidenceSelection }),
  updateBriefingProject: (patch) => set((state) => {
    const briefingProject = { ...state.briefingProject, ...patch };
    persistBriefingProject(
      briefingProject,
      typeof window === 'undefined' ? undefined : window.localStorage,
    );
    return { briefingProject };
  }),
  addBriefingEvidence: (evidence) => set((state) => {
    const existing = state.briefingProject.evidence.find((item) => item.id === evidence.id);
    const briefingProject = {
      ...state.briefingProject,
      evidence: existing
        ? state.briefingProject.evidence
        : [...state.briefingProject.evidence, evidence].slice(-200),
    };
    persistBriefingProject(
      briefingProject,
      typeof window === 'undefined' ? undefined : window.localStorage,
    );
    return { briefingProject };
  }),
  updateBriefingEvidence: (id, patch) => set((state) => {
    const briefingProject = {
      ...state.briefingProject,
      evidence: state.briefingProject.evidence.map((item) =>
        item.id === id ? { ...item, ...patch } : item),
    };
    persistBriefingProject(
      briefingProject,
      typeof window === 'undefined' ? undefined : window.localStorage,
    );
    return { briefingProject };
  }),
  removeBriefingEvidence: (id) => set((state) => {
    const briefingProject = {
      ...state.briefingProject,
      evidence: state.briefingProject.evidence.filter((item) => item.id !== id),
    };
    persistBriefingProject(
      briefingProject,
      typeof window === 'undefined' ? undefined : window.localStorage,
    );
    return { briefingProject };
  }),
  openSubjectDossier: (target) => set({ subjectDossierTarget: target }),
  clearSubjectDossier: () => set({ subjectDossierTarget: null }),
  setPostIngestBriefOpen: (postIngestBriefOpen) => set({ postIngestBriefOpen }),
  markFindingsSeen: () => set({ lastFindingsSeenAt: new Date().toISOString() }),
}));

function readAudioAlertsPreference() {
  return typeof window !== 'undefined'
    && window.localStorage.getItem('onrecord-audio-alerts') === 'enabled';
}
