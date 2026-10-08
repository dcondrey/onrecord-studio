import { useMemo, type ReactNode } from 'react';
import { invokeCommand } from '../../ipc';
import { useGatedLoad } from '../../hooks/useGatedLoad';
import { useCaseScope } from '../../hooks/useCaseScope';
import { useAppStore } from '../../store/useAppStore';
import type { SavedSource, StoryLead } from '../../types';
import { cn } from '../../utils/cn';
import LeadFigure from '../charts/figures/LeadFigure';
import RankFactors from '../charts/figures/RankFactors';
import ObjectLink from '../objects/ObjectLink';
import { formatShortDate } from '../objects/labels';
import Badge from '../ui/Badge';
import Button from '../ui/Button';
import InlineError from '../ui/InlineError';
import Icon from '../ui/icons';
import StatusDot, { type StatusDotState } from '../ui/statusDot';
import { deadlineLedgerHeadline, rankFeed } from './AttentionFeed';
import CaseScopeBar from './CaseScopeBar';
import LeadLine, { LeadByline, LeadKicker } from './LeadLine';
import { useSinceInstant } from './SinceControl';
import { isUnassessed } from './storyFactors';

const FURTHER_LIMIT = 7;
const TABLES_READ = '\n\nTables read:\n';
const MEETING_HORIZON_DAYS = 14;

/** Identifiers such as "H-24-1180" are set unbroken: a line break at their hyphens reads as two numbers. */
function keepIdentifiers(text: string): ReactNode[] {
  return text.split(/(\S+)/).map((part, index) => (/-/.test(part) && /\d/.test(part) ? <span key={index} className="whitespace-nowrap">{part}</span> : part));
}

function sourceName(portals: readonly SavedSource[], portalKey: string | null): string | undefined {
  if (!portalKey) return undefined;
  return portals.find((saved) => saved.registration.portalKey === portalKey)?.registration.displayName;
}

/** A section's title over a heavy rule, the way a front page separates its departments. */
export function Rubric({ id, title, aside, className }: { id: string; title: string; aside?: ReactNode; className?: string }) {
  return (
    <div className={cn('flex items-baseline justify-between gap-4 border-b border-fg/85 pb-2', className)}>
      <h3 id={id} className="text-overline uppercase text-fg">{title}</h3>
      {aside}
    </div>
  );
}

/**
 * The overview as a front page: the top-ranked lead as the lead story with the figure that verifies
 * it, the further leads ranked beneath it with their scores at sparkline scale, the docket of dated
 * items beside them, and the state of every source along the foot.
 */
