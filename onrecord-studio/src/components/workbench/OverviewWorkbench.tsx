import { useCallback, useMemo, useRef, useState, type ReactNode } from 'react';
import { invokeCommand } from '../../ipc';
import { useCaseScope } from '../../hooks/useCaseScope';
import { useGatedLoad } from '../../hooks/useGatedLoad';
import { useKeepInView } from '../../hooks/useKeepInView';
import { useRankedLeads } from '../../hooks/useRankedLeads';
import { nextUpcomingMeeting } from '../../hooks/useWorkspaceBadges';
import { useAppStore } from '../../store/useAppStore';
import { comparisonOf } from '../../utils/attentionFactors';
import { cn } from '../../utils/cn';
import { meetingWhen } from '../dashboard/AttentionFeed';
import CaseScopeBar from '../dashboard/CaseScopeBar';
import { useSinceInstant } from '../dashboard/SinceControl';
import { findingFamily } from '../objects/findingFamily';
import { formatShortDate } from '../objects/labels';
import Badge from '../ui/Badge';
import Button from '../ui/Button';
import Icon, { type IconName } from '../ui/icons';
import StatusDot from '../ui/statusDot';
import LeadCard from './LeadCard';
import LeadDetail, { ContextSkeleton, StageSkeleton } from './LeadDetail';
import Workbench, { LeadQueue, Pane, QueueSkeleton, THREE_COLUMNS_FROM, useWorkbenchQueue, useWorkbenchWidth } from './Workbench';

const MEETING_HORIZON_DAYS = 14;
/** The deadline ledger's selection id; never a finding id, which is a hex digest. */
export const LEDGER_ID = 'ledger';
const WATCHING_OPEN_KEY = 'overview.watching.open';
/** The window height below which Watching starts folded, so the queue keeps four cards in view. */
const WATCHING_OPEN_MIN_HEIGHT = 860;
/** Watch changes read for the Watching row, newest first; a full page in the window is a floor. */
const WATCH_CHANGE_PAGE = 20;

/**
 * The overview as an investigation workbench: the ranked leads in the Since window and the watch
 * status on the left, the selected lead in the centre, the records around it on the right. The
 * queue is the Story desk's ranking, so "#3" is the same lead on both. The counted deadline ledger
 * is watched, not queued (its totals are all-time, not found in this window); selecting it draws
 * the ledger on the stage.
 */
