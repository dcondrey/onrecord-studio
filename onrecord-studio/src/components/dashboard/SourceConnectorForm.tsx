import { cn } from '../../utils/cn';
import Icon from '../ui/icons';
import { Select } from '../ui/select';
import { BUILT_IN_SOURCE_OPTIONS } from '../../types';
import type { SourceConnectorController } from '../../hooks/useSourceConnector';

/** The inline "add a data source" form. Rendered once for the desktop layout and once, behind a
 * toggle, for the mobile layout — hence the distinct `inputId` per instance. */
export function SourceConnectorForm({ connector, inputId }: { connector: SourceConnectorController; inputId: string }) {
  const {
    portals,
    portalUrlDraft,
    setPortalUrlDraft,
    error,
    setError,
    isDiscovering,
    selectedSource,
    chooseFiles,
    crawlSettingsAreCustom,
    setCrawlSettingsOpen,
    setPasteOpen,
    startSync,
    syncCapacityReached,
    activeIsSyncing,
    activePortal,
    pauseSync,
    activeIsPaused,
    resumeSync,
    setPendingRemoval,
    setPendingPurge,
  } = connector;

  return (
    <form onSubmit={(event) => void startSync(event)} className="source-composer flex min-w-0 flex-1 items-center p-1">
      <Select
        aria-label="Built-in source connector"
        title={selectedSource.description}
        value={selectedSource.id}
        onChange={(event) => {
          const next = BUILT_IN_SOURCE_OPTIONS.find((source) => source.id === event.target.value) ?? BUILT_IN_SOURCE_OPTIONS[0];
          setPortalUrlDraft(next.url ?? '');
          if (error) setError(null);
        }}
        disabled={isDiscovering}
        className="h-8 w-44 border-0 bg-transparent px-2 text-label text-muted shadow-none"
      >
        <optgroup label="San Diego sources">
          {BUILT_IN_SOURCE_OPTIONS.map((source) => <option key={source.id} value={source.id}>{source.label}</option>)}
        </optgroup>
      </Select>
      <label htmlFor={inputId} className="sr-only">Public data source URL</label>
      <div className="relative min-w-0 flex-1 self-stretch">
        <Icon name="globe" className="pointer-events-none absolute left-2.5 top-1/2 h-4 w-4 -translate-y-1/2 text-muted" />
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
          className="h-full w-full rounded-lg border-0 bg-transparent pl-8 pr-2 text-label text-fg outline-none placeholder:text-muted"
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
        id={`${inputId}-import-files`}
        disabled={isDiscovering}
        aria-label="Add files"
        title="Add files"
        className="source-tool-button focus-ring grid h-8 w-8 shrink-0 place-items-center rounded-lg text-muted transition hover:text-highlight disabled:opacity-50"
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
        className="source-tool-button focus-ring relative grid h-8 w-8 shrink-0 place-items-center rounded-lg text-muted transition hover:text-highlight disabled:opacity-50"
      >
        <Icon name="sliders" className="h-4 w-4" />
        {crawlSettingsAreCustom && (
          <span aria-hidden="true" className="absolute right-1 top-1 h-1.5 w-1.5 rounded-full bg-accent" />
        )}
      </button>
      <button
        type="button"
        onClick={() => setPasteOpen(true)}
        disabled={isDiscovering}
        aria-label="Paste content"
        title="Paste content"
        className="source-tool-button focus-ring grid h-8 w-8 shrink-0 place-items-center rounded-lg text-muted transition hover:text-highlight disabled:opacity-50"
      >
        <Icon name="clipboard" className="h-4 w-4" />
      </button>
      <button
        type="submit"
        disabled={isDiscovering || syncCapacityReached || (activeIsSyncing && !portalUrlDraft.trim())}
        className={cn(
          'focus-ring flex h-8 shrink-0 items-center justify-center gap-1.5 rounded-lg border px-3 text-label font-semibold transition',
          isDiscovering || syncCapacityReached || (activeIsSyncing && !portalUrlDraft.trim())
            ? 'border-control bg-surface text-muted'
            : 'primary-action border-transparent text-white hover:brightness-105',
        )}
      >
        <Icon name="sync" className={cn('h-3.5 w-3.5', isDiscovering && 'animate-spin')} />
        <span>{isDiscovering ? 'Discovering' : activeIsSyncing && !portalUrlDraft.trim() ? 'Syncing' : 'Add'}</span>
      </button>
      {activeIsSyncing && (
        <button type="button" onClick={() => void pauseSync()} title="Pause synchronization" aria-label="Pause synchronization" className="source-tool-button focus-ring grid h-8 w-8 shrink-0 place-items-center rounded-lg text-muted transition hover:text-warning">
          <Icon name="pause" className="h-3.5 w-3.5" />
        </button>
      )}
      {activeIsPaused && !activeIsSyncing && (
        <button type="button" onClick={() => void resumeSync()} title="Resume synchronization" aria-label="Resume synchronization" className="source-tool-button focus-ring grid h-8 w-8 shrink-0 place-items-center rounded-lg text-warning transition hover:text-success">
          <Icon name="play" className="h-3.5 w-3.5" />
        </button>
      )}
      {/* Two controls, because they are two operations. The withdrawal keeps the evidence and so
          does not carry the trash glyph; the purge destroys it and does. A single button with a
          "also delete the documents" checkbox would put the difference between keeping and
          erasing evidence one stray click away. */}
      {activePortal && (
        <button type="button" onClick={() => setPendingRemoval(activePortal)} title="Remove source and keep its documents" aria-label="Remove source" className="source-tool-button focus-ring grid h-8 w-8 shrink-0 place-items-center rounded-lg text-muted transition hover:text-warning">
          <Icon name="eject" className="h-3.5 w-3.5" />
        </button>
      )}
      {activePortal && (
        <button type="button" onClick={() => setPendingPurge(activePortal)} title="Delete source and every document ingested from it" aria-label="Delete source and its documents" className="source-tool-button focus-ring grid h-8 w-8 shrink-0 place-items-center rounded-lg text-muted transition hover:text-danger">
          <Icon name="trash" className="h-3.5 w-3.5" />
        </button>
      )}
    </form>
  );
}
