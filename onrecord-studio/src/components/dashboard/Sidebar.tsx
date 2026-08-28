import { useEffect } from 'react';
import { useAppStore, type WorkspaceView } from '../../store/useAppStore';
import { cn } from '../../utils/cn';
import Icon, { type IconName } from '../ui/icons';
import { OPEN_COMMAND_PALETTE_EVENT } from './CommandPalette';

const workspaceItems: Array<{
  view: WorkspaceView;
  label: string;
  description: string;
  icon: IconName;
}> = [
  { view: 'overview', label: 'Overview', description: 'Signals and performance', icon: 'grid' },
  { view: 'patterns', label: 'Patterns', description: 'Cross-file connections', icon: 'sparkles' },
  { view: 'evidence', label: 'Evidence', description: 'Search and case review', icon: 'evidence' },
  { view: 'activity', label: 'Activity', description: 'Ingestion operations', icon: 'activity' },
];

export default function Sidebar() {
  // FEATURE(navigation): Mirror these workspace actions into native macOS and
  // Windows application menus when platform menu commands are introduced.
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
        className={cn(
          'sidebar-surface fixed inset-y-0 left-0 z-50 flex shrink-0 flex-col border-r shadow-2xl shadow-black/10 backdrop-blur-xl transition-[width,transform] duration-300 md:relative md:z-20 md:translate-x-0 md:shadow-none',
          isCollapsed ? 'w-[68px]' : 'w-[248px]',
          isSidebarOpen ? 'translate-x-0' : '-translate-x-full',
        )}
      >
        <div className={cn('flex h-[64px] shrink-0 items-center border-b border-slate-800/70', isCollapsed ? 'justify-center px-3' : 'gap-2.5 px-3.5')}>
          <div className="app-brand-mark relative grid h-9 w-9 shrink-0 place-items-center overflow-hidden rounded-xl text-[10px] font-black tracking-[-0.08em] text-white">
            <span className="relative z-10">OR</span>
            <span className="absolute inset-x-2 bottom-1.5 h-px bg-white/60" />
          </div>
          {!isCollapsed && (
            <div className="min-w-0">
              <div className="truncate text-[13px] font-semibold tracking-[-0.015em] text-slate-100">OnRecord Studio</div>
              <div className="mt-0.5 text-[12px] text-slate-600">Local investigation workspace</div>
            </div>
          )}
          <button
            type="button"
            aria-label="Close navigation"
            onClick={() => setSidebarOpen(false)}
            className="focus-ring ml-auto rounded-lg p-2 text-slate-500 hover:bg-slate-800 hover:text-slate-200 md:hidden"
          >
            <Icon name="x" />
          </button>
        </div>

        <nav aria-label="Primary navigation" className="shrink-0 px-2.5 py-3">
          {!isCollapsed && <div className="sidebar-section-label mb-1.5 px-2">Workspace</div>}
          <div className="space-y-0.5">
            {workspaceItems.map((item, index) => {
              const selected = workspaceView === item.view;
              return (
                <button
                  key={item.view}
                  type="button"
                  title={`${item.label} (${isApplePlatform() ? '⌘' : 'Ctrl'}${index + 1}) — ${item.description}`}
                  onClick={() => setWorkspaceView(item.view)}
                  className={cn(
                    'sidebar-nav-item focus-ring group relative flex w-full items-center overflow-hidden rounded-lg border text-left transition',
                    isCollapsed ? 'justify-center px-2 py-2' : 'gap-2 px-2 py-1.5',
                    selected
                      ? 'sidebar-nav-item-active border-transparent text-cyan-100'
                      : 'border-transparent text-slate-500 hover:bg-slate-800/55 hover:text-slate-200',
                  )}
                >
                  <span className={cn('grid h-7 w-7 shrink-0 place-items-center rounded-md transition', selected ? 'bg-cyan-400/10 text-cyan-300' : 'text-slate-500 group-hover:text-slate-300')}>
                    <Icon name={item.icon} className="h-4 w-4" />
                  </span>
                  {!isCollapsed && (
                    <span className="min-w-0 flex-1 truncate text-[13px] font-medium">{item.label}</span>
                  )}
                  {!isCollapsed && <span className="text-[11px] text-slate-600">{isApplePlatform() ? '⌘' : 'Ctrl'}{index + 1}</span>}
                </button>
              );
            })}
          </div>
        </nav>

        <div className="mx-4 h-px bg-slate-800/70" />

        <nav aria-label="Data sources" className="min-h-0 flex-1 overflow-y-auto px-2.5 py-3">
          {!isCollapsed && (
            <div className="mb-2 flex items-center justify-between px-2">
              <span className="sidebar-section-label">Data sources</span>
              <span className="numeric-value text-[11px] text-slate-600">{portals.length}</span>
            </div>
          )}
          <button
            type="button"
            title={isCollapsed ? 'All jurisdictions' : undefined}
            onClick={() => setActivePortal(null)}
            className={cn(
              'sidebar-nav-item focus-ring relative mb-0.5 flex w-full items-center overflow-hidden rounded-lg border text-left transition',
              isCollapsed ? 'justify-center px-2 py-2' : 'gap-2 px-2 py-1.5',
              activePortalKey === null
                ? 'sidebar-source-active border-transparent text-indigo-100'
                : 'border-transparent text-slate-500 hover:bg-slate-800/50 hover:text-slate-200',
            )}
          >
            <span className="grid h-7 w-7 shrink-0 place-items-center rounded-md">
              <Icon name="globe" className="h-4 w-4" />
            </span>
            {!isCollapsed && <span className="min-w-0 flex-1 text-[13px] font-medium">All jurisdictions</span>}
            {!isCollapsed && <span className="numeric-value rounded-md bg-slate-900 px-1.5 py-0.5 text-[11px] text-slate-600">{portals.length}</span>}
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
                    : 'border-transparent hover:border-slate-800 hover:bg-slate-800/45',
                )}
              >
                <div className={cn('flex items-center', isCollapsed ? 'justify-center' : 'gap-2.5')}>
                  <span className={cn('relative grid h-7 w-7 shrink-0 place-items-center rounded-lg font-mono text-[9px] font-semibold', selected ? 'bg-indigo-400/10 text-indigo-200' : 'bg-slate-900 text-slate-500')}>
                    {portal.displayName.slice(0, 2).toUpperCase()}
                    {/* Paused is its own colour rather than the idle grey. A source the operator
                        halted and one that has simply finished are different facts, and a shared
                        dot would say neither. */}
                    <span
                      title={degraded ? `${portal.displayName} needs attention` : paused ? `${portal.displayName} is paused` : undefined}
                      className={cn('portal-status-dot absolute -right-0.5 -top-0.5 h-2 w-2 rounded-full border-2', degraded ? 'bg-rose-400' : syncing ? 'status-pulse bg-emerald-400' : paused ? 'bg-amber-400' : 'bg-slate-600')}
                    />
                  </span>
                  {!isCollapsed && <span className={cn('min-w-0 flex-1 truncate text-[13px] font-medium', selected ? 'text-indigo-100' : 'text-slate-300')}>{portal.displayName}</span>}
                  {!isCollapsed && paused && <span className="shrink-0 rounded-md bg-amber-400/10 px-1.5 py-0.5 text-[9px] font-semibold uppercase tracking-wide text-amber-300/90">Paused</span>}
                  {!isCollapsed && degraded && <span className="shrink-0 rounded-md bg-rose-400/10 px-1.5 py-0.5 text-[9px] font-semibold uppercase tracking-wide text-rose-300/90">Attention</span>}
                </div>
                {!isCollapsed && runtime && totalTasks > 0 && (
                  <div className="mt-1 pl-9">
                    <div className="numeric-value text-[11px] text-slate-600">
                      {settledTasks.toLocaleString()} of {totalTasks.toLocaleString()} done
                      {runtime.failedTasks > 0 && (
                        <span className="text-amber-500/80"> · {runtime.failedTasks.toLocaleString()} failed</span>
                      )}
                    </div>
                    <div
                      role="progressbar"
                      aria-valuenow={percentComplete}
                      aria-valuemin={0}
                      aria-valuemax={100}
                      aria-label={`${portal.displayName} ingestion progress`}
                      className="mt-1 h-1 w-full overflow-hidden rounded-full bg-slate-800"
                    >
                      <div
                        className={cn('h-full rounded-full transition-[width] duration-500', syncing ? 'bg-emerald-400' : 'bg-slate-600')}
                        style={{ width: `${percentComplete}%` }}
                      />
                    </div>
                  </div>
                )}
              </button>
            );
          })}

          {portals.length === 0 && !isCollapsed && (
            <div className="mx-1 mt-3 rounded-xl border border-dashed border-slate-800 bg-slate-900/25 p-3.5">
              <div className="flex items-center gap-2 text-[10px] font-semibold text-slate-400">
                <Icon name="database" className="h-4 w-4 text-cyan-400/70" />
                {workspaceInitializationStatus === 'loading'
                  ? 'Loading saved sources'
                  : workspaceInitializationStatus === 'error'
                    ? 'Saved sources unavailable'
                    : 'No sources connected'}
              </div>
              <p className="mt-2 text-[9px] leading-relaxed text-slate-600">
                {workspaceInitializationStatus === 'loading'
                  ? 'Checking the local source catalog.'
                  : workspaceInitializationStatus === 'error'
                    ? 'The source catalog did not load. Existing sources may still be present; check the activity log.'
                    : 'Add a URL, file, or pasted source in the toolbar to build your first audit workspace.'}
              </p>
            </div>
          )}
        </nav>

        <div className="border-t border-slate-800/70 p-2.5">
          {!isCollapsed ? (
            <>
              <button type="button" onClick={openCommandPalette} className="command-trigger focus-ring flex w-full items-center gap-2 rounded-lg border px-2.5 py-2 text-[12px] font-medium text-slate-500 transition hover:text-slate-200">
                <Icon name="search" className="h-4 w-4" />
                <span className="flex-1 text-left">Commands</span>
                <span className="keycap rounded-md border px-1.5 py-0.5 text-[11px]">{isApplePlatform() ? '⌘K' : 'Ctrl K'}</span>
              </button>
              <div className="mt-2 flex items-center justify-between px-2 text-[12px] text-slate-600">
                <span className="flex items-center gap-2"><span className={cn('h-1.5 w-1.5 rounded-full', syncingPortalKeys.length ? 'bg-emerald-400' : 'bg-slate-600')} /> Worker capacity</span>
                <span className={cn('numeric-value', syncingPortalKeys.length > 0 && 'font-semibold text-emerald-300')}>{syncingPortalKeys.length}/5</span>
              </div>
              <div
                className="mt-1 flex items-center justify-between gap-3 px-2 text-[12px] text-slate-600"
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
                      ? 'bg-emerald-400'
                      : ocrCapability
                        ? 'bg-amber-400'
                        : 'bg-slate-600',
                  )} />
                  Local OCR
                </span>
                <span className="truncate text-right text-[11px]">
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
            <button type="button" onClick={openCommandPalette} title={`Commands (${isApplePlatform() ? '⌘' : 'Ctrl'}K)`} aria-label="Open command palette" className="command-trigger focus-ring mx-auto grid h-9 w-9 place-items-center rounded-lg border text-slate-500"><Icon name="search" className="h-4 w-4" /></button>
          )}
          <button
            type="button"
            onClick={toggleSidebarCollapsed}
            aria-label={isCollapsed ? 'Expand sidebar' : 'Collapse sidebar'}
            className="focus-ring mt-1.5 hidden w-full items-center justify-center gap-2 rounded-lg py-1.5 text-[12px] font-medium text-slate-600 transition hover:bg-slate-800/50 hover:text-slate-300 md:flex"
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
