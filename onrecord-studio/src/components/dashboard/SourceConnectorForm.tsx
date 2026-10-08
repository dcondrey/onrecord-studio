import { useEffect, useId, useRef, useState, type KeyboardEvent } from 'react';
import { cn } from '../../utils/cn';
import Icon from '../ui/icons';
import { Select } from '../ui/select';
import { BUILT_IN_SOURCE_OPTIONS } from '../../types';
import Button from '../ui/Button';
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
  } = connector;
  const addDisabled = isDiscovering || syncCapacityReached || (activeIsSyncing && !portalUrlDraft.trim());

  return (
    <form onSubmit={(event) => void startSync(event)} className="source-composer flex h-8 min-w-0 flex-1 items-center gap-1 px-px">
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
        size="sm"
        className="w-32 border-0 2xl:w-44 bg-transparent text-control text-fg-secondary shadow-none"
      >
        <optgroup label="San Diego sources">
          {/* The field beside it already asks for a URL, so the catch-all option needs no second label for it. */}
          {BUILT_IN_SOURCE_OPTIONS.map((source) => <option key={source.id} value={source.id}>{source.id === 'auto' ? 'Any source' : source.label}</option>)}
        </optgroup>
      </Select>
      <label htmlFor={inputId} className="sr-only">Public data source URL</label>
      <div className="relative h-7 min-w-[5rem] flex-1">
        <Icon name="globe" size="sm" className="pointer-events-none absolute left-2 top-1/2 -translate-y-1/2 text-muted" />
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
          className="h-full w-full rounded-md border-0 bg-transparent pl-7 pr-2 text-control text-fg placeholder:text-muted"
        />
        <datalist id={`${inputId}-saved-sources`}>
          {portals.map(({ registration }) => (
            <option key={registration.portalKey} value={registration.canonicalUrl}>
              {registration.displayName}
            </option>
          ))}
        </datalist>
      </div>
      <span className="sr-only" role="status">{selectedSource.description}</span>
      {/* The label names the action only. Whether a source is syncing is shown on its sidebar row. */}
      <Button
        type="submit"
        variant="primary"
        size="sm"
        disabled={addDisabled}
        title={syncCapacityReached ? 'Five sources are syncing; add another when one finishes.' : undefined}
      >
        {isDiscovering ? 'Discovering…' : 'Add'}
      </Button>
      <SourceToolsMenu
        items={[
          { label: 'Add files', disabled: isDiscovering, onSelect: () => void chooseFiles() },
          { label: 'Crawl settings', hint: crawlSettingsAreCustom ? 'Customized' : undefined, disabled: isDiscovering, onSelect: () => setCrawlSettingsOpen(true) },
          { label: 'Paste content', disabled: isDiscovering, onSelect: () => setPasteOpen(true) },
        ]}
        flagged={crawlSettingsAreCustom}
      />
    </form>
  );
}

interface SourceTool {
  label: string;
  hint?: string;
  disabled?: boolean;
  onSelect: () => void;
}

/** The per-source tools behind one "More" button, so the composer stays a single row. */
function SourceToolsMenu({ items, flagged }: { items: SourceTool[]; flagged: boolean }) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const menuId = useId();

  const enabledItems = () => Array.from(menu.current?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]:not(:disabled)') ?? []);

  const close = (returnFocus: boolean) => {
    setOpen(false);
    if (returnFocus) trigger.current?.focus();
  };

  useEffect(() => {
    if (!open) return;
    menu.current?.querySelector<HTMLButtonElement>('[role="menuitem"]:not(:disabled)')?.focus();
    const onPointer = (event: PointerEvent) => {
      if (!root.current?.contains(event.target as Node)) setOpen(false);
    };
    window.addEventListener('pointerdown', onPointer);
    return () => window.removeEventListener('pointerdown', onPointer);
  }, [open]);

  const onMenuKey = (event: KeyboardEvent<HTMLDivElement>) => {
    const list = enabledItems();
    const index = list.indexOf(document.activeElement as HTMLButtonElement);
    const move = (next: number) => { event.preventDefault(); list[(next + list.length) % list.length]?.focus(); };
    if (event.key === 'ArrowDown') move(index + 1);
    else if (event.key === 'ArrowUp') move(index - 1);
    else if (event.key === 'Home') move(0);
    else if (event.key === 'End') move(list.length - 1);
    else if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); close(true); }
    else if (event.key === 'Tab') setOpen(false);
  };

  return (
    <div ref={root} className="relative shrink-0">
      <button
        ref={trigger}
        type="button"
        aria-label="Source options"
        title="Source options"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? menuId : undefined}
        onClick={() => setOpen((current) => !current)}
        onKeyDown={(event) => {
          if (event.key === 'ArrowDown' && !open) { event.preventDefault(); setOpen(true); }
        }}
        className="source-tool-button focus-ring relative grid h-7 w-7 place-items-center rounded-md text-muted transition-colors hover:bg-raised hover:text-fg aria-expanded:bg-raised aria-expanded:text-fg"
      >
        <svg aria-hidden="true" viewBox="0 0 16 16" className="h-3.5 w-3.5" fill="currentColor">
          <circle cx="3" cy="8" r="1.5" />
          <circle cx="8" cy="8" r="1.5" />
          <circle cx="13" cy="8" r="1.5" />
        </svg>
        {flagged && <span aria-hidden="true" className="absolute right-1 top-1 h-1.5 w-1.5 rounded-full bg-accent" />}
      </button>
      {open && (
        <div
          ref={menu}
          id={menuId}
          role="menu"
          aria-label="Source options"
          onKeyDown={onMenuKey}
          className="motion-pop absolute right-0 top-full z-popover mt-2 w-72 rounded-xl border border-line-strong bg-panel p-1 shadow-popover"
        >
          {items.map((item, index) => (
            <button
              key={item.label}
              type="button"
              role="menuitem"
              aria-label={item.label}
              aria-describedby={item.hint ? `${menuId}-hint-${index}` : undefined}
              disabled={item.disabled}
              onClick={() => { close(true); item.onSelect(); }}
              className={cn(
                'focus-ring flex h-8 w-full items-center justify-between gap-3 rounded-md px-2.5 text-left text-control transition-colors disabled:opacity-50',
                'text-fg hover:bg-raised focus-visible:bg-raised',
              )}
            >
              <span className="truncate">{item.label}</span>
              {item.hint && <span id={`${menuId}-hint-${index}`} className="shrink-0 text-caption text-muted">{item.hint}</span>}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