export default function OverviewWorkbench({ ledgerFigure, onOpenLedger, onOpenFollowing }: {
  ledgerFigure: ReactNode;
  onOpenLedger(): void;
  onOpenFollowing(): void;
}) {
  const since = useSinceInstant();
  const portals = useAppStore((state) => state.portals);
  const setSincePreset = useAppStore((state) => state.setSincePreset);
  const requestFindingsRefresh = useAppStore((state) => state.requestFindingsRefresh);
  const selected = useAppStore((state) => state.workbenchSelection.overview ?? null);
  const selectWorkbenchLead = useAppStore((state) => state.selectWorkbenchLead);
  const caseScope = useCaseScope();
  const ranked = useRankedLeads();
  const ledger = useLedgerTotals();
  const { leads } = ranked;

  const sourceName = useCallback((portalKey: string | null) => portals.find((saved) => saved.registration.portalKey === portalKey)?.registration.displayName, [portals]);
  const queue = useMemo(() => leads.map((lead, index) => ({ id: lead.findingId, headline: lead.headline, kind: findingFamily(lead.kind), rank: index + 1, lead })), [leads]);
  const current = selected === LEDGER_ID && ledger ? LEDGER_ID : queue.some((item) => item.id === selected) ? selected : queue[0]?.id ?? null;
  useWorkbenchQueue('overview', ranked.status === 'ready' ? queue : null, current);
  const select = useCallback((id: string) => selectWorkbenchLead('overview', id), [selectWorkbenchLead]);
  const selectIfQueued = useCallback((id: string) => {
    if (!queue.some((item) => item.id === id)) return false;
    select(id);
    return true;
  }, [queue, select]);
  const index = queue.findIndex((item) => item.id === current);

  // A source name says something only when the queue holds more than one source's leads.
  const spansSources = new Set(leads.map((lead) => lead.portalKey).filter(Boolean)).size > 1;
  const scaleMax = leads[0]?.publicInterest.totalBasisPoints ?? 1;
  const items = leads.map((lead, at) => ({
    id: lead.findingId,
    content: <LeadCard lead={lead} rank={at + 1} scaleMax={scaleMax} source={spansSources ? sourceName(lead.portalKey) : undefined} />,
  }));

  const navigator = (
    <div className="workbench-nav">
      <Pane
        label="Lead queue"
        title="Leads"
        meta={ranked.status === 'ready' ? leads.length.toLocaleString() : undefined}
        actions={<span className="text-caption text-muted">by attention score</span>}
        className="workbench-queue"
        bodyClassName="overflow-y-auto"
      >
        <div aria-busy={ranked.refreshing || undefined}>
          <div className="px-2 pt-2 empty:hidden"><CaseScopeBar state={caseScope} noun="leads" /></div>
          {ranked.status === 'pending' ? <QueueSkeleton /> : ranked.status === 'failed' ? (
            <div role="alert" className="px-4 py-4">
              <p className="text-body font-medium text-fg">Leads couldn’t be loaded.</p>
              {ranked.message && <p className="mt-1 break-words text-caption text-muted">{ranked.message}</p>}
              <Button size="sm" className="mt-3" onClick={requestFindingsRefresh}>Try again</Button>
            </div>
          ) : items.length === 0 ? (
            <div className="px-4 py-6 text-center">
              {since && ranked.total > 0 ? (
                <>
                  <p className="text-heading text-fg-secondary">Nothing new since {formatShortDate(since)}</p>
                  <p className="mt-1 text-caption text-muted">
                    Widen the window to see every lead ranked so far.{' '}
                    <Button variant="link" size="sm" className="h-auto px-0 align-baseline text-caption" onClick={() => setSincePreset('all')}>Any time</Button>
                  </p>
                </>
              ) : (
                <>
                  <p className="text-heading text-fg-secondary">No leads yet</p>
                  <p className="mt-1 text-caption text-muted">No supported accountability lead is available yet. Choose an issue and collect its records; record-level clues remain in Story desk’s All findings.</p>
                </>
              )}
            </div>
          ) : (
            <LeadQueue label="Leads by attention score" items={items} selectedId={current} onSelect={select} />
          )}
        </div>
      </Pane>
      <details className="order-3 rounded-lg border border-line p-3">
        <summary className="focus-ring cursor-pointer text-caption text-muted">Deadlines and saved searches</summary>
        <WatchStatus ledger={ledger} ledgerSelected={current === LEDGER_ID} onSelectLedger={() => select(LEDGER_ID)} onOpenFollowing={onOpenFollowing} />
      </details>
    </div>
  );

  let main: ReactNode;
  if (current === LEDGER_ID) {
    main = (
      // The lead stage's own header, so the stage keeps one height and one header row whichever it shows.
      <Pane
        label="Selected lead"
        title="Deadline ledger"
        role="stage"
        actions={<Button size="sm" variant="ghost" icon="bars" onClick={onOpenLedger}>Open in response analytics</Button>}
        elevated
      >
        <article aria-label="Deadline ledger" className="motion-fade px-6 py-6">
          <p className="mb-4 text-caption text-muted">All-time totals across every request, not only this window.</p>
          {ledgerFigure}
        </article>
      </Pane>
    );
  } else if (ranked.status === 'pending') {
    main = <PendingDetail />;
  } else if (!current) {
    main = (
      <Pane label="Selected lead" role="stage" elevated>
        <StageWaiting text={ranked.status === 'failed' ? 'A lead opens here once the queue loads.' : 'When a lead is ranked it opens here, with its figure, its evidence and the records around it.'} />
      </Pane>
    );
  } else {
    const lead = leads[index];
    main = (
      <LeadDetail
        findingId={current}
        storyLead={lead}
        position={{ index: index + 1, total: leads.length }}
        compare={comparisonOf(leads, index)}
        source={spansSources ? sourceName(lead.portalKey) : undefined}
        onSelectFinding={selectIfQueued}
      />
    );
  }

  return <Workbench navigator={navigator} wide={current === LEDGER_ID || (!current && ranked.status !== 'pending')}>{main}</Workbench>;
}

/** The stage and context while the queue is still being read: shapes, never a number. */
export function PendingDetail() {
  return (
    <>
      <Pane label="Selected lead" title="Lead" role="stage" elevated><StageSkeleton /></Pane>
      <Pane label="Records around this lead" title="Around this lead" role="context"><ContextSkeleton /></Pane>
    </>
  );
}

