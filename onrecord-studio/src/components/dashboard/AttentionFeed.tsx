import { useMemo } from 'react';
import { invokeCommand } from '../../ipc';
import { useGatedLoad } from '../../hooks/useGatedLoad';
import { nextUpcomingMeeting } from '../../hooks/useWorkspaceBadges';
import { useAppStore } from '../../store/useAppStore';
import type { AgencyPerformanceProfile, Finding, MeetingCalendar, WatchChange } from '../../types';
import { cn } from '../../utils/cn';
import ObjectLink from '../objects/ObjectLink';
import { claimBasisLabel, formatShortDate, severityLabel, severityTone } from '../objects/labels';
import Icon, { type IconName } from '../ui/icons';
import { useSinceInstant } from './SinceControl';
import CaseScopeBar from './CaseScopeBar';
import { useCaseScope } from '../../hooks/useCaseScope';

const FEED_LIMIT = 8;
const FINDINGS_SCAN_LIMIT = 200;
const MEETING_HORIZON_DAYS = 14;

interface FeedItem {
  key: string;
  rank: number;
  icon: IconName;
  tone: 'danger' | 'warning' | 'accent' | 'muted';
  eyebrow: string;
  body: React.ReactNode;
  meta: string;
}

interface FeedSources {
  findings: Finding[];
  calendar: MeetingCalendar | null;
  changes: WatchChange[];
}

/**
 * The few things that most need a reader's attention, from every source the workspace has, ranked
 * in one list. Each item is a link to the record behind it. A source that failed to load is named
 * in the footer rather than silently leaving the feed shorter.
 */
export default function AttentionFeed() {
  const since = useSinceInstant();
  const findingsRefreshRequests = useAppStore((state) => state.findingsRefreshRequests);
  const profiles = useAppStore((state) => state.agencyProfiles);
  const activePortalKey = useAppStore((state) => state.activePortalKey);
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);

  const findings = useGatedLoad(`${since}:${findingsRefreshRequests}`, () => invokeCommand('list_findings', { filter: { discoveredAfter: since }, limit: FINDINGS_SCAN_LIMIT }));
  const calendar = useGatedLoad('calendar', () => invokeCommand('get_meeting_calendar', {}));
  const changes = useGatedLoad(`changes:${findingsRefreshRequests}`, () => invokeCommand('list_watch_changes', { watchId: null, limit: 20 }));

  const caseScope = useCaseScope();
  const items = useMemo(() => rankFeed({
    findings: findings.status === 'ready'
      ? findings.data.filter((finding) => caseScope.status !== 'ready' || caseScope.value.findingIds.has(finding.id))
      : [],
    calendar: calendar.status === 'ready' ? calendar.data : null,
    changes: changes.status === 'ready' ? changes.data.filter((change) => !since || change.detectedAt > since) : [],
  }, caseScope.status === 'ready' ? [] : profiles.filter((profile) => !activePortalKey || profile.portalSubdomain === activePortalKey), setWorkspaceView), [activePortalKey, calendar, caseScope, changes, findings, profiles, setWorkspaceView, since]);

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
    <section aria-labelledby="attention-heading" className="app-panel overflow-hidden">
      <div className="flex flex-wrap items-center justify-between gap-2 border-b border-line px-5 py-3.5">
        <div>
          <h3 id="attention-heading" className="text-sm font-semibold text-fg">Needs attention</h3>
          <p className="mt-0.5 text-caption text-muted">
            {newFindings === null ? 'Ranked across findings, deadlines, meetings and watches.' : `${newFindings.toLocaleString()}${newFindings === FINDINGS_SCAN_LIMIT ? '+' : ''} finding${newFindings === 1 ? '' : 's'} found in this window, ranked with deadlines, meetings and watches.`}
          </p>
        </div>
        <button type="button" onClick={() => setWorkspaceView('story')} className="motion-press focus-ring flex items-center gap-1 rounded-lg px-2 py-1 text-label font-semibold text-accent hover:bg-accent/10">
          All findings <Icon name="arrow" className="h-3.5 w-3.5" />
        </button>
      </div>
      <div className="px-5 pt-3 empty:hidden"><CaseScopeBar state={caseScope} noun="findings" /></div>
      {pending ? (
        <div role="status" aria-label="Loading what needs attention" className="space-y-3 p-5">
          {[0, 1, 2].map((row) => <div key={row} className="flex gap-3"><div className="skeleton h-8 w-8 rounded-lg" /><div className="flex-1 space-y-1.5"><div className="skeleton h-3 w-1/3 rounded" /><div className="skeleton h-4 w-5/6 rounded" /></div></div>)}
        </div>
      ) : shown.length === 0 ? (
        <div className="motion-reveal px-5 py-8 text-center">
          <Icon name="shield" className="mx-auto h-5 w-5 text-muted" />
          <p className="mt-2 text-label font-medium text-fg-secondary">Nothing new in this window</p>
          <p className="mt-1 text-caption text-muted">Widen the window, or open the story desk for everything found so far.</p>
        </div>
      ) : (
        <ol className="motion-stagger divide-y divide-line">
          {shown.map((item) => (
            <li key={item.key} className="flex items-start gap-3 px-5 py-3 transition hover:bg-raised/40">
              <span className={cn('mt-0.5 grid h-8 w-8 shrink-0 place-items-center rounded-lg', toneClass(item.tone))}>
                <Icon name={item.icon} className="h-4 w-4" />
              </span>
              <div className="min-w-0 flex-1">
                <div className="text-caption font-semibold uppercase tracking-wider text-muted">{item.eyebrow}</div>
                <div className="mt-0.5 text-label leading-snug text-fg">{item.body}</div>
              </div>
              <span className="shrink-0 pt-0.5 text-caption text-muted">{item.meta}</span>
            </li>
          ))}
        </ol>
      )}
      {(items.length > FEED_LIMIT || unavailable.length > 0) && (
        <div className="border-t border-line px-5 py-2.5 text-caption text-muted">
          {items.length > FEED_LIMIT && <span>{(items.length - FEED_LIMIT).toLocaleString()} more in this window. </span>}
          {unavailable.length > 0 && <span className="text-warning">Not included because they did not load: {unavailable.join(', ')}.</span>}
        </div>
      )}
    </section>
  );
}

