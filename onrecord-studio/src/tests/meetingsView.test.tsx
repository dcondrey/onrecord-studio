// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { CalendarMeeting, FindingCitation, MeetingCalendar } from '../types';
const bridge = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('../ipc', () => ({ invokeCommand: bridge.invoke }));
import MeetingsView from '../components/dashboard/MeetingsView';

const cite = (quote: string, start: number | null = 0): FindingCitation => ({
  portalKey: 'city', documentId: 2, characterStart: start, characterEnd: start === null ? null : start + quote.length, quote, label: 'agenda.pdf',
});

function meeting(id: string, body: string, overrides: Partial<CalendarMeeting> = {}): CalendarMeeting {
  return {
    id, jurisdiction: 'City of San Diego', body, bodyCitation: cite(body), date: '2026-10-01', dateCitation: cite('THURSDAY, OCTOBER 1, 2026'),
    dateNote: null, timing: 'UPCOMING', documents: [{ role: 'AGENDA', portalKey: 'city', documentId: 2, label: 'agenda.pdf', sourceUrl: null }],
    commentDeadlines: [], commentInstructions: [], suggestions: [], ...overrides,
  };
}

function calendar(meetings: CalendarMeeting[]): MeetingCalendar {
  return { today: '2026-09-30', meetings, limitations: ['Bounded lookup.'] };
}

function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>((done) => { resolve = done; }); return { promise, resolve }; }
beforeEach(() => { bridge.invoke.mockReset(); });
afterEach(cleanup);

describe('meetings view', () => {
  it('discards an earlier calendar read that resolves after a newer one', async () => {
    const first = deferred<MeetingCalendar>();
    const second = deferred<MeetingCalendar>();
    bridge.invoke.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    render(<MeetingsView />);
    fireEvent.click(screen.getByRole('button', { name: 'Reading documents…' }));
    await act(async () => second.resolve(calendar([meeting('new', 'Planning Commission')])));
    await act(async () => first.resolve(calendar([meeting('old', 'Board of Supervisors')])));
    expect(screen.getByText('Planning Commission')).toBeTruthy();
    expect(screen.queryByText('Board of Supervisors')).toBeNull();
  });

  it('never shows a guessed date or deadline, and opens a cited draft', async () => {
    const undated = meeting('undated', 'Board of Education', { date: null, dateCitation: null, dateNote: 'No weekday and full date appears in the heading or file name.', timing: 'DATE_NOT_RECORDED' });
    const upcoming = meeting('council', 'City Council', {
      suggestions: [{
        id: 's1', agendaItem: 'Item 605', title: 'Riverbed Encampment Resolution Funding', itemCitation: cite('Item 605: Riverbed'),
        reasons: [{ kind: 'EVIDENCE_GAP', basis: 'STATED', reason: 'An evidence gap is open.', matched: 'Amendment', citation: cite('First Amendment to the Agreement', 120), findingId: null, gapKey: 'g', claimCheck: null }],
        comment: { text: 'Public comment: City Council\nComment deadline: not recorded in the collected documents.', bases: ['STATED'], deadline: null, instructions: [] },
      }],
    });
    bridge.invoke.mockResolvedValue(calendar([upcoming, undated]));
    render(<MeetingsView />);
    await screen.findByText('City Council');
    const undatedCard = screen.getByRole('article', { name: 'Board of Education date not recorded' });
    expect(undatedCard.textContent).toContain('Date not recorded');
    expect(undatedCard.textContent).toContain('No weekday and full date');
    expect(screen.getAllByText('not recorded').length).toBeGreaterThan(0);
    fireEvent.click(screen.getByText('Item 605: Riverbed Encampment Resolution Funding'));
    expect(screen.getByText('Evidence gap (untested)')).toBeTruthy();
    expect(screen.getByText('Basis: Stated')).toBeTruthy();
    expect(screen.getByText(/characters 120–152/)).toBeTruthy();
    expect((screen.getByRole('textbox') as HTMLTextAreaElement).value).toContain('Comment deadline: not recorded');
  });
});