/** The whole workbench while the saved sources load, laid out as it will be once they have. */
export function WorkbenchSkeleton() {
  return (
    <Workbench
      navigator={(
        <div className="workbench-nav">
          <Pane label="Lead queue" title="Leads" className="workbench-queue" bodyClassName="overflow-hidden"><QueueSkeleton /></Pane>
          <section aria-label="Watch status" className="workbench-watch workbench-pane flex shrink-0 flex-col rounded-xl border border-line bg-panel">
            <header className="flex h-11 items-center gap-2 px-4"><h2 className="text-overline uppercase text-muted">Watching</h2><div aria-hidden="true" className="skeleton ml-2 h-3 w-32 rounded" /></header>
          </section>
        </div>
      )}
    >
      <PendingDetail />
    </Workbench>
  );
}

function StageWaiting({ text }: { text: string }) {
  return (
    <div className="grid h-full min-h-[18rem] place-items-center px-8 py-12 text-center">
      <div className="max-w-sm">
        <span aria-hidden="true" className="mx-auto grid h-10 w-10 place-items-center rounded-xl bg-raised text-fg-secondary"><Icon name="flag" size="lg" /></span>
        <p className="mt-3 text-body text-muted">{text}</p>
      </div>
    </div>
  );
}

interface LedgerTotals {
  misses: number;
  requests: number;
  departments: number;
}

/**
 * The counted deadline ledger's all-time totals over the profiles the analytics ledger draws (the
 * active source's, or every source's). Null while a case scope is on, since these totals cover
 * every request rather than the case's, and when no department has a counted miss.
 */
export function useLedgerTotals(): LedgerTotals | null {
  const profiles = useAppStore((state) => state.agencyProfiles);
  const activePortalKey = useAppStore((state) => state.activePortalKey);
  const caseScope = useCaseScope();
  return useMemo(() => {
    if (caseScope.status !== 'off') return null;
    const scoped = profiles.filter((profile) => !activePortalKey || profile.portalSubdomain === activePortalKey);
    const misses = scoped.reduce((sum, profile) => sum + profile.statutoryViolationCount, 0);
    if (misses === 0) return null;
    return {
      misses,
      requests: scoped.reduce((sum, profile) => sum + profile.totalRequestsProcessed, 0),
      departments: scoped.filter((profile) => profile.statutoryViolationCount > 0).length,
    };
  }, [activePortalKey, caseScope.status, profiles]);
}

function readWatchingOpen(): boolean | null {
  try {
    const stored = window.localStorage.getItem(WATCHING_OPEN_KEY);
    return stored === 'true' ? true : stored === 'false' ? false : null;
  } catch {
    return null;
  }
}

function storeWatchingOpen(open: boolean) {
  try {
    window.localStorage.setItem(WATCHING_OPEN_KEY, String(open));
  } catch {
    // An unwritable store only means the choice is not remembered next launch.
  }
}

/** One watch row: an icon tile, a title and an optional figure, then lines that wrap, never truncate. */
function WatchRow({ icon, title, figure, lines, current, onClick, children }: {
  icon: IconName;
  title: ReactNode;
  figure?: ReactNode;
  lines?: ReactNode[];
  current?: boolean;
  onClick(): void;
  children?: ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-current={current ? 'true' : undefined}
      className="lead-card focus-ring flex w-full items-start gap-2.5 rounded-lg border border-transparent px-2.5 py-2 text-left hover:bg-raised/60"
    >
      <span aria-hidden="true" className="grid h-6 w-6 shrink-0 place-items-center rounded-md bg-raised text-fg-secondary"><Icon name={icon} size="sm" /></span>
      <span className="min-w-0 flex-1">
        <span className="flex min-h-6 items-start gap-2">
          <span className="flex min-h-6 min-w-0 flex-1 flex-wrap items-center gap-x-1.5 text-body font-medium text-fg">{title}</span>
          {figure !== undefined && <span className="shrink-0 pt-0.5 text-caption font-medium tabular-nums text-fg-secondary">{figure}</span>}
        </span>
        {lines?.map((line, at) => <span key={at} className="block text-caption tabular-nums text-muted">{line}</span>)}
        {children}
      </span>
    </button>
  );
}

/** Collection work across every saved source, settled and waiting, on one track. */
function CollectionMeter({ completed, failed, active, total }: { completed: number; failed: number; active: number; total: number }) {
  const share = (count: number) => `${(count / total) * 100}%`;
  return (
    <span
      role="progressbar"
      aria-label="Collection tasks settled across sources"
      aria-valuemin={0}
      aria-valuemax={total}
      aria-valuenow={completed + failed}
      aria-valuetext={`${(completed + failed).toLocaleString()} of ${total.toLocaleString()} tasks settled${failed > 0 ? `, ${failed.toLocaleString()} failed` : ''}`}
      className="mt-2 flex h-1.5 w-full overflow-hidden rounded-full bg-viz-track"
    >
      <span className="h-full bg-success-mark" style={{ width: share(completed) }} />
      {failed > 0 && <span className="watch-meter-failed h-full" style={{ width: share(failed) }} />}
      {active > 0 && <span className="h-full bg-success-mark/50" style={{ width: share(active) }} />}
    </span>
  );
}