export default function FrontPage({ onOpenLedger }: { onOpenLedger: () => void }) {
  const since = useSinceInstant();
  const findingsRefreshRequests = useAppStore((state) => state.findingsRefreshRequests);
  const requestFindingsRefresh = useAppStore((state) => state.requestFindingsRefresh);
  const setSincePreset = useAppStore((state) => state.setSincePreset);
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);
  const portals = useAppStore((state) => state.portals);
  const caseScope = useCaseScope();
  const leads = useGatedLoad(`story:${findingsRefreshRequests}`, () => invokeCommand('list_story_leads', { limit: 100 }));
  const visible = useMemo(
    () => leads.status === 'ready'
      ? leads.data.filter((lead) => (!since || lead.firstSeenAt > since) && (caseScope.status !== 'ready' || caseScope.value.findingIds.has(lead.findingId)))
      : [],
    [caseScope, leads, since],
  );
  const [first, ...rest] = visible;
  const further = rest.slice(0, FURTHER_LIMIT);
  const scaleMax = first?.publicInterest.totalBasisPoints ?? 1;

  return (
    <div data-front-page className="space-y-10">
      <div className="empty:hidden"><CaseScopeBar state={caseScope} noun="leads" /></div>
      {leads.status === 'pending' ? (
        <FrontPageSkeleton />
      ) : leads.status === 'failed' ? (
        <InlineError summary="Leads couldn’t be loaded. The local evidence store is busy; try again in a moment." detail={leads.message} onRetry={requestFindingsRefresh} />
      ) : !first ? (
        <section aria-labelledby="lead-story-rubric" className="motion-reveal">
          <Rubric id="lead-story-rubric" title="Lead story" />
          <div className="py-10 text-center">
            <p className="text-heading text-fg-secondary">{since && leads.data.length > 0 ? `No new leads since ${formatShortDate(since)}` : 'No leads yet'}</p>
            <p className="mt-1 text-caption text-muted">
              {since && leads.data.length > 0 ? (
                <>Widen the window to see every lead ranked so far. <Button variant="link" size="sm" className="h-auto px-0 align-baseline text-caption" onClick={() => setSincePreset('all')}>Any time</Button></>
              ) : 'Leads appear here once the detectors finish a sweep of what has been collected.'}
            </p>
          </div>
        </section>
      ) : (
        <LeadStory lead={first} leads={visible} total={visible.length} source={sourceName(portals, first.portalKey)} />
      )}

      <div className="grid gap-x-10 gap-y-10 xl:grid-cols-[minmax(0,1fr)_18rem]">
        <section aria-labelledby="further-leads-title" className="min-w-0">
          <Rubric
            id="further-leads-title"
            title="Further leads"
            aside={leads.status === 'ready' && visible.length > 0 && (
              <Button variant="link" size="sm" onClick={() => setWorkspaceView('story')} className="h-auto gap-1 px-0 text-caption">
                <span className="tabular-nums">All {visible.length.toLocaleString()} ranked</span> <Icon name="arrow" size="sm" />
              </Button>
            )}
          />
          {leads.status === 'pending' ? (
            <div aria-hidden="true" className="divide-y divide-line">
              {[0, 1, 2, 3].map((row) => (
                <div key={row} className="grid grid-cols-[2.25rem_minmax(0,1fr)_8.5rem] gap-x-3 py-4">
                  <div className="skeleton h-6 w-4 rounded" />
                  <div className="space-y-2"><div className="skeleton h-3 w-28 rounded" /><div className="skeleton h-4 w-4/5 rounded" /><div className="skeleton h-3 w-2/5 rounded" /></div>
                  <div className="skeleton h-1.5 self-center rounded-full" />
                </div>
              ))}
            </div>
          ) : further.length > 0 ? (
            <ol className="motion-stagger divide-y divide-line" aria-label="Further leads by attention score">
              {further.map((lead, index) => (
                <li key={lead.findingId} className="min-w-0">
                  <LeadLine lead={lead} rank={index + 2} scaleMax={scaleMax} source={sourceName(portals, lead.portalKey)} />
                </li>
              ))}
            </ol>
          ) : leads.status === 'ready' && first ? (
            <p className="py-6 text-caption text-muted">The lead story is the only lead in this window.</p>
          ) : null}
          {further.length > 0 && (
            <p className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 border-t border-line pt-3 text-caption text-muted">
              <span>Bars share one scale: the lead story’s score fills the track.</span>
              <span className="inline-flex items-center gap-1.5"><span aria-hidden="true" className="h-1.5 w-3 rounded-full bg-lead-mark" />pattern signal factors</span>
              <span className="inline-flex items-center gap-1.5"><span aria-hidden="true" className="h-1.5 w-3 rounded-full bg-violation" />counted violation</span>
              {further.some((lead) => lead.publicInterest.factors.some((factor) => isUnassessed(lead, factor.name))) && (
                <span className="inline-flex items-center gap-1.5"><span aria-hidden="true" className="h-1.5 w-2 rounded-[2px] border border-dashed border-muted" />evidence strength not assessed</span>
              )}
              {visible.length > further.length + 1 && (
                <span className="tabular-nums">{(visible.length - further.length - 1).toLocaleString()} more on the Story desk</span>
              )}
            </p>
          )}
        </section>
        <Docket onOpenLedger={onOpenLedger} />
      </div>

      <HealthStrip />
    </div>
  );
}

/**
 * The lead story: kicker, the headline at banner size, the finding's own explanation as the
 * standfirst, the byline, its two actions, and its figure. The finding is read under a gate keyed
 * to its id, so a slow read for a lead the page has moved past never lands on this one.
 */
