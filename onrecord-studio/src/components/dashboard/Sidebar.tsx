import { useEffect, useState } from 'react';
import { useAppStore } from '../../store/useAppStore';
import { cn } from '../../utils/cn';
import { PRODUCT_BYLINE, PRODUCT_NAME } from '../../config/brand';
import Icon, { type IconName } from '../ui/icons';
import { openSourceConnector } from './sourceEvents';
import { OPEN_COMMAND_PALETTE_EVENT } from './CommandPalette';
import { isApplePlatform } from './NavigationControls';
import { useWorkspaceBadges } from '../../hooks/useWorkspaceBadges';
import { workspaceCommandRegistry } from '../../commands/registry';
import StatusDot from '../ui/statusDot';
import Badge, { CountBadge } from '../ui/Badge';
import Button from '../ui/Button';
import type { SourceKind } from '../../types';

const ROW = 'sidebar-nav-item focus-ring group relative flex h-8 w-full items-center rounded-md text-left transition-colors duration-fast';

function sourceIcon(kind: SourceKind): IconName {
  if (kind === 'LOCAL_FILE' || kind === 'PASTED_CONTENT' || kind === 'DIRECT_FILE') return 'database';
  if (kind === 'NEXT_REQUEST') return 'building';
  return 'globe';
}

