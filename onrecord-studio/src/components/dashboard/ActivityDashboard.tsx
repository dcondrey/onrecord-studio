import { useEffect, useMemo, useState } from 'react';
import { useAppStore } from '../../store/useAppStore';
import { cn } from '../../utils/cn';
import { useMetrics } from '../../hooks/useMetrics';
import Badge from '../ui/Badge';
import Button from '../ui/Button';
import { SearchField } from '../ui/Field';
import PageHeader from '../ui/PageHeader';
import { PanelHeader } from '../ui/Panel';
import SegmentedControl from '../ui/SegmentedControl';
import Stat from '../ui/Stat';
import StatusDot from '../ui/statusDot';
import { downloadFile } from './downloadFile';

type LogFilter = 'all' | 'errors';

export default function ActivityDashboard() {
  const runtimes = useAppStore((state) => state.portalRuntimes);
  const syncLogs = useAppStore((state) => state.syncLogs);
  const syncingPortalKeys = useAppStore((state) => state.syncingPortalKeys);
  const collecting = syncingPortalKeys.length;
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
    downloadFile(`onrecord-activity-${new Date().toISOString().replace(/[:.]/g, '-')}.json`, JSON.stringify(artifact, null, 2), 'application/json');
  };

  return (
    <div className="min-h-0 flex-1 overflow-y-auto">
      <div className="mx-auto w-full max-w-content space-y-6 px-4 pb-10 pt-6 md:px-6">
        <PageHeader
          title="Collection activity"
          summary="Source queues, parsing throughput and problems for this session."
          actions={
            <Badge tone={collecting > 0 ? 'success' : 'neutral'} className="h-8 gap-2 px-3 text-control">
              <StatusDot state={collecting > 0 ? 'active' : 'idle'} pulse={collecting > 0} label={collecting > 0 ? 'Collecting' : 'Idle'} />
              {collecting > 0 ? `${collecting.toLocaleString()} ${collecting === 1 ? 'source' : 'sources'} collecting` : 'Nothing collecting'}
            </Badge>
          }
        />

        <section aria-label="Session throughput" className="app-panel grid grid-cols-1 divide-y divide-line overflow-hidden sm:grid-cols-2 sm:divide-y-0 xl:grid-cols-4 xl:divide-x">
          <Stat label="Requests" value={metrics.requestsProcessed} detail="This session, from runtime snapshots" />
          <Stat label="Tasks completed" value={metrics.completedTasks} detail={`This session · ${metrics.queuedTasks.toLocaleString()} waiting in queues`} />
          <Stat label="Documents saved" value={metrics.documentsDownloaded} detail={`This session · ${formatBytes(metrics.bytesTransferred)}`} />
          <Stat
            label="Failed tasks"
            value={metrics.failedTasks}
            detail={metrics.taskFailureRatePercentage === null ? 'No tasks have finished yet' : `${metrics.taskFailureRatePercentage.toFixed(1)}% of finished tasks`}
            className={metrics.failedTasks > 0 ? '[&_.numeric-value]:text-danger' : undefined}
          />
        </section>

        <div className="grid grid-cols-1 gap-4 xl:grid-cols-[minmax(0,1fr)_380px]">
          <section aria-labelledby="event-stream-heading" className="app-panel flex min-h-[520px] flex-col overflow-hidden">
            <PanelHeader
              titleId="event-stream-heading"
              title="Event stream"
              description="Newest first."
              actions={<>
                <SearchField size="sm" value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Filter events" aria-label="Filter activity" wrapperClassName="w-44" />
                <SegmentedControl<LogFilter>
                  label="Event level"
                  mode="filter"
                  size="sm"
                  value={filter}
                  onChange={setFilter}
                  options={[{ value: 'all', label: 'All' }, { value: 'errors', label: 'Problems' }]}
                />
                <Button size="sm" disabled={visibleLogs.length === 0} onClick={exportActivity} className="whitespace-nowrap">Export</Button>
              </>}
            />
            <div className="activity-stream min-h-0 flex-1 overflow-y-auto p-3 font-mono text-code">
              {visibleLogs.length === 0 ? (
                <div className="grid h-full min-h-72 place-items-center px-6 text-center font-sans">
                  <div>
                    <p className="text-heading text-fg-secondary">{syncLogs.length === 0 ? 'No activity recorded in this session' : 'No events match this filter'}</p>
                    <p className="mt-1 text-caption text-muted">Source discovery, parsing and preservation events appear here as they happen.</p>
                  </div>
                </div>
              ) : renderedLogs.map((log, index) => (
                <LogLine key={`${index}-${log}`} log={log} />
              ))}
              {visibleLimit < visibleLogs.length && (
                <Button size="sm" variant="ghost" className="mt-2 w-full font-sans" onClick={() => setVisibleLimit((value) => Math.min(visibleLogs.length, value + 75))}>Show 75 older events</Button>
              )}
            </div>
            {syncLogs.length > 0 && (
              <div className="flex items-center justify-between border-t border-line px-5 py-3 text-caption text-muted">
                <span className="tabular-nums">{visibleLogs.length.toLocaleString()} of {syncLogs.length.toLocaleString()} events</span>
                <Button size="sm" variant="ghost" onClick={resetMetrics}>Clear session telemetry</Button>
              </div>
            )}
          </section>

          <section aria-labelledby="runtimes-heading" className="app-panel min-h-[420px] overflow-hidden">
            <PanelHeader titleId="runtimes-heading" title="Source runtimes" description="Queue health for each source." />
            <div className="space-y-3 p-4">
              {runtimes.length === 0 ? (
                <div className="grid min-h-72 place-items-center rounded-lg border border-dashed border-line p-6 text-center">
                  <div>
                    <p className="text-heading text-fg-secondary">No source runtimes</p>
                    <p className="mt-1 text-caption text-muted">Connect or resume a source to see its queue here.</p>
                  </div>
                </div>
              ) : runtimes.map((runtime) => {
                const total = runtime.completedTasks + runtime.failedTasks + runtime.queuedTasks + runtime.activeTasks;
                const progress = total > 0 ? (runtime.completedTasks + runtime.failedTasks) / total : 0;
                return (
                  <article key={runtime.portal.portalKey} className="rounded-lg border border-line p-4">
                    <div className="flex items-center justify-between gap-3">
                      <h4 className="min-w-0 truncate text-control text-fg" title={runtime.portal.displayName}>{runtime.portal.displayName}</h4>
                      <Badge tone={runtime.isRunning ? 'success' : 'neutral'}>
                        <StatusDot state={runtime.isRunning ? 'active' : 'idle'} pulse={runtime.isRunning} label={runtime.isRunning ? 'Running' : 'Idle'} />
                        {runtime.isRunning ? 'Running' : 'Idle'}
                      </Badge>
                    </div>
                    <div className="mt-3 h-1.5 overflow-hidden rounded-full bg-raised" role="progressbar" aria-label={`${runtime.portal.displayName} progress`} aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(progress * 100)}>
                      <div className="h-full origin-left rounded-full bg-accent transition-transform" style={{ transform: `scaleX(${Math.max(progress, runtime.isRunning ? 0.03 : 0)})` }} />
                    </div>
                    <dl className="mt-3 flex flex-wrap gap-x-4 gap-y-1 text-caption text-muted">
                      <RuntimeValue label="Done" value={runtime.completedTasks} />
                      <RuntimeValue label="Active" value={runtime.activeTasks} />
                      <RuntimeValue label="Queued" value={runtime.queuedTasks} />
                      <RuntimeValue label="Documents" value={runtime.documentsProcessed} />
                    </dl>
                  </article>
                );
              })}
            </div>
          </section>
        </div>
      </div>
    </div>
  );
}

function RuntimeValue({ label, value }: { label: string; value: number }) {
  return (
    <div className="flex items-baseline gap-1">
      <dt>{label}</dt>
      <dd className="tabular-nums text-fg-secondary">{value.toLocaleString()}</dd>
    </div>
  );
}

function LogLine({ log }: { log: string }) {
  const error = isErrorLog(log);
  const completed = !error && (log.includes('completed') || log.includes('passed'));
  return (
    <div className={cn('grid grid-cols-[8px_minmax(0,1fr)] items-start gap-3 rounded-md px-2 py-1.5 transition-colors hover:bg-raised/60', error ? 'text-danger' : 'text-fg-secondary')}>
      <StatusDot className="mt-1" state={error ? 'error' : completed ? 'active' : 'note'} label={error ? 'Problem' : completed ? 'Completed' : 'Notice'} />
      <span className="break-words">{log}</span>
    </div>
  );
}

function isErrorLog(value: string) {
  const normalized = value.toLocaleLowerCase();
  return normalized.includes('failed') || normalized.includes('error') || normalized.includes('warning');
}

function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes.toLocaleString()} B saved`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB saved`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB saved`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB saved`;
}
