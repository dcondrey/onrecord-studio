import { cn } from '../../utils/cn';
import DialogBackdrop from '../ui/DialogBackdrop';
import Icon from '../ui/icons';
import {
  DEFAULT_CRAWL_SETTINGS_DRAFT,
  type CrawlSettingsDraft,
} from '../../utils/crawlPolicy';
import { DEFAULT_GUIDANCE_FILTER_DRAFT, type GuidanceFilterDraft } from '../../utils/sourceGuidance';
import type { SourceConnectorController } from '../../hooks/useSourceConnector';

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

// Only a portal with a metadata API reads these; a crawled site or catalog ignores them.
const metadataPageFields: CrawlNumberField[] = [
  { field: 'metadataStartPage', label: 'Metadata start page', hint: 'First page of the request index to fetch. Earlier pages are not read.', min: 1, max: 50_000, step: 1 },
  { field: 'metadataPageSize', label: 'Metadata page size', hint: 'Requests fetched per index page.', min: 1, max: 100, step: 1 },
];

interface GuidanceListField {
  field: Exclude<
    keyof GuidanceFilterDraft,
    | 'minimumFileSizeBytes'
    | 'maximumFileSizeBytes'
    | 'documentDateFrom'
    | 'documentDateTo'
    | 'restrictDownloadsToSourceHost'
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

/** The five modals behind the connector: remove confirmation, purge confirmation, paste-content,
 * crawl settings, and guided repair. Grouped in one component because at most one is ever open at
 * a time and they all read from the same `useSourceConnector` state. */
export function SourceConnectorDialogs({ connector }: { connector: SourceConnectorController }) {
  const {
    pendingRemoval,
    setPendingRemoval,
    removeSource,
    pendingPurge,
    setPendingPurge,
    purgePreview,
    purgePreviewError,
    isPurging,
    purgeSource,
    syncingPortalKeys,
    isPasteOpen,
    closePasteDialog,
    importPaste,
    pasteTitle,
    setPasteTitle,
    reuseRepairForPublisher,
    setReuseRepairForPublisher,
    pasteContent,
    setPasteContent,
    isDiscovering,
    isCrawlSettingsOpen,
    setCrawlSettingsOpen,
    selectedPreset,
    effectiveCrawlDraft,
    updateCrawlDraft,
    crawlSettings,
    crawlSettingsAreCustom,
    setCrawlDraft,
    crawlDraft,
    repairPreview,
    closeRepairDialog,
    retryWithGuidance,
    repairLinkSelector,
    setRepairLinkSelector,
    repairDownloadSelector,
    setRepairDownloadSelector,
    previewRepairSelectors,
    isPreviewingSelectors,
    repairSeedUrl,
    setRepairSeedUrl,
    hasGuidanceFilters,
    guidanceDraft,
    updateGuidanceDraft,
    guidanceFilters,
    setGuidanceDraft,
  } = connector;

  return (
    <>
      {pendingRemoval && (
        <DialogBackdrop labelledBy="remove-source-title" onClose={() => setPendingRemoval(null)} className="app-backdrop fixed inset-0 z-[100] grid place-items-center p-4 backdrop-blur-sm">
          <div className="app-panel w-full max-w-md overflow-hidden shadow-2xl shadow-black/40">
            <div className="flex items-start justify-between border-b border-line p-4">
              <div>
                <div className="app-eyebrow text-danger">Sources</div>
                <h2 id="remove-source-title" className="mt-1 text-sm font-semibold text-fg">Remove {pendingRemoval.displayName}?</h2>
              </div>
              <button type="button" onClick={() => setPendingRemoval(null)} aria-label="Close removal confirmation" className="rounded-lg p-2 text-muted hover:bg-raised hover:text-fg-secondary"><Icon name="x" /></button>
            </div>
            <div className="space-y-2 p-4 text-label leading-relaxed text-muted">
              {/* Saying which of the two this is, is the point. A "Remove" that silently keeps
                  hundreds of ingested records and one that silently destroys them are both false
                  statements about the evidence if the interface does not distinguish them. */}
              <p>Stops synchronizing it and takes it off the source list. The queued crawl and the learned connector recipe are discarded.</p>
              <p className="text-fg-secondary">Documents already ingested from this source are kept and stay searchable and exportable.</p>
            </div>
            <div className="flex items-center justify-end gap-2 border-t border-line px-4 py-3">
              <button type="button" onClick={() => setPendingRemoval(null)} className="rounded-lg px-3 py-2 text-caption font-semibold text-muted hover:text-fg-secondary">Cancel</button>
              <button type="button" onClick={() => void removeSource()} className="focus-ring rounded-lg border border-danger/30 bg-danger/10 px-4 py-2 text-caption font-semibold text-danger">Remove and keep documents</button>
            </div>
          </div>
        </DialogBackdrop>
      )}
      {pendingPurge && (
        <DialogBackdrop labelledBy="purge-source-title" onClose={() => setPendingPurge(null)} className="app-backdrop fixed inset-0 z-[100] grid place-items-center p-4 backdrop-blur-sm">
          <div className="app-panel w-full max-w-md overflow-hidden shadow-2xl shadow-black/40">
            <div className="flex items-start justify-between border-b border-line p-4">
              <div>
                <div className="app-eyebrow text-danger">Sources</div>
                <h2 id="purge-source-title" className="mt-1 text-sm font-semibold text-fg">Delete {pendingPurge.displayName} and everything from it?</h2>
              </div>
              <button type="button" onClick={() => setPendingPurge(null)} aria-label="Close deletion confirmation" className="rounded-lg p-2 text-muted hover:bg-raised hover:text-fg-secondary"><Icon name="x" /></button>
            </div>
            <div className="space-y-2 p-4 text-label leading-relaxed text-muted">
              {/* The counts are the point. "This cannot be undone" asks the operator to approve an
                  unknown quantity of evidence loss; the numbers say what is at stake. Until they
                  arrive the confirming button stays disabled rather than defaulting to zero, because
                  a zero here would read as "nothing to lose". */}
              {purgePreviewError ? (
                <p role="alert" className="text-danger">Could not count what this would destroy: {purgePreviewError}. Nothing has been deleted.</p>
              ) : purgePreview === null ? (
                <p>Counting what this would destroy…</p>
              ) : (
                <p className="text-fg-secondary">
                  Destroys <span className="numeric-value font-semibold text-danger">{purgePreview.documentCount.toLocaleString()}</span> ingested document{purgePreview.documentCount === 1 ? '' : 's'},{' '}
                  <span className="numeric-value font-semibold text-danger">{purgePreview.requestCount.toLocaleString()}</span> request record{purgePreview.requestCount === 1 ? '' : 's'}, and{' '}
                  <span className="numeric-value font-semibold text-danger">{purgePreview.evidenceFileCount.toLocaleString()}</span> downloaded evidence file{purgePreview.evidenceFileCount === 1 ? '' : 's'} on disk, along with every search, provenance, and compliance record derived from them.
                </p>
              )}
              <p>This cannot be undone from inside the application. To stop synchronizing but keep the evidence, close this and use Remove source instead.</p>
              {syncingPortalKeys.includes(pendingPurge.portalKey) && (
                <p className="text-warning">This source is still synchronizing. Its crawl is stopped before anything is deleted, but a document it finalizes first is deleted too, so the final count can exceed the one above.</p>
              )}
              <p className="text-fg-secondary">The registration is deleted as well, so the source can be added again from scratch afterwards.</p>
            </div>
            <div className="flex items-center justify-end gap-2 border-t border-line px-4 py-3">
              <button type="button" onClick={() => setPendingPurge(null)} className="rounded-lg px-3 py-2 text-caption font-semibold text-muted hover:text-fg-secondary">Cancel</button>
              <button type="button" disabled={purgePreview === null || isPurging} onClick={() => void purgeSource()} className="focus-ring rounded-lg border border-danger/30 bg-danger/20 px-4 py-2 text-caption font-semibold text-danger disabled:cursor-not-allowed disabled:border-control disabled:bg-surface disabled:text-muted">{isPurging ? 'Deleting…' : 'Delete source and documents'}</button>
            </div>
          </div>
        </DialogBackdrop>
      )}
      {isPasteOpen && (
        <DialogBackdrop labelledBy="paste-source-title" onClose={closePasteDialog} className="app-backdrop fixed inset-0 z-[100] grid place-items-center p-4 backdrop-blur-sm">
          <form onSubmit={(event) => void importPaste(event)} className="app-panel w-full max-w-2xl overflow-hidden shadow-2xl shadow-black/40">
            <div className="flex items-start justify-between border-b border-line p-4">
              <div>
                <div className="app-eyebrow text-accent">Add data</div>
                <h2 id="paste-source-title" className="mt-1 text-sm font-semibold text-fg">Paste content to analyze</h2>
                <p className="mt-1 text-caption text-muted">Text, CSV, JSON, XML, HTML, and tabular content are detected automatically. Paste a press release, agenda, minutes or budget message to check its figures against the released records.</p>
              </div>
              <button type="button" onClick={closePasteDialog} aria-label="Close paste dialog" className="rounded-lg p-2 text-muted hover:bg-raised hover:text-fg-secondary"><Icon name="x" /></button>
            </div>
            <div className="space-y-3 p-4">
              <label className="block text-caption font-semibold text-muted">
                Source title
                <input value={pasteTitle} onChange={(event) => setPasteTitle(event.target.value)} maxLength={256} className="focus-ring mt-1.5 h-10 w-full rounded-xl border border-control bg-canvas/70 px-3 text-label text-fg" />
              </label>
              <label className="flex items-start gap-2 rounded-xl border border-line bg-canvas/40 p-3">
                <input type="checkbox" checked={reuseRepairForPublisher} onChange={(event) => setReuseRepairForPublisher(event.target.checked)} className="focus-ring mt-0.5 h-3.5 w-3.5 rounded border-control bg-canvas accent-accent" />
                <span><span className="block text-caption font-semibold text-muted">Approve for this publisher hostname</span><span className="mt-1 block text-caption leading-relaxed text-muted">Reuse this mapping for a different source root only when its normalized hostname is exactly the same. Leave off for a source-specific layout.</span></span>
              </label>
              <label className="block text-caption font-semibold text-muted">
                Content
                <textarea value={pasteContent} onChange={(event) => setPasteContent(event.target.value)} autoFocus rows={14} placeholder="Paste the source material here…" className="focus-ring mt-1.5 w-full resize-y rounded-xl border border-control bg-canvas/70 p-3 font-mono text-label leading-relaxed text-fg-secondary placeholder:text-muted" />
              </label>
            </div>
            <div className="flex items-center justify-between border-t border-line px-4 py-3">
              <span className="font-mono text-caption text-muted">{new Blob([pasteContent]).size.toLocaleString()} / 16,777,216 bytes</span>
              <div className="flex gap-2">
                <button type="button" onClick={closePasteDialog} className="rounded-lg px-3 py-2 text-caption font-semibold text-muted hover:text-fg-secondary">Cancel</button>
                <button type="submit" disabled={isDiscovering || !pasteContent.trim() || !pasteTitle.trim()} className="focus-ring rounded-lg border border-accent/30 bg-accent/10 px-4 py-2 text-caption font-semibold text-accent disabled:opacity-40">{isDiscovering ? 'Analyzing…' : 'Add & analyze'}</button>
              </div>
            </div>
          </form>
        </DialogBackdrop>
      )}
      {isCrawlSettingsOpen && (
        <DialogBackdrop labelledBy="crawl-settings-title" onClose={() => setCrawlSettingsOpen(false)} className="app-backdrop fixed inset-0 z-[100] grid place-items-center p-4 backdrop-blur-sm">
          <div className="app-panel w-full max-w-2xl overflow-hidden shadow-2xl shadow-black/40">
            <div className="flex items-start justify-between border-b border-line p-4">
              <div>
                <div className="app-eyebrow text-accent">Acquisition</div>
                <h2 id="crawl-settings-title" className="mt-1 text-sm font-semibold text-fg">Crawl settings</h2>
                <p className="mt-1 max-w-xl text-caption leading-relaxed text-muted">Applied to the next source you add, and saved with that source when it differs from the defaults. Crawl limits govern crawled websites and catalogs; request pacing governs every network source; the metadata page window governs portals with a request API, and a non-default window is remembered so a resume continues from it.</p>
              </div>
              <button type="button" onClick={() => setCrawlSettingsOpen(false)} aria-label="Close crawl settings" className="rounded-lg p-2 text-muted hover:bg-raised hover:text-fg-secondary"><Icon name="x" /></button>
            </div>
            <div className="space-y-3 p-4">
              <div className="grid gap-3 sm:grid-cols-3">
                {selectedPreset && <p className="text-label text-accent sm:col-span-3">{selectedPreset.label}: {selectedPreset.coverage}</p>}
                {crawlNumberFields.map(({ field, label, hint, min, max, step }) => (
                  <label key={field} className="block text-caption font-semibold text-muted">
                    {label}
                    <input
                      type="number"
                      inputMode="decimal"
                      min={min}
                      max={max}
                      step={step}
                      value={effectiveCrawlDraft[field]}
                      onChange={(event) => updateCrawlDraft({ [field]: event.target.value })}
                      className="focus-ring mt-1.5 h-10 w-full rounded-xl border border-control bg-canvas/70 px-3 font-mono text-label text-fg"
                    />
                    <span className="mt-1 block text-caption font-normal leading-relaxed text-muted">{hint}</span>
                  </label>
                ))}
              </div>
              <label className="flex items-start gap-2.5 rounded-xl border border-line bg-canvas/70 p-3">
                <input
                  type="checkbox"
                  checked={effectiveCrawlDraft.followSupportedExternalPlatforms}
                  onChange={(event) => updateCrawlDraft({ followSupportedExternalPlatforms: event.target.checked })}
                  className="focus-ring mt-0.5 h-3.5 w-3.5 shrink-0 rounded border-control bg-canvas accent-accent"
                />
                <span>
                  <span className="block text-caption font-semibold text-muted">Follow supported external platforms</span>
                  <span className="mt-1 block text-caption leading-relaxed text-muted">Follows links off the source host into recognized document platforms such as hosted folders and drives.</span>
                </span>
              </label>
              <div className="grid gap-3 sm:grid-cols-3">
                {metadataPageFields.map(({ field, label, hint, min, max, step }) => (
                  <label key={field} className="block text-caption font-semibold text-muted">
                    {label}
                    <input
                      type="number"
                      inputMode="numeric"
                      min={min}
                      max={max}
                      step={step}
                      value={crawlDraft[field]}
                      onChange={(event) => updateCrawlDraft({ [field]: event.target.value })}
                      className="focus-ring mt-1.5 h-10 w-full rounded-xl border border-control bg-canvas/70 px-3 font-mono text-label text-fg"
                    />
                    <span className="mt-1 block text-caption font-normal leading-relaxed text-muted">{hint}</span>
                  </label>
                ))}
              </div>
              {!crawlSettings.ok && <p role="alert" className="text-caption text-danger">{crawlSettings.problem}</p>}
            </div>
            <div className="flex items-center justify-between border-t border-line px-4 py-3">
              <button type="button" onClick={() => setCrawlDraft(DEFAULT_CRAWL_SETTINGS_DRAFT)} disabled={!crawlSettingsAreCustom} className="rounded-lg px-3 py-2 text-caption font-semibold text-muted hover:text-fg-secondary disabled:opacity-40">Reset to defaults</button>
              <button type="button" onClick={() => setCrawlSettingsOpen(false)} disabled={!crawlSettings.ok} className="focus-ring rounded-lg border border-accent/30 bg-accent/10 px-4 py-2 text-caption font-semibold text-accent disabled:opacity-40">Done</button>
            </div>
          </div>
        </DialogBackdrop>
      )}
      {repairPreview && (
        <DialogBackdrop labelledBy="repair-source-title" onClose={closeRepairDialog} className="app-backdrop fixed inset-0 z-[100] grid place-items-center p-4 backdrop-blur-sm">
          <form onSubmit={(event) => void retryWithGuidance(event)} className="app-panel flex max-h-[calc(100vh-2rem)] w-full max-w-3xl flex-col overflow-hidden shadow-2xl shadow-black/40">
            <div className="flex items-start justify-between border-b border-line p-4">
              <div>
                <div className="app-eyebrow text-warning">Guided repair</div>
                <h2 id="repair-source-title" className="mt-1 text-sm font-semibold text-fg">Show OnRecord where the data lives</h2>
                <p className="mt-1 max-w-xl text-caption leading-relaxed text-muted">The site is reachable, but it did not expose a reliable static data link. Paste the underlying download, folder, catalog, dashboard, or API URL. This guidance is saved and reused automatically.</p>
              </div>
              <button type="button" onClick={closeRepairDialog} aria-label="Close repair dialog" className="rounded-lg p-2 text-muted hover:bg-raised hover:text-fg-secondary"><Icon name="x" /></button>
            </div>
            <div className="space-y-4 overflow-y-auto p-4">
              <div className="rounded-xl border border-line bg-canvas/70 p-3">
                <div className="flex items-center justify-between gap-3">
                  <span className="text-caption font-semibold text-muted">Detected {repairPreview.detectedKind.replace(/_/g, ' ').toLocaleLowerCase()}</span>
                  <span className="font-mono text-caption text-muted">{(repairPreview.confidenceBasisPoints / 100).toFixed(0)}% confidence</span>
                </div>
                {repairPreview.warnings.map((warning) => <p key={warning} className="mt-2 text-caption leading-relaxed text-warning">{warning}</p>)}
              </div>
              {repairPreview.candidateLinks.length > 0 && (
                <div>
                  <div className="mb-1.5 text-caption font-bold uppercase tracking-wider text-muted">Possible data links</div>
                  <div className="max-h-40 space-y-1 overflow-y-auto">
                    {repairPreview.candidateLinks.slice(0, 30).map((link) => (
                      <button key={link.url} type="button" onClick={() => setRepairSeedUrl(link.url)} className={cn('block w-full truncate rounded-lg border px-3 py-2 text-left text-caption transition', repairSeedUrl === link.url ? 'border-accent/30 bg-accent/10 text-accent' : 'border-control text-muted hover:text-fg-secondary')} title={link.url}>
                        {link.label || link.url}
                      </button>
                    ))}
                  </div>
                </div>
              )}
              <details className="rounded-xl border border-line bg-canvas/40" open={repairPreview.selectorPreviewApplied || undefined}>
                <summary className="focus-ring cursor-pointer list-none rounded-xl px-3 py-2.5 text-caption font-semibold text-muted">CSS selector mapping <span className="ml-2 font-normal text-muted">Preview exact matches before saving</span></summary>
                <div className="space-y-3 border-t border-line p-3"><div className="grid gap-3 sm:grid-cols-2"><label className="text-caption font-semibold text-muted">Link selector<input value={repairLinkSelector} onChange={(event) => setRepairLinkSelector(event.target.value)} placeholder="main a[href]" className="focus-ring mt-1.5 h-10 w-full rounded-xl border border-control bg-canvas/70 px-3 font-mono text-label text-fg" /></label><label className="text-caption font-semibold text-muted">Forced-download selector<input value={repairDownloadSelector} onChange={(event) => setRepairDownloadSelector(event.target.value)} placeholder="a.download[href]" className="focus-ring mt-1.5 h-10 w-full rounded-xl border border-control bg-canvas/70 px-3 font-mono text-label text-fg" /></label></div><div className="flex items-center justify-between"><p className="text-caption text-muted">Selectors are parsed natively; invalid CSS is rejected and no recipe is persisted by previewing.</p><button type="button" onClick={() => void previewRepairSelectors()} disabled={isPreviewingSelectors || (!repairLinkSelector.trim() && !repairDownloadSelector.trim())} className="rounded-lg border border-highlight/30 bg-highlight/10 px-3 py-2 text-caption font-semibold text-highlight disabled:opacity-40">{isPreviewingSelectors ? 'Matching…' : 'Preview selector matches'}</button></div>{repairPreview.selectorPreviewApplied && <p role="status" className="text-caption text-success">Selector preview returned {repairPreview.candidateLinks.length.toLocaleString()} matched resource links. Choose one above or refine the selectors.</p>}</div>
              </details>
              <label className="block text-caption font-semibold text-muted">
                Data, folder, dashboard, or API URL
                <input type="url" required value={repairSeedUrl} onChange={(event) => setRepairSeedUrl(event.target.value)} placeholder="https://…" className="focus-ring mt-1.5 h-10 w-full rounded-xl border border-control bg-canvas/70 px-3 text-label text-fg" />
              </label>
              <details className="rounded-xl border border-line bg-canvas/40">
                <summary className="focus-ring flex cursor-pointer list-none items-center justify-between gap-3 rounded-xl px-3 py-2.5 text-caption font-semibold text-muted">
                  <span>
                    Scope imported documents
                    <span className="ml-2 font-normal text-muted">Optional filename, content, agency, size, and date rules</span>
                  </span>
                  {hasGuidanceFilters && <span className="rounded-full bg-accent/10 px-2 py-0.5 text-caption text-accent">Customized</span>}
                </summary>
                <div className="space-y-4 border-t border-line p-3">
                  <p className="text-caption leading-relaxed text-muted">
                    Include rules admit a document when any entry in that field matches. Exclude rules take precedence. Simple lists accept lines or commas; regular expressions use one line each.
                  </p>
                  <div className="grid gap-3 sm:grid-cols-2">
                    {guidanceListFields.map(({ field, label, hint }) => (
                      <label key={field} className="block text-caption font-semibold text-muted">
                        {label}
                        <textarea
                          rows={2}
                          value={guidanceDraft[field]}
                          onChange={(event) => updateGuidanceDraft({ [field]: event.target.value })}
                          placeholder="One rule per line"
                          className="focus-ring mt-1.5 w-full resize-y rounded-xl border border-control bg-canvas/70 p-2.5 font-mono text-label leading-relaxed text-fg placeholder:text-muted"
                        />
                        <span className="mt-1 block text-caption font-normal leading-relaxed text-muted">{hint}</span>
                      </label>
                    ))}
                  </div>
                  <div className="grid gap-3 sm:grid-cols-2">
                    {guidanceSizeFields.map(({ field, label, hint }) => (
                      <label key={field} className="block text-caption font-semibold text-muted">
                        {label}
                        <input
                          type="number"
                          inputMode="numeric"
                          min={0}
                          step={1}
                          value={guidanceDraft[field]}
                          onChange={(event) => updateGuidanceDraft({ [field]: event.target.value })}
                          className="focus-ring mt-1.5 h-10 w-full rounded-xl border border-control bg-canvas/70 px-3 font-mono text-label text-fg"
                        />
                        <span className="mt-1 block text-caption font-normal leading-relaxed text-muted">{hint}</span>
                      </label>
                    ))}
                    {guidanceDateFields.map(({ field, label, hint }) => (
                      <label key={field} className="block text-caption font-semibold text-muted">
                        {label}
                        <input
                          type="date"
                          value={guidanceDraft[field]}
                          onChange={(event) => updateGuidanceDraft({ [field]: event.target.value })}
                          className="focus-ring mt-1.5 h-10 w-full rounded-xl border border-control bg-canvas/70 px-3 font-mono text-label text-fg"
                        />
                        <span className="mt-1 block text-caption font-normal leading-relaxed text-muted">{hint}</span>
                      </label>
                    ))}
                  </div>
                  <label className="flex items-start gap-2.5 rounded-xl border border-line bg-canvas/40 p-3 text-caption font-semibold text-muted">
                    <input
                      type="checkbox"
                      checked={guidanceDraft.restrictDownloadsToSourceHost}
                      onChange={(event) => updateGuidanceDraft({ restrictDownloadsToSourceHost: event.target.checked })}
                      className="focus-ring mt-0.5 h-3.5 w-3.5 shrink-0"
                    />
                    <span>
                      Download only from this source&apos;s own host
                      <span className="mt-1 block text-caption font-normal leading-relaxed text-muted">
                        Off by default. Many government portals serve their documents from S3, Azure Blob, Granicus or
                        CivicPlus, and turning this on refuses those. Hosts listed in the source&apos;s allowed hostnames stay
                        permitted, and which links get followed is unaffected either way.
                      </span>
                    </span>
                  </label>
                  {!guidanceFilters.ok && <p role="alert" className="text-caption text-danger">{guidanceFilters.problem}</p>}
                  <div className="flex justify-end">
                    <button
                      type="button"
                      onClick={() => setGuidanceDraft(DEFAULT_GUIDANCE_FILTER_DRAFT)}
                      disabled={!hasGuidanceFilters}
                      className="rounded-lg px-3 py-2 text-caption font-semibold text-muted hover:text-fg-secondary disabled:opacity-40"
                    >
                      Clear scope rules
                    </button>
                  </div>
                </div>
              </details>
            </div>
            <div className="flex justify-end gap-2 border-t border-line px-4 py-3">
              <button type="button" onClick={closeRepairDialog} className="rounded-lg px-3 py-2 text-caption font-semibold text-muted hover:text-fg-secondary">Cancel</button>
              <button type="submit" disabled={isDiscovering || !repairSeedUrl.trim() || !guidanceFilters.ok} className="focus-ring rounded-lg border border-accent/30 bg-accent/10 px-4 py-2 text-caption font-semibold text-accent disabled:opacity-40">{isDiscovering ? 'Testing…' : 'Use this data source'}</button>
            </div>
          </form>
        </DialogBackdrop>
      )}
    </>
  );
}
