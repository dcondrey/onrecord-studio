import { create } from 'zustand';

// Explicit interfaces matching your native Rust data shapes
export interface RequestMetricSummary {
  id: u64;
  portal: string;
  title: string;
  status: string;
  date: string;
  documentsCount: number;
}

export interface IngestionMetrics {
  pagesProcessed: number;
  documentsDownloaded: number;
  anomaliesFound: number;
  bytesTransferred: number;
}

interface AppState {
  // 1. Core Structural Workspace States
  activePortal: string;
  isSyncing: boolean;
  syncLogs: string[];
  metrics: IngestionMetrics;
  
  // 2. Query, Filter, & Discovery States
  searchTerm: string;
  selectedAgencyId: number | null;
  selectedStatus: string | null;
  filteredResults: u64[]; // Holds target structural IDs returned from Rust FTS5
  
  // 3. UI Navigation & Drill-Down Focused States
  selectedRequestId: u64 | null;
  isExtractionTerminalOpen: boolean;

  // 4. State Modification Actions
  setActivePortal: (portal: string) => void;
  setIsSyncing: (syncing: boolean) => void;
  appendSyncLog: (log: string) => void;
  incrementMetric: (metricKey: keyof Omit<IngestionMetrics, 'bytesTransferred'>) => void;
  addBytesTransferred: (bytes: number) => void;
  resetTelemetryMetrics: () => void;
  
  setSearchFilters: (term: string, agencyId: number | null, status: string | null) => void;
  setFilteredResults: (ids: u64[]) => void;
  setSelectedRequest: (requestId: u64 | null) => void;
  setTerminalVisibility: (visible: boolean) => void;
}

export const useAppStore = create<AppState>((set) => ({
  // --- INITIAL STATES ---
  activePortal: 'sandiego',
  isSyncing: false,
  syncLogs: [],
  metrics: {
    pagesProcessed: 0,
    documentsDownloaded: 0,
    anomaliesFound: 0,
    bytesTransferred: 0,
  },
  searchTerm: '',
  selectedAgencyId: null,
  selectedStatus: null,
  filteredResults: [],
  selectedRequestId: null,
  isExtractionTerminalOpen: false,

  // --- MUTATOR ACTIONS ---
  
  // Switches the target municipality context and safely resets localized analytical view indices
  setActivePortal: (portal) => set({ 
    activePortal: portal,
    selectedRequestId: null,
    filteredResults: [],
    searchTerm: '',
    selectedAgencyId: null,
    selectedStatus: null
  }),
  
  setIsSyncing: (syncing) => set({ isSyncing: syncing }),
  
  appendSyncLog: (log) => set((state) => ({
    syncLogs: [
      `[${new Date().toLocaleTimeString()}] ${log}`,
      ...state.syncLogs.slice(0, 99) // Strictly cap at 100 historical log items to prevent UI execution bloat
    ]
  })),
  
  incrementMetric: (metricKey) => set((state) => ({
    metrics: {
      ...state.metrics,
      [metricKey]: state.metrics[metricKey] + 1
    }
  })),
  
  addBytesTransferred: (bytes) => set((state) => ({
    metrics: {
      ...state.metrics,
      bytesTransferred: state.metrics.bytesTransferred + bytes
    }
  })),
  
  resetTelemetryMetrics: () => set({
    syncLogs: [],
    metrics: {
      pagesProcessed: 0,
      documentsDownloaded: 0,
      anomaliesFound: 0,
      bytesTransferred: 0,
    }
  }),

  setSearchFilters: (term, agencyId, status) => set({
    searchTerm: term,
    selectedAgencyId: agencyId,
    selectedStatus: status
  }),

  setFilteredResults: (ids) => set({ filteredResults: ids }),

  setSelectedRequest: (requestId) => set({ selectedRequestId: requestId }),

  setTerminalVisibility: (visible) => set({ isExtractionTerminalOpen: visible })
}));
