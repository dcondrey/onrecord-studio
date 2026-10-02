import { useEffect, useMemo, useState } from 'react';
import { useAppStore } from '../../store/useAppStore';
import { cn } from '../../utils/cn';
import Icon from '../ui/icons';
import { useMetrics } from '../../hooks/useMetrics';
import AuditTimeline from './AuditTimeline';
import StatusDot from '../ui/statusDot';

type LogFilter = 'all' | 'errors';

export default function ActivityDashboard() {
  const runtimes = useAppStore((state) => state.portalRuntimes);
  const syncLogs = useAppStore((state) => state.syncLogs);
  const syncingPortalKeys = useAppStore((state) => state.syncingPortalKeys);
  const metrics = useMetrics();
  const resetMetrics = useAppStore((state) => state.resetTelemetryMetrics);
  const [filter, setFilter] = useState<LogFilter>('all');
  const [query, setQuery] = useState('');
  const [visibleLimit, setVisibleLimit] = useState(75);
  const visibleLogs = useMemo(() => syncLogs.filter((log) => {
    if (filter === 'errors' && !isErrorLog(log)) return false;
    return !query.trim() || log.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase());
  }), [filter, query, syncLogs]);
  const renderedLogs = visibleLogs.slice(0, visibleLimit);

  useEffect(() => setVisibleLimit(75), [filter, query]);

  const exportActivity = () => {
    const artifact = {
      schemaVersion: 1,
      exportedAt: new Date().toISOString(),
      filter: { level: filter, query: query.trim() || null },
      eventCount: visibleLogs.length,
      events: visibleLogs.map((message) => ({ message })),
    };
    const url = URL.createObjectURL(new Blob([JSON.stringify(artifact, null, 2)], { type: 'application/json' }));
    const anchor = document.createElement('a');
    anchor.href = url;
    anchor.download = `onrecord-activity-${new Date().toISOString().replace(/[:.]/g, '-')}.json`;
    anchor.click();
    URL.revokeObjectURL(url);
  };

  return (
    <div className="min-h-0 flex-1 overflow-y-auto">
      <div className="mx-auto w-full max-w-[1700px] space-y-4 p-4 sm:p-5 lg:p-6">
        <section className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
          <div>
            <div className="app-eyebrow flex items-center gap-2 text-success"><Icon name="activity" className="h-3.5 w-3.5" /> Operations</div>
            <h2 className="mt-2 text-[26px] font-semibold tracking-[-0.035em] text-fg sm:text-[30px]">Ingestion control room</h2>
            <p className="mt-1.5 text-label text-muted">Monitor isolated source queues, parsing throughput, and operational exceptions.</p>
          </div>
          <div className="secondary-action flex items-center gap-2 rounded-xl border px-3 py-2.5 text-caption text-muted">
            <StatusDot state={syncingPortalKeys.length > 0 ? 'active' : 'idle'} label={syncingPortalKeys.length > 0 ? 'Ingestion active' : 'Ingestion idle'} className={syncingPortalKeys.length > 0 ? 'status-pulse rounded-full' : undefined} />
            {syncingPortalKeys.length > 0 ? `${syncingPortalKeys.length} isolated worker pool${syncingPortalKeys.length === 1 ? '' : 's'} active` : 'All worker pools idle'}
          </div>
        </section>

        <section aria-label="Session throughput" className="instrument-strip app-panel grid grid-cols-1 overflow-hidden sm:grid-cols-2 xl:grid-cols-4">
          <ActivityMetric label="Requests processed" value={metrics.requestsProcessed} detail="Current runtime snapshots" color="text-accent" accent="bg-accent" />
          <ActivityMetric label="Tasks completed" value={metrics.completedTasks} detail={`${metrics.queuedTasks.toLocaleString()} waiting in queues`} color="text-success" accent="bg-success" />
          <ActivityMetric label="Documents saved" value={metrics.documentsDownloaded} detail={formatBytes(metrics.bytesTransferred)} color="text-highlight" accent="bg-highlight" />
          <ActivityMetric label="Task exceptions" value={metrics.failedTasks + metrics.anomaliesFound} detail={`${metrics.taskFailureRatePercentage.toFixed(1)}% of finished tasks failed`} color="text-danger" accent="bg-danger" />
        </section>

        <div className="grid grid-cols-1 gap-4 xl:grid-cols-[minmax(0,1fr)_380px]">
          <section className="app-panel flex min-h-[520px] flex-col overflow-hidden">
            <div className="flex flex-col gap-3 border-b border-line px-4 py-3.5 sm:flex-row sm:items-center sm:justify-between">
              <div>
                <div className="app-eyebrow">Event stream</div>
                <div className="mt-1 text-caption text-muted">Newest events first · retained for this session</div>
              </div>
              <div className="flex items-center gap-2">
                <div className="relative min-w-0 flex-1 sm:w-48 sm:flex-none">
                  <Icon name="search" className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted" />
                  <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Filter activity" aria-label="Filter activity" className="focus-ring h-8 w-full rounded-lg border border-control bg-canvas/70 pl-8 pr-2 text-caption text-fg-secondary placeholder:text-muted" />
                </div>
                <div className="flex rounded-lg border border-line bg-canvas/70 p-0.5">
                  {(['all', 'errors'] as LogFilter[]).map((value) => (
                    <button key={value} type="button" onClick={() => setFilter(value)} className={cn('focus-ring rounded-md px-2.5 py-1.5 text-caption font-semibold capitalize transition', filter === value ? 'bg-raised text-fg' : 'text-muted hover:text-muted')}>{value}</button>
                  ))}
                </div>
                <button type="button" disabled={visibleLogs.length === 0} onClick={exportActivity} className="focus-ring rounded-lg border border-control px-2.5 py-1.5 text-caption font-semibold text-muted transition hover:text-fg-secondary disabled:opacity-40">Export JSON</button>
              </div>
            </div>
            <div className="activity-stream data-grid min-h-0 flex-1 overflow-y-auto p-3 font-mono text-caption leading-relaxed">
              {visibleLogs.length === 0 ? (
                <div className="grid h-full min-h-72 place-items-center text-center">
                  <div className="chart-ready-card rounded-xl border px-5 py-4 backdrop-blur-xl">
                    <Icon name="activity" className="mx-auto h-5 w-5 text-accent" />
                    <p className="mt-3 font-sans font-medium text-muted">{syncLogs.length === 0 ? 'No activity recorded in this session.' : 'No events match this filter.'}</p>
                    <p className="mt-1 font-sans text-caption text-muted">Source discovery, parsing, and preservation events will stream here.</p>
                  </div>
                </div>
              ) : renderedLogs.map((log, index) => (
                <div key={`${index}-${log}`} className={cn('group grid grid-cols-[6px_minmax(0,1fr)] gap-3 rounded-lg px-2 py-2 transition hover:bg-surface/70', isErrorLog(log) ? 'text-danger' : 'text-muted')}>
                  <StatusDot className="mt-1" state={isErrorLog(log) ? 'error' : log.includes('completed') || log.includes('passed') ? 'active' : 'note'} label={isErrorLog(log) ? 'Error' : log.includes('completed') || log.includes('passed') ? 'Completed' : 'Notice'} />
                  <span className="break-words">{log}</span>
                </div>
              ))}
              {visibleLimit < visibleLogs.length && <button type="button" onClick={() => setVisibleLimit((value) => Math.min(visibleLogs.length, value + 75))} className="focus-ring mt-2 w-full rounded-lg border border-control py-2 text-caption font-semibold text-muted hover:text-fg-secondary">Show 75 older events</button>}
            </div>
            {syncLogs.length > 0 && (
              <div className="flex items-center justify-between border-t border-line px-4 py-3 text-caption text-muted">
                <span>{visibleLogs.length.toLocaleString()} of {syncLogs.length.toLocaleString()} events</span>
                <button type="button" onClick={resetMetrics} className="focus-ring rounded-lg px-2 py-1 font-semibold text-muted transition hover:bg-raised hover:text-fg-secondary">Clear session telemetry</button>
              </div>
            )}
          </section>

          <div className="space-y-4">
          <section className="app-panel min-h-[420px] overflow-hidden">
            <div className="border-b border-line px-4 py-3.5">
              <div className="app-eyebrow">Source runtimes</div>
              <div className="mt-1 text-caption text-muted">Queue and rate-budget health by jurisdiction</div>
            </div>
            <div className="space-y-2 p-3">
              {runtimes.length === 0 ? (
                <div className="data-grid grid min-h-72 place-items-center rounded-xl border border-dashed border-line bg-canvas/40 p-6 text-center">
                  <div>
                    <Icon name="database" className="mx-auto h-5 w-5 text-muted" />
                    <p className="mt-3 text-label font-medium text-muted">No active source runtimes</p>
                    <p className="mt-1 text-caption leading-relaxed text-muted">Connect or resume a source to watch its isolated queue here.</p>
                  </div>
                </div>
              ) : runtimes.map((runtime) => {
                const total = runtime.completedTasks + runtime.failedTasks + runtime.queuedTasks + runtime.activeTasks;
                const progress = total > 0 ? (runtime.completedTasks + runtime.failedTasks) / total : 0;
                return (
                  <article key={runtime.portal.portalKey} className="rounded-xl border border-line bg-canvas/40 p-3.5">
                    <div className="flex items-start justify-between gap-3">
                      <div className="min-w-0">
                        <div className="truncate text-label font-semibold text-fg-secondary">{runtime.portal.displayName}</div>
                        <div className="mt-1 font-mono text-caption text-muted">{runtime.portal.portalKey}</div>
                      </div>
                      <span className={cn('flex items-center gap-1.5 rounded-full border px-2 py-1 text-caption font-semibold', runtime.isRunning ? 'border-success/30 bg-success/10 text-success' : 'border-line-strong bg-surface text-muted')}>
                        <StatusDot state={runtime.isRunning ? 'active' : 'idle'} label={runtime.isRunning ? 'Running' : 'Stopped'} />
                        {runtime.isRunning ? 'Running' : 'Idle'}
                      </span>
                    </div>
                    <div className="mt-3 h-1.5 overflow-hidden rounded-full bg-raised">
                      <div className="h-full rounded-full bg-gradient-to-r from-accent to-highlight transition-[width]" style={{ width: `${Math.max(progress * 100, runtime.isRunning ? 3 : 0)}%` }} />
                    </div>
                    <dl className="mt-3 grid grid-cols-3 gap-2 text-center">
                      <RuntimeValue label="Done" value={runtime.completedTasks} />
                      <RuntimeValue label="Active" value={runtime.activeTasks} />
                      <RuntimeValue label="Queued" value={runtime.queuedTasks} />
                    </dl>
                    <div className="mt-3 flex items-center justify-between border-t border-line pt-2 font-mono text-caption text-muted">
                      <span>{runtime.rateLimitTokens.toFixed(1)} rate tokens</span>
                      <span>{runtime.documentsProcessed.toLocaleString()} documents</span>
                    </div>
                  </article>
                );
              })}
            </div>
          </section>
          <AuditTimeline />
          </div>
        </div>
      </div>
    </div>
  );
}

function ActivityMetric({ label, value, detail, color, accent }: { label: string; value: number; detail: string; color: string; accent: string }) {
  return (
    <article className="instrument-cell relative overflow-hidden p-4">
      <span className={cn('instrument-accent absolute left-0 top-5 h-8 w-0.5 rounded-full', accent)} aria-hidden="true" />
      <div className="app-eyebrow">{label}</div>
      <div className={cn('numeric-value mt-2 text-2xl font-semibold tracking-tight', color)}>{value.toLocaleString()}</div>
      <div className="mt-1 text-caption text-muted">{detail}</div>
    </article>
  );
}

function RuntimeValue({ label, value }: { label: string; value: number }) {
  return <div className="rounded-lg bg-surface/70 px-2 py-2"><dt className="text-caption uppercase tracking-wider text-muted">{label}</dt><dd className="mt-1 font-mono text-label text-muted">{value.toLocaleString()}</dd></div>;
}

function isErrorLog(value: string) {
  const normalized = value.toLocaleLowerCase();
  return normalized.includes('failed') || normalized.includes('error') || normalized.includes('warning');
}

function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes.toLocaleString()} B preserved`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB preserved`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB preserved`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB preserved`;
}
