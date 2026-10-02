import { useEffect, useState } from 'react';
import { invokeCommand } from '../ipc';
import { useAppStore, type WorkspaceView } from '../store/useAppStore';
import type { MeetingCalendar } from '../types';
import { useLatestRequestGate } from './useLatestRequestGate';

export interface WorkspaceBadge {
  text: string;
  label: string;
  tone: 'accent' | 'warning';
}

/**
 * The one fact per workspace worth seeing before opening it. A count that failed to load, or a
 * calendar with no recorded upcoming date, shows no badge: absence is never drawn as zero.
 */
export function useWorkspaceBadges(): Partial<Record<WorkspaceView, WorkspaceBadge>> {
  const lastFindingsSeenAt = useAppStore((state) => state.lastFindingsSeenAt);
  const findingsRefreshRequests = useAppStore((state) => state.findingsRefreshRequests);
  const portalCount = useAppStore((state) => state.portals.length);
  const failedTasks = useAppStore((state) => state.portalRuntimes.reduce((sum, runtime) => sum + runtime.failedTasks, 0));
  const [newFindings, setNewFindings] = useState<number | null>(null);
  const [nextMeeting, setNextMeeting] = useState<{ days: number; body: string } | null>(null);
  const findingsGate = useLatestRequestGate();
  const meetingsGate = useLatestRequestGate();

  useEffect(() => {
    const token = findingsGate.begin();
    setNewFindings(null);
    invokeCommand('count_findings', { filter: { discoveredAfter: lastFindingsSeenAt } })
      .then((count) => { if (findingsGate.isCurrent(token)) setNewFindings(count); })
      .catch(() => { if (findingsGate.isCurrent(token)) setNewFindings(null); });
    return findingsGate.invalidate;
  }, [findingsGate, findingsRefreshRequests, lastFindingsSeenAt, portalCount]);

  useEffect(() => {
    const token = meetingsGate.begin();
    setNextMeeting(null);
    invokeCommand('get_meeting_calendar', {})
      .then((calendar) => { if (meetingsGate.isCurrent(token)) setNextMeeting(nextUpcomingMeeting(calendar)); })
      .catch(() => { if (meetingsGate.isCurrent(token)) setNextMeeting(null); });
    return meetingsGate.invalidate;
  }, [meetingsGate, portalCount]);

  const badges: Partial<Record<WorkspaceView, WorkspaceBadge>> = {};
  if (newFindings) {
    badges.story = { text: newFindings > 99 ? '99+' : String(newFindings), label: `${newFindings} new finding${newFindings === 1 ? '' : 's'}`, tone: 'accent' };
  }
  if (nextMeeting) {
    const text = nextMeeting.days === 0 ? 'Today' : `${nextMeeting.days}d`;
    badges.meetings = { text, label: `Next meeting: ${nextMeeting.body}, ${nextMeeting.days === 0 ? 'today' : `in ${nextMeeting.days} day${nextMeeting.days === 1 ? '' : 's'}`}`, tone: 'accent' };
  }
  if (failedTasks > 0) {
    badges.activity = { text: String(failedTasks), label: `${failedTasks} failed task${failedTasks === 1 ? '' : 's'}`, tone: 'warning' };
  }
  return badges;
}

/** Days from the calendar's own `today` to the nearest meeting with a recorded upcoming date. */
export function nextUpcomingMeeting(calendar: MeetingCalendar): { days: number; body: string } | null {
  const today = Date.parse(`${calendar.today.slice(0, 10)}T00:00:00Z`);
  if (Number.isNaN(today)) return null;
  let best: { days: number; body: string } | null = null;
  for (const meeting of calendar.meetings) {
    if (meeting.timing !== 'UPCOMING' || !meeting.date) continue;
    const date = Date.parse(`${meeting.date.slice(0, 10)}T00:00:00Z`);
    if (Number.isNaN(date) || date < today) continue;
    const days = Math.round((date - today) / 86_400_000);
    if (!best || days < best.days) best = { days, body: meeting.body };
  }
  return best;
}