function LeadStory({ lead, leads, total, source }: { lead: StoryLead; leads: readonly StoryLead[]; total: number; source?: string }) {
  const openLeadDossier = useAppStore((state) => state.openLeadDossier);
  const setActivePortal = useAppStore((state) => state.setActivePortal);
  const setSearchFilters = useAppStore((state) => state.setSearchFilters);
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);
  const finding = useGatedLoad(`finding:${lead.findingId}`, () => invokeCommand('list_findings', { filter: { id: lead.findingId, includeDismissed: true }, limit: 1 }).then((found) => found[0] ?? null));
  const read = finding.status === 'ready' ? finding.data : null;
  const explanation = read?.explanation ?? '';
  const tablesAt = explanation.lastIndexOf(TABLES_READ);
  const dek = (tablesAt < 0 ? explanation : explanation.slice(0, tablesAt)).trim();
  const inspect = () => {
    if (lead.portalKey) setActivePortal(lead.portalKey);
    setSearchFilters('', lead.subject.agencyId ?? null, null, null);
    setWorkspaceView('evidence');
  };
  return (
    <article aria-labelledby="lead-story-title" className="motion-reveal">
      <Rubric id="lead-story-rubric" title="Lead story" aside={<span className="text-caption tabular-nums text-muted">Ranked first of {total.toLocaleString()}</span>} />
      <div className="grid gap-x-10 gap-y-6 pt-6 xl:grid-cols-[minmax(0,5fr)_minmax(0,6fr)]">
        <div className="min-w-0">
          <LeadKicker lead={lead} />
          <h3 id="lead-story-title" className="mt-3 text-balance text-banner text-fg">
            <ObjectLink target={{ kind: 'finding', id: lead.findingId, label: lead.headline }} className="text-fg no-underline decoration-fg/30 underline-offset-4 hover:underline">{keepIdentifiers(lead.headline)}</ObjectLink>
          </h3>
          {finding.status === 'pending' ? (
            <div aria-hidden="true" className="mt-4 space-y-2"><div className="skeleton h-3.5 w-full rounded" /><div className="skeleton h-3.5 w-11/12 rounded" /><div className="skeleton h-3.5 w-3/5 rounded" /></div>
          ) : dek ? (
            <p className="mt-4 line-clamp-5 max-w-prose whitespace-pre-line text-pretty text-dek text-fg-secondary">{dek}</p>
          ) : finding.status === 'failed' ? (
            <p className="mt-4 text-caption text-muted">The finding’s explanation couldn’t be read. It is one click away in the evidence chain.</p>
          ) : null}
          <LeadByline lead={lead} source={source} className="mt-4" />
          <div className="mt-6 flex flex-wrap items-center gap-2">
            <Button variant="primary" onClick={() => openLeadDossier(lead.findingId)} aria-label={`Open evidence chain: ${lead.headline}`}>Open evidence chain</Button>
            <Button variant="ghost" onClick={inspect}>Inspect records</Button>
          </div>
        </div>
        <div className="min-w-0 [--viz-surface:rgb(var(--bg))] xl:border-l xl:border-line xl:pl-10">
          {read?.figure ? (
            <LeadFigure figure={read.figure} headline={null} size="card" sources={{ tables: ['findings'], note: 'the lead’s stored figure, drawn from the rows its builder read' }} onActivate={() => openLeadDossier(lead.findingId)} />
          ) : finding.status === 'pending' ? (
            <div aria-hidden="true" className="space-y-3"><div className="skeleton h-4 w-2/3 rounded" />{[0, 1, 2, 3, 4].map((row) => <div key={row} className="skeleton h-3 rounded" style={{ width: `${88 - row * 12}%` }} />)}</div>
          ) : (
            <RankFactors leads={leads} onOpen={() => openLeadDossier(lead.findingId)} />
          )}
        </div>
      </div>
    </article>
  );
}

/**
 * Dated items that are not leads: the standing deadline ledger (all-time totals, so outside the
 * window and the ranking), the next meeting and watch changes. Opens what each one names.
 */
