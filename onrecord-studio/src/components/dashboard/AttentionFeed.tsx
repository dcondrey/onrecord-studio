import { useMemo, type ReactNode } from 'react';
import { cn } from '../../utils/cn';
import { invokeCommand } from '../../ipc';
import { useGatedLoad } from '../../hooks/useGatedLoad';
import { nextUpcomingMeeting } from '../../hooks/useWorkspaceBadges';
import { useAppStore } from '../../store/useAppStore';
import type { AgencyPerformanceProfile, Finding, FindingSeverity, MeetingCalendar, SavedSource, WatchChange } from '../../types';
import { countedHeadline } from '../charts/figures/DepartmentLedger';
import { findingFamily } from '../objects/findingFamily';
import { claimBasisLabel } from '../objects/labels';
import Badge from '../ui/Badge';
import Button from '../ui/Button';
import FamilyBadge, { ClaimBasisBadge } from '../ui/FamilyBadge';
import Skeleton from '../ui/Skeleton';
import Icon from '../ui/icons';
import { useSinceInstant } from './SinceControl';
import CaseScopeBar from './CaseScopeBar';
import InsightCard, { type InsightTarget } from './InsightCard';
import { useCaseScope } from '../../hooks/useCaseScope';
import { formatShortDate } from '../objects/labels';

const FEED_LIMIT = 8;
const FINDINGS_SCAN_LIMIT = 200;
const MEETING_HORIZON_DAYS = 14;

export interface FeedItem {
  key: string;
  rank: number;
  badges: ReactNode;
  severity?: FindingSeverity;
  headline: string;
  open?: InsightTarget;
  subject?: string;
  time?: string;
  timeLabel?: string;
  meta?: string;
}

interface FeedSources {
  findings: Finding[];
  calendar: MeetingCalendar | null;
  changes: WatchChange[];
}

/**
 * The one feed row for counted deadline misses. They are all-time totals, not items found in this
 * window, so the row sits outside the ranked list and its count; it opens the deadline ledger.
 */
export function deadlineLedgerHeadline(profiles: readonly AgencyPerformanceProfile[]): string | null {
  const withRequests = profiles.filter((profile) => profile.totalRequestsProcessed > 0);
  return withRequests.some((profile) => profile.statutoryViolationCount > 0) ? countedHeadline(withRequests) : null;
}

