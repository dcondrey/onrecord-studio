import { useCallback, useEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { setTheme as setNativeTheme } from '@tauri-apps/api/app';
import { isTauri } from '@tauri-apps/api/core';
import Header from './components/dashboard/Header';
import MainLayout from './components/dashboard/MainLayout';
import Sidebar from './components/dashboard/Sidebar';
import CommandPalette, { OPEN_COMMAND_PALETTE_EVENT } from './components/dashboard/CommandPalette';
import { useAppStore } from './store/useAppStore';
import { invokeCommand } from './ipc';
import type { IngestionTelemetry } from './types';
import { shouldReportFailure, type ReportedFailure } from './utils/repeatedFailure';
import { resolveTheme, THEME_STORAGE_KEY } from './utils/theme';
import {
  CONTRAST_STORAGE_KEY,
  MOTION_STORAGE_KEY,
  resolveContrast,
  resolveMotion,
  systemAccessibilityPreferences,
} from './utils/accessibility';
import { useAudioAlerts } from './hooks/useAudioAlerts';
import { workspaceCommandRegistry } from './commands/registry';
import LicenseGate from './components/licensing/LicenseGate';
import { AutomaticDiscoveryProvider } from './components/dashboard/AutomaticDiscoveryProvider';
import type { LicenseStatus } from './types';

const RUNTIME_FAILURE_REMINDER_MILLISECONDS = 60_000;

const BROWSER_DEVELOPMENT_LICENSE: LicenseStatus = {
  enforced: false,
  authorized: true,
  state: 'DEVELOPMENT',
  message: 'Browser preview uses development access.',
  bundleId: 'com.onrecord.studio',
  appVersion: null,
  originalAppVersion: null,
  appTransactionId: null,
  appId: null,
  environment: 'browser',
  originalPurchaseDate: null,
  signedDate: null,
  revocationDate: null,
};

export default function App() {
  const [license, setLicense] = useState<LicenseStatus | null>(() =>
    isTauri() ? null : BROWSER_DEVELOPMENT_LICENSE,
  );
  const [refreshing, setRefreshing] = useState(false);
  const [licenseError, setLicenseError] = useState<string | null>(null);

  useEffect(() => {
    if (!isTauri()) return;
    let active = true;
    void invokeCommand('get_license_status', {})
      .then((status) => {
        if (active) setLicense(status);
      })
      .catch((error) => {
        if (active) setLicenseError(String(error));
      });
    return () => {
      active = false;
    };
  }, []);

  const refreshLicense = useCallback(() => {
    if (!isTauri() || refreshing) return;
    setRefreshing(true);
    setLicenseError(null);
    void invokeCommand('refresh_license', {})
      .then(setLicense)
      .catch((error) => setLicenseError(String(error)))
      .finally(() => setRefreshing(false));
  }, [refreshing]);

  if (!license?.authorized) {
    return (
      <LicenseGate
        status={license}
        refreshing={refreshing}
        error={licenseError}
        onRefresh={refreshLicense}
      />
    );
  }
  return <LicensedWorkspace />;
}

function LicensedWorkspace() {
  const theme = useAppStore((state) => state.theme);
  const motion = useAppStore((state) => state.motion);
  const contrast = useAppStore((state) => state.contrast);
  const [systemTheme, setSystemTheme] = useState<'light' | 'dark'>(() =>
    window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light',
  );
  const [systemAccessibility, setSystemAccessibility] = useState(systemAccessibilityPreferences);
  const appendSyncLog = useAppStore((state) => state.appendSyncLog);
  const incrementMetric = useAppStore((state) => state.incrementMetric);
  const addBytesTransferred = useAppStore((state) => state.addBytesTransferred);
  const setPortals = useAppStore((state) => state.setPortals);
  const setWorkspaceInitialization = useAppStore((state) => state.setWorkspaceInitialization);
  const setOcrCapability = useAppStore((state) => state.setOcrCapability);
  const upsertPortal = useAppStore((state) => state.upsertPortal);
  const setPortalRuntimes = useAppStore((state) => state.setPortalRuntimes);
  const setPortalSyncState = useAppStore((state) => state.setPortalSyncState);
  const setAnalyticsData = useAppStore((state) => state.setAnalyticsData);
  const setAnalyticsError = useAppStore((state) => state.setAnalyticsError);
  const analyticsRefreshRequests = useAppStore((state) => state.analyticsRefreshRequests);
  const setPendingSourceRepair = useAppStore((state) => state.setPendingSourceRepair);
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);
  const requestEvidenceRefresh = useAppStore((state) => state.requestEvidenceRefresh);
  const runtimeRefreshFailure = useRef<ReportedFailure | null>(null);
  const audioAlertsEnabled = useAppStore((state) => state.audioAlertsEnabled);
  const { triggerFailureAlert, triggerSuccessAlert } = useAudioAlerts(audioAlertsEnabled);

  const resolvedTheme = resolveTheme(theme, systemTheme === 'dark');
  const resolvedMotion = resolveMotion(motion, systemAccessibility.prefersReducedMotion);
  const resolvedContrast = resolveContrast(contrast, systemAccessibility.prefersMoreContrast);
  const platform = getPlatform();

  useEffect(() => {
    const appearance = window.matchMedia('(prefers-color-scheme: dark)');
    const updateSystemTheme = () => setSystemTheme(appearance.matches ? 'dark' : 'light');
    updateSystemTheme();
    appearance.addEventListener('change', updateSystemTheme);
    return () => appearance.removeEventListener('change', updateSystemTheme);
  }, []);

  useEffect(() => {
    document.documentElement.dataset.theme = resolvedTheme;
    document.documentElement.dataset.themePreference = theme;
    document.querySelector<HTMLMetaElement>('meta[name="theme-color"]')?.setAttribute(
      'content',
      resolvedTheme === 'dark' ? '#050a12' : '#eef2f6',
    );
    window.localStorage.setItem(THEME_STORAGE_KEY, theme);
    if (isTauri()) void setNativeTheme(theme === 'auto' ? null : theme);
  }, [resolvedTheme, theme]);

  useEffect(() => {
    const motionQuery = window.matchMedia('(prefers-reduced-motion: reduce)');
    const contrastQuery = window.matchMedia('(prefers-contrast: more)');
    const update = () =>
      setSystemAccessibility({
        prefersReducedMotion: motionQuery.matches,
        prefersMoreContrast: contrastQuery.matches,
      });
    update();
    motionQuery.addEventListener('change', update);
    contrastQuery.addEventListener('change', update);
    return () => {
      motionQuery.removeEventListener('change', update);
      contrastQuery.removeEventListener('change', update);
    };
  }, []);

  useEffect(() => {
    document.documentElement.dataset.motion = resolvedMotion;
    document.documentElement.dataset.motionPreference = motion;
    document.documentElement.dataset.contrast = resolvedContrast;
    document.documentElement.dataset.contrastPreference = contrast;
    window.localStorage.setItem(MOTION_STORAGE_KEY, motion);
    window.localStorage.setItem(CONTRAST_STORAGE_KEY, contrast);
  }, [contrast, motion, resolvedContrast, resolvedMotion]);

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
      setAnalyticsError(String(error));
      appendSyncLog(`Analytics refresh failed: ${String(error)}`);
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
      const message = String(error);
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
  }, [appendSyncLog, setPortalRuntimes, setPortalSyncState]);

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
        const message = String(error);
        setWorkspaceInitialization('error', message);
        appendSyncLog(`Workspace initialization failed: ${message}`);
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
        appendSyncLog(`OCR capability probe failed: ${String(error)}`);
      });
    return () => {
      active = false;
    };
  }, [appendSyncLog, refreshAnalytics, refreshRuntimes, setOcrCapability, setPortals, setWorkspaceInitialization]);

  // A surface that writes what the aggregates read asks for this rather than re-running the
  // queries itself, so the analytics refresh stays owned in one place.
  useEffect(() => {
    if (analyticsRefreshRequests === 0) return;
    void refreshAnalytics();
  }, [analyticsRefreshRequests, refreshAnalytics]);

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
      if (workspace) {
        setWorkspaceView(workspace.view);
      } else if (payload === 'add-source') {
        document.getElementById('portal-url-desktop')?.focus();
      } else if (payload === 'command-palette') {
        window.dispatchEvent(new Event(OPEN_COMMAND_PALETTE_EVENT));
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
  }, [addBytesTransferred, appendSyncLog, incrementMetric, refreshAnalytics, refreshRuntimes, requestEvidenceRefresh, setPendingSourceRepair, setPortalSyncState, triggerFailureAlert, triggerSuccessAlert, upsertPortal]);

  return (
    <div data-theme={resolvedTheme} data-platform={platform} className="app-theme flex h-screen w-screen overflow-hidden antialiased">
      <Sidebar />
      <div className="flex min-w-0 flex-1 flex-col overflow-hidden">
        <Header />
        <AutomaticDiscoveryProvider><MainLayout /></AutomaticDiscoveryProvider>
      </div>
      <CommandPalette />
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

function getPlatform() {
  const platform = navigator.platform.toLocaleLowerCase();
  const userAgent = navigator.userAgent.toLocaleLowerCase();
  if (platform.includes('mac') || userAgent.includes('mac os')) return 'macos';
  if (platform.includes('win') || userAgent.includes('windows')) return 'windows';
  return 'other';
}
