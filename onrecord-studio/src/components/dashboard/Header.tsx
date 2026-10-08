import { useEffect, useRef, useState, type KeyboardEvent } from 'react';
import { useAppStore } from '../../store/useAppStore';
import Button, { buttonClass } from '../ui/Button';
import Dialog, { DialogBody, DialogHeader } from '../ui/Dialog';
import { useSourceConnector, type SourceConnectorController } from '../../hooks/useSourceConnector';
import { cn } from '../../utils/cn';
import TrialBanner from '../licensing/TrialBanner';
import { HeaderIdentity } from './HeaderIdentity';
import { HeaderControls } from './HeaderControls';
import { SourceConnectorForm } from './SourceConnectorForm';
import { SourceConnectorDialogs } from './SourceConnectorDialogs';
import { OPEN_SOURCE_CONNECTOR_EVENT } from './sourceEvents';
import NavigationControls from './NavigationControls';

// openFileImport (utils/sanDiego.ts) opens the connector and then clicks `${id}-import-files`.
const CONNECTOR_INPUT_ID = 'portal-url-mobile';

export default function Header() {
  const setSidebarOpen = useAppStore((state) => state.setSidebarOpen);

  const connector = useSourceConnector();
  const { isConnectorOpen, setConnectorOpen, cancelSourceOperation, error, activePortal } = connector;

  useEffect(() => {
    const open = () => {
      setSidebarOpen(false);
      setConnectorOpen(true);
      requestAnimationFrame(() => document.getElementById(CONNECTOR_INPUT_ID)?.focus());
    };
    window.addEventListener(OPEN_SOURCE_CONNECTOR_EVENT, open);
    return () => window.removeEventListener(OPEN_SOURCE_CONNECTOR_EVENT, open);
  }, [setConnectorOpen, setSidebarOpen]);

  const closeConnector = () => {
    cancelSourceOperation();
    setConnectorOpen(false);
  };

  return (
    <>
      <header className="header-surface relative z-header shrink-0 border-b">
        <TrialBanner />
        <div className="flex h-14 items-center gap-3 px-4 md:px-6">
          <Button
            variant="ghost"
            iconOnly
            icon="menu"
            aria-label="Open navigation"
            onClick={() => setSidebarOpen(true)}
            className="-ml-1 md:hidden"
          />
          <NavigationControls />
          <div className="flex min-w-0 flex-1 items-center gap-1">
            <HeaderIdentity activePortal={activePortal} />
            <SourceActionsMenu connector={connector} />
          </div>
          <HeaderControls isConnectorOpen={isConnectorOpen} onOpenConnector={() => setConnectorOpen(true)} />
        </div>
        {error && !isConnectorOpen && (
          <div role="alert" className="border-t border-danger/30 bg-danger/10 px-4 py-2 text-caption text-danger md:px-6">
            {error}
          </div>
        )}
      </header>
      {isConnectorOpen && (
        <Dialog size="lg" onClose={closeConnector}>
          <DialogHeader
            title="Add a source"
            description="Choose a built-in San Diego source, paste a public records portal or page URL, or add files."
          />
          <DialogBody className="space-y-3">
            <div className="flex">
              <SourceConnectorForm connector={connector} inputId={CONNECTOR_INPUT_ID} />
            </div>
            {error && <p role="alert" className="text-caption text-danger">{error}</p>}
          </DialogBody>
        </Dialog>
      )}
      <SourceConnectorDialogs connector={connector} />
    </>
  );
}

/**
 * What can be done to the active source, one menu beside its name. Remove and delete stay two
 * items with their own confirmations: one keeps the evidence and one destroys it.
 */
