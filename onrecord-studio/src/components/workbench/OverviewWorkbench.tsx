import { useCallback, useMemo, type ReactNode } from 'react';
import { invokeCommand } from '../../ipc';
import { useCaseScope } from '../../hooks/useCaseScope';
import { useGatedLoad } from '../../hooks/useGatedLoad';
import { nextUpcomingMeeting } from '../../hooks/useWorkspaceBadges';
import { useAppStore } from '../../store/useAppStore';
import type { Finding, StoryLead } from '../../types';
import { isLegalClaimKind } from '../../types/findings';
import { cn } from '../../utils/cn';
import { deadlineLedgerHeadline, meetingWhen } from '../dashboard/AttentionFeed';
import CaseScopeBar from '../dashboard/CaseScopeBar';
import { useSinceInstant } from '../dashboard/SinceControl';
import { findingFamily } from '../objects/findingFamily';
import { formatShortDate } from '../objects/labels';
import Badge from '../ui/Badge';
import Button from '../ui/Button';
import FamilyBadge from '../ui/FamilyBadge';
import Icon, { type IconName } from '../ui/icons';
import StatusDot from '../ui/statusDot';
import LeadCard from './LeadCard';
import LeadDetail from './LeadDetail';
import Workbench, { LeadQueue, Pane, useWorkbenchQueue } from './Workbench';

const FINDINGS_SCAN_LIMIT = 200;
const MEETING_HORIZON_DAYS = 14;
/** The deadline ledger's selection id; never a finding id, which is a hex digest. */
const LEDGER_ID = 'ledger';
const SEVERITY_RANK = { CRITICAL: 100, SERIOUS: 70, NOTABLE: 40, INFO: 10 } as const;

/** Severity first; a counted statutory violation outranks a pattern signal of the same weight. */
function rankLeads(findings: readonly Finding[]): Finding[] {
  const rank = (finding: Finding) => SEVERITY_RANK[finding.severity] + (isLegalClaimKind(finding.kind) ? 15 : 0);
  return findings.filter((finding) => !finding.dismissedAt).sort((a, b) => rank(b) - rank(a) || b.discoveredAt.localeCompare(a.discoveredAt));
}

/**
 * The overview as an investigation workbench: the leads found in the Since window and the watch
 * status on the left, the selected lead in the centre, the records around it on the right. The
 * counted deadline ledger is watched, not queued (its totals are all-time, not found in this
 * window); selecting it draws the ledger on the stage.
 */