export default function AttentionFeed({ onOpenLedger, variant = 'panel' }: {
  onOpenLedger?: () => void;
  /** 'rail' is the overview canvas's right column: one card per row, its list scrolling inside the rail. */
  variant?: 'panel' | 'rail';
} = {}) {
  const rail = variant === 'rail';
  const since = useSinceInstant();
  const findingsRefreshRequests = useAppStore((state) => state.findingsRefreshRequests);
  const portals = useAppStore((state) => state.portals);
  const setSincePreset = useAppStore((state) => state.setSincePreset);
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);

  const findings = useGatedLoad(`${since}:${findingsRefreshRequests}`, () => invokeCommand('list_findings', { filter: { discoveredAfter: since }, limit: FINDINGS_SCAN_LIMIT }));
  const calendar = useGatedLoad('calendar', () => invokeCommand('get_meeting_calendar', {}));
  const changes = useGatedLoad(`changes:${findingsRefreshRequests}`, () => invokeCommand('list_watch_changes', { watchId: null, limit: 20 }));

  const caseScope = useCaseScope();
  const agencyProfiles = useAppStore((state) => state.agencyProfiles);
  const activePortalKey = useAppStore((state) => state.activePortalKey);
  const ledger = useMemo(
    () => deadlineLedgerHeadline(agencyProfiles.filter((profile) => !activePortalKey || profile.portalSubdomain === activePortalKey)),
    [activePortalKey, agencyProfiles],
  );
  const items = useMemo(() => rankFeed({
    findings: findings.status === 'ready'
      ? findings.data.filter((finding) => caseScope.status !== 'ready' || caseScope.value.findingIds.has(finding.id))
      : [],
    calendar: calendar.status === 'ready' ? calendar.data : null,
    changes: changes.status === 'ready' ? changes.data.filter((change) => !since || change.detectedAt > since) : [],
  }, portals, setWorkspaceView), [calendar, caseScope, changes, findings, portals, setWorkspaceView, since]);

  const pending = findings.status === 'pending';
  const unavailable = [
    findings.status === 'failed' && 'findings',
    calendar.status === 'failed' && 'the meeting calendar',
    changes.status === 'failed' && 'watch changes',
  ].filter(Boolean) as string[];
  const shown = items.slice(0, FEED_LIMIT);
  const newFindings = findings.status === 'ready'
    ? (caseScope.status === 'ready' ? findings.data.filter((finding) => caseScope.value.findingIds.has(finding.id)).length : findings.data.length)
    : null;

  return (
    <section aria-labelledby="attention-heading" className={cn('app-panel overflow-hidden', rail && 'flex flex-col xl:max-h-[calc(100vh-5.5rem)]')}>
      <div className={cn('flex flex-wrap items-start justify-between gap-x-4 gap-y-2 border-b border-line', rail ? 'px-4 py-3.5' : 'px-5 py-4')}>
        <div className="min-w-0 flex-1">
          <h3 id="attention-heading" className="text-heading text-fg">Needs attention</h3>
          <p className="mt-1 text-caption tabular-nums text-muted">
            {newFindings === null ? 'Ranked across findings, deadlines, meetings and watches.' : `${newFindings.toLocaleString()}${newFindings === FINDINGS_SCAN_LIMIT ? '+' : ''} finding${newFindings === 1 ? '' : 's'} found in this window, ranked with deadlines, meetings and watches.`}
          </p>
        </div>
        <Button variant="link" size="sm" onClick={() => setWorkspaceView('story')} className="h-5 gap-1 px-0">
          All findings <Icon name="arrow" size="sm" />
        </Button>
      </div>
      <div className={cn('pt-4 empty:hidden', rail ? 'px-4' : 'px-5')}><CaseScopeBar state={caseScope} noun="findings" /></div>
      {ledger && onOpenLedger && (caseScope.status === 'off' ? (
        <div className="px-4 pt-4">
          <InsightCard
            nested
            badges={<Badge tone="violation">Counted</Badge>}
            headline={ledger}
            open={{ onOpen: onOpenLedger }}
            subject="Deadline ledger"
            meta="all-time totals · opens the ledger in Response analytics"
          />
        </div>
      ) : (
        <p className="px-5 pt-3 text-caption text-muted">The deadline ledger is hidden while a case scope is active: its totals cover every request, not this case's.</p>
      ))}
      {pending ? (
        <AttentionFeedSkeleton rail={rail} />
      ) : shown.length === 0 ? (
        <div className="motion-reveal px-5 py-8 text-center">
          <p className="text-heading text-fg-secondary">{since ? `Nothing new since ${formatShortDate(since)}` : 'Nothing found yet'}</p>
          <p className="mt-1 text-caption text-muted">
            {since ? (
              <>Widen the window to see everything found so far. <Button variant="link" size="sm" className="h-auto px-0 align-baseline text-caption" onClick={() => setSincePreset('all')}>Any time</Button></>
            ) : 'Findings, upcoming meetings and watch changes appear here as sources are collected.'}
          </p>
        </div>
      ) : (
        <ol className={cn('motion-stagger grid', rail ? 'min-h-0 gap-2 overflow-y-auto p-3 md:grid-cols-2 xl:grid-cols-1' : 'gap-3 p-4 lg:grid-cols-2')}>
          {shown.map((item) => (
            <li key={item.key} className="min-w-0">
              <InsightCard nested density={rail ? 'compact' : 'regular'} badges={item.badges} severity={item.severity} headline={item.headline} open={item.open} subject={item.subject} time={item.time} timeLabel={item.timeLabel} meta={item.meta} />
            </li>
          ))}
        </ol>
      )}
      {(items.length > FEED_LIMIT || unavailable.length > 0) && (
        <div className={cn('flex flex-wrap items-center gap-x-3 gap-y-1 border-t border-line py-3 text-caption text-muted', rail ? 'px-4' : 'px-5')}>
          {items.length > FEED_LIMIT && (
            <span className="tabular-nums">
              {(items.length - FEED_LIMIT).toLocaleString()} more {items.length - FEED_LIMIT === 1 ? 'item' : 'items'} in this window
              {' — '}
              <Button variant="link" size="sm" className="h-auto px-0 align-baseline text-caption" onClick={() => setWorkspaceView('story')}>View all</Button>
            </span>
          )}
          {unavailable.length > 0 && <span className="text-warning">Not included because they did not load: {unavailable.join(', ')}.</span>}
        </div>
      )}
    </section>
  );
}