function Docket({ onOpenLedger }: { onOpenLedger: () => void }) {
  const since = useSinceInstant();
  const findingsRefreshRequests = useAppStore((state) => state.findingsRefreshRequests);
  const portals = useAppStore((state) => state.portals);
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);
  const agencyProfiles = useAppStore((state) => state.agencyProfiles);
  const activePortalKey = useAppStore((state) => state.activePortalKey);
  const caseScope = useCaseScope();
  const calendar = useGatedLoad('calendar', () => invokeCommand('get_meeting_calendar', {}));
  const changes = useGatedLoad(`changes:${findingsRefreshRequests}`, () => invokeCommand('list_watch_changes', { watchId: null, limit: 20 }));
  const ledger = useMemo(
    () => deadlineLedgerHeadline(agencyProfiles.filter((profile) => !activePortalKey || profile.portalSubdomain === activePortalKey)),
    [activePortalKey, agencyProfiles],
  );
  const items = useMemo(() => rankFeed({
    findings: [],
    calendar: calendar.status === 'ready' ? calendar.data : null,
    changes: changes.status === 'ready' ? changes.data.filter((change) => !since || change.detectedAt > since) : [],
  }, portals, setWorkspaceView), [calendar, changes, portals, setWorkspaceView, since]);
  const unavailable = [calendar.status === 'failed' && 'the meeting calendar', changes.status === 'failed' && 'watch changes'].filter(Boolean) as string[];
  const pending = calendar.status === 'pending' || changes.status === 'pending';
  const shown = items.slice(0, 5);
  return (
    <aside aria-labelledby="docket-title" className="min-w-0">
      <Rubric id="docket-title" title="On the docket" />
      <ul className="divide-y divide-line">
        {ledger && (caseScope.status === 'off' ? (
          <li className="relative py-4">
            <p className="flex items-center gap-2 text-overline uppercase text-fg-secondary">Standing record <Badge tone="violation" className="normal-case tracking-normal">Counted</Badge></p>
            <h4 className="mt-1.5 text-body font-medium text-fg">
              <button type="button" onClick={onOpenLedger} className="focus-ring rounded-sm text-left hover:underline">{ledger}</button>
            </h4>
            <p className="mt-1 text-caption text-muted">Deadline ledger · all-time totals · opens in Response analytics</p>
          </li>
        ) : (
          <li className="py-4 text-caption text-muted">The deadline ledger is hidden while a case scope is active: its totals cover every request, not this case’s.</li>
        ))}
        {pending && !shown.length && (
          <li aria-hidden="true" className="space-y-2 py-4"><div className="skeleton h-3 w-20 rounded" /><div className="skeleton h-4 w-3/4 rounded" /><div className="skeleton h-3 w-1/2 rounded" /></li>
        )}
        {shown.map((item) => {
          const meeting = item.key.startsWith('meeting:');
          return (
            <li key={item.key} className="relative py-4">
              <p className="flex items-center gap-1.5 text-overline uppercase text-fg-secondary">
                <Icon name={meeting ? 'calendar' : 'activity'} size="xs" className="text-muted" />
                {meeting ? 'Meeting' : 'Watch'}
                {item.meta && <span className="font-normal normal-case tracking-normal text-muted">· {item.meta}</span>}
              </p>
              <h4 className="mt-1.5 text-body font-medium text-fg">
                {item.open && 'onOpen' in item.open ? (
                  <button type="button" onClick={item.open.onOpen} className="focus-ring rounded-sm text-left hover:underline">{item.headline}</button>
                ) : item.headline}
              </h4>
              <p className="mt-1 flex flex-wrap gap-x-1.5 text-caption text-muted">
                {[item.time && <time key="time" dateTime={item.time} className="tabular-nums">{item.timeLabel ?? formatShortDate(item.time)}</time>, item.subject && <span key="subject">{item.subject}</span>]
                  .filter(Boolean)
                  .flatMap((part, index) => (index === 0 ? [part] : [<span key={`dot-${index}`} aria-hidden="true">·</span>, part]))}
              </p>
            </li>
          );
        })}
        {!pending && !shown.length && !ledger && (
          <li className="py-4 text-caption text-muted">No meeting in the next {MEETING_HORIZON_DAYS} days and no watch change in this window.</li>
        )}
      </ul>
      {unavailable.length > 0 && <p className="border-t border-line pt-3 text-caption text-warning">Not included because they did not load: {unavailable.join(', ')}.</p>}
    </aside>
  );
}

type SourceHealth = { state: StatusDotState; word: string; pulse: boolean };

function healthOf(source: SavedSource, syncing: boolean): SourceHealth {
  if (source.healthStatus === 'degraded') return { state: 'error', word: 'Needs attention', pulse: false };
  if (syncing) return { state: 'active', word: 'Syncing', pulse: true };
  if (source.lifecycleState === 'PAUSED') return { state: 'note', word: 'Paused', pulse: false };
  return { state: 'idle', word: 'Idle', pulse: false };
}

/**
 * Every saved source in one row: its state, and for a source with crawl work this session the work
 * settled against the total, with failures stated apart. A source with no crawl this session has
 * no meter: no work is not 0% of work.
 */