export default function OverviewWorkbench({ ledgerFigure, onOpenLedger, onOpenFollowing }: {
  ledgerFigure: ReactNode;
  onOpenLedger(): void;
  onOpenFollowing(): void;
}) {
  const sinceInstant = useSinceInstant();
  const findingsRefreshRequests = useAppStore((state) => state.findingsRefreshRequests);
  const portals = useAppStore((state) => state.portals);
  const setSincePreset = useAppStore((state) => state.setSincePreset);
  const selected = useAppStore((state) => state.workbenchSelection.overview ?? null);
  const selectWorkbenchLead = useAppStore((state) => state.selectWorkbenchLead);
  const agencyProfiles = useAppStore((state) => state.agencyProfiles);
  const activePortalKey = useAppStore((state) => state.activePortalKey);
  const caseScope = useCaseScope();

  const findings = useGatedLoad(`${sinceInstant}:${findingsRefreshRequests}`, () => invokeCommand('list_findings', { filter: { discoveredAfter: sinceInstant }, limit: FINDINGS_SCAN_LIMIT }));
  const ranking = useGatedLoad(`story:${findingsRefreshRequests}`, () => invokeCommand('list_story_leads', { limit: 100 }));

  const leads = useMemo(() => (findings.status === 'ready'
    ? rankLeads(findings.data.filter((finding) => caseScope.status !== 'ready' || caseScope.value.findingIds.has(finding.id)))
    : []), [caseScope, findings]);
  const ranked = useMemo(() => new Map<string, { lead: StoryLead; rank: number }>(
    ranking.status === 'ready' ? ranking.data.map((lead, index) => [lead.findingId, { lead, rank: index + 1 }]) : [],
  ), [ranking]);
  const ledger = useMemo(
    () => (caseScope.status === 'off' ? deadlineLedgerHeadline(agencyProfiles.filter((profile) => !activePortalKey || profile.portalSubdomain === activePortalKey)) : null),
    [activePortalKey, agencyProfiles, caseScope.status],
  );
  const sourceName = useCallback((portalKey: string | null) => portals.find((saved) => saved.registration.portalKey === portalKey)?.registration.displayName, [portals]);

  const queue = useMemo(() => leads.map((finding) => ({ id: finding.id, headline: finding.headline, kind: findingFamily(finding.kind) })), [leads]);
  useWorkbenchQueue('overview', findings.status === 'ready' ? queue : null);

  const current = selected === LEDGER_ID && ledger ? LEDGER_ID : queue.some((item) => item.id === selected) ? selected : queue[0]?.id ?? null;
  const select = useCallback((id: string) => selectWorkbenchLead('overview', id), [selectWorkbenchLead]);
  const selectIfQueued = useCallback((id: string) => {
    if (!queue.some((item) => item.id === id)) return false;
    select(id);
    return true;
  }, [queue, select]);
  const index = queue.findIndex((item) => item.id === current);
  const position = index >= 0 ? { index: index + 1, total: queue.length } : undefined;

  // A source name says something only when the queue holds more than one source's leads.
  const spansSources = new Set(leads.map((finding) => finding.subject.portalKey).filter(Boolean)).size > 1;
  const items = leads.map((finding) => ({
    id: finding.id,
    content: (
      <LeadCard
        badges={<>
          <FamilyBadge family={findingFamily(finding.kind)} />
          {isLegalClaimKind(finding.kind) && <Badge tone="violation">Counted</Badge>}
        </>}
        headline={finding.headline}
        severity={finding.severity}
        subject={spansSources ? sourceName(finding.subject.portalKey) : undefined}
        time={finding.discoveredAt}
      />
    ),
  }));

  const navigator = (
    <div className="workbench-nav">
      <Pane
        label="Lead queue"
        title="Leads"
        meta={findings.status === 'ready' ? `${leads.length.toLocaleString()}${findings.data.length === FINDINGS_SCAN_LIMIT ? '+' : ''}` : undefined}
        actions={sinceInstant ? <span className="text-caption text-muted">since {formatShortDate(sinceInstant)}</span> : <span className="text-caption text-muted">any time</span>}
        className="min-h-[15rem] flex-1"
        bodyClassName="overflow-y-auto"
      >
        <div className="px-2 pt-2 empty:hidden"><CaseScopeBar state={caseScope} noun="findings" /></div>
        {findings.status === 'pending' ? <QueueSkeleton /> : findings.status === 'failed' ? (
          <p role="alert" className="px-4 py-3 text-body text-danger">Leads couldn’t be loaded. {findings.message}</p>
        ) : items.length === 0 ? (
          <div className="px-4 py-6 text-center">
            <p className="text-heading text-fg-secondary">{sinceInstant ? `Nothing new since ${formatShortDate(sinceInstant)}` : 'No leads yet'}</p>
            <p className="mt-1 text-caption text-muted">
              {sinceInstant
                ? <>Widen the window to see every lead found so far. <Button variant="link" size="sm" className="h-auto px-0 align-baseline text-caption" onClick={() => setSincePreset('all')}>Any time</Button></>
                : 'Leads appear here as sources are collected and the detectors run.'}
            </p>
          </div>
        ) : (
          <LeadQueue label="Leads, most serious first" items={items} selectedId={current} onSelect={select} />
        )}
      </Pane>
      <WatchStatus ledger={ledger} ledgerSelected={current === LEDGER_ID} onSelectLedger={() => select(LEDGER_ID)} onOpenFollowing={onOpenFollowing} />
    </div>
  );

  let main: ReactNode;
  if (current === LEDGER_ID) {
    main = (
      <Pane label="Selected lead" elevated>
        <article aria-labelledby="ledger-stage-title" className="motion-fade">
          <header className="flex flex-wrap items-center gap-x-3 gap-y-2 border-b border-line px-6 py-4">
            <h2 id="ledger-stage-title" className="text-heading text-fg">Deadline ledger</h2>
            <Badge tone="violation">Counted</Badge>
            <span className="text-caption text-muted">All-time totals across every request, not only this window</span>
            <Button size="sm" icon="bars" className="ml-auto" onClick={onOpenLedger}>Open in response analytics</Button>
          </header>
          <div className="px-6 py-6">{ledgerFigure}</div>
        </article>
      </Pane>
    );
  } else if (findings.status === 'pending') {
    main = <><Pane label="Selected lead" elevated><StageWaiting text="Ranking what was found in this window…" busy /></Pane><Pane label="Records around this lead" title="Around this lead"><ContextWaiting /></Pane></>;
  } else if (!current) {
    main = (
      <Pane label="Selected lead" elevated>
        <StageWaiting text={findings.status === 'failed' ? 'A lead opens here once the queue loads.' : 'When a lead is found it opens here, with its figure, its evidence chain and the records around it.'} />
      </Pane>
    );
  } else {
    const story = ranked.get(current);
    main = <LeadDetail findingId={current} storyLead={story?.lead} rank={story?.rank} position={position} onSelectFinding={selectIfQueued} />;
  }

  return <Workbench navigator={navigator} wide={current === LEDGER_ID || (!current && findings.status !== 'pending')}>{main}</Workbench>;
}

