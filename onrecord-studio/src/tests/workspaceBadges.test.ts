import { describe, expect, it } from 'vitest';
import { nextUpcomingMeeting } from '../hooks/useWorkspaceBadges';
import type { CalendarMeeting, MeetingCalendar } from '../types';

const meeting = (id: string, timing: CalendarMeeting['timing'], date: string | null): CalendarMeeting => ({
  id, jurisdiction: 'CA', body: `Body ${id}`, timing, date,
  bodyCitation: { portalKey: 'p', documentId: 1, characterStart: null, characterEnd: null, quote: null, label: null },
  dateCitation: null, dateNote: null, startTime: null, documents: [], revisions: [], commentDeadlines: [], commentInstructions: [], suggestions: [],
} as CalendarMeeting);

const calendar = (meetings: CalendarMeeting[]): MeetingCalendar => ({ today: '2026-10-01', meetings, postingCounts: { agendaFilesWithBothTimes: 0, under72HoursBeforeStart: 0, afterMeetingStart: 0 }, limitations: [] });

describe('nextUpcomingMeeting', () => {
  it('counts days to the nearest recorded upcoming date and ignores past or undated meetings', () => {
    const next = nextUpcomingMeeting(calendar([
      meeting('a', 'UPCOMING', '2026-10-09'),
      meeting('b', 'PAST', '2026-10-02'),
      meeting('c', 'DATE_NOT_RECORDED', null),
      meeting('d', 'UPCOMING', '2026-10-04'),
    ]));
    expect(next).toEqual({ days: 3, body: 'Body d' });
  });

  it('reports no meeting rather than a zero-day one when nothing upcoming has a date', () => {
    expect(nextUpcomingMeeting(calendar([meeting('c', 'DATE_NOT_RECORDED', null), meeting('b', 'PAST', '2026-09-01')]))).toBeNull();
  });
});
