import { useMemo } from 'react';
import { useAppStore } from '../../../store/useAppStore';
import RuntimeBars, { type RuntimeRow } from '../../charts/figures/RuntimeBars';
import { VIZ } from '../../charts/core/claim';
import Badge from '../../ui/Badge';
import Button from '../../ui/Button';
import Skeleton from '../../ui/Skeleton';
import StatusDot from '../../ui/statusDot';
import CanvasModule from './CanvasModule';

const EVENTS_SHOWN = 8;

type Runtime = NonNullable<RuntimeRow['runtime']>;
const total = (runtime: Runtime) => runtime.completedTasks + runtime.failedTasks + runtime.activeTasks + runtime.queuedTasks;

/**
 * The module's sentence: tasks done and failed, and where the failures are. Stopped hosts and
 * refused links are in the expanded figure's own headline, which states every count.
 */
export function compactHeadline(rows: readonly RuntimeRow[]): string {
  const run = rows.flatMap((row) => (row.runtime ? [{ label: row.label, runtime: row.runtime }] : []));
  if (run.length === 0) return 'No source has a collection queue this session.';
  const tasks = run.reduce((sum, row) => sum + total(row.runtime), 0);
  const done = run.reduce((sum, row) => sum + row.runtime.completedTasks, 0);
  const failed = run.reduce((sum, row) => sum + row.runtime.failedTasks, 0);
  const failing = run.filter((row) => row.runtime.failedTasks > 0).sort((a, b) => b.runtime.failedTasks - a.runtime.failedTasks);
  const where = failing.length === 1 && run.length > 1 ? `, all in ${failing[0].label}` : failing.length > 1 ? `, most in ${failing[0].label}` : '';
  return `${done.toLocaleString('en-US')} of ${tasks.toLocaleString('en-US')} collection tasks done; ${failed.toLocaleString('en-US')} failed${where}.`;
}

/**
 * Collection activity per source: done, failed, active and queued tasks on one task scale shared by
 * every source (the same encoding as the Activity view's runtime bars), the source's state with a
 * breathing dot while it collects, and the latest collection event. A source with no queue this
 * session says so in words; it never draws an empty bar.
 */