function StageWaiting({ text, busy = false }: { text: string; busy?: boolean }) {
  return (
    <div role={busy ? 'status' : undefined} aria-busy={busy || undefined} className="grid h-full min-h-[18rem] place-items-center px-8 py-12 text-center">
      <div className="max-w-sm">
        <span aria-hidden="true" className="mx-auto grid h-10 w-10 place-items-center rounded-xl bg-raised text-fg-secondary"><Icon name="flag" size="lg" /></span>
        <p className="mt-3 text-body text-muted">{text}</p>
      </div>
    </div>
  );
}

function ContextWaiting() {
  return (
    <div aria-hidden="true" className="space-y-5 px-4 py-4">
      {[0, 1, 2].map((block) => (
        <div key={block} className="space-y-2">
          <div className="skeleton h-3 w-24 rounded" />
          <div className="skeleton h-4 w-full rounded" />
          <div className="skeleton h-4 w-4/5 rounded" />
        </div>
      ))}
    </div>
  );
}

function QueueSkeleton() {
  return (
    <div role="status" aria-busy="true" className="space-y-1 p-2">
      <span className="sr-only">Loading leads</span>
      {[0, 1, 2, 3].map((card) => (
        <div key={card} aria-hidden="true" className="space-y-2 rounded-lg px-3 py-2.5">
          <div className="skeleton h-5 w-20 rounded-full" />
          <div className="skeleton h-4 w-full rounded" />
          <div className="skeleton h-4 w-3/4 rounded" />
          <div className="skeleton h-3 w-1/2 rounded" />
        </div>
      ))}
    </div>
  );
}

/** One watch row: an icon tile, a title, one caption line and an optional figure at the end. */
function WatchRow({ icon, title, caption, figure, hint, current, onClick, children }: {
  icon: IconName;
  title: ReactNode;
  caption?: ReactNode;
  figure?: ReactNode;
  /** The full wording when the row shortens it. */
  hint?: string;
  current?: boolean;
  onClick(): void;
  children?: ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={hint}
      aria-current={current ? 'true' : undefined}
      className={cn('lead-card focus-ring flex w-full gap-2.5 rounded-lg border border-transparent px-2.5 py-1.5 text-left hover:bg-raised/60', caption || children ? 'items-start' : 'items-center')}
    >
      <span aria-hidden="true" className="grid h-6 w-6 shrink-0 place-items-center rounded-md bg-raised text-fg-secondary"><Icon name={icon} size="sm" /></span>
      <span className="min-w-0 flex-1">
        <span className="flex h-6 min-w-0 items-center gap-1.5 text-body font-medium text-fg">
          {title}
          {figure && <span className="ml-auto shrink-0 pl-2 text-caption font-medium tabular-nums text-fg-secondary">{figure}</span>}
        </span>
        {caption && <span className="-mt-0.5 block truncate text-caption tabular-nums text-muted">{caption}</span>}
        {children}
      </span>
    </button>
  );
}

/**
 * What the reader keeps an eye on: collection across every saved source (the sidebar lists them
 * one by one; this is their sum, so a stalled run shows without opening anything), the counted
 * deadline ledger, the next public meeting, and what the watches saw change.
 */
