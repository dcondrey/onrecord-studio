import { type FormEvent, useEffect, useMemo, useState } from 'react';
import { formatNativeError, invokeCommand } from '../ipc';
import { sanDiegoGuidance, sanDiegoSourceForUrl } from '../utils/sanDiego';
import { open } from '@tauri-apps/plugin-dialog';
import { useAppStore } from '../store/useAppStore';
import {
  crawlPolicyOverride,
  metadataPageWindowOverride,
  DEFAULT_CRAWL_SETTINGS_DRAFT,
  parseCrawlSettings,
  type CrawlSettingsDraft,
} from '../utils/crawlPolicy';
import {
  DEFAULT_GUIDANCE_FILTER_DRAFT,
  guidanceFiltersAreCustom,
  parseGuidanceFilters,
  type GuidanceFilterDraft,
} from '../utils/sourceGuidance';
import { BUILT_IN_SOURCE_OPTIONS, type PortalRegistration, type SourceDiscoveryPreview, type SourceGuidance, type SourceKind, type SourcePurgePreview, type SourceRecheckSummary } from '../types';
import { useLatestRequestGate } from './useLatestRequestGate';

type SourceGuidanceScope = Omit<SourceGuidance,
  'reuseForPublisherPattern' | 'additionalSeedUrls' | 'allowedHostnames' | 'includeUrlPrefixes' | 'excludeUrlPrefixes' | 'linkSelector' | 'downloadSelector'>;

function buildRepairGuidance(scope: SourceGuidanceScope, additionalSeedUrls: string[], linkSelector: string, downloadSelector: string, reuseForPublisherPattern: boolean): SourceGuidance {
  return {
    reuseForPublisherPattern,
    additionalSeedUrls,
    allowedHostnames: [],
    includeUrlPrefixes: [],
    excludeUrlPrefixes: [],
    linkSelector: linkSelector.trim() || null,
    downloadSelector: downloadSelector.trim() || null,
    ...scope,
  };
}

/**
 * All state and handlers behind "add and manage a data source": the connector form, its guided
 * repair flow, crawl/guidance settings, and the remove/purge confirmations. Extracted from
 * `Header.tsx` as one unit because the pieces genuinely share state (a single in-flight
 * discovery, a single error banner) rather than being independently reusable.
 */
