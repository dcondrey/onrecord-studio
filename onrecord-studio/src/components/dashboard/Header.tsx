import { type FormEvent, useEffect, useMemo, useState } from 'react';
import { invokeCommand } from '../../ipc';
import { open } from '@tauri-apps/plugin-dialog';
import { useAppStore } from '../../store/useAppStore';
import { cn } from '../../utils/cn';
import {
  crawlPolicyOverride,
  DEFAULT_CRAWL_SETTINGS_DRAFT,
  parseCrawlSettings,
  type CrawlSettingsDraft,
} from '../../utils/crawlPolicy';
import {
  DEFAULT_GUIDANCE_FILTER_DRAFT,
  guidanceFiltersAreCustom,
  parseGuidanceFilters,
  type GuidanceFilterDraft,
} from '../../utils/sourceGuidance';
import Icon from '../ui/icons';
import { Select } from '../ui/select';
import { BUILT_IN_SOURCE_OPTIONS, type PortalRegistration, type SourceDiscoveryPreview, type SourceKind } from '../../types';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';

const viewLabels = {
  overview: 'Intelligence overview',
  patterns: 'Corpus connections',
  evidence: 'Evidence workspace',
  activity: 'Ingestion activity',
};

interface CrawlNumberField {
  field: Exclude<keyof CrawlSettingsDraft, 'followSupportedExternalPlatforms'>;
  label: string;
  hint: string;
  min: number;
  max: number;
  step: number;
}

// Ranges match the backend validators; see src/utils/crawlPolicy.ts for where they come from.
const crawlNumberFields: CrawlNumberField[] = [
  { field: 'maximumDepth', label: 'Crawl depth', hint: 'Link hops from the starting page. 0 fetches only the seed URLs.', min: 0, max: 32, step: 1 },
  { field: 'maximumPages', label: 'Page limit', hint: 'Pages fetched per source before the crawl stops.', min: 1, max: 100_000, step: 1 },
  { field: 'maximumResources', label: 'Document limit', hint: 'Documents queued per source before the crawl stops.', min: 1, max: 1_000_000, step: 1 },
  { field: 'burstCapacity', label: 'Burst capacity', hint: 'Requests allowed back to back before pacing applies.', min: 1, max: 100, step: 1 },
  { field: 'refillTokensPerSecond', label: 'Requests per second', hint: 'Sustained request rate. Fractional values are allowed.', min: 0.001, max: 100, step: 0.1 },
];

interface GuidanceListField {
  field: Exclude<
    keyof GuidanceFilterDraft,
    'minimumFileSizeBytes' | 'maximumFileSizeBytes' | 'documentDateFrom' | 'documentDateTo'
  >;
  label: string;
  hint: string;
}

// Mirrors the `FrontierExclusion` rules this guidance drives (src-tauri/src/ingestion/queue.rs).
// Each field is one rule per line, or comma-separated.
const guidanceListFields: GuidanceListField[] = [
  { field: 'includeFilenameGlobs', label: 'Filename include globs', hint: 'Only admits a discovered file whose name matches one of these globs, e.g. *.pdf' },
  { field: 'excludeFilenameGlobs', label: 'Filename exclude globs', hint: 'Refuses a discovered file whose name matches one of these globs.' },
  { field: 'includeFilenamePatterns', label: 'Filename include patterns', hint: 'Only admits a filename matching one of these regular expressions. Use one expression per line.' },
  { field: 'excludeFilenamePatterns', label: 'Filename exclude patterns', hint: 'Refuses a filename matching one of these regular expressions. Use one expression per line.' },
  { field: 'includeContentKeywords', label: 'Content include keywords', hint: 'Only admits a document whose extracted text contains one of these words. Checked after extraction.' },
  { field: 'excludeContentKeywords', label: 'Content exclude keywords', hint: 'Refuses a document whose extracted text contains one of these words.' },
  { field: 'includeContentPatterns', label: 'Content include patterns', hint: 'Only admits a document whose extracted text matches one of these regular expressions. Use one expression per line.' },
  { field: 'excludeContentPatterns', label: 'Content exclude patterns', hint: 'Refuses a document whose extracted text matches one of these regular expressions. Use one expression per line.' },
  { field: 'includeOrganizations', label: 'Organization include', hint: 'Only admits a document from a request whose agency name contains one of these fragments.' },
  { field: 'excludeOrganizations', label: 'Organization exclude', hint: 'Refuses a document from a request whose agency name contains one of these fragments.' },
];