const SEVERITY_RANK = { CRITICAL: 100, SERIOUS: 70, NOTABLE: 40, INFO: 10 } as const;

function sourceName(portals: SavedSource[], portalKey: string | null | undefined): string | undefined {
  if (!portalKey) return undefined;
  return portals.find((saved) => saved.registration.portalKey === portalKey)?.registration.displayName;
}

/**
 * Severity first; a counted statutory violation and an imminent meeting outrank a pattern signal
 * of the same weight. Only items dated inside the window enter: all-time per-agency totals belong
 * in Compare, where they are not counted as "found in this window".
 */
export function rankFeed(sources: FeedSources, portals: SavedSource[], openView: (view: 'meetings') => void): FeedItem[] {
  const items: FeedItem[] = [];
  for (const finding of sources.findings) {
    if (finding.dismissedAt) continue;
    items.push({
      key: `finding:${finding.id}`,
      rank: SEVERITY_RANK[finding.severity] + (claimBasisLabel(finding.kind) === 'Counted violation' ? 15 : 0),
      badges: <><FamilyBadge family={findingFamily(finding.kind)} /><ClaimBasisBadge kind={finding.kind} /></>,
      severity: finding.severity,
      headline: finding.headline,
      open: { target: { kind: 'finding', id: finding.id, label: finding.headline } },
      subject: sourceName(portals, finding.subject.portalKey),
      time: finding.discoveredAt,
    });
  }
  const meeting = sources.calendar ? nextUpcomingMeeting(sources.calendar) : null;
  if (meeting && meeting.days <= MEETING_HORIZON_DAYS) {
    const { date, startTime, jurisdiction } = meeting.meeting;
    items.push({
      key: 'meeting:next',
      rank: meeting.days <= 3 ? 90 : 50,
      badges: <Badge tone="neutral"><Icon name="calendar" size="xs" />Meeting</Badge>,
      headline: meeting.body,
      open: { onOpen: () => openView('meetings') },
      subject: jurisdiction,
      time: date ?? undefined,
      timeLabel: date ? meetingWhen(date, startTime) : undefined,
      meta: meeting.days === 0 ? 'today' : `in ${meeting.days} day${meeting.days === 1 ? '' : 's'}`,
    });
  }
  for (const change of sources.changes) {
    items.push({
      key: `change:${change.id}`,
      rank: 35,
      badges: <Badge tone="neutral"><Icon name="activity" size="xs" />Watch</Badge>,
      headline: `${change.subject}: ${change.oldValue ?? 'not in baseline'} → ${change.newValue ?? 'no longer present'}`,
      subject: sentenceCase(change.changeKind),
      time: change.detectedAt,
    });
  }
  return items.sort((left, right) => right.rank - left.rank);
}

const MEETING_DAY = new Intl.DateTimeFormat('en-US', { weekday: 'short', month: 'short', day: 'numeric', timeZone: 'UTC' });

/**
 * "Tue, Oct 13 · 6:00 PM". The agenda's date and clock time are the publisher's local wall time, so
 * both are formatted as written, never converted through the reader's zone.
 */
export function meetingWhen(date: string, startTime: string | null): string {
  const day = MEETING_DAY.format(new Date(`${date.slice(0, 10)}T00:00:00Z`));
  const match = startTime ? /^(\d{1,2}):(\d{2})/.exec(startTime) : null;
  if (!match) return day;
  const hour = Number(match[1]);
  return `${day} · ${hour % 12 === 0 ? 12 : hour % 12}:${match[2]} ${hour < 12 ? 'AM' : 'PM'}`;
}

/** Four cards in the feed's two-column grid, at card height, while findings load. */
export function AttentionFeedSkeleton({ rail = false }: { rail?: boolean } = {}) {
  return (
    <div className={cn('grid', rail ? 'gap-2 p-3 md:grid-cols-2 xl:grid-cols-1' : 'gap-3 p-4 lg:grid-cols-2')}>
      <Skeleton variant="card" label="Loading what needs attention" />
      {[1, 2, 3].map((slot) => <div key={slot} aria-hidden="true"><Skeleton variant="card" label="" /></div>)}
    </div>
  );
}

function sentenceCase(enumValue: string): string {
  const words = enumValue.toLowerCase().split('_').join(' ');
  return words.charAt(0).toUpperCase() + words.slice(1);
}