export function useSourceConnector() {
  const portals = useAppStore((state) => state.portals);
  const activePortalKey = useAppStore((state) => state.activePortalKey);
  const portalUrlDraft = useAppStore((state) => state.portalUrlDraft);
  const syncingPortalKeys = useAppStore((state) => state.syncingPortalKeys);
  const syncRunIdsByPortal = useAppStore((state) => state.syncRunIdsByPortal);
  const setPortalUrlDraft = useAppStore((state) => state.setPortalUrlDraft);
  const setPortalSyncState = useAppStore((state) => state.setPortalSyncState);
  const setActivePortal = useAppStore((state) => state.setActivePortal);
  const upsertPortal = useAppStore((state) => state.upsertPortal);
  const setPortals = useAppStore((state) => state.setPortals);
  const setPortalLifecycleState = useAppStore((state) => state.setPortalLifecycleState);
  const removePortalFromWorkspace = useAppStore((state) => state.removePortal);
  const requestEvidenceRefresh = useAppStore((state) => state.requestEvidenceRefresh);
  const requestFindingsRefresh = useAppStore((state) => state.requestFindingsRefresh);
  const requestAnalyticsRefresh = useAppStore((state) => state.requestAnalyticsRefresh);
  const appendSyncLog = useAppStore((state) => state.appendSyncLog);
  const pendingSourceRepair = useAppStore((state) => state.pendingSourceRepair);
  const setPendingSourceRepair = useAppStore((state) => state.setPendingSourceRepair);
  const [isDiscovering, setIsDiscovering] = useState(false);
  const sourceRequestGate = useLatestRequestGate();
  const repairDiscoveryGate = useLatestRequestGate();
  const [isConnectorOpen, setConnectorOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [isPasteOpen, setPasteOpen] = useState(false);
  // Holds the source a removal was requested for, not just a boolean, so the confirmation names
  // the source the operator actually clicked even if the selection moves under it.
  const [pendingRemoval, setPendingRemoval] = useState<PortalRegistration | null>(null);
  // Keys of sources with a check in flight, so a check started before switching sources still
  // disables only its own button.
  const [recheckingKeys, setRecheckingKeys] = useState<ReadonlySet<string>>(() => new Set());
  // The purge confirmation is a separate piece of state from the removal confirmation, not a mode
  // flag on one dialog. `remove_source` withdraws a source and keeps its documents; `purge_source`
  // destroys them and deletes the registration. An operator who meant one and got the other has
  // either kept evidence they meant to erase or erased evidence they meant to keep, so the two
  // never share a control, a dialog, or a confirming verb.
  const [pendingPurge, setPendingPurge] = useState<PortalRegistration | null>(null);
  const [purgePreview, setPurgePreview] = useState<SourcePurgePreview | null>(null);
  const [purgePreviewError, setPurgePreviewError] = useState<string | null>(null);
  const [isPurging, setIsPurging] = useState(false);
  const purgePreviewGate = useLatestRequestGate();
  const [pasteTitle, setPasteTitle] = useState('Pasted research material');
  const [pasteContent, setPasteContent] = useState('');
  const [repairPreview, setRepairPreview] = useState<SourceDiscoveryPreview | null>(null);
  const [repairRootUrl, setRepairRootUrl] = useState('');
  const [repairSeedUrl, setRepairSeedUrl] = useState('');
  const [repairLinkSelector, setRepairLinkSelector] = useState('');
  const [repairDownloadSelector, setRepairDownloadSelector] = useState('');
  const [reuseRepairForPublisher, setReuseRepairForPublisher] = useState(false);
  const [isPreviewingSelectors, setIsPreviewingSelectors] = useState(false);
  const selectedPreset = sanDiegoSourceForUrl(portalUrlDraft);
  const selectedSourceId = selectedPreset?.id ?? 'auto';
  const [isCrawlSettingsOpen, setCrawlSettingsOpen] = useState(false);
  const [crawlDraft, setCrawlDraft] = useState<CrawlSettingsDraft>(DEFAULT_CRAWL_SETTINGS_DRAFT);
  const effectiveCrawlDraft = useMemo(() => {
    const raw = parseCrawlSettings(crawlDraft);
    if (!selectedPreset || !raw.ok || crawlPolicyOverride(raw.crawlPolicy)) return crawlDraft;
    return {
      ...crawlDraft,
      maximumDepth: String(selectedPreset.crawlPolicy.maximumDepth),
      maximumPages: String(selectedPreset.crawlPolicy.maximumPages),
      maximumResources: String(selectedPreset.crawlPolicy.maximumResources),
      followSupportedExternalPlatforms: selectedPreset.crawlPolicy.followSupportedExternalPlatforms,
    };
  }, [crawlDraft, selectedPreset]);
  const crawlSettings = useMemo(() => parseCrawlSettings(effectiveCrawlDraft), [effectiveCrawlDraft]);
  const crawlSettingsAreCustom = useMemo(
    () =>
      (Object.keys(DEFAULT_CRAWL_SETTINGS_DRAFT) as (keyof CrawlSettingsDraft)[]).some(
        (field) => crawlDraft[field] !== DEFAULT_CRAWL_SETTINGS_DRAFT[field],
      ),
    [crawlDraft],
  );
  const updateCrawlDraft = (patch: Partial<CrawlSettingsDraft>) =>
    setCrawlDraft({ ...effectiveCrawlDraft, ...patch });
  const [guidanceDraft, setGuidanceDraft] = useState<GuidanceFilterDraft>(DEFAULT_GUIDANCE_FILTER_DRAFT);
  const guidanceFilters = useMemo(() => parseGuidanceFilters(guidanceDraft), [guidanceDraft]);
  const hasGuidanceFilters = useMemo(() => guidanceFiltersAreCustom(guidanceDraft), [guidanceDraft]);
  const updateGuidanceDraft = (patch: Partial<GuidanceFilterDraft>) =>
    setGuidanceDraft((draft) => ({ ...draft, ...patch }));
  const resolveGuidanceFilters = () => {
    if (!guidanceFilters.ok) {
      setError(guidanceFilters.problem);
      return null;
    }
    return guidanceFilters;
  };
  // Both ingestion entry points refuse to start on a half-typed limit rather than sending a
  // number the backend would reject with an opaque serde error.
  const resolveCrawlSettings = () => {
    if (!crawlSettings.ok) {
      setError(crawlSettings.problem);
      setCrawlSettingsOpen(true);
      return null;
    }
    return crawlSettings;
  };
  const activeSource = useMemo(
    () => portals.find((saved) => saved.registration.portalKey === activePortalKey) ?? null,
    [activePortalKey, portals],
  );
  const activePortal = activeSource?.registration ?? null;
  const activeIsPaused = activeSource?.lifecycleState === 'PAUSED';
  const activeIsSyncing = activePortal ? syncingPortalKeys.includes(activePortal.portalKey) : false;
  const syncCapacityReached = syncingPortalKeys.length >= 5;
  const selectedSource = BUILT_IN_SOURCE_OPTIONS.find((source) => source.id === selectedSourceId) ?? BUILT_IN_SOURCE_OPTIONS[0];

  const cancelSourceOperation = () => {
    sourceRequestGate.invalidate();
    repairDiscoveryGate.invalidate();
    setIsDiscovering(false);
  };
  const closePasteDialog = () => {
    cancelSourceOperation();
    setPasteOpen(false);
  };
  const closeRepairDialog = () => {
    cancelSourceOperation();
    setRepairPreview(null);
    setRepairLinkSelector('');
    setRepairDownloadSelector('');
    setReuseRepairForPublisher(false);
    setIsPreviewingSelectors(false);
  };

  useEffect(() => {
    if (!pendingSourceRepair) return;
    const request = repairDiscoveryGate.begin();
    setError(pendingSourceRepair.reason);
    void invokeCommand('preview_data_source', { url: pendingSourceRepair.sourceUrl })
      .then((preview) => {
        if (!repairDiscoveryGate.isCurrent(request)) return;
        setRepairPreview(preview);
        setRepairRootUrl(pendingSourceRepair.sourceUrl);
        setRepairSeedUrl(
          preview.candidateLinks.find((link) => link.downloadCandidate)?.url ?? '',
        );
        setGuidanceDraft(DEFAULT_GUIDANCE_FILTER_DRAFT);
        setRepairLinkSelector('');
        setRepairDownloadSelector('');
        setReuseRepairForPublisher(false);
      })
      .catch(() => {
        // The persisted ingestion failure remains visible even if previewing also fails.
      })
      .finally(() => {
        if (repairDiscoveryGate.isCurrent(request)) setPendingSourceRepair(null);
      });
    return repairDiscoveryGate.invalidate;
  }, [pendingSourceRepair, repairDiscoveryGate, setPendingSourceRepair]);

  const startSync = async (event: FormEvent) => {
    event.preventDefault();
    const targetUrl = portalUrlDraft.trim() || activePortal?.canonicalUrl || '';
    if (!targetUrl) {
      setError('Paste a public website, catalog, API, or document URL to add it.');
      return;
    }
    const settings = resolveCrawlSettings();
    if (!settings) return;
    repairDiscoveryGate.invalidate();
    const request = sourceRequestGate.begin();
    setPendingSourceRepair(null);
    setError(null);
    setIsDiscovering(true);
    appendSyncLog(`Verifying portal layout and public network route for ${targetUrl}.`);
    try {
      const preset = sanDiegoSourceForUrl(targetUrl);
      if (preset?.access === 'export') {
        setError(`${preset.coverage} Use Import files in the toolbar, or Analytical Import for a table.`);
        return;
      }
      const { source, syncRunId, detectedKind } = await invokeCommand('start_data_source', {
        url: preset?.url ?? targetUrl,
        crawlPolicy: crawlPolicyOverride(settings.crawlPolicy) ?? preset?.crawlPolicy ?? null,
        guidance: preset ? sanDiegoGuidance(preset) : null,
        rateLimitPolicy: settings.rateLimitPolicy,
        metadataPageWindow: metadataPageWindowOverride(settings.metadataPageWindow),
      });
      if (!sourceRequestGate.isCurrent(request)) return;
      registerSourceStart(source, syncRunId, detectedKind);
      setPortalUrlDraft('');
      setConnectorOpen(false);
    } catch (reason) {
      if (!sourceRequestGate.isCurrent(request)) return;
      const message = String(reason);
      setError(message);
      appendSyncLog(`Automatic source discovery failed: ${message}`);
      try {
        const preview = await invokeCommand('preview_data_source', { url: targetUrl });
        if (!sourceRequestGate.isCurrent(request)) return;
        setRepairPreview(preview);
        setRepairRootUrl(targetUrl);
        setRepairSeedUrl(preview.candidateLinks.find((link) => link.downloadCandidate)?.url ?? '');
        setGuidanceDraft(DEFAULT_GUIDANCE_FILTER_DRAFT);
      } catch {
        // The original actionable discovery error remains visible.
      }
    } finally {
      if (sourceRequestGate.isCurrent(request)) setIsDiscovering(false);
    }
  };

  const registerSourceStart = (
    source: PortalRegistration,
    syncRunId: string,
    detectedKind: SourceKind,
  ) => {
    upsertPortal(source);
    setActivePortal(source.portalKey);
    setPortalSyncState(source.portalKey, true, syncRunId);
    appendSyncLog(`${source.displayName}: detected ${detectedKind.replace(/_/g, ' ').toLocaleLowerCase()} source.`);
  };

  const chooseFiles = async () => {
    repairDiscoveryGate.invalidate();
    const request = sourceRequestGate.begin();
    setPendingSourceRepair(null);
    setError(null);
    try {
      const selected = await open({ multiple: true, directory: false });
      if (!sourceRequestGate.isCurrent(request)) return;
      if (!selected) return;
      const paths = Array.isArray(selected) ? selected : [selected];
      setIsDiscovering(true);
      const { source, syncRunId, detectedKind } = await invokeCommand('ingest_local_files', { paths });
      if (!sourceRequestGate.isCurrent(request)) return;
      registerSourceStart(source, syncRunId, detectedKind);
      setConnectorOpen(false);
    } catch (reason) {
      if (!sourceRequestGate.isCurrent(request)) return;
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      if (sourceRequestGate.isCurrent(request)) setIsDiscovering(false);
    }
  };

  const importPaste = async (event: FormEvent) => {
    event.preventDefault();
    repairDiscoveryGate.invalidate();
    const request = sourceRequestGate.begin();
    setPendingSourceRepair(null);
    setError(null);
    setIsDiscovering(true);
    try {
      const { source, syncRunId, detectedKind } = await invokeCommand('ingest_pasted_content', {
        input: {
          title: pasteTitle,
          content: pasteContent,
          declaredMediaType: null,
        },
      });
      if (!sourceRequestGate.isCurrent(request)) return;
      registerSourceStart(source, syncRunId, detectedKind);
      setPasteContent('');
      setPasteOpen(false);
      setConnectorOpen(false);
    } catch (reason) {
      if (!sourceRequestGate.isCurrent(request)) return;
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      if (sourceRequestGate.isCurrent(request)) setIsDiscovering(false);
    }
  };

  const retryWithGuidance = async (event: FormEvent) => {
    event.preventDefault();
    if (!repairRootUrl || !repairSeedUrl.trim()) return;
    const settings = resolveCrawlSettings();
    if (!settings) return;
    const filters = resolveGuidanceFilters();
    if (!filters) return;
    repairDiscoveryGate.invalidate();
    const request = sourceRequestGate.begin();
    setPendingSourceRepair(null);
    setError(null);
    setIsDiscovering(true);
    try {
      const { source, syncRunId, detectedKind } = await invokeCommand('start_data_source', {
        url: repairRootUrl,
        crawlPolicy: crawlPolicyOverride(settings.crawlPolicy),
        guidance: buildRepairGuidance(filters.filters, [repairSeedUrl.trim()], repairLinkSelector, repairDownloadSelector, reuseRepairForPublisher),
        rateLimitPolicy: settings.rateLimitPolicy,
        metadataPageWindow: metadataPageWindowOverride(settings.metadataPageWindow),
      });
      if (!sourceRequestGate.isCurrent(request)) return;
      registerSourceStart(source, syncRunId, detectedKind);
      appendSyncLog(`${source.displayName}: guided connector recipe saved for future synchronizations.`);
      setRepairPreview(null);
      setPortalUrlDraft('');
      setConnectorOpen(false);
    } catch (reason) {
      if (!sourceRequestGate.isCurrent(request)) return;
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      if (sourceRequestGate.isCurrent(request)) setIsDiscovering(false);
    }
  };

  const previewRepairSelectors = async () => {
    if (!repairRootUrl || (!repairLinkSelector.trim() && !repairDownloadSelector.trim())) return;
    const filters = resolveGuidanceFilters();
    if (!filters) return;
    const request = repairDiscoveryGate.begin();
    setIsPreviewingSelectors(true);
    setError(null);
    try {
      const preview = await invokeCommand('preview_data_source', {
        url: repairRootUrl,
        guidance: buildRepairGuidance(filters.filters, [], repairLinkSelector, repairDownloadSelector, false),
      });
      if (!repairDiscoveryGate.isCurrent(request)) return;
      setRepairPreview(preview);
      setRepairSeedUrl(preview.candidateLinks.find((link) => link.downloadCandidate)?.url ?? preview.candidateLinks[0]?.url ?? repairSeedUrl);
    } catch (reason) {
      if (repairDiscoveryGate.isCurrent(request)) setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      if (repairDiscoveryGate.isCurrent(request)) setIsPreviewingSelectors(false);
    }
  };

  // Every handler below captures the source it was invoked for and re-checks it before writing.
  // These calls outlive a click: pausing San Diego and immediately selecting Oakland would
  // otherwise let San Diego's result rewrite Oakland's state, and this application states things
  // about evidence.
  const pauseSync = async () => {
    const source = activePortal;
    if (!source) return;
    const syncRunId = syncRunIdsByPortal[source.portalKey];
    setError(null);
    try {
      await invokeCommand('pause_source_sync', { sourceKey: source.portalKey });
      setPortalSyncState(source.portalKey, false, syncRunId);
      setPortalLifecycleState(source.portalKey, 'PAUSED');
      appendSyncLog(
        `${source.displayName}: paused. Queued work is kept, so resuming continues rather than restarting.`,
      );
    } catch (reason) {
      setError(String(reason));
    }
  };

  const resumeSync = async () => {
    const source = activePortal;
    if (!source) return;
    setError(null);
    try {
      const { syncRunId } = await invokeCommand('resume_source_sync', {
        sourceKey: source.portalKey,
      });
      setPortalLifecycleState(source.portalKey, 'ACTIVE');
      setPortalSyncState(source.portalKey, true, syncRunId);
      appendSyncLog(`${source.displayName}: resumed from the queue its pause left behind.`);
    } catch (reason) {
      setError(String(reason));
    }
  };

  // The summary names its source, so it stays true in the shared log whichever source is selected
  // when it arrives.
  const recheckSource = async () => {
    const source = activePortal;
    if (!source || recheckingKeys.has(source.portalKey)) return;
    setError(null);
    setRecheckingKeys((keys) => new Set(keys).add(source.portalKey));
    appendSyncLog(`${source.displayName}: fetching collected documents again to compare them with what was collected.`);
    try {
      const summary = await invokeCommand('recheck_source', {
        sourceKey: source.portalKey,
        maxDocuments: RECHECK_DOCUMENTS_PER_RUN,
      });
      appendSyncLog(describeRecheck(source.displayName, summary));
    } catch (reason) {
      appendSyncLog(`${source.displayName}: source check failed: ${formatNativeError(reason)}`);
    } finally {
      setRecheckingKeys((keys) => {
        const next = new Set(keys);
        next.delete(source.portalKey);
        return next;
      });
    }
  };

  // Off unless chosen. The schedule runs only while the app is open, and the backend returns the
  // saved sources, so what the control shows is what was stored.
  const setRecheckSchedule = async (intervalDays: number | null) => {
    const source = activePortal;
    if (!source) return;
    setError(null);
    try {
      setPortals(await invokeCommand('set_source_recheck_schedule', { sourceKey: source.portalKey, intervalDays }));
      appendSyncLog(intervalDays === null
        ? `${source.displayName}: scheduled source checks are off.`
        : `${source.displayName}: while the app is open, collected documents are checked against the source every ${intervalDays === 1 ? 'day' : `${intervalDays} days`}.`);
    } catch (reason) {
      setError(formatNativeError(reason));
    }
  };

  const removeSource = async () => {
    // The source the confirmation was opened for, not whichever one is selected when it resolves.
    const source = pendingRemoval;
    if (!source) return;
    const syncRunId = syncRunIdsByPortal[source.portalKey];
    setError(null);
    try {
      const removal = await invokeCommand('remove_source', { sourceKey: source.portalKey });
      // These reflect a removal that has already happened on the backend, so they apply
      // unconditionally regardless of what the operator is looking at by the time this resolves.
      setPortalSyncState(source.portalKey, false, syncRunId);
      removePortalFromWorkspace(source.portalKey);
      requestEvidenceRefresh();
      requestFindingsRefresh();
      requestAnalyticsRefresh();
      appendSyncLog(
        `${source.displayName}: removed. ${removal.retainedDocuments.toLocaleString()} ingested ` +
          `document${removal.retainedDocuments === 1 ? '' : 's'} kept; ` +
          `${removal.discardedFrontierTasks.toLocaleString()} queued task` +
          `${removal.discardedFrontierTasks === 1 ? '' : 's'} discarded.`,
      );
      // Only dismiss the confirmation if it is still showing for the source this removal was
      // requested for. The operator may have already closed it and opened a different source's
      // confirmation, which must not vanish out from under them.
      setPendingRemoval((current) => (current?.portalKey === source.portalKey ? null : current));
    } catch (reason) {
      setError(String(reason));
    }
  };

  // The count fetch outlives the dialog it was opened for. Opening the purge confirmation for San
  // Diego, cancelling, and opening it for Oakland would otherwise render San Diego's counts under
  // Oakland's heading, and the operator would approve a destruction against numbers belonging to a
  // different source. Proven by `sourcePurge.test.tsx`, which removes this guard to fail.
  useEffect(() => {
    if (!pendingPurge) {
      purgePreviewGate.invalidate();
      return;
    }
    const request = purgePreviewGate.begin();
    setPurgePreview(null);
    setPurgePreviewError(null);
    void invokeCommand('preview_source_purge', { sourceKey: pendingPurge.portalKey })
      .then((preview) => {
        if (purgePreviewGate.isCurrent(request)) setPurgePreview(preview);
      })
      .catch((reason) => {
        if (purgePreviewGate.isCurrent(request)) {
          setPurgePreviewError(reason instanceof Error ? reason.message : String(reason));
        }
      });
    return purgePreviewGate.invalidate;
  }, [pendingPurge, purgePreviewGate]);

  const purgeSource = async () => {
    // The source the confirmation was opened for, for the same reason `removeSource` captures it.
    const source = pendingPurge;
    if (!source) return;
    const syncRunId = syncRunIdsByPortal[source.portalKey];
    setError(null);
    setIsPurging(true);
    try {
      const purge = await invokeCommand('purge_source', { sourceKey: source.portalKey });
      // These reflect a destruction that has already happened on the backend, so they apply
      // unconditionally regardless of what the operator is looking at by the time this resolves.
      setPortalSyncState(source.portalKey, false, syncRunId);
      removePortalFromWorkspace(source.portalKey);
      requestEvidenceRefresh();
      requestFindingsRefresh();
      requestAnalyticsRefresh();
      // The counts reported here are what the purge actually destroyed, which is the
      // authoritative figure. The preview's counts were a forecast and can differ if the crawl
      // finalized a document between the two calls.
      appendSyncLog(
        `${source.displayName}: purged. ${purge.documentsRemoved.toLocaleString()} document` +
          `${purge.documentsRemoved === 1 ? '' : 's'} and ` +
          `${purge.requestsRemoved.toLocaleString()} request` +
          `${purge.requestsRemoved === 1 ? '' : 's'} destroyed, ` +
          `${purge.localPaths.length.toLocaleString()} evidence file` +
          `${purge.localPaths.length === 1 ? '' : 's'} deleted. ` +
          'The registration is gone, so this source can be added again from scratch.',
      );
      // Only dismiss the confirmation if it is still showing for the source this purge was
      // requested for. The operator may have already closed it and opened a different source's
      // confirmation, which must not vanish out from under them.
      setPendingPurge((current) => (current?.portalKey === source.portalKey ? null : current));
    } catch (reason) {
      setError(String(reason));
    } finally {
      setIsPurging(false);
    }
  };

  return {
    portals,
    portalUrlDraft,
    setPortalUrlDraft,
    syncingPortalKeys,
    isDiscovering,
    isConnectorOpen,
    setConnectorOpen,
    error,
    setError,
    isPasteOpen,
    setPasteOpen,
    pendingRemoval,
    setPendingRemoval,
    pendingPurge,
    setPendingPurge,
    purgePreview,
    purgePreviewError,
    isPurging,
    pasteTitle,
    setPasteTitle,
    pasteContent,
    setPasteContent,
    repairPreview,
    repairLinkSelector,
    setRepairLinkSelector,
    repairDownloadSelector,
    setRepairDownloadSelector,
    repairSeedUrl,
    setRepairSeedUrl,
    reuseRepairForPublisher,
    setReuseRepairForPublisher,
    isPreviewingSelectors,
    selectedPreset,
    isCrawlSettingsOpen,
    setCrawlSettingsOpen,
    crawlDraft,
    setCrawlDraft,
    effectiveCrawlDraft,
    crawlSettings,
    crawlSettingsAreCustom,
    updateCrawlDraft,
    guidanceDraft,
    setGuidanceDraft,
    guidanceFilters,
    hasGuidanceFilters,
    updateGuidanceDraft,
    activePortal,
    activeIsPaused,
    activeIsSyncing,
    syncCapacityReached,
    selectedSource,
    cancelSourceOperation,
    closePasteDialog,
    closeRepairDialog,
    startSync,
    chooseFiles,
    importPaste,
    retryWithGuidance,
    previewRepairSelectors,
    pauseSync,
    resumeSync,
    recheckSource,
    activeIsRechecking: activePortal ? recheckingKeys.has(activePortal.portalKey) : false,
    activeRecheckIntervalDays: activeSource?.recheckIntervalDays ?? null,
    setRecheckSchedule,
    removeSource,
    purgeSource,
  };
}

export type SourceConnectorController = ReturnType<typeof useSourceConnector>;

const RECHECK_DOCUMENTS_PER_RUN = 100;

/** States only what the fetches observed: a different digest is not proof of alteration, and an
 * unreachable document is not evidence of removal. */
export function describeRecheck(sourceName: string, summary: SourceRecheckSummary): string {
  const checked = summary.sameBytes + summary.differentBytes + summary.gone + summary.unreachable;
  const parts = [
    `${summary.sameBytes} served the same bytes`,
    `${summary.differentBytes} served different bytes (a changed file, or a page that replaced it)`,
    `${summary.gone} answered not found or gone (HTTP 404/410)`,
    `${summary.unreachable} could not be reached, which says nothing about the record`,
  ];
  const remainder = summary.notChecked > 0
    ? ` ${summary.notChecked} collected documents were not checked in this run; run it again to continue.`
    : '';
  const stopped = summary.stoppedEarly ? ` Stopped early: ${summary.stoppedEarly}.` : '';
  return `${sourceName}: checked ${checked} documents against the source. ${parts.join('; ')}.${remainder}${stopped}`;
}