export default function Sidebar() {
  const [showTools, setShowTools] = useState(false);
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
  const modifier = isApplePlatform() ? '⌘' : 'Ctrl ';
  // Only an incomplete capability is worth a line; a working OCR stack is the expected state.
  const ocrGap = ocrCapability && !(ocrCapability.imageOcrAvailable && ocrCapability.pdfOcrAvailable)
    ? (ocrCapability.imageOcrAvailable ? 'Scanned PDFs can’t be read here' : 'Scanned images and PDFs can’t be read here')
    : null;

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
          className="fixed inset-0 z-overlay bg-scrim md:hidden"
        />
      )}
      <aside
        aria-label="Workspace navigation"
        className={cn(
          'sidebar-surface fixed inset-y-0 left-0 z-popover flex shrink-0 flex-col border-r shadow-popover transition-transform duration-slow ease-out md:relative md:z-sticky md:translate-x-0 md:shadow-none',
          isCollapsed ? 'w-14' : 'w-[248px]',
          isSidebarOpen ? 'translate-x-0' : '-translate-x-full',
        )}
      >
        <div className={cn('flex h-14 shrink-0 items-center border-b border-line', isCollapsed ? 'justify-center px-2' : 'gap-2.5 px-3.5')}>
          <img src="/brand-mark.png" alt="" className="h-7 w-7 shrink-0 rounded-lg" />
          {!isCollapsed && (
            <div className="min-w-0">
              <div className="truncate text-heading text-fg">{PRODUCT_NAME}</div>
              <div className="truncate text-caption text-muted">{PRODUCT_BYLINE}</div>
            </div>
          )}
          <Button
            variant="ghost"
            iconOnly
            icon="x"
            aria-label="Close navigation"
            onClick={() => setSidebarOpen(false)}
            className="ml-auto md:hidden"
          />
        </div>

        <nav aria-label="Primary navigation" className="shrink-0 px-2 py-3">
          {/* The command palette's visible entry point; ⌘K is not discoverable on its own. */}
          <button
            type="button"
            onClick={() => window.dispatchEvent(new Event(OPEN_COMMAND_PALETTE_EVENT))}
            aria-label={isCollapsed ? 'Search and commands' : undefined}
            aria-keyshortcuts={isApplePlatform() ? 'Meta+K' : 'Control+K'}
            title={`Search and commands (${modifier}K)`}
            className={cn(
              'focus-ring mb-3 flex h-8 w-full items-center rounded-md border border-line bg-canvas text-muted transition-colors duration-fast hover:border-line-strong hover:text-fg-secondary',
              isCollapsed ? 'justify-center' : 'gap-2 px-2',
            )}
          >
            <Icon name="search" size="sm" className="shrink-0" />
            {!isCollapsed && (
              <>
                <span className="min-w-0 flex-1 truncate text-left text-control font-normal">Search and commands</span>
                <kbd aria-hidden="true" className="text-caption tabular-nums text-muted">{isApplePlatform() ? '⌘K' : 'Ctrl K'}</kbd>
              </>
            )}
          </button>
          {!isCollapsed && <div className="mb-1 px-2 text-overline uppercase text-muted">Investigate</div>}
          <div className="space-y-px">
            {[...workspaceCommandRegistry].sort((a, b) => {
              const order = ['overview', 'story', 'evidence', 'briefing'];
              return (order.indexOf(a.view) < 0 ? 99 : order.indexOf(a.view)) - (order.indexOf(b.view) < 0 ? 99 : order.indexOf(b.view));
            }).filter((item) => showTools || ['overview', 'story', 'evidence', 'briefing', workspaceView].includes(item.view)).map((item) => {
              const selected = workspaceView === item.view;
              const badge = badges[item.view];
              return (
                <button
                  key={item.view}
                  type="button"
                  aria-current={selected ? 'page' : undefined}
                  aria-label={isCollapsed ? `${item.navigationLabel}${badge ? `, ${badge.label}` : ''}` : undefined}
                  title={`${item.navigationLabel} (${modifier}${item.shortcutIndex}) — ${item.detail}`}
                  onClick={() => setWorkspaceView(item.view)}
                  className={cn(
                    ROW,
                    isCollapsed ? 'justify-center' : 'gap-2 px-2',
                    selected ? 'sidebar-nav-item-active text-accent' : 'text-fg-secondary hover:bg-raised hover:text-fg',
                  )}
                >
                  <span className={cn('relative grid h-6 w-6 shrink-0 place-items-center', selected ? 'text-accent' : 'text-muted group-hover:text-fg-secondary')}>
                    <Icon name={item.icon} size="md" />
                    {isCollapsed && badge && (
                      <span aria-hidden="true" className={cn('absolute -right-0.5 -top-0.5 h-1.5 w-1.5 rounded-full', badge.tone === 'warning' ? 'bg-warning-mark' : 'bg-accent')} />
                    )}
                  </span>
                  {!isCollapsed && (
                    <>
                      <span className="min-w-0 flex-1 truncate text-body font-medium">{item.navigationLabel}</span>
                      <kbd aria-hidden="true" className="text-caption tabular-nums text-muted opacity-0 transition-opacity duration-fast group-hover:opacity-100 group-focus-visible:opacity-100">
                        {modifier}{item.shortcutIndex}
                      </kbd>
                      {badge && (
                        <CountBadge
                          value={badge.text}
                          title={badge.label}
                          aria-hidden="true"
                          className={badge.tone === 'warning' ? 'bg-warning/10 text-warning' : 'bg-accent/10 text-accent'}
                        />
                      )}
                      {badge && <span className="sr-only">, {badge.label}</span>}
                    </>
                  )}
                </button>
              );
            })}
            <button type="button" className={cn(ROW, 'gap-2 px-2 text-muted')} aria-expanded={showTools} onClick={() => setShowTools((shown) => !shown)} title="Analysis, patterns, meetings, comparisons and activity">
              <Icon name="sliders" size="md" />
              {!isCollapsed && <span>{showTools ? 'Fewer tools' : 'More tools'}</span>}
              {isCollapsed && <span className="sr-only">More tools</span>}
            </button>
          </div>
        </nav>

        <div className="mx-3 h-px shrink-0 bg-line" />

        <nav aria-label="Data sources" className="min-h-[120px] flex-1 overflow-y-auto px-2 py-3">
          {!isCollapsed && (
            <div className="mb-1 flex h-6 items-center justify-between pl-2">
              <span className="text-overline uppercase text-muted">Data sources</span>
              <Button variant="ghost" size="sm" iconOnly icon="filePlus" aria-label="Add source" title="Add source" onClick={openSourceConnector} className="h-6 w-6" />
            </div>
          )}
          <div className="space-y-px">
            <button
              type="button"
              title={isCollapsed ? 'All saved sources' : undefined}
              aria-label={isCollapsed ? `All saved sources, ${portals.length}` : undefined}
              aria-current={activePortalKey === null ? 'true' : undefined}
              onClick={() => setActivePortal(null)}
              className={cn(
                ROW,
                isCollapsed ? 'justify-center' : 'gap-2 px-2',
                activePortalKey === null ? 'sidebar-nav-item-active text-accent' : 'text-fg-secondary hover:bg-raised hover:text-fg',
              )}
            >
              <span className="grid h-6 w-6 shrink-0 place-items-center"><Icon name="grid" size="md" /></span>
              {!isCollapsed && <span className="min-w-0 flex-1 truncate text-body font-medium">All saved sources</span>}
              {!isCollapsed && workspaceInitializationStatus === 'ready' && <CountBadge value={portals.length} />}
            </button>

            {portals.map(({ registration: portal, lifecycleState, healthStatus, lastError }) => {
              const runtime = runtimeByPortal.get(portal.portalKey);
              const paused = lifecycleState === 'PAUSED';
              const degraded = healthStatus === 'degraded';
              const syncing = syncingPortalKeys.includes(portal.portalKey);
              const selected = activePortalKey === portal.portalKey;
              // Progress is settled work against total work. In-flight downloads are capped by the
              // concurrency limit, so counting them as done would make a long crawl look stalled.
              const settledTasks = runtime ? runtime.completedTasks + runtime.failedTasks : 0;
              const totalTasks = runtime ? settledTasks + runtime.queuedTasks + runtime.activeTasks : 0;
              const failedTasks = runtime?.failedTasks ?? 0;
              // Failures count as settled but are reported apart, never folded into one figure.
              const percentComplete = totalTasks > 0 ? Math.round((settledTasks / totalTasks) * 100) : 0;
              const showProgress = !isCollapsed && totalTasks > 0 && (syncing || failedTasks > 0);
              // Paused is its own state rather than idle: a source the operator halted and one that
              // has simply finished are different facts.
              const status = degraded ? 'needs attention' : syncing ? 'syncing' : paused ? 'paused' : 'idle';
              return (
                <button
                  key={portal.portalKey}
                  type="button"
                  aria-label={isCollapsed ? `${portal.displayName}, ${status}${failedTasks > 0 ? `, ${failedTasks.toLocaleString()} failed` : ''}` : undefined}
                  aria-current={selected ? 'true' : undefined}
                  title={degraded && lastError ? `${portal.displayName} needs attention: ${lastError}` : portal.displayName}
                  onClick={() => setActivePortal(portal.portalKey)}
                  className={cn(
                    'sidebar-nav-item focus-ring group relative block w-full rounded-md text-left transition-colors duration-fast',
                    selected ? 'sidebar-nav-item-active text-accent' : 'text-fg-secondary hover:bg-raised hover:text-fg',
                  )}
                >
                  <span className={cn('flex h-8 items-center', isCollapsed ? 'justify-center' : 'gap-2 px-2')}>
                    <span className={cn('relative grid h-6 w-6 shrink-0 place-items-center', !selected && 'text-muted group-hover:text-fg-secondary')}>
                      <Icon name={sourceIcon(portal.sourceKind)} size="md" />
                      {isCollapsed && (syncing || degraded || paused) && (
                        <span className="absolute -right-0.5 -top-0.5 grid h-3 w-3 place-items-center rounded-full bg-surface">
                          <StatusDot state={degraded ? 'error' : syncing ? 'active' : 'idle'} pulse={syncing && !degraded} label="" className={cn(paused && !degraded && !syncing && 'text-warning')} />
                        </span>
                      )}
                    </span>
                    {!isCollapsed && (
                      <>
                        <span className="min-w-0 flex-1 truncate text-body font-medium">{portal.displayName}</span>
                        {degraded ? (
                          <Badge tone="danger" glyph="diamond">Attention</Badge>
                        ) : paused ? (
                          <Badge tone="warning">Paused</Badge>
                        ) : syncing ? (
                          <StatusDot state="active" pulse label="syncing" className="mr-1" />
                        ) : null}
                        {!degraded && !paused && !syncing && <span className="sr-only">idle</span>}
                      </>
                    )}
                  </span>
                  {showProgress && (
                    <span className="block px-2 pb-2 pl-10">
                      <span className="flex min-w-0 items-baseline gap-1 text-caption tabular-nums text-muted">
                        <span className="shrink-0">{settledTasks.toLocaleString()}/{totalTasks.toLocaleString()}</span>
                        {failedTasks > 0 && <span className="min-w-0 truncate text-danger">· {failedTasks.toLocaleString()} failed</span>}
                      </span>
                      {syncing && (
                        <span
                          role="progressbar"
                          aria-valuenow={percentComplete}
                          aria-valuemin={0}
                          aria-valuemax={100}
                          aria-label={`${portal.displayName} ingestion progress`}
                          className="mt-1 block h-0.5 w-full overflow-hidden rounded-full bg-raised"
                        >
                          <span className="block h-full origin-left rounded-full bg-success-mark transition-transform duration-slow" style={{ transform: `scaleX(${percentComplete / 100})` }} />
                        </span>
                      )}
                    </span>
                  )}
                  {!isCollapsed && selected && degraded && (
                    <span role="status" className="block px-2 pb-2 pl-10 text-caption text-danger [overflow-wrap:anywhere]">
                      {lastError ?? 'The last sync did not finish cleanly, and no error was recorded.'}
                    </span>
                  )}
                </button>
              );
            })}
          </div>

          {workspaceInitializationStatus === 'loading' && portals.length === 0 && (
            <div role="status" aria-busy="true" className="mt-px space-y-1 px-2">
              <span className="sr-only">Loading saved sources</span>
              {[0, 1, 2].map((row) => <div key={row} aria-hidden="true" className="skeleton h-7 rounded-md" />)}
            </div>
          )}

          {workspaceInitializationStatus === 'ready' && portals.length === 0 && !isCollapsed && (
            <div className="mx-1 mt-3 rounded-lg border border-dashed border-line p-3">
              <p className="text-body font-medium text-fg-secondary">No sources yet</p>
              <p className="mt-1 text-caption text-muted">Add a records portal, website or file to start.</p>
              {/* Secondary: the main pane's empty state carries the one primary action. */}
              <Button size="sm" onClick={openSourceConnector} className="mt-3 w-full">Add a source</Button>
            </div>
          )}

          {workspaceInitializationStatus === 'error' && portals.length === 0 && (
            <p
              role="status"
              title={isCollapsed ? 'Sources couldn’t load' : undefined}
              className={cn('mt-px flex h-8 items-center gap-2 text-caption text-danger', isCollapsed ? 'justify-center' : 'px-2')}
            >
              <span className="grid h-6 w-6 shrink-0 place-items-center"><Icon name="flag" size="sm" /></span>
              <span className={isCollapsed ? 'sr-only' : 'min-w-0 truncate'}>Sources couldn’t load</span>
            </p>
          )}
        </nav>

        <div className="shrink-0 border-t border-line p-2">
          {ocrGap && !isCollapsed && (
            <p className="mb-1 flex items-start gap-2 px-2 py-1 text-caption text-warning" title={ocrCapability?.diagnostics.join(' — ') || undefined}>
              <Icon name="flag" size="xs" className="mt-0.5" />
              {ocrGap}
            </p>
          )}
          <button
            type="button"
            onClick={toggleSidebarCollapsed}
            aria-label={isCollapsed ? 'Expand navigation' : undefined}
            className="focus-ring hidden h-8 w-full items-center justify-center gap-2 rounded-md text-control text-muted transition-colors duration-fast hover:bg-raised hover:text-fg-secondary md:flex"
          >
            <Icon name="collapse" size="sm" className={cn('transition-transform duration-base', isCollapsed && 'rotate-180')} />
            {!isCollapsed && 'Collapse navigation'}
          </button>
        </div>
      </aside>
    </>
  );
}