function toneClass(tone: FeedItem['tone']) {
  return tone === 'danger' ? 'bg-danger/10 text-danger' : tone === 'warning' ? 'bg-warning/10 text-warning' : tone === 'accent' ? 'bg-accent/10 text-accent' : 'bg-surface text-muted';
}

const SEVERITY_RANK = { CRITICAL: 100, SERIOUS: 70, NOTABLE: 40, INFO: 10 } as const;

/** Severity first; a counted statutory violation and an imminent meeting outrank a pattern signal of the same weight. */
export function rankFeed(sources: FeedSources, profiles: AgencyPerformanceProfile[], openView: (view: 'meetings') => void): FeedItem[] {
  const items: FeedItem[] = [];
  for (const finding of sources.findings) {
    if (finding.dismissedAt) continue;
    items.push({
      key: `finding:${finding.id}`,
      rank: SEVERITY_RANK[finding.severity] + (claimBasisLabel(finding.kind) === 'Counted violation' ? 15 : 0),
      icon: 'flag',
      tone: severityTone(finding.severity),
      eyebrow: `${severityLabel(finding.severity)} · ${claimBasisLabel(finding.kind)}`,
      body: <ObjectLink target={{ kind: 'finding', id: finding.id, label: finding.headline }} className="text-fg decoration-line-strong hover:text-accent">{finding.headline}</ObjectLink>,
      meta: formatShortDate(finding.discoveredAt),
    });
  }
  for (const profile of profiles) {
    if (profile.statutoryViolationCount <= 0) continue;
    items.push({
      key: `agency:${profile.portalSubdomain}:${profile.agencyId}`,
      rank: 80,
      icon: 'building',
      tone: 'warning',
      eyebrow: 'Counted statutory violations',
      body: <><ObjectLink target={{ kind: 'agency', portalKey: profile.portalSubdomain, agencyId: profile.agencyId, label: profile.agencyName }} className="font-medium">{profile.agencyName}</ObjectLink>{` missed the statutory deadline on ${profile.statutoryViolationCount.toLocaleString()} request${profile.statutoryViolationCount === 1 ? '' : 's'}`}</>,
      meta: profile.portalSubdomain,
    });
  }
  const meeting = sources.calendar ? nextUpcomingMeeting(sources.calendar) : null;
  if (meeting && meeting.days <= MEETING_HORIZON_DAYS) {
    items.push({
      key: 'meeting:next',
      rank: meeting.days <= 3 ? 90 : 50,
      icon: 'calendar',
      tone: meeting.days <= 3 ? 'warning' : 'accent',
      eyebrow: 'Upcoming meeting',
      body: <button type="button" onClick={() => openView('meetings')} className="focus-ring text-left text-accent underline decoration-accent/30 underline-offset-2 hover:decoration-accent">{meeting.body}</button>,
      meta: meeting.days === 0 ? 'Today' : `In ${meeting.days} day${meeting.days === 1 ? '' : 's'}`,
    });
  }
  for (const change of sources.changes) {
    items.push({
      key: `change:${change.id}`,
      rank: 35,
      icon: 'activity',
      tone: 'accent',
      eyebrow: `Watch · ${change.changeKind.toLowerCase().split('_').join(' ')}`,
      body: <span>{change.subject}: {change.oldValue ?? 'not in baseline'} → {change.newValue ?? 'no longer present'}</span>,
      meta: formatShortDate(change.detectedAt),
    });
  }
  return items.sort((left, right) => right.rank - left.rank);
}