function HealthStrip() {
  const portals = useAppStore((state) => state.portals);
  const runtimes = useAppStore((state) => state.portalRuntimes);
  const syncingPortalKeys = useAppStore((state) => state.syncingPortalKeys);
  const activePortalKey = useAppStore((state) => state.activePortalKey);
  const setActivePortal = useAppStore((state) => state.setActivePortal);
  const analyticsUpdatedAt = useAppStore((state) => state.analyticsUpdatedAt);
  const analyticsError = useAppStore((state) => state.analyticsError);
  const runtimeByPortal = new Map(runtimes.map((runtime) => [runtime.portal.portalKey, runtime]));
  if (portals.length === 0) return null;
  return (
    <section aria-labelledby="health-title">
      <Rubric
        id="health-title"
        title="Data health"
        aside={<span className={cn('text-caption', analyticsError ? 'text-warning' : 'text-muted')}>{analyticsError ? 'Response analytics: the last refresh failed' : analyticsUpdatedAt ? `Response analytics updated ${formatShortDate(analyticsUpdatedAt)}` : 'Response analytics not yet calculated'}</span>}
      />
      <ul className="grid grid-cols-[repeat(auto-fill,minmax(14rem,1fr))] gap-x-6">
        {portals.map((source) => {
          const key = source.registration.portalKey;
          const runtime = runtimeByPortal.get(key);
          const health = healthOf(source, syncingPortalKeys.includes(key));
          const settled = runtime ? runtime.completedTasks + runtime.failedTasks : 0;
          const total = runtime ? settled + runtime.queuedTasks + runtime.activeTasks : 0;
          const failed = runtime?.failedTasks ?? 0;
          const selected = activePortalKey === key;
          return (
            <li key={key} className="border-b border-line">
              <button
                type="button"
                aria-pressed={selected}
                onClick={() => setActivePortal(selected ? null : key)}
                className={cn('focus-ring -mx-2 flex h-full w-[calc(100%+1rem)] flex-col gap-1.5 rounded-md px-2 py-3 text-left transition-colors hover:bg-raised/60', selected && 'bg-raised/60')}
              >
                <span className="flex min-w-0 items-center gap-2">
                  <StatusDot state={health.state} label="" pulse={health.pulse} />
                  <span className="min-w-0 truncate text-body font-medium text-fg">{source.registration.displayName}</span>
                </span>
                <span className="flex flex-wrap gap-x-1.5 pl-4 text-caption tabular-nums text-muted">
                  <span className={health.state === 'error' ? 'text-danger' : undefined}>{health.word}</span>
                  {total > 0 && <><span aria-hidden="true">·</span><span>{settled.toLocaleString()} of {total.toLocaleString()} settled</span></>}
                  {failed > 0 && <><span aria-hidden="true">·</span><span className="text-danger">{failed.toLocaleString()} failed</span></>}
                </span>
                {total > 0 && (
                  <span aria-hidden="true" className="relative ml-4 mt-0.5 flex h-1 w-[calc(100%-1rem)] overflow-hidden rounded-full bg-line">
                    <span className="h-full bg-success-mark" style={{ width: `${((settled - failed) / total) * 100}%` }} />
                    {failed > 0 && <span className="ml-px h-full bg-danger-mark" style={{ width: `${(failed / total) * 100}%` }} />}
                  </span>
                )}
              </button>
            </li>
          );
        })}
      </ul>
    </section>
  );
}

/** The front page's shape while sources load: the lead story block and four ranked lines. */
export function FrontPageSkeleton() {
  return (
    <div role="status" aria-busy="true" className="space-y-4">
      <span className="sr-only">Loading the lead story and ranked leads</span>
      <div aria-hidden="true">
        <div className="border-b border-fg/85 pb-2"><div className="skeleton h-3 w-20 rounded" /></div>
        <div className="grid gap-x-10 gap-y-6 pt-6 xl:grid-cols-[minmax(0,5fr)_minmax(0,6fr)]">
          <div className="space-y-3"><div className="skeleton h-3 w-32 rounded" /><div className="skeleton h-8 w-11/12 rounded" /><div className="skeleton h-8 w-3/5 rounded" /><div className="skeleton mt-4 h-3.5 w-full rounded" /><div className="skeleton h-3.5 w-4/5 rounded" /></div>
          <div className="space-y-3">{[0, 1, 2, 3, 4].map((row) => <div key={row} className="skeleton h-3 rounded" style={{ width: `${90 - row * 12}%` }} />)}</div>
        </div>
      </div>
    </div>
  );
}
