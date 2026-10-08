import { useCallback, useEffect, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';
import { isTauri } from '@tauri-apps/api/core';
import Header from './components/dashboard/Header';
import MainLayout from './components/dashboard/MainLayout';
import Sidebar from './components/dashboard/Sidebar';
import CommandPalette, { OPEN_COMMAND_PALETTE_EVENT } from './components/dashboard/CommandPalette';
import HelpCenter from './components/help/HelpCenter';
import CollectionPlanDialog from './components/dashboard/CollectionPlanDialog';
import Inspector from './components/objects/Inspector';
import { useAppStore } from './store/useAppStore';
import { formatNativeError, invokeCommand } from './ipc';
import type { IngestionTelemetry } from './types';
import { shouldReportFailure, type ReportedFailure } from './utils/repeatedFailure';
import { displayError } from './utils/errorMessage';
import { useAudioAlerts } from './hooks/useAudioAlerts';
import { useAppearance } from './hooks/useAppearance';
import { contrastCommandRegistry, motionCommandRegistry, themeCommandRegistry, workspaceCommandRegistry } from './commands/registry';
import { openHelp } from './components/help/helpEvents';
import LicenseSwitch from './components/licensing/LicenseSwitch';
import { LicenseProvider } from './components/licensing/LicenseProvider';
import { ErrorBoundary } from './components/ui/ErrorBoundary';
import { AutomaticDiscoveryProvider } from './components/dashboard/AutomaticDiscoveryProvider';

const RUNTIME_FAILURE_REMINDER_MILLISECONDS = 60_000;

export default function App() {
  useAppearance();
  return (
    <ErrorBoundary label="OnRecord Studio" fullScreen>
      <LicenseProvider>
        <LicenseSwitch>
          <LicensedWorkspace />
        </LicenseSwitch>
      </LicenseProvider>
    </ErrorBoundary>
  );
}

function LicensedWorkspace() {
  const workspaceView = useAppStore((state) => state.workspaceView);
  const appendSyncLog = useAppStore((state) => state.appendSyncLog);
  const incrementMetric = useAppStore((state) => state.incrementMetric);
  const addBytesTransferred = useAppStore((state) => state.addBytesTransferred);
  const setPortals = useAppStore((state) => state.setPortals);
  const setWorkspaceInitialization = useAppStore((state) => state.setWorkspaceInitialization);
  const setOcrCapability = useAppStore((state) => state.setOcrCapability);
  const upsertPortal = useAppStore((state) => state.upsertPortal);
  const setPortalRuntimes = useAppStore((state) => state.setPortalRuntimes);
  const setPortalRuntimesFailed = useAppStore((state) => state.setPortalRuntimesFailed);
  const workspaceInitializationAttempt = useAppStore((state) => state.workspaceInitializationAttempt);
  const inspectorOpen = useAppStore((state) => state.inspectorStack.length > 0);
  const runtimeRefreshRequests = useAppStore((state) => state.runtimeRefreshRequests);
  const setPortalSyncState = useAppStore((state) => state.setPortalSyncState);
  const setAnalyticsData = useAppStore((state) => state.setAnalyticsData);
  const setAnalyticsError = useAppStore((state) => state.setAnalyticsError);
  const analyticsRefreshRequests = useAppStore((state) => state.analyticsRefreshRequests);
  const setPendingSourceRepair = useAppStore((state) => state.setPendingSourceRepair);
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);
  const requestEvidenceRefresh = useAppStore((state) => state.requestEvidenceRefresh);
  const requestFindingsRefresh = useAppStore((state) => state.requestFindingsRefresh);
  const setPostIngestBriefOpen = useAppStore((state) => state.setPostIngestBriefOpen);
  const runtimeRefreshFailure = useRef<ReportedFailure | null>(null);
  const audioAlertsEnabled = useAppStore((state) => state.audioAlertsEnabled);
  const { triggerFailureAlert, triggerSuccessAlert } = useAudioAlerts(audioAlertsEnabled);

  const refreshAnalytics = useCallback(async () => {
    try {
      const [profiles, correlations] = await Promise.all([
        invokeCommand('get_agency_performance_profiles', { config: null }),
        invokeCommand('get_cross_agency_correlations', {
          portalKey: null,
          minimumComplaints: 2,
          resultLimit: 1000,
          windowHours: 48,
        }),
      ]);
      setAnalyticsData(profiles, correlations);
    } catch (error) {
      // The log alone is not enough: without this the panels keep rendering an empty result,
      // which reads as "examined, nothing found" rather than "never ran".
      setAnalyticsError(displayError(error).message);
      appendSyncLog(`Analytics refresh failed: ${formatNativeError(error)}`);
    }
  }, [appendSyncLog, setAnalyticsData, setAnalyticsError]);

  const refreshRuntimes = useCallback(async () => {
    try {
      const runtimes = await invokeCommand('list_portal_runtimes', {});
      if (runtimeRefreshFailure.current) {
        runtimeRefreshFailure.current = null;
        appendSyncLog('Portal runtime refresh recovered.');
      }
      setPortalRuntimes(runtimes);
    } catch (error) {
      const message = formatNativeError(error);
      setPortalRuntimesFailed(displayError(error).message);
      const now = Date.now();
      if (
        shouldReportFailure(
          runtimeRefreshFailure.current,
          message,
          now,
          RUNTIME_FAILURE_REMINDER_MILLISECONDS,
        )
      ) {
        runtimeRefreshFailure.current = { message, reportedAt: now };
        appendSyncLog(`Portal runtime refresh failed: ${message}`);
      }
    }
  }, [appendSyncLog, setPortalRuntimes, setPortalRuntimesFailed]);

  useEffect(() => {
    let active = true;
    setWorkspaceInitialization('loading');
    void invokeCommand('list_saved_portals', {})
      .then((savedPortals) => {
        if (!active) return;
        setPortals(savedPortals);
        setWorkspaceInitialization('ready');
      })
      .catch((error) => {
        if (!active) return;
        setWorkspaceInitialization('error', displayError(error).message);
        appendSyncLog(`Workspace initialization failed: ${formatNativeError(error)}`);
      });
    // Runtime and analytics failures have their own visible/logged states and must not decide
    // whether an empty saved-source list is genuine.
    void refreshRuntimes();
    void refreshAnalytics();
    void invokeCommand('get_ocr_capability', {})
      .then((capability) => {
        if (!active) return;
        setOcrCapability(capability);
        if (!capability.imageOcrAvailable || !capability.pdfOcrAvailable) {
          appendSyncLog(
            `OCR capability is limited: ${capability.diagnostics.join('; ') || 'required local tools or language packs are unavailable'}`,
          );
        }
      })
      .catch((error) => {
        if (!active) return;
        setOcrCapability(null);
        appendSyncLog(`OCR capability probe failed: ${formatNativeError(error)}`);
      });
    return () => {
      active = false;
    };
  }, [appendSyncLog, refreshAnalytics, refreshRuntimes, setOcrCapability, setPortals, setWorkspaceInitialization, workspaceInitializationAttempt]);

  // A surface that writes what the aggregates read asks for this rather than re-running the
  // queries itself, so the analytics refresh stays owned in one place.
  useEffect(() => {
    if (analyticsRefreshRequests === 0) return;
    void refreshAnalytics();
  }, [analyticsRefreshRequests, refreshAnalytics]);

  useEffect(() => {
    if (runtimeRefreshRequests === 0) return;
    void refreshRuntimes();
  }, [runtimeRefreshRequests, refreshRuntimes]);

  useEffect(() => {
    const reconcile = () => void refreshRuntimes();
    const refreshWhenVisible = () => {
      if (document.visibilityState === 'visible') reconcile();
    };
    // Telemetry drives lifecycle state immediately. This slow watchdog reconciles state after a
    // suspended process or missed bridge event without keeping a 1.5-second polling loop alive.
    const timer = window.setInterval(reconcile, 30_000);
    window.addEventListener('focus', reconcile);
    document.addEventListener('visibilitychange', refreshWhenVisible);
    return () => {
      window.clearInterval(timer);
      window.removeEventListener('focus', reconcile);
      document.removeEventListener('visibilitychange', refreshWhenVisible);
    };
  }, [refreshRuntimes]);

  useEffect(() => {
    if (!isTauri()) return;
    let active = true;
    let unlisten: (() => void) | null = null;
    void listen<string>('onrecord-native-command', ({ payload }) => {
      const workspace = workspaceCommandRegistry.find((command) => command.id === payload);
      const theme = themeCommandRegistry.find((command) => command.id === payload);
      const motion = motionCommandRegistry.find((command) => command.id === payload);
      const contrast = contrastCommandRegistry.find((command) => command.id === payload);
      const store = useAppStore.getState();
      if (workspace) {
        setWorkspaceView(workspace.view);
      } else if (theme) {
        store.setTheme(theme.theme);
      } else if (motion) {
        store.setMotion(motion.motion);
      } else if (contrast) {
        store.setContrast(contrast.contrast);
      } else if (payload === 'add-source') {
        document.getElementById('portal-url-desktop')?.focus();
      } else if (payload === 'command-palette') {
        window.dispatchEvent(new Event(OPEN_COMMAND_PALETTE_EVENT));
      } else if (payload === 'toggle-sidebar') {
        store.toggleSidebarCollapsed();
      } else if (payload === 'open-help') {
        openHelp();
      }
    }).then((stop) => {
      if (active) unlisten = stop;
      else stop();
    }).catch((error) => appendSyncLog(`Native menu listener unavailable: ${String(error)}`));
    return () => {
      active = false;
      unlisten?.();
    };
  }, [appendSyncLog, setWorkspaceView]);

  useEffect(() => {
    let active = true;
    let stopListening: (() => void) | null = null;
    let reconnectTimer: number | null = null;
    let reconnectAttempt = 0;
    let evidenceRefreshTimer: number | null = null;
    const refreshEvidenceSoon = () => {
      if (evidenceRefreshTimer !== null) return;
      evidenceRefreshTimer = window.setTimeout(() => {
        evidenceRefreshTimer = null;
        requestEvidenceRefresh();
      }, 750);
    };
    const seenTelemetrySequences = new Set<string>();
    const connect = () => {
      void listen<IngestionTelemetry>('ingestion-telemetry', ({ payload }) => {
      if (payload.protocolVersion !== 1 || !/^\d+$/.test(payload.sequence)) {
        appendSyncLog('Ignored an ingestion event with an unsupported bridge protocol.');
        return;
      }
      if (seenTelemetrySequences.has(payload.sequence)) return;
      if (seenTelemetrySequences.size >= 4_096) seenTelemetrySequences.clear();
      seenTelemetrySequences.add(payload.sequence);
      switch (payload.type) {
        case 'portalVerified':
          upsertPortal(payload.payload.portal);
          appendSyncLog(`${payload.payload.portal.displayName} passed API and network verification.`);
          break;
        case 'syncStarted':
          setPortalSyncState(payload.payload.portalKey, true, payload.payload.syncRunId);
          appendSyncLog(`${payload.payload.portalKey}: isolated synchronization pool started.`);
          break;
        case 'pageStarted':
          appendSyncLog(`${payload.payload.portalKey}: requesting page ${payload.payload.page}.`);
          break;
        case 'pageCompleted':
          refreshEvidenceSoon();
          incrementMetric('pagesProcessed');
          appendSyncLog(`${payload.payload.portalKey}: page ${payload.payload.page} committed ${payload.payload.recordsFound} requests.`);
          break;
        case 'documentStarted':
          appendSyncLog(`${payload.payload.portalKey}: acquiring ${payload.payload.fileName}.`);
          break;
        case 'documentExtracted':
          appendSyncLog(`${payload.payload.portalKey}: document ${payload.payload.documentId} indexed ${payload.payload.characterCount.toLocaleString()} characters${payload.payload.wasOcrProcessed ? ' using OCR' : ''}.`);
          break;
        case 'documentExpanded':
          appendSyncLog(`${payload.payload.portalKey}: ${payload.payload.format} document ${payload.payload.documentId} expanded into ${payload.payload.entryCount} entries.`);
          break;
        case 'documentDeferred':
          refreshEvidenceSoon();
          appendSyncLog(`${payload.payload.portalKey}: document ${payload.payload.documentId} deferred (${payload.payload.reason}).`);
          break;
        case 'documentSalvaged':
          appendSyncLog(`${payload.payload.portalKey}: document ${payload.payload.documentId} indexed through bounded ${payload.payload.format} salvage after the primary parser failed.`);
          break;
        case 'documentCompleted':
          refreshEvidenceSoon();
          incrementMetric('documentsDownloaded');
          addBytesTransferred(payload.payload.bytesSaved);
          break;
        case 'syncStopped':
          refreshEvidenceSoon();
          setPortalSyncState(payload.payload.portalKey, false, payload.payload.syncRunId);
          appendSyncLog(`${payload.payload.portalKey}: synchronization pool stopped.`);
          void refreshRuntimes();
          void refreshAnalytics();
          // Autonomous: run all detectors against the corpus now that new records have landed,
          // and bump the inbox counter so `FindingsInbox` re-reads the ranked list.
          void invokeCommand('refresh_findings', {})
            .catch((error: unknown) => {
              appendSyncLog(`Findings refresh failed: ${formatNativeError(error)}`);
            })
            .finally(() => {
              requestFindingsRefresh();
              // Surfacing without hunting: after ingest, open the post-ingest brief so a user who
              // did nothing but add a source sees the newest findings without navigating. If the
              // detector run failed the brief still opens; it renders the "no new findings" state
              // rather than staying silent, which would otherwise look identical to a healthy
              // ingest that surfaced nothing.
              setPostIngestBriefOpen(true);
            });
          triggerSuccessAlert();
          break;
        case 'taskFailed':
          incrementMetric('anomaliesFound');
          appendSyncLog(`${payload.payload.portalKey}: ${payload.payload.targetId} failed — ${payload.payload.errorMsg}`);
          triggerFailureAlert();
          break;
        case 'taskRetryScheduled':
          appendSyncLog(`${payload.payload.portalKey}: retrying ${payload.payload.targetId} in ${payload.payload.delaySeconds}s after attempt ${payload.payload.attempt} — ${payload.payload.errorMsg}`);
          break;
        case 'hostStopped':
          appendSyncLog(
            `${payload.payload.portalKey}: stopped contacting ${payload.payload.host} after ${payload.payload.requestsSpent.toLocaleString()} requests — ${payload.payload.reason}`,
          );
          void refreshRuntimes();
          break;
        case 'sourceNeedsGuidance':
          setPendingSourceRepair(payload.payload);
          appendSyncLog(`${payload.payload.portalKey}: guided source repair is available — ${payload.payload.reason}`);
          break;
        case 'corpusReconciliationStarted':
          appendSyncLog('Checking the derived corpus index for missing or outdated documents.');
          break;
        case 'corpusReconciliationCompleted':
          appendSyncLog(
            `Corpus index ready: ${payload.payload.indexedDocuments.toLocaleString()} rebuilt, ${payload.payload.skippedDocuments.toLocaleString()} current, ${payload.payload.failedDocuments.toLocaleString()} failed${payload.payload.firstError ? ` — ${payload.payload.firstError}` : ''}.`,
          );
          break;
        case 'legacyCoreDatabaseMigrated':
          appendSyncLog(
            `Legacy core database migrated: ${payload.payload.copiedRows.toLocaleString()} rows across ${payload.payload.copiedTables.toLocaleString()} tables copied, backup retained at ${payload.payload.backupPath}${payload.payload.tablesAbsentFromLegacy.length > 0 ? `; absent from the legacy file: ${payload.payload.tablesAbsentFromLegacy.join(', ')}` : ''}${payload.payload.tablesDroppedFromLegacy.length > 0 ? `; dropped from the legacy file: ${payload.payload.tablesDroppedFromLegacy.join(', ')}` : ''}.`,
          );
          break;
        default:
          appendSyncLog(`Ignored an unrecognized ingestion event: ${assertNever(payload)}.`);
          break;
      }
      }).then((unlisten) => {
        reconnectAttempt = 0;
        if (active) stopListening = unlisten;
        else unlisten();
      }).catch((error) => {
        if (!active) return;
        reconnectAttempt += 1;
        const delay = Math.min(30_000, 1_000 * (2 ** Math.min(reconnectAttempt - 1, 5)));
        appendSyncLog(`Telemetry listener failed; retrying in ${Math.round(delay / 1_000)}s: ${String(error)}`);
        reconnectTimer = window.setTimeout(connect, delay);
      });
    };
    connect();
    return () => {
      active = false;
      if (reconnectTimer !== null) window.clearTimeout(reconnectTimer);
      if (evidenceRefreshTimer !== null) window.clearTimeout(evidenceRefreshTimer);
      stopListening?.();
    };
  }, [addBytesTransferred, appendSyncLog, incrementMetric, refreshAnalytics, refreshRuntimes, requestEvidenceRefresh, requestFindingsRefresh, setPendingSourceRepair, setPortalSyncState, setPostIngestBriefOpen, triggerFailureAlert, triggerSuccessAlert, upsertPortal]);

  return (
    <div className="app-theme flex h-screen w-screen overflow-hidden antialiased">
      {/* Above the palette as well as the views: the command bar offers the leads the checks found. */}
      <AutomaticDiscoveryProvider>
        <Sidebar />
        {/* From 1440px the open inspector sits beside the content instead of covering it. */}
        <div className={`flex min-w-0 flex-1 flex-col overflow-hidden${inspectorOpen ? ' min-[1440px]:pr-[480px]' : ''}`}>
          <Header />
          <ErrorBoundary label="This view" resetKey={workspaceView}>
            <MainLayout />
          </ErrorBoundary>
        </div>
        <Inspector />
        <CommandPalette />
        <HelpCenter />
        <CollectionPlanDialog />
      </AutomaticDiscoveryProvider>
    </div>
  );
}

/**
 * Compile-time exhaustiveness guard for the ingestion telemetry union: adding a backend variant
 * without a matching `case` above turns this call into a `tsc` error. It deliberately does not
 * throw. The switch runs inside the `listen()` callback, which sits outside the `.catch()` on the
 * `listen()` promise, so throwing here would kill the listener for every event after the first
 * unrecognized one -- converting a forward-compatibility gap into dead telemetry.
 */
function assertNever(value: never): string {
  return String((value as { type?: unknown }).type ?? 'unknown');
}