/**
 * What the reader keeps an eye on: collection across every saved source (the sidebar lists them
 * one by one; this is their sum, so a stalled run shows without opening anything), the counted
 * deadline ledger, the next public meeting, and what the watches saw change. Folded, the header
 * carries the same facts in one line.
 */
function WatchStatus({ ledger, ledgerSelected, onSelectLedger, onOpenFollowing }: {
  ledger: LedgerTotals | null;
  ledgerSelected: boolean;
  onSelectLedger(): void;
  onOpenFollowing(): void;
}) {
  const portals = useAppStore((state) => state.portals);
  const runtimes = useAppStore((state) => state.portalRuntimes);
  const syncing = useAppStore((state) => state.syncingPortalKeys);
  const findingsRefreshRequests = useAppStore((state) => state.findingsRefreshRequests);
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);
  const since = useSinceInstant();
  const width = useWorkbenchWidth();
  const [chosen, setChosen] = useState<boolean | null>(readWatchingOpen);
  // Until the reader chooses, Watching is open only where the navigator has room for it and four
  // queued leads beside it.
  const open = chosen ?? (width >= THREE_COLUMNS_FROM && window.innerHeight >= WATCHING_OPEN_MIN_HEIGHT);
  const toggle = () => {
    setChosen(!open);
    storeWatchingOpen(!open);
  };
  // The ledger row drawn on the stage stays in view inside Watching's own scroller.
  const body = useRef<HTMLDivElement>(null);
  useKeepInView(body, '[aria-current="true"]', [ledgerSelected, open]);
  const calendar = useGatedLoad('calendar', () => invokeCommand('get_meeting_calendar', {}));
  const changes = useGatedLoad(`changes:${findingsRefreshRequests}`, () => invokeCommand('list_watch_changes', { watchId: null, limit: WATCH_CHANGE_PAGE }));

  const live = new Set(portals.map((saved) => saved.registration.portalKey));
  const running = runtimes.filter((runtime) => live.has(runtime.portal.portalKey));
  const sum = (pick: (runtime: (typeof running)[number]) => number) => running.reduce((total, runtime) => total + pick(runtime), 0);
  const completed = sum((runtime) => runtime.completedTasks);
  const failed = sum((runtime) => runtime.failedTasks);
  const active = sum((runtime) => runtime.activeTasks);
  const total = completed + failed + active + sum((runtime) => runtime.queuedTasks);
  const settled = completed + failed;
  const paused = portals.filter((saved) => saved.lifecycleState === 'PAUSED').length;
  const degraded = portals.filter((saved) => saved.healthStatus === 'degraded').length;
  const collecting = syncing.filter((key) => live.has(key)).length;
  const meeting = calendar.status === 'ready' ? nextUpcomingMeeting(calendar.data) : null;
  const upcoming = meeting && meeting.days <= MEETING_HORIZON_DAYS ? meeting : null;
  const recent = changes.status === 'ready' ? changes.data.filter((change) => !since || change.detectedAt > since) : [];
  // The Following tab counts changes since each watch's last check; this counts the Since window,
  // and says so, so the two figures never read as one count that disagrees with itself.
  const recentCount = `${recent.length.toLocaleString()}${changes.status === 'ready' && recent.length >= WATCH_CHANGE_PAGE ? '+' : ''}`;
  const recentWords = `${recentCount} watch change${recent.length === 1 ? '' : 's'}${since ? ` since ${formatShortDate(since)}` : ''}`;
  const state = [
    collecting > 0 && `${collecting} collecting`,
    paused > 0 && `${paused} paused`,
    degraded > 0 && `${degraded} need${degraded === 1 ? 's' : ''} attention`,
  ].filter(Boolean).join(' · ') || 'none collecting';
  const sources = `${portals.length.toLocaleString()} source${portals.length === 1 ? '' : 's'}`;
  const percent = total > 0 ? `${Math.floor((settled / total) * 100)}%` : null;
  const dot = <StatusDot state={degraded > 0 ? 'error' : collecting > 0 ? 'active' : 'idle'} pulse={collecting > 0 && degraded === 0} label={degraded > 0 ? 'needs attention' : collecting > 0 ? 'collecting' : 'idle'} />;
  // Folded, the header carries the same facts as figures beside the icons the rows use; each
  // states its words to a screen reader, so the line stays one line at the navigator's width.
  const summary = [
    <span key="sources" className="inline-flex items-center gap-1" title={sources}>
      <Icon name="database" size="xs" className="text-muted" /><span aria-hidden="true">{portals.length.toLocaleString()}</span><span className="sr-only">{sources}</span>{dot}
    </span>,
    percent && <span key="percent" title={`${settled.toLocaleString()} of ${total.toLocaleString()} tasks settled`}>{percent}<span className="sr-only"> of collection tasks settled</span></span>,
    ledger && <span key="ledger" title={`${ledger.misses.toLocaleString()} counted deadline misses`}><span aria-hidden="true">§ {ledger.misses.toLocaleString()}</span><span className="sr-only">{ledger.misses.toLocaleString()} counted deadline misses</span></span>,
    changes.status === 'ready' && (
      <span key="changes" className="inline-flex items-center gap-1" title={recentWords}>
        <Icon name="activity" size="xs" className="text-muted" /><span aria-hidden="true">{recentCount}</span><span className="sr-only">{recentWords}</span>
      </span>
    ),
  ].filter(Boolean);

  return (
    <section aria-label="Watch status" className={cn('workbench-watch workbench-pane flex shrink-0 flex-col rounded-xl border border-line bg-panel', open && 'workbench-watch-open')}>
      <h2 className="shrink-0">
        <button
          type="button"
          aria-expanded={open}
          aria-controls="watching-body"
          onClick={toggle}
          className="focus-ring flex h-11 w-full items-center gap-2 rounded-xl px-4 text-left"
        >
          <span className="text-overline uppercase text-muted">Watching</span>
          {!open && (
            <span className="flex min-w-0 flex-1 items-center gap-2.5 whitespace-nowrap text-caption tabular-nums text-fg-secondary">{summary}</span>
          )}
          <Icon name="chevron" size="xs" className={cn('ml-auto shrink-0 text-muted transition-transform', open && 'rotate-90')} />
        </button>
      </h2>
      {open && (
        <div ref={body} id="watching-body" className="min-h-0 space-y-0.5 overflow-y-auto overscroll-contain border-t border-line p-2">
          <WatchRow
            icon="database"
            title={<><span>{sources}</span>{dot}</>}
            figure={percent ?? undefined}
            lines={[
              state,
              total > 0
                ? <>{settled.toLocaleString()}/{total.toLocaleString()} settled{failed > 0 && <span className="whitespace-nowrap text-danger"> · {failed.toLocaleString()} failed</span>}</>
                : 'no queue this session',
            ]}
            onClick={() => setWorkspaceView('activity')}
          >
            {total > 0 && <CollectionMeter completed={completed} failed={failed} active={active} total={total} />}
          </WatchRow>
          {ledger && (
            <WatchRow
              icon="bars"
              title={<><span>Deadline ledger</span><Badge tone="violation" className="h-[18px] px-1.5">Counted</Badge></>}
              lines={[
                `${ledger.misses.toLocaleString()} missed of ${ledger.requests.toLocaleString()} requests`,
                `${ledger.departments.toLocaleString()} department${ledger.departments === 1 ? '' : 's'} · all\u2011time`,
              ]}
              current={ledgerSelected}
              onClick={onSelectLedger}
            />
          )}
          {upcoming && (
            <WatchRow
              icon="calendar"
              title={<span>{upcoming.body}</span>}
              figure={upcoming.days === 0 ? 'today' : `in ${upcoming.days} day${upcoming.days === 1 ? '' : 's'}`}
              lines={[upcoming.meeting.date ? meetingWhen(upcoming.meeting.date, upcoming.meeting.startTime) : 'Date not published']}
              onClick={() => setWorkspaceView('meetings')}
            />
          )}
          <WatchRow
            icon="activity"
            title={changes.status === 'ready' && recent.length > 0 ? `${recentCount} watch change${recent.length === 1 ? '' : 's'}` : 'Watch changes'}
            figure={changes.status === 'failed' ? 'didn’t load'
              : changes.status !== 'ready' ? undefined
                : recent.length === 0 ? (since ? `none since ${formatShortDate(since)}` : 'none yet')
                  : since ? `since ${formatShortDate(since)}` : undefined}
            lines={recent[0] ? [`${recent[0].subject}: ${recent[0].oldValue ?? 'not in baseline'} → ${recent[0].newValue ?? 'no longer present'}`] : undefined}
            onClick={onOpenFollowing}
          />
        </div>
      )}
    </section>
  );
}