const guidanceSizeFields: { field: 'minimumFileSizeBytes' | 'maximumFileSizeBytes'; label: string; hint: string }[] = [
  { field: 'minimumFileSizeBytes', label: 'Minimum file size (bytes)', hint: 'Refuses a document smaller than this, when its size is known before download.' },
  { field: 'maximumFileSizeBytes', label: 'Maximum file size (bytes)', hint: 'Refuses a document larger than this, when its size is known before download.' },
];

const guidanceDateFields: { field: 'documentDateFrom' | 'documentDateTo'; label: string; hint: string }[] = [
  { field: 'documentDateFrom', label: 'Document date from', hint: 'Refuses a document dated earlier than this when the source reports a document-carried date.' },
  { field: 'documentDateTo', label: 'Document date to', hint: 'Refuses a document dated later than this when the source reports a document-carried date.' },
];

export default function Header() {
  const portals = useAppStore((state) => state.portals);
  const activePortalKey = useAppStore((state) => state.activePortalKey);
  const portalUrlDraft = useAppStore((state) => state.portalUrlDraft);
  const syncingPortalKeys = useAppStore((state) => state.syncingPortalKeys);
  const syncRunIdsByPortal = useAppStore((state) => state.syncRunIdsByPortal);
  const workspaceView = useAppStore((state) => state.workspaceView);
  const theme = useAppStore((state) => state.theme);
  const audioAlertsEnabled = useAppStore((state) => state.audioAlertsEnabled);
  const setPortalUrlDraft = useAppStore((state) => state.setPortalUrlDraft);
  const setPortalSyncState = useAppStore((state) => state.setPortalSyncState);
  const setActivePortal = useAppStore((state) => state.setActivePortal);
  const upsertPortal = useAppStore((state) => state.upsertPortal);
  const setPortalLifecycleState = useAppStore((state) => state.setPortalLifecycleState);
  const removePortalFromWorkspace = useAppStore((state) => state.removePortal);
  const appendSyncLog = useAppStore((state) => state.appendSyncLog);
  const setSidebarOpen = useAppStore((state) => state.setSidebarOpen);
  const setTheme = useAppStore((state) => state.setTheme);
  const setAudioAlertsEnabled = useAppStore((state) => state.setAudioAlertsEnabled);
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
  const [pasteTitle, setPasteTitle] = useState('Pasted research material');
  const [pasteContent, setPasteContent] = useState('');
  const [repairPreview, setRepairPreview] = useState<SourceDiscoveryPreview | null>(null);
  const [repairRootUrl, setRepairRootUrl] = useState('');
  const [repairSeedUrl, setRepairSeedUrl] = useState('');
  const [selectedSourceId, setSelectedSourceId] = useState('auto');
  const [isCrawlSettingsOpen, setCrawlSettingsOpen] = useState(false);
  const [crawlDraft, setCrawlDraft] = useState<CrawlSettingsDraft>(DEFAULT_CRAWL_SETTINGS_DRAFT);
  const crawlSettings = useMemo(() => parseCrawlSettings(crawlDraft), [crawlDraft]);
  const crawlSettingsAreCustom = useMemo(
    () =>
      (Object.keys(DEFAULT_CRAWL_SETTINGS_DRAFT) as (keyof CrawlSettingsDraft)[]).some(
        (field) => crawlDraft[field] !== DEFAULT_CRAWL_SETTINGS_DRAFT[field],
      ),
    [crawlDraft],
  );
  const updateCrawlDraft = (patch: Partial<CrawlSettingsDraft>) =>
    setCrawlDraft((draft) => ({ ...draft, ...patch }));
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
      const { source, syncRunId, detectedKind } = await invokeCommand('start_data_source', {
        url: targetUrl,
        crawlPolicy: crawlPolicyOverride(settings.crawlPolicy),
        guidance: null,
        rateLimitPolicy: settings.rateLimitPolicy,
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
        guidance: {
          additionalSeedUrls: [repairSeedUrl.trim()],
          allowedHostnames: [],
          includeUrlPrefixes: [],
          excludeUrlPrefixes: [],
          linkSelector: null,
          downloadSelector: null,
          ...filters.filters,
        },
        rateLimitPolicy: settings.rateLimitPolicy,
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

  const removeSource = async () => {
    // The source the confirmation was opened for, not whichever one is selected when it resolves.
    const source = pendingRemoval;
    if (!source) return;
    const syncRunId = syncRunIdsByPortal[source.portalKey];
    setError(null);
    try {
      const removal = await invokeCommand('remove_source', { sourceKey: source.portalKey });
      setPortalSyncState(source.portalKey, false, syncRunId);
      removePortalFromWorkspace(source.portalKey);
      appendSyncLog(
        `${source.displayName}: removed. ${removal.retainedDocuments.toLocaleString()} ingested ` +
          `document${removal.retainedDocuments === 1 ? '' : 's'} kept; ` +
          `${removal.discardedFrontierTasks.toLocaleString()} queued task` +
          `${removal.discardedFrontierTasks === 1 ? '' : 's'} discarded.`,
      );
      setPendingRemoval(null);
    } catch (reason) {
      setError(String(reason));
    }
  };

  const connectorForm = (inputId: string) => (
    <form onSubmit={(event) => void startSync(event)} className="source-composer flex min-w-0 flex-1 items-center p-1">
      <Select
        aria-label="Built-in source connector"
        title={selectedSource.description}
        value={selectedSource.id}
        onChange={(event) => {
          const next = BUILT_IN_SOURCE_OPTIONS.find((source) => source.id === event.target.value) ?? BUILT_IN_SOURCE_OPTIONS[0];
          setSelectedSourceId(next.id);
          if (error) setError(null);
        }}
        disabled={isDiscovering}
        className="h-8 w-44 border-0 bg-transparent px-2 text-[10px] text-slate-400 shadow-none"
      >
        <optgroup label="Built-in connectors">
          {BUILT_IN_SOURCE_OPTIONS.map((source) => <option key={source.id} value={source.id}>{source.label}</option>)}
        </optgroup>
      </Select>
      <label htmlFor={inputId} className="sr-only">Public data source URL</label>
      <div className="relative min-w-0 flex-1 self-stretch">
        <Icon name="globe" className="pointer-events-none absolute left-2.5 top-1/2 h-4 w-4 -translate-y-1/2 text-slate-500" />
        <input
          id={inputId}
          list={`${inputId}-saved-sources`}
          type="url"
          inputMode="url"
          value={portalUrlDraft}
          onChange={(event) => {
            setPortalUrlDraft(event.target.value);
            if (error) setError(null);
          }}
          placeholder={selectedSource.placeholder}
          autoComplete="url"
          className="h-full w-full rounded-lg border-0 bg-transparent pl-8 pr-2 text-[11px] text-slate-100 outline-none placeholder:text-slate-600"
        />
        <datalist id={`${inputId}-saved-sources`}>
          {portals.map(({ registration }) => (
            <option key={registration.portalKey} value={registration.canonicalUrl}>
              {registration.displayName}
            </option>
          ))}
        </datalist>
      </div>
      <span className="source-composer-divider" aria-hidden="true" />
      <button
        type="button"
        onClick={() => void chooseFiles()}
        disabled={isDiscovering}
        aria-label="Add files"
        title="Add files"
        className="source-tool-button focus-ring grid h-8 w-8 shrink-0 place-items-center rounded-lg text-slate-500 transition hover:text-indigo-200 disabled:opacity-50"
      >
        <Icon name="filePlus" className="h-4 w-4" />
      </button>
      <span className="sr-only" role="status">{selectedSource.description}</span>
      <button
        type="button"
        onClick={() => setCrawlSettingsOpen(true)}
        disabled={isDiscovering}
        aria-label="Crawl settings"
        title={crawlSettingsAreCustom ? 'Crawl settings (customized)' : 'Crawl settings'}
        className="source-tool-button focus-ring relative grid h-8 w-8 shrink-0 place-items-center rounded-lg text-slate-500 transition hover:text-indigo-200 disabled:opacity-50"
      >
        <Icon name="sliders" className="h-4 w-4" />
        {crawlSettingsAreCustom && (
          <span aria-hidden="true" className="absolute right-1 top-1 h-1.5 w-1.5 rounded-full bg-cyan-300" />
        )}
      </button>
      <button
        type="button"
        onClick={() => setPasteOpen(true)}
        disabled={isDiscovering}
        aria-label="Paste content"
        title="Paste content"
        className="source-tool-button focus-ring grid h-8 w-8 shrink-0 place-items-center rounded-lg text-slate-500 transition hover:text-indigo-200 disabled:opacity-50"
      >
        <Icon name="clipboard" className="h-4 w-4" />
      </button>
      <button
        type="submit"
        disabled={isDiscovering || syncCapacityReached || (activeIsSyncing && !portalUrlDraft.trim())}
        className={cn(
          'focus-ring flex h-8 shrink-0 items-center justify-center gap-1.5 rounded-lg border px-3 text-[10px] font-semibold transition',
          isDiscovering || syncCapacityReached || (activeIsSyncing && !portalUrlDraft.trim())
            ? 'cursor-not-allowed border-slate-800 bg-slate-900 text-slate-600'
            : 'primary-action border-transparent text-white hover:brightness-105',
        )}
      >
        <Icon name="sync" className={cn('h-3.5 w-3.5', isDiscovering && 'animate-spin')} />
        <span>{isDiscovering ? 'Discovering' : activeIsSyncing && !portalUrlDraft.trim() ? 'Syncing' : 'Add'}</span>
      </button>
      {activeIsSyncing && (
        <button type="button" onClick={() => void pauseSync()} title="Pause synchronization" aria-label="Pause synchronization" className="source-tool-button focus-ring grid h-8 w-8 shrink-0 place-items-center rounded-lg text-slate-500 transition hover:text-amber-300">
          <Icon name="pause" className="h-3.5 w-3.5" />
        </button>
      )}
      {activeIsPaused && !activeIsSyncing && (
        <button type="button" onClick={() => void resumeSync()} title="Resume synchronization" aria-label="Resume synchronization" className="source-tool-button focus-ring grid h-8 w-8 shrink-0 place-items-center rounded-lg text-amber-300 transition hover:text-emerald-300">
          <Icon name="play" className="h-3.5 w-3.5" />
        </button>
      )}
      {activePortal && (
        <button type="button" onClick={() => setPendingRemoval(activePortal)} title="Remove source" aria-label="Remove source" className="source-tool-button focus-ring grid h-8 w-8 shrink-0 place-items-center rounded-lg text-slate-500 transition hover:text-rose-300">
          <Icon name="trash" className="h-3.5 w-3.5" />
        </button>
      )}
    </form>
  );

  return (
    <>
    <header className="header-surface relative z-30 shrink-0 border-b backdrop-blur-xl">
      <div className="flex min-h-[68px] items-center gap-3 px-4 sm:px-5 lg:px-6">
        <button
          type="button"
          aria-label="Open navigation"
          onClick={() => setSidebarOpen(true)}
          className="focus-ring -ml-1 rounded-xl border border-slate-800 bg-slate-900/50 p-2.5 text-slate-400 hover:text-slate-100 md:hidden"
        >
          <Icon name="menu" />
        </button>

        <div className="min-w-0 flex-1 lg:w-[210px] lg:flex-none xl:w-[270px]">
          <div className="flex items-center gap-2 text-[9px] font-medium text-slate-600">
            <span>{viewLabels[workspaceView]}</span>
            <Icon name="chevron" className="h-2.5 w-2.5" />
            <span className="truncate text-slate-500">{activePortal?.displayName ?? 'All jurisdictions'}</span>
          </div>
          <div className="mt-1 flex min-w-0 items-center gap-2">
            <h1 className="truncate text-[15px] font-semibold tracking-[-0.015em] text-slate-100">
              {activePortal?.displayName ?? 'Cross-portal investigation'}
            </h1>
            {activePortal && (
              <span className="hidden shrink-0 items-center gap-1 rounded-full border border-emerald-500/20 bg-emerald-500/[0.08] px-2 py-0.5 text-[8px] font-semibold uppercase tracking-wider text-emerald-300 sm:flex">
                <Icon name="shield" className="h-2.5 w-2.5" /> Verified
              </span>
            )}
          </div>
        </div>

        <div className="hidden min-w-[300px] max-w-[720px] flex-1 lg:flex">{connectorForm('portal-url-desktop')}</div>

        <div className="ml-auto flex shrink-0 items-center gap-2">
          <button
            type="button"
            title={audioAlertsEnabled ? 'Disable completion and failure sounds' : 'Enable completion and failure sounds'}
            aria-label={audioAlertsEnabled ? 'Disable audio alerts' : 'Enable audio alerts'}
            aria-pressed={audioAlertsEnabled}
            onClick={() => setAudioAlertsEnabled(!audioAlertsEnabled)}
            className={cn('focus-ring grid h-9 w-9 place-items-center rounded-xl border transition', audioAlertsEnabled ? 'border-cyan-400/30 bg-cyan-400/10 text-cyan-300' : 'border-slate-800 bg-slate-900/50 text-slate-500 hover:text-slate-300')}
          >
            <Icon name="volume" className="h-3.5 w-3.5" />
          </button>
          <div className="theme-switcher flex items-center rounded-xl border p-0.5" role="group" aria-label="Appearance">
            <button
              type="button"
              title="Light appearance"
              aria-label="Use light appearance"
              aria-pressed={theme === 'light'}
              onClick={() => setTheme('light')}
              className={cn('focus-ring grid h-8 w-8 place-items-center rounded-[9px] transition', theme === 'light' ? 'theme-switcher-active' : 'text-slate-500 hover:text-slate-300')}
            >
              <Icon name="sun" className="h-3.5 w-3.5" />
            </button>
            <button
              type="button"
              title="Dark appearance"
              aria-label="Use dark appearance"
              aria-pressed={theme === 'dark'}
              onClick={() => setTheme('dark')}
              className={cn('focus-ring grid h-8 w-8 place-items-center rounded-[9px] transition', theme === 'dark' ? 'theme-switcher-active' : 'text-slate-500 hover:text-slate-300')}
            >
              <Icon name="moon" className="h-3.5 w-3.5" />
            </button>
            <button
              type="button"
              title="Automatic appearance"
              aria-label="Follow system appearance"
              aria-pressed={theme === 'auto'}
              onClick={() => setTheme('auto')}
              className={cn('focus-ring grid h-8 w-8 place-items-center rounded-[9px] transition', theme === 'auto' ? 'theme-switcher-active' : 'text-slate-500 hover:text-slate-300')}
            >
              <Icon name="system" className="h-3.5 w-3.5" />
            </button>
          </div>
          <div className="system-status hidden h-9 items-center gap-2 rounded-xl border px-2.5 text-[9px] sm:flex" role="status" title={syncingPortalKeys.length > 0 ? `${syncingPortalKeys.length} sources synchronizing` : 'All systems ready'}>
            <span className={cn('grid h-5 w-5 place-items-center rounded-md', syncingPortalKeys.length > 0 ? 'status-pulse bg-emerald-400/10 text-emerald-300' : 'bg-emerald-400/10 text-emerald-300')}>
              <Icon name={syncingPortalKeys.length > 0 ? 'sync' : 'shield'} className={cn('h-3.5 w-3.5', syncingPortalKeys.length > 0 && 'animate-spin')} />
            </span>
            <span className="hidden text-slate-500 2xl:inline">{syncingPortalKeys.length > 0 ? `${syncingPortalKeys.length} source${syncingPortalKeys.length === 1 ? '' : 's'} live` : 'System ready'}</span>
          </div>
          <button
            type="button"
            aria-label="Connect a data source"
            onClick={() => {
              if (isConnectorOpen) cancelSourceOperation();
              setConnectorOpen((open) => !open);
            }}
            className={cn('focus-ring rounded-xl border p-2.5 transition lg:hidden', isConnectorOpen ? 'border-cyan-400/30 bg-cyan-400/10 text-cyan-300' : 'border-slate-800 bg-slate-900/50 text-slate-400 hover:text-slate-100')}
          >
            <Icon name="database" />
          </button>
        </div>
      </div>
      {isConnectorOpen && <div className="border-t border-slate-800/70 px-4 py-3 lg:hidden">{connectorForm('portal-url-mobile')}</div>}
      {error && (
        <div role="alert" className="border-t border-rose-500/10 bg-rose-500/[0.05] px-5 py-2 text-right text-[9px] text-rose-300">
          {error}
        </div>
      )}
    </header>
    {pendingRemoval && (
      <div className="app-backdrop fixed inset-0 z-[100] grid place-items-center p-4 backdrop-blur-sm" role="dialog" aria-modal="true" aria-labelledby="remove-source-title">
        <div className="app-panel w-full max-w-md overflow-hidden shadow-2xl shadow-black/40">
          <div className="flex items-start justify-between border-b border-slate-800 p-4">
            <div>
              <div className="app-eyebrow text-rose-300/70">Sources</div>
              <h2 id="remove-source-title" className="mt-1 text-sm font-semibold text-slate-100">Remove {pendingRemoval.displayName}?</h2>
            </div>
            <button type="button" onClick={() => setPendingRemoval(null)} aria-label="Close removal confirmation" className="rounded-lg p-2 text-slate-600 hover:bg-slate-800 hover:text-slate-300"><Icon name="x" /></button>
          </div>
          <div className="space-y-2 p-4 text-[10px] leading-relaxed text-slate-400">
            {/* Saying which of the two this is, is the point. A "Remove" that silently keeps
                hundreds of ingested records and one that silently destroys them are both false
                statements about the evidence if the interface does not distinguish them. */}
            <p>Stops synchronizing it and takes it off the source list. The queued crawl and the learned connector recipe are discarded.</p>
            <p className="text-slate-300">Documents already ingested from this source are kept and stay searchable and exportable.</p>
          </div>
          <div className="flex items-center justify-end gap-2 border-t border-slate-800 px-4 py-3">
            <button type="button" onClick={() => setPendingRemoval(null)} className="rounded-lg px-3 py-2 text-[9px] font-semibold text-slate-500 hover:text-slate-300">Cancel</button>
            <button type="button" onClick={() => void removeSource()} className="focus-ring rounded-lg border border-rose-400/25 bg-rose-500/10 px-4 py-2 text-[9px] font-semibold text-rose-200">Remove and keep documents</button>
          </div>
        </div>
      </div>
    )}
    {isPasteOpen && (
      <div className="app-backdrop fixed inset-0 z-[100] grid place-items-center p-4 backdrop-blur-sm" role="dialog" aria-modal="true" aria-labelledby="paste-source-title">
        <form onSubmit={(event) => void importPaste(event)} className="app-panel w-full max-w-2xl overflow-hidden shadow-2xl shadow-black/40">
          <div className="flex items-start justify-between border-b border-slate-800 p-4">
            <div>
              <div className="app-eyebrow text-cyan-300/70">Add data</div>
              <h2 id="paste-source-title" className="mt-1 text-sm font-semibold text-slate-100">Paste content to analyze</h2>
              <p className="mt-1 text-[9px] text-slate-600">Text, CSV, JSON, XML, HTML, and tabular content are detected automatically.</p>
            </div>
            <button type="button" onClick={closePasteDialog} aria-label="Close paste dialog" className="rounded-lg p-2 text-slate-600 hover:bg-slate-800 hover:text-slate-300"><Icon name="x" /></button>
          </div>
          <div className="space-y-3 p-4">
            <label className="block text-[9px] font-semibold text-slate-500">
              Source title
              <input value={pasteTitle} onChange={(event) => setPasteTitle(event.target.value)} maxLength={256} className="focus-ring mt-1.5 h-10 w-full rounded-xl border border-slate-800 bg-slate-950/70 px-3 text-[11px] text-slate-200" />
            </label>
            <label className="block text-[9px] font-semibold text-slate-500">
              Content
              <textarea value={pasteContent} onChange={(event) => setPasteContent(event.target.value)} autoFocus rows={14} placeholder="Paste the source material here…" className="focus-ring mt-1.5 w-full resize-y rounded-xl border border-slate-800 bg-slate-950/70 p-3 font-mono text-[10px] leading-relaxed text-slate-300 placeholder:text-slate-700" />
            </label>
          </div>
          <div className="flex items-center justify-between border-t border-slate-800 px-4 py-3">
            <span className="font-mono text-[8px] text-slate-600">{new Blob([pasteContent]).size.toLocaleString()} / 16,777,216 bytes</span>
            <div className="flex gap-2">
              <button type="button" onClick={closePasteDialog} className="rounded-lg px-3 py-2 text-[9px] font-semibold text-slate-500 hover:text-slate-300">Cancel</button>
              <button type="submit" disabled={isDiscovering || !pasteContent.trim() || !pasteTitle.trim()} className="focus-ring rounded-lg border border-cyan-400/25 bg-cyan-500/10 px-4 py-2 text-[9px] font-semibold text-cyan-200 disabled:opacity-40">{isDiscovering ? 'Analyzing…' : 'Add & analyze'}</button>
            </div>
          </div>
        </form>
      </div>
    )}
    {isCrawlSettingsOpen && (
      <div className="app-backdrop fixed inset-0 z-[100] grid place-items-center p-4 backdrop-blur-sm" role="dialog" aria-modal="true" aria-labelledby="crawl-settings-title">
        <div className="app-panel w-full max-w-2xl overflow-hidden shadow-2xl shadow-black/40">
          <div className="flex items-start justify-between border-b border-slate-800 p-4">
            <div>
              <div className="app-eyebrow text-cyan-300/70">Acquisition</div>
              <h2 id="crawl-settings-title" className="mt-1 text-sm font-semibold text-slate-100">Crawl settings</h2>
              <p className="mt-1 max-w-xl text-[9px] leading-relaxed text-slate-600">Applied to the next source you add, and saved with that source when it differs from the defaults. Crawl limits govern crawled websites and catalogs; request pacing governs every network source.</p>
            </div>
            <button type="button" onClick={() => setCrawlSettingsOpen(false)} aria-label="Close crawl settings" className="rounded-lg p-2 text-slate-600 hover:bg-slate-800 hover:text-slate-300"><Icon name="x" /></button>
          </div>
          <div className="space-y-3 p-4">
            <div className="grid gap-3 sm:grid-cols-3">
              {crawlNumberFields.map(({ field, label, hint, min, max, step }) => (
                <label key={field} className="block text-[9px] font-semibold text-slate-500">
                  {label}
                  <input
                    type="number"
                    inputMode="decimal"
                    min={min}
                    max={max}
                    step={step}
                    value={crawlDraft[field]}
                    onChange={(event) => updateCrawlDraft({ [field]: event.target.value })}
                    className="focus-ring mt-1.5 h-10 w-full rounded-xl border border-slate-800 bg-slate-950/70 px-3 font-mono text-[11px] text-slate-200"
                  />
                  <span className="mt-1 block text-[8px] font-normal leading-relaxed text-slate-600">{hint}</span>
                </label>
              ))}
            </div>
            <label className="flex items-start gap-2.5 rounded-xl border border-slate-800 bg-slate-950/50 p-3">
              <input
                type="checkbox"
                checked={crawlDraft.followSupportedExternalPlatforms}
                onChange={(event) => updateCrawlDraft({ followSupportedExternalPlatforms: event.target.checked })}
                className="focus-ring mt-0.5 h-3.5 w-3.5 shrink-0 rounded border-slate-700 bg-slate-950 accent-cyan-400"
              />
              <span>
                <span className="block text-[9px] font-semibold text-slate-400">Follow supported external platforms</span>
                <span className="mt-1 block text-[8px] leading-relaxed text-slate-600">Follows links off the source host into recognized document platforms such as hosted folders and drives.</span>
              </span>
            </label>
            {!crawlSettings.ok && <p role="alert" className="text-[9px] text-rose-300">{crawlSettings.problem}</p>}
          </div>
          <div className="flex items-center justify-between border-t border-slate-800 px-4 py-3">
            <button type="button" onClick={() => setCrawlDraft(DEFAULT_CRAWL_SETTINGS_DRAFT)} disabled={!crawlSettingsAreCustom} className="rounded-lg px-3 py-2 text-[9px] font-semibold text-slate-500 hover:text-slate-300 disabled:opacity-40">Reset to defaults</button>
            <button type="button" onClick={() => setCrawlSettingsOpen(false)} disabled={!crawlSettings.ok} className="focus-ring rounded-lg border border-cyan-400/25 bg-cyan-500/10 px-4 py-2 text-[9px] font-semibold text-cyan-200 disabled:opacity-40">Done</button>
          </div>
        </div>
      </div>
    )}
    {repairPreview && (
      <div className="app-backdrop fixed inset-0 z-[100] grid place-items-center p-4 backdrop-blur-sm" role="dialog" aria-modal="true" aria-labelledby="repair-source-title">
        <form onSubmit={(event) => void retryWithGuidance(event)} className="app-panel flex max-h-[calc(100vh-2rem)] w-full max-w-3xl flex-col overflow-hidden shadow-2xl shadow-black/40">
          <div className="flex items-start justify-between border-b border-slate-800 p-4">
            <div>
              {/* FEATURE(source-repair): Add an interactive field-mapping preview here so users can
                  approve inferred columns/entities before committing a repaired source recipe. */}
              <div className="app-eyebrow text-amber-300/70">Guided repair</div>
              <h2 id="repair-source-title" className="mt-1 text-sm font-semibold text-slate-100">Show OnRecord where the data lives</h2>
              <p className="mt-1 max-w-xl text-[9px] leading-relaxed text-slate-600">The site is reachable, but it did not expose a reliable static data link. Paste the underlying download, folder, catalog, dashboard, or API URL. This guidance is saved and reused automatically.</p>
            </div>
            <button type="button" onClick={closeRepairDialog} aria-label="Close repair dialog" className="rounded-lg p-2 text-slate-600 hover:bg-slate-800 hover:text-slate-300"><Icon name="x" /></button>
          </div>
          <div className="space-y-4 overflow-y-auto p-4">
            <div className="rounded-xl border border-slate-800 bg-slate-950/50 p-3">
              <div className="flex items-center justify-between gap-3">
                <span className="text-[9px] font-semibold text-slate-400">Detected {repairPreview.detectedKind.replace(/_/g, ' ').toLocaleLowerCase()}</span>
                <span className="font-mono text-[8px] text-slate-600">{(repairPreview.confidenceBasisPoints / 100).toFixed(0)}% confidence</span>
              </div>
              {repairPreview.warnings.map((warning) => <p key={warning} className="mt-2 text-[9px] leading-relaxed text-amber-200/70">{warning}</p>)}
            </div>
            {repairPreview.candidateLinks.length > 0 && (
              <div>
                <div className="mb-1.5 text-[8px] font-bold uppercase tracking-wider text-slate-600">Possible data links</div>
                <div className="max-h-40 space-y-1 overflow-y-auto">
                  {repairPreview.candidateLinks.slice(0, 30).map((link) => (
                    <button key={link.url} type="button" onClick={() => setRepairSeedUrl(link.url)} className={cn('block w-full truncate rounded-lg border px-3 py-2 text-left text-[9px] transition', repairSeedUrl === link.url ? 'border-cyan-400/30 bg-cyan-400/[0.07] text-cyan-200' : 'border-slate-800 text-slate-500 hover:text-slate-300')} title={link.url}>
                      {link.label || link.url}
                    </button>
                  ))}
                </div>
              </div>
            )}
            <label className="block text-[9px] font-semibold text-slate-500">
              Data, folder, dashboard, or API URL
              <input type="url" required value={repairSeedUrl} onChange={(event) => setRepairSeedUrl(event.target.value)} placeholder="https://…" className="focus-ring mt-1.5 h-10 w-full rounded-xl border border-slate-800 bg-slate-950/70 px-3 text-[10px] text-slate-200" />
            </label>
            <details className="rounded-xl border border-slate-800 bg-slate-950/35">
              <summary className="focus-ring flex cursor-pointer list-none items-center justify-between gap-3 rounded-xl px-3 py-2.5 text-[9px] font-semibold text-slate-400">
                <span>
                  Scope imported documents
                  <span className="ml-2 font-normal text-slate-600">Optional filename, content, agency, size, and date rules</span>
                </span>
                {hasGuidanceFilters && <span className="rounded-full bg-cyan-400/10 px-2 py-0.5 text-[8px] text-cyan-300">Customized</span>}
              </summary>
              <div className="space-y-4 border-t border-slate-800 p-3">
                <p className="text-[8px] leading-relaxed text-slate-600">
                  Include rules admit a document when any entry in that field matches. Exclude rules take precedence. Simple lists accept lines or commas; regular expressions use one line each.
                </p>
                <div className="grid gap-3 sm:grid-cols-2">
                  {guidanceListFields.map(({ field, label, hint }) => (
                    <label key={field} className="block text-[9px] font-semibold text-slate-500">
                      {label}
                      <textarea
                        rows={2}
                        value={guidanceDraft[field]}
                        onChange={(event) => updateGuidanceDraft({ [field]: event.target.value })}
                        placeholder="One rule per line"
                        className="focus-ring mt-1.5 w-full resize-y rounded-xl border border-slate-800 bg-slate-950/70 p-2.5 font-mono text-[10px] leading-relaxed text-slate-200 placeholder:text-slate-700"
                      />
                      <span className="mt-1 block text-[8px] font-normal leading-relaxed text-slate-600">{hint}</span>
                    </label>
                  ))}
                </div>
                <div className="grid gap-3 sm:grid-cols-2">
                  {guidanceSizeFields.map(({ field, label, hint }) => (
                    <label key={field} className="block text-[9px] font-semibold text-slate-500">
                      {label}
                      <input
                        type="number"
                        inputMode="numeric"
                        min={0}
                        step={1}
                        value={guidanceDraft[field]}
                        onChange={(event) => updateGuidanceDraft({ [field]: event.target.value })}
                        className="focus-ring mt-1.5 h-10 w-full rounded-xl border border-slate-800 bg-slate-950/70 px-3 font-mono text-[10px] text-slate-200"
                      />
                      <span className="mt-1 block text-[8px] font-normal leading-relaxed text-slate-600">{hint}</span>
                    </label>
                  ))}
                  {guidanceDateFields.map(({ field, label, hint }) => (
                    <label key={field} className="block text-[9px] font-semibold text-slate-500">
                      {label}
                      <input
                        type="date"
                        value={guidanceDraft[field]}
                        onChange={(event) => updateGuidanceDraft({ [field]: event.target.value })}
                        className="focus-ring mt-1.5 h-10 w-full rounded-xl border border-slate-800 bg-slate-950/70 px-3 font-mono text-[10px] text-slate-200"
                      />
                      <span className="mt-1 block text-[8px] font-normal leading-relaxed text-slate-600">{hint}</span>
                    </label>
                  ))}
                </div>
                {!guidanceFilters.ok && <p role="alert" className="text-[9px] text-rose-300">{guidanceFilters.problem}</p>}
                <div className="flex justify-end">
                  <button
                    type="button"
                    onClick={() => setGuidanceDraft(DEFAULT_GUIDANCE_FILTER_DRAFT)}
                    disabled={!hasGuidanceFilters}
                    className="rounded-lg px-3 py-2 text-[9px] font-semibold text-slate-500 hover:text-slate-300 disabled:opacity-40"
                  >
                    Clear scope rules
                  </button>
                </div>
              </div>
            </details>
          </div>
          <div className="flex justify-end gap-2 border-t border-slate-800 px-4 py-3">
            <button type="button" onClick={closeRepairDialog} className="rounded-lg px-3 py-2 text-[9px] font-semibold text-slate-500 hover:text-slate-300">Cancel</button>
            <button type="submit" disabled={isDiscovering || !repairSeedUrl.trim() || !guidanceFilters.ok} className="focus-ring rounded-lg border border-cyan-400/25 bg-cyan-500/10 px-4 py-2 text-[9px] font-semibold text-cyan-200 disabled:opacity-40">{isDiscovering ? 'Testing…' : 'Use this data source'}</button>
          </div>
        </form>
      </div>
    )}
    </>
  );
}
