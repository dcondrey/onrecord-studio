import { useEffect } from 'react';
import { useAppStore } from '../../store/useAppStore';
import { cn } from '../../utils/cn';
import { PRODUCT_BYLINE, PRODUCT_NAME } from '../../config/brand';
import Icon from '../ui/icons';
import { OPEN_COMMAND_PALETTE_EVENT } from './CommandPalette';
import { openSourceConnector } from './sourceEvents';
import { useWorkspaceBadges } from '../../hooks/useWorkspaceBadges';
import { workspaceCommandRegistry } from '../../commands/registry';
import StatusDot from '../ui/statusDot';

export default function Sidebar() {
  const badges = useWorkspaceBadges();
  const portals = useAppStore((state) => state.portals);
  const workspaceInitializationStatus = useAppStore((state) => state.workspaceInitializationStatus);
  const ocrCapability = useAppStore((state) => state.ocrCapability);
  const runtimes = useAppStore((state) => state.portalRuntimes);
  const activePortalKey = useAppStore((state) => state.activePortalKey);
  const syncingPortalKeys = useAppStore((state) => state.syncingPortalKeys);
  const workspaceView = useAppStore((state) => state.workspaceView);
  const isSidebarOpen = useAppStore((state) => state.isSidebarOpen);
  const isCollapsed = useAppStore((state) => state.isSidebarCollapsed);
  const setActivePortal = useAppStore((state) => state.setActivePortal);
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);
  const setSidebarOpen = useAppStore((state) => state.setSidebarOpen);
  const toggleSidebarCollapsed = useAppStore((state) => state.toggleSidebarCollapsed);
  const runtimeByPortal = new Map(runtimes.map((runtime) => [runtime.portal.portalKey, runtime]));

  useEffect(() => {
    const mobile = window.matchMedia('(max-width: 767px)');
    const expandForMobile = () => {
      if (mobile.matches && useAppStore.getState().isSidebarCollapsed) toggleSidebarCollapsed();
    };
    expandForMobile();
    mobile.addEventListener('change', expandForMobile);
    return () => mobile.removeEventListener('change', expandForMobile);
  }, [toggleSidebarCollapsed]);

  return (
    <>
      {isSidebarOpen && (
        <button
          type="button"
          aria-label="Close navigation"
          onClick={() => setSidebarOpen(false)}
          className="app-backdrop fixed inset-0 z-40 backdrop-blur-sm md:hidden"
        />
      )}
      <aside
        aria-label="Workspace navigation"
        className={cn(
          'sidebar-surface fixed inset-y-0 left-0 z-50 flex shrink-0 flex-col border-r shadow-2xl shadow-black/10 backdrop-blur-xl transition-[width,transform] duration-300 md:relative md:z-20 md:translate-x-0 md:shadow-none',
          isCollapsed ? 'w-[68px]' : 'w-[248px]',
          isSidebarOpen ? 'translate-x-0' : '-translate-x-full',
        )}
      >
        <div className={cn('flex h-[64px] shrink-0 items-center border-b border-line', isCollapsed ? 'justify-center px-3' : 'gap-2.5 px-3.5')}>
          <img src="/brand-mark.png" alt="" className="h-9 w-9 shrink-0 rounded-[9px]" />
          {!isCollapsed && (
            <div className="min-w-0">
              <div className="truncate text-label font-semibold tracking-[-0.015em] text-fg">{PRODUCT_NAME}</div>
              <div className="mt-0.5 truncate text-caption text-muted">{PRODUCT_BYLINE}</div>
            </div>
          )}
          <button
            type="button"
            aria-label="Close navigation"
            onClick={() => setSidebarOpen(false)}
            className="focus-ring ml-auto rounded-lg p-2 text-muted hover:bg-raised hover:text-fg md:hidden"
          >
            <Icon name="x" />
          </button>
        </div>

        <nav aria-label="Primary navigation" className="shrink-0 px-2.5 py-3">
          {!isCollapsed && <div className="sidebar-section-label mb-1.5 px-2">Workspace</div>}
          <div className="space-y-0.5">
            {workspaceCommandRegistry.map((item) => {
              const selected = workspaceView === item.view;
              return (
                <button
                  key={item.view}
                  type="button"
                  aria-current={selected ? 'page' : undefined}
                  aria-label={isCollapsed ? item.navigationLabel : undefined}
                  title={`${item.navigationLabel} (${isApplePlatform() ? '⌘' : 'Ctrl'}${item.shortcutIndex}) — ${item.detail}`}
                  onClick={() => setWorkspaceView(item.view)}
                  className={cn(
                    'sidebar-nav-item focus-ring group relative flex w-full items-center overflow-hidden rounded-lg border text-left transition',
                    isCollapsed ? 'justify-center px-2 py-2' : 'gap-2 px-2 py-1.5',
                    selected
                      ? 'sidebar-nav-item-active border-transparent text-accent'
                      : 'border-transparent text-muted hover:bg-raised/70 hover:text-fg',
                  )}
                >
                  <span className={cn('grid h-7 w-7 shrink-0 place-items-center rounded-md transition', selected ? 'bg-accent/10 text-accent' : 'text-muted group-hover:text-fg-secondary')}>
                    <Icon name={item.icon} className="h-4 w-4" />
                  </span>
                  {!isCollapsed && (
                    <span className="min-w-0 flex-1 truncate text-label font-medium">{item.navigationLabel}</span>
                  )}
                  {badges[item.view] && (
                    <span
                      title={badges[item.view]!.label}
                      className={cn(
                        'motion-pop numeric-value rounded-full px-1.5 py-px text-caption font-semibold',
                        badges[item.view]!.tone === 'warning' ? 'bg-warning/20 text-warning' : 'bg-accent/20 text-accent',
                        isCollapsed && 'absolute right-1 top-0.5 px-1',
                      )}
                    >
                      <span aria-hidden="true">{badges[item.view]!.text}</span>
                      <span className="sr-only">{badges[item.view]!.label}</span>
                    </span>
                  )}
                  {!isCollapsed && <span className="text-label text-muted">{isApplePlatform() ? '⌘' : 'Ctrl'}{item.shortcutIndex}</span>}
                </button>
              );
            })}
          </div>
        </nav>

        <div className="mx-4 h-px bg-raised/70" />

        <nav aria-label="Data sources" className="min-h-0 flex-1 overflow-y-auto px-2.5 py-3">
          {!isCollapsed && (
            <div className="mb-2 flex items-center justify-between px-2">
              <span className="sidebar-section-label">Data sources</span>
              <span className="numeric-value text-label text-muted">{portals.length}</span>
            </div>
          )}
          <button
            type="button"
            title={isCollapsed ? 'All saved sources' : undefined}
            onClick={() => setActivePortal(null)}
            className={cn(
              'sidebar-nav-item focus-ring relative mb-0.5 flex w-full items-center overflow-hidden rounded-lg border text-left transition',
              isCollapsed ? 'justify-center px-2 py-2' : 'gap-2 px-2 py-1.5',
              activePortalKey === null
                ? 'sidebar-source-active border-transparent text-highlight'
                : 'border-transparent text-muted hover:bg-raised/70 hover:text-fg',
            )}
          >
            <span className="grid h-7 w-7 shrink-0 place-items-center rounded-md">
              <Icon name="globe" className="h-4 w-4" />
            </span>
            {!isCollapsed && <span className="min-w-0 flex-1 text-label font-medium">All saved sources</span>}
            {!isCollapsed && <span className="numeric-value rounded-md bg-surface px-1.5 py-0.5 text-label text-muted">{portals.length}</span>}
          </button>

          {portals.map(({ registration: portal, lifecycleState, healthStatus, lastError }) => {
            const runtime = runtimeByPortal.get(portal.portalKey);
            const paused = lifecycleState === 'PAUSED';
            const degraded = healthStatus === 'degraded';
            const syncing = syncingPortalKeys.includes(portal.portalKey);
            const selected = activePortalKey === portal.portalKey;
            // `activeTasks` is how many downloads are in flight right now, which is a concurrency
            // limit and never rises. Reporting it beside the queue depth read as "2 of 1500 done"
            // and made a source that had already finished hundreds of documents look stalled.
            // Progress is settled work against total work; in-flight tasks are neither.
            const settledTasks = runtime ? runtime.completedTasks + runtime.failedTasks : 0;
            const totalTasks = runtime
              ? settledTasks + runtime.queuedTasks + runtime.activeTasks
              : 0;
            // Failures are settled work, so they belong in the numerator, but they are not
            // successes and must never be folded silently into one figure.
            const percentComplete = totalTasks > 0 ? Math.round((settledTasks / totalTasks) * 100) : 0;
            return (
              <button
                key={portal.portalKey}
                type="button"
                title={
                  degraded
                    ? `${portal.displayName} needs attention${lastError ? `: ${lastError}` : ''}`
                    : isCollapsed
                      ? portal.displayName
                      : undefined
                }
                onClick={() => setActivePortal(portal.portalKey)}
                className={cn(
                  'sidebar-nav-item focus-ring relative mb-0.5 w-full overflow-hidden rounded-lg border text-left transition',
                  isCollapsed ? 'grid place-items-center px-2 py-2' : 'px-2 py-1.5',
                  selected
                    ? 'sidebar-source-active border-transparent'
                    : 'border-transparent hover:border-control hover:bg-raised/40',
                )}
              >
                <div className={cn('flex items-center', isCollapsed ? 'justify-center' : 'gap-2.5')}>
                  <span className={cn('relative grid h-7 w-7 shrink-0 place-items-center rounded-lg font-mono text-caption font-semibold', selected ? 'bg-highlight/10 text-highlight' : 'bg-surface text-muted')}>
                    {portal.displayName.slice(0, 2).toUpperCase()}
                    {/* Paused is its own colour rather than the idle grey. A source the operator
                        halted and one that has simply finished are different facts, and a shared
                        dot would say neither. */}
                    <span className={cn('portal-status-dot absolute -right-0.5 -top-0.5 grid h-3.5 w-3.5 place-items-center rounded-full border-2', syncing && 'status-pulse')} style={{ backgroundColor: 'var(--sidebar-surface)' }}>
                      <StatusDot
                        state={degraded ? 'error' : syncing ? 'active' : paused ? 'note' : 'idle'}
                        label={degraded ? `${portal.displayName} needs attention` : syncing ? `${portal.displayName} is syncing` : paused ? `${portal.displayName} is paused` : `${portal.displayName} is idle`}
                        className={cn('h-2 w-2', paused && 'text-warning')}
                      />
                    </span>
                  </span>
                  {!isCollapsed && <span className={cn('min-w-0 flex-1 truncate text-label font-medium', selected ? 'text-highlight' : 'text-fg-secondary')}>{portal.displayName}</span>}
                  {!isCollapsed && paused && <span className="shrink-0 rounded-md bg-warning/10 px-1.5 py-0.5 text-caption font-semibold uppercase tracking-wide text-warning">Paused</span>}
                  {!isCollapsed && degraded && <span className="shrink-0 rounded-md bg-danger/10 px-1.5 py-0.5 text-caption font-semibold uppercase tracking-wide text-danger">Attention</span>}
                </div>
                {!isCollapsed && runtime && totalTasks > 0 && (
                  <div className="mt-1 pl-9">
                    <div className="numeric-value text-label text-muted">
                      {settledTasks.toLocaleString()} of {totalTasks.toLocaleString()} done
                      {runtime.failedTasks > 0 && (
                        <span className="text-warning"> · {runtime.failedTasks.toLocaleString()} failed</span>
                      )}
                    </div>
                    <div
                      role="progressbar"
                      aria-valuenow={percentComplete}
                      aria-valuemin={0}
                      aria-valuemax={100}
                      aria-label={`${portal.displayName} ingestion progress`}
                      className="mt-1 h-1 w-full overflow-hidden rounded-full bg-raised"
                    >
                      <div
                        className={cn('h-full rounded-full transition-[width] duration-500', syncing ? 'bg-success' : 'bg-muted')}
                        style={{ width: `${percentComplete}%` }}
                      />
                    </div>
                  </div>
                )}
              </button>
            );
          })}

          {portals.length === 0 && !isCollapsed && (
            <div className="mx-1 mt-3 rounded-xl border border-dashed border-line bg-surface/40 p-3.5">
              <div className="flex items-center gap-2 text-label font-semibold text-muted">
                <Icon name="database" className="h-4 w-4 text-accent" />
                {workspaceInitializationStatus === 'loading'
                  ? 'Loading saved sources'
                  : workspaceInitializationStatus === 'error'
                    ? 'Saved sources unavailable'
                    : 'No sources connected'}
              </div>
              <p className="mt-2 text-caption leading-relaxed text-muted">
                {workspaceInitializationStatus === 'loading'
                  ? 'Checking the local source catalog.'
                  : workspaceInitializationStatus === 'error'
                    ? 'The source catalog did not load. Existing sources may still be present; check the activity log.'
                    : 'Add a records portal, website, or file to start.'}
              </p>
              {workspaceInitializationStatus === 'ready' && (
                <button type="button" onClick={openSourceConnector} className="focus-ring mt-3 w-full rounded-lg border primary-action border-transparent px-2.5 py-1.5 text-label font-semibold text-white">
                  Add a source
                </button>
              )}
            </div>
          )}
        </nav>

        <div className="border-t border-line p-2.5">
          {!isCollapsed ? (
            <>
              <button type="button" onClick={openCommandPalette} className="command-trigger focus-ring flex w-full items-center gap-2 rounded-lg border px-2.5 py-2 text-label font-medium text-muted transition hover:text-fg">
                <Icon name="search" className="h-4 w-4" />
                <span className="flex-1 text-left">Commands</span>
                <span className="keycap rounded-md border px-1.5 py-0.5 text-label">{isApplePlatform() ? '⌘K' : 'Ctrl K'}</span>
              </button>
              <div className="mt-2 flex items-center justify-between px-2 text-label text-muted">
                <span className="flex items-center gap-2"><StatusDot state={syncingPortalKeys.length ? 'active' : 'idle'} label={syncingPortalKeys.length ? 'Workers busy' : 'Workers idle'} /> Worker capacity</span>
                <span className={cn('numeric-value', syncingPortalKeys.length > 0 && 'font-semibold text-success')}>{syncingPortalKeys.length}/5</span>
              </div>
              <div
                className="mt-1 flex items-center justify-between gap-3 px-2 text-label text-muted"
                title={ocrCapability
                  ? [
                      `${ocrCapability.engine}${ocrCapability.engineVersion ? ` ${ocrCapability.engineVersion}` : ''}`,
                      ocrCapability.pdfRenderer
                        ? `${ocrCapability.pdfRenderer}${ocrCapability.pdfRendererVersion ? ` ${ocrCapability.pdfRendererVersion}` : ''}`
                        : null,
                      ocrCapability.languageProbeComplete
                        ? `Languages: ${ocrCapability.languagePacks.join(', ') || 'none'}`
                        : 'Language packs were not reported',
                      ...ocrCapability.diagnostics,
                    ].filter(Boolean).join(' — ')
                  : 'OCR capability probe has not completed'}
              >
                <span className="flex min-w-0 items-center gap-2">
                  <span className={cn(
                    'h-1.5 w-1.5 shrink-0 rounded-full',
                    ocrCapability?.imageOcrAvailable && ocrCapability.pdfOcrAvailable
                      ? 'bg-success'
                      : ocrCapability
                        ? 'bg-warning'
                        : 'bg-muted',
                  )} />
                  Local OCR
                </span>
                <span className="truncate text-right text-label">
                  {!ocrCapability
                    ? 'Checking'
                    : ocrCapability.imageOcrAvailable && ocrCapability.pdfOcrAvailable
                      ? 'Image + PDF'
                      : ocrCapability.imageOcrAvailable
                        ? 'Image only'
                        : 'Unavailable'}
                </span>
              </div>
            </>
          ) : (
            <button type="button" onClick={openCommandPalette} title={`Commands (${isApplePlatform() ? '⌘' : 'Ctrl'}K)`} aria-label="Open command palette" className="command-trigger focus-ring mx-auto grid h-9 w-9 place-items-center rounded-lg border text-muted"><Icon name="search" className="h-4 w-4" /></button>
          )}
          <button
            type="button"
            onClick={toggleSidebarCollapsed}
            aria-label={isCollapsed ? 'Expand navigation' : undefined}
            className="focus-ring mt-1.5 hidden w-full items-center justify-center gap-2 rounded-lg py-1.5 text-label font-medium text-muted transition hover:bg-raised/70 hover:text-fg-secondary md:flex"
          >
            <Icon name="collapse" className={cn('h-3.5 w-3.5 transition-transform', isCollapsed && 'rotate-180')} />
            {!isCollapsed && 'Collapse navigation'}
          </button>
        </div>
      </aside>
    </>
  );
}

function openCommandPalette() {
  window.dispatchEvent(new Event(OPEN_COMMAND_PALETTE_EVENT));
}

function isApplePlatform() {
  const platform = navigator.platform.toLocaleLowerCase();
  const userAgent = navigator.userAgent.toLocaleLowerCase();
  return platform.includes('mac') || userAgent.includes('mac os');
}