export default function CollectionModule({ expanded, onToggle, span }: { expanded: boolean; onToggle(): void; span?: string }) {
  const portals = useAppStore((state) => state.portals);
  const runtimes = useAppStore((state) => state.portalRuntimes);
  const runtimesStatus = useAppStore((state) => state.portalRuntimesStatus);
  const runtimesError = useAppStore((state) => state.portalRuntimesError);
  const syncingPortalKeys = useAppStore((state) => state.syncingPortalKeys);
  const syncLogs = useAppStore((state) => state.syncLogs);
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);
  const rows = useMemo<(RuntimeRow & { status: 'syncing' | 'paused' | 'attention' | 'idle' })[]>(() => portals.map(({ registration, lifecycleState, healthStatus }) => {
    const runtime = runtimes.find((entry) => entry.portal.portalKey === registration.portalKey) ?? null;
    const syncing = syncingPortalKeys.includes(registration.portalKey);
    return {
      key: registration.portalKey,
      label: registration.displayName,
      state: runtime ? (runtime.isRunning ? 'Running' : 'Idle') : undefined,
      runtime,
      status: healthStatus === 'degraded' ? 'attention' : syncing ? 'syncing' : lifecycleState === 'PAUSED' ? 'paused' : 'idle',
    };
  }), [portals, runtimes, syncingPortalKeys]);
  const collecting = rows.filter((row) => row.status === 'syncing').length;
  const scaleMax = Math.max(1, ...rows.map((row) => (row.runtime ? total(row.runtime) : 0)));
  const common = {
    id: 'canvas-collection',
    icon: 'activity' as const,
    title: 'Collection',
    badges: runtimesStatus === 'ready' && (collecting > 0
      ? <Badge tone="success"><StatusDot state="active" pulse label="" />Collecting</Badge>
      : <Badge tone="neutral"><StatusDot state="idle" label="" />Idle</Badge>),
    expanded,
    onToggle,
    expandLabel: 'Show collection tasks and events',
    collapseLabel: 'Hide collection tasks and events',
    span,
  };

  if (runtimesStatus === 'pending') {
    return (
      <CanvasModule {...common} headline={null}>
        <Skeleton variant="lines" lines={4} label="Reading collection queues" />
      </CanvasModule>
    );
  }
  if (runtimesStatus === 'failed') {
    return <CanvasModule {...common} headline={`Collection queues couldn’t be read${runtimesError ? `: ${runtimesError}` : '.'}`} />;
  }
  if (rows.length === 0) {
    return <CanvasModule {...common} headline="No source is saved yet, so nothing is being collected." />;
  }

  const latest = syncLogs[0];
  return (
    <CanvasModule
      {...common}
      headline={compactHeadline(rows)}
      footer={latest ? <p className="truncate font-mono text-code" title={latest}>{latest}</p> : undefined}
      detail={(
        <div className="grid grid-cols-1 gap-5">
          <RuntimeBars
            storageKey="overview-runtimes"
            status="ready"
            rows={rows}
            onShowFailures={() => setWorkspaceView('activity')}
          />
          <section aria-labelledby="canvas-collection-events" className="grid gap-2">
            <h4 id="canvas-collection-events" className="text-overline uppercase text-muted">Latest events</h4>
            {syncLogs.length === 0 ? (
              <p className="text-body text-muted">No collection event has been recorded in this session.</p>
            ) : (
              <ol className="grid gap-1 font-mono text-code text-fg-secondary">
                {syncLogs.slice(0, EVENTS_SHOWN).map((log, index) => <li key={`${index}:${log}`} className="truncate" title={log}>{log}</li>)}
              </ol>
            )}
            <Button size="sm" className="mt-1 w-fit" onClick={() => setWorkspaceView('activity')}>Open Activity</Button>
          </section>
        </div>
      )}
    >
      <ul className="grid gap-3">
        {rows.map((row) => {
          const runtime = row.runtime;
          const tasks = runtime ? total(runtime) : 0;
          const share = (n: number) => `${(n / scaleMax) * 100}%`;
          return (
            <li key={row.key} className="grid gap-1.5">
              <div className="flex min-w-0 items-center gap-2">
                <StatusDot
                  state={row.status === 'attention' ? 'error' : row.status === 'syncing' ? 'active' : 'idle'}
                  pulse={row.status === 'syncing'}
                  label={row.status === 'attention' ? 'Needs attention' : row.status === 'syncing' ? 'Collecting' : row.status === 'paused' ? 'Paused' : 'Idle'}
                  className={row.status === 'paused' ? 'text-warning' : undefined}
                />
                <span className="min-w-0 flex-1 truncate text-body text-fg" title={row.label}>{row.label}</span>
                <span className="shrink-0 text-caption tabular-nums text-fg-secondary">
                  {runtime ? (
                    <>
                      {(runtime.completedTasks + runtime.failedTasks).toLocaleString('en-US')}/{tasks.toLocaleString('en-US')}
                      {runtime.failedTasks > 0 && <span className="text-fg"> · ▲ {runtime.failedTasks.toLocaleString('en-US')} failed</span>}
                    </>
                  ) : <span className="italic text-muted">{row.status === 'paused' ? 'paused' : 'no queue this session'}</span>}
                </span>
              </div>
              {runtime && tasks > 0 && (
                <div aria-hidden="true" className="flex h-1.5 gap-px" style={{ width: share(tasks) }}>
                  {runtime.completedTasks > 0 && <span className="h-full first:rounded-l-full last:rounded-r-full" style={{ flexGrow: runtime.completedTasks, flexBasis: 0, background: VIZ.positive }} />}
                  {runtime.failedTasks > 0 && <span className="h-full min-w-[3px] first:rounded-l-full last:rounded-r-full" style={{ flexGrow: runtime.failedTasks, flexBasis: 0, background: VIZ.ink }} />}
                  {runtime.activeTasks > 0 && <span className="h-full min-w-[3px] first:rounded-l-full last:rounded-r-full" style={{ flexGrow: runtime.activeTasks, flexBasis: 0, background: VIZ.mark }} />}
                  {runtime.queuedTasks > 0 && <span className="h-full min-w-[3px] first:rounded-l-full last:rounded-r-full" style={{ flexGrow: runtime.queuedTasks, flexBasis: 0, boxShadow: `inset 0 0 0 1px ${VIZ.mark}` }} />}
                </div>
              )}
            </li>
          );
        })}
      </ul>
    </CanvasModule>
  );
}