function SourceActionsMenu({ connector }: { connector: SourceConnectorController }) {
  const { activePortal, activeIsSyncing, activeIsPaused, activeIsRechecking, pauseSync, resumeSync, recheckSource, setPendingRemoval, setPendingPurge } = connector;
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    menu.current?.querySelector<HTMLElement>('[role="menuitem"]:not([aria-disabled="true"])')?.focus();
    const onPointer = (event: PointerEvent) => {
      if (!root.current?.contains(event.target as Node)) setOpen(false);
    };
    window.addEventListener('pointerdown', onPointer);
    return () => window.removeEventListener('pointerdown', onPointer);
  }, [open]);

  if (!activePortal) return null;

  const close = (returnFocus: boolean) => {
    setOpen(false);
    if (returnFocus) trigger.current?.focus();
  };
  const choose = (action: () => void) => () => {
    close(true);
    action();
  };
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const items = Array.from(menu.current?.querySelectorAll<HTMLElement>('[role="menuitem"]') ?? []);
    const index = items.indexOf(document.activeElement as HTMLElement);
    const move = (next: number) => { event.preventDefault(); items[(next + items.length) % items.length]?.focus(); };
    if (event.key === 'ArrowDown') move(index + 1);
    else if (event.key === 'ArrowUp') move(index < 0 ? items.length - 1 : index - 1);
    else if (event.key === 'Home') move(0);
    else if (event.key === 'End') move(items.length - 1);
    else if (event.key === 'Escape' || event.key === 'Tab') { event.preventDefault(); close(true); }
  };
  const item = 'flex h-8 w-full items-center rounded-md px-2.5 text-left text-control text-fg outline-none transition-colors hover:bg-raised focus-visible:bg-raised disabled:pointer-events-none disabled:text-muted';

  return (
    <div ref={root} className="relative shrink-0">
      <button
        ref={trigger}
        type="button"
        className={buttonClass({ variant: 'ghost', iconOnly: true })}
        aria-label="Source actions"
        title={`Actions for ${activePortal.displayName}`}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((current) => !current)}
      >
        <svg viewBox="0 0 16 16" className="h-4 w-4" fill="currentColor" aria-hidden="true">
          <circle cx="3.5" cy="8" r="1.25" /><circle cx="8" cy="8" r="1.25" /><circle cx="12.5" cy="8" r="1.25" />
        </svg>
      </button>
      {open && (
        <div
          ref={menu}
          role="menu"
          aria-label={`Actions for ${activePortal.displayName}`}
          onKeyDown={onKeyDown}
          onBlur={(event) => {
            if (!root.current?.contains(event.relatedTarget as Node | null)) close(false);
          }}
          className="glass motion-pop absolute left-0 top-full z-popover mt-2 w-64 rounded-xl p-1.5"
        >
          {activeIsSyncing && (
            <button type="button" role="menuitem" tabIndex={-1} className={item} onClick={choose(() => void pauseSync())}>Pause synchronization</button>
          )}
          {activeIsPaused && !activeIsSyncing && (
            <button type="button" role="menuitem" tabIndex={-1} className={item} onClick={choose(() => void resumeSync())}>Resume synchronization</button>
          )}
          {!activeIsPaused && (
            <button
              type="button"
              role="menuitem"
              tabIndex={-1}
              disabled={activeIsRechecking}
              title="Fetch collected documents again and record whether the source still serves the same bytes"
              className={item}
              onClick={choose(() => void recheckSource())}
            >
              {activeIsRechecking ? 'Checking against source…' : 'Check against source'}
            </button>
          )}
          <div role="separator" className="my-1 h-px bg-line" />
          <button type="button" role="menuitem" tabIndex={-1} title="Remove source and keep its documents" className={item} onClick={choose(() => setPendingRemoval(activePortal))}>Remove source</button>
          <button type="button" role="menuitem" tabIndex={-1} title="Delete source and every document ingested from it" className={cn(item, 'text-danger hover:bg-danger/10 focus-visible:bg-danger/10')} onClick={choose(() => setPendingPurge(activePortal))}>Delete source and its documents</button>
        </div>
      )}
    </div>
  );
}