function WatchStatus({ ledger, ledgerSelected, onSelectLedger, onOpenFollowing }: {
  ledger: string | null;
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
  const calendar = useGatedLoad('calendar', () => invokeCommand('get_meeting_calendar', {}));
  const changes = useGatedLoad(`changes:${findingsRefreshRequests}`, () => invokeCommand('list_watch_changes', { watchId: null, limit: 20 }));

  const live = new Set(portals.map((saved) => saved.registration.portalKey));
  const running = runtimes.filter((runtime) => live.has(runtime.portal.portalKey));
  const settled = running.reduce((sum, runtime) => sum + runtime.completedTasks + runtime.failedTasks, 0);
  const total = running.reduce((sum, runtime) => sum + runtime.completedTasks + runtime.failedTasks + runtime.queuedTasks + runtime.activeTasks, 0);
  const failed = running.reduce((sum, runtime) => sum + runtime.failedTasks, 0);
  const paused = portals.filter((saved) => saved.lifecycleState === 'PAUSED').length;
  const degraded = portals.filter((saved) => saved.healthStatus === 'degraded').length;
  const collecting = syncing.filter((key) => live.has(key)).length;
  const meeting = calendar.status === 'ready' ? nextUpcomingMeeting(calendar.data) : null;
  const upcoming = meeting && meeting.days <= MEETING_HORIZON_DAYS ? meeting : null;
  const recent = changes.status === 'ready' ? changes.data.filter((change) => !since || change.detectedAt > since) : [];
  const state = [
    collecting > 0 && `${collecting} collecting`,
    paused > 0 && `${paused} paused`,
    degraded > 0 && `${degraded} need${degraded === 1 ? 's' : ''} attention`,
  ].filter(Boolean).join(' · ') || 'none collecting';

  return (
    <Pane label="Watch status" title="Watching" className="shrink-0" bodyClassName="space-y-0.5 p-2">
      <WatchRow
        icon="database"
        title={<><span>{portals.length.toLocaleString()} source{portals.length === 1 ? '' : 's'}</span><StatusDot state={degraded > 0 ? 'error' : collecting > 0 ? 'active' : 'idle'} pulse={collecting > 0 && degraded === 0} label={degraded > 0 ? 'needs attention' : collecting > 0 ? 'collecting' : 'idle'} /></>}
        caption={total > 0 ? `${state} · ${settled.toLocaleString()}/${total.toLocaleString()} tasks${failed > 0 ? ` · ${failed.toLocaleString()} failed` : ''}` : `${state} · no queue this session`}
        figure={total > 0 ? `${Math.floor((settled / total) * 100)}%` : undefined}
        onClick={() => setWorkspaceView('activity')}
      >
        {total > 0 && (
          <span role="progressbar" aria-label="Collection tasks settled across sources" aria-valuemin={0} aria-valuemax={total} aria-valuenow={settled} className="mt-1.5 block h-1 overflow-hidden rounded-full bg-raised">
            <span className="block h-full origin-left rounded-full bg-success-mark transition-transform duration-slow" style={{ transform: `scaleX(${settled / total})` }} />
          </span>
        )}
      </WatchRow>
      {ledger && (
        <WatchRow
          icon="bars"
          title={<><span>Deadline ledger</span><Badge tone="violation" className="h-[18px] px-1.5">Counted</Badge></>}
          hint={ledger}
          current={ledgerSelected}
          onClick={onSelectLedger}
        />
      )}
      {upcoming && (
        <WatchRow
          icon="calendar"
          title={<span className="truncate">{upcoming.body}</span>}
          caption={upcoming.meeting.date ? meetingWhen(upcoming.meeting.date, upcoming.meeting.startTime) : 'Date not published'}
          figure={upcoming.days === 0 ? 'today' : `in ${upcoming.days} day${upcoming.days === 1 ? '' : 's'}`}
          onClick={() => setWorkspaceView('meetings')}
        />
      )}
      <WatchRow
        icon="activity"
        title={changes.status === 'ready' && recent.length > 0 ? `${recent.length.toLocaleString()} watch change${recent.length === 1 ? '' : 's'}` : 'Watch changes'}
        hint={recent[0] ? `${recent[0].subject}: ${recent[0].oldValue ?? 'not in baseline'} → ${recent[0].newValue ?? 'no longer present'}` : undefined}
        figure={changes.status === 'failed' ? 'didn’t load' : recent.length === 0 && changes.status === 'ready' ? (since ? 'none in window' : 'none yet') : undefined}
        onClick={onOpenFollowing}
      />
    </Pane>
  );
}
